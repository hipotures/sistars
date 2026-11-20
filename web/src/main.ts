import './style.css';
import initWasm, { WasmEngine } from './wasm/sistars_engine';
import { GalaxyRenderer, type HitTarget } from './renderer';
import { UiController } from './ui';
import type {
  CoEvolutionResult,
  DebrisFieldView,
  EngineMeta,
  FleetView,
  HudSnapshot,
  PlanetView,
  RenderFrame,
  ReportsSnapshot,
  SimConfig,
  TechCatalogView,
  TournamentResult,
} from './types';

const MAX_HISTORY_SNAPSHOTS = 600;
const HISTORY_SAMPLE_EVERY_TICKS = 2;

function randomSeed(): number {
  return Math.floor(Math.random() * 900000 + 100000);
}

async function copyTextToClipboard(text: string): Promise<boolean> {
  if (navigator.clipboard && window.isSecureContext) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      // Fall through to legacy execCommand fallback
    }
  }
  const ta = document.createElement('textarea');
  ta.value = text;
  ta.setAttribute('readonly', '');
  ta.style.position = 'fixed';
  ta.style.left = '-9999px';
  ta.style.top = '0';
  document.body.appendChild(ta);
  ta.focus();
  ta.select();
  let ok = false;
  try {
    ok = document.execCommand('copy');
  } catch {
    ok = false;
  }
  document.body.removeChild(ta);
  return ok;
}

function downloadJsonFile(filename: string, text: string): void {
  const blob = new Blob([text], { type: 'application/json' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}

async function bootstrap(): Promise<void> {
  const wasmExports = await initWasm();

  const engine = new WasmEngine(randomSeed());
  const meta = engine.get_metadata() as EngineMeta;
  const catalog = engine.get_tech_catalog() as TechCatalogView;
  let config = engine.get_config() as SimConfig;

  const canvas = document.getElementById('galaxyCanvas') as HTMLCanvasElement;
  const overlayCanvas = document.getElementById('overlayCanvas') as HTMLCanvasElement;
  const renderer = await GalaxyRenderer.create(canvas, overlayCanvas, meta.worldW, meta.worldH);

  const buildZeroCopyViews = (): {
    planetsBuf: Float32Array;
    fleetsBuf: Float32Array;
    debrisBuf: Float32Array;
  } => {
    const buf = wasmExports.memory.buffer;
    return {
      planetsBuf: new Float32Array(buf, engine.planets_render_ptr(), engine.planets_render_len()),
      fleetsBuf: new Float32Array(buf, engine.fleets_render_ptr(), engine.fleets_render_len()),
      debrisBuf: new Float32Array(buf, engine.debris_render_ptr(), engine.debris_render_len()),
    };
  };

  let speed = 1;
  let lastNonZeroSpeed = 1;
  let tickAccum = 0;
  let hoveredTarget: HitTarget = null;
  let lastHud = engine.take_hud_snapshot() as HudSnapshot;
  let liveViews = buildZeroCopyViews();
  renderer.setWorldSize(lastHud.worldW, lastHud.worldH);

  // Debug Performance HUD, Yearly Telemetry & History Scrubber state
  let debugPerfEnabled = false;
  let perfFrames = 0;
  let perfTicks = 0;
  let perfWasmSum = 0;
  let perfUiSum = 0;
  let perfDrawSum = 0;
  let perfWindowStart = performance.now();
  let latestPerf = { fps: 0, tps: 0, wasmMs: 0, uiMs: 0, drawMs: 0 };

  interface YearlyTelemetrySample {
    year: number;
    ticks: number;
    fps: number;
    tps: number;
    wasmMs: number;
    drawMs: number;
    fleets: number;
    battles: number;
    invasions: number;
    salvaged: number;
    gateJumps: number;
    empires: {
      planets: number;
      res: number;
      factories: number;
      gates: number;
      kills: number;
      unlockedTechs: number;
      avgTechLv: number;
    }[];
  }

  let debugTelemetryEnabled = false;
  let telemetrySamples: YearlyTelemetrySample[] = [];
  let lastTelemetryYear = -1;

  let debugHistoryEnabled = false;
  let historyBuffer: RenderFrame[] = [];
  let historyViewIndex: number | null = null;
  let lastRecordedTick = -1;

  const activeFrame = (): RenderFrame => {
    if (historyViewIndex !== null && historyBuffer[historyViewIndex]) {
      return historyBuffer[historyViewIndex];
    }
    return { hud: lastHud, ...liveViews };
  };

  const syncHistoryBar = (): void => {
    const total = historyBuffer.length;
    const isLive = historyViewIndex === null;
    const idx = isLive ? Math.max(0, total - 1) : historyViewIndex!;
    const frame = activeFrame();
    ui.updateHistoryBar(debugHistoryEnabled, idx, total, frame.hud.year, isLive);
  };

  const recordHistoryFrame = (hud: HudSnapshot, force = false): void => {
    if (!debugHistoryEnabled) return;
    if (!force && hud.ticks - lastRecordedTick < HISTORY_SAMPLE_EVERY_TICKS) return;
    lastRecordedTick = hud.ticks;
    // Copy compact typed arrays for replay buffer
    const archived: RenderFrame = {
      hud: { ...hud, newCombatFx: [], newJumpFx: [] },
      planetsBuf: new Float32Array(liveViews.planetsBuf),
      fleetsBuf: new Float32Array(liveViews.fleetsBuf),
      debrisBuf: new Float32Array(liveViews.debrisBuf),
    };
    historyBuffer.push(archived);
    if (historyBuffer.length > MAX_HISTORY_SNAPSHOTS) {
      historyBuffer.shift();
    }
  };

  const recordYearlyTelemetry = (hud: HudSnapshot, force = false): void => {
    if (!debugTelemetryEnabled) return;
    const yr = Math.floor(hud.year);
    if (!force && yr <= lastTelemetryYear) return;
    lastTelemetryYear = yr;
    telemetrySamples.push({
      year: yr,
      ticks: hud.ticks,
      fps: Number(latestPerf.fps.toFixed(1)),
      tps: Number(latestPerf.tps.toFixed(1)),
      wasmMs: Number(latestPerf.wasmMs.toFixed(2)),
      drawMs: Number(latestPerf.drawMs.toFixed(2)),
      fleets: hud.fleetCount,
      battles: hud.spaceBattlesCount,
      invasions: hud.invasionsCount,
      salvaged: hud.totalSalvaged,
      gateJumps: hud.gateJumpsCount,
      empires: hud.empires.map((emp) => {
        const avgLv = emp.techLv.reduce((a, b) => a + b, 0) / Math.max(1, emp.techLv.length);
        return {
          planets: emp.stats.planets,
          res: Math.round(emp.stats.res),
          factories: Math.round(emp.stats.factories),
          gates: emp.stats.gates,
          kills: emp.doctrine.kills,
          unlockedTechs: emp.loadout.unlockedCount,
          avgTechLv: Number(avgLv.toFixed(2)),
        };
      }),
    });
    ui.updateTelemetryInfo(telemetrySamples.length, yr);
  };

  const resetTelemetry = (): void => {
    telemetrySamples = [];
    lastTelemetryYear = -1;
    if (debugTelemetryEnabled) {
      recordYearlyTelemetry(lastHud, true);
    } else {
      ui.updateTelemetryInfo(0, null);
    }
  };

  const resetHistory = (): void => {
    historyBuffer = [];
    historyViewIndex = null;
    lastRecordedTick = -1;
    if (debugHistoryEnabled) {
      recordHistoryFrame(lastHud, true);
    }
    syncHistoryBar();
  };

  const refreshLiveState = (clearVisualFx = false, resetHist = false): void => {
    if (clearVisualFx) renderer.clearFx();
    lastHud = engine.take_hud_snapshot() as HudSnapshot;
    liveViews = buildZeroCopyViews();
    renderer.setWorldSize(lastHud.worldW, lastHud.worldH);
    renderer.ingestFx(lastHud);
    if (resetHist) {
      resetHistory();
      resetTelemetry();
    } else if (historyViewIndex === null) {
      recordHistoryFrame(lastHud);
      recordYearlyTelemetry(lastHud);
      syncHistoryBar();
    }
    ui.update(activeFrame().hud, true);
  };

  const ui = new UiController(meta, catalog, config, {
    onSpeedChange: (newSpeed) => {
      speed = newSpeed;
      if (newSpeed > 0) lastNonZeroSpeed = newSpeed;
      tickAccum = 0;
      if (newSpeed > 0 && historyViewIndex !== null) {
        historyViewIndex = null;
        ui.update(lastHud, true);
        syncHistoryBar();
      }
    },
    onStepTick: () => {
      speed = 0;
      tickAccum = 0;
      ui.setActiveSpeedButton(0);
      historyViewIndex = null;
      engine.step(1);
      lastHud = engine.take_hud_snapshot() as HudSnapshot;
      liveViews = buildZeroCopyViews();
      renderer.ingestFx(lastHud);
      recordHistoryFrame(lastHud, true);
      ui.update(lastHud, true);
      syncHistoryBar();
    },
    onReset: () => {
      ui.setSelection(null);
      engine.init(randomSeed());
      refreshLiveState(true, true);
    },
    onParamChange: (key, value) => {
      const reset = engine.update_param(key, value, randomSeed());
      config = engine.get_config() as SimConfig;
      ui.setConfig(config);
      if (reset) {
        ui.setSelection(null);
        refreshLiveState(true, true);
      } else {
        refreshLiveState(false, false);
      }
    },
    onGalaxyChange: (size, shape) => {
      const ok = engine.set_galaxy_options(size, shape, randomSeed());
      if (ok) {
        config = engine.get_config() as SimConfig;
        ui.setConfig(config);
        ui.setSelection(null);
        refreshLiveState(true, true);
      }
    },
    onSelectObject: (sel) => {
      ui.setSelection(sel);
    },
    onLaunchRaces: (races) => {
      const ok = engine.set_races(races, randomSeed());
      if (ok) {
        ui.setSelection(null);
        refreshLiveState(true, true);
      }
    },
    onDebugPerfToggle: (enabled) => {
      debugPerfEnabled = enabled;
      perfFrames = 0;
      perfTicks = 0;
      perfWasmSum = 0;
      perfUiSum = 0;
      perfDrawSum = 0;
      perfWindowStart = performance.now();
      const f = activeFrame();
      ui.updatePerfHud(
        enabled,
        renderer.backendName,
        0,
        0,
        0,
        0,
        0,
        f.hud.planetCount,
        f.hud.fleetCount,
      );
    },
    onDebugHistoryToggle: (enabled) => {
      debugHistoryEnabled = enabled;
      if (enabled) {
        historyBuffer = [];
        historyViewIndex = null;
        lastRecordedTick = -1;
        recordHistoryFrame(lastHud, true);
      } else {
        historyBuffer = [];
        historyViewIndex = null;
        ui.update(lastHud, true);
      }
      syncHistoryBar();
    },
    onDebugTelemetryToggle: (enabled) => {
      debugTelemetryEnabled = enabled;
      if (enabled) {
        telemetrySamples = [];
        lastTelemetryYear = -1;
        recordYearlyTelemetry(lastHud, true);
      }
    },
    onCopyTelemetry: () => {
      const payload = {
        meta: {
          galaxySize: config.galaxySize,
          galaxyShape: config.galaxyShape,
          mapSeed: lastHud.mapSeed,
          worldW: lastHud.worldW,
          worldH: lastHud.worldH,
          planets: lastHud.planetCount,
          empires: lastHud.empires.map((e) => ({
            id: e.id,
            name: e.name,
            prt: e.prt,
            lrtMask: e.lrtMask,
            color: e.color,
          })),
        },
        samples: telemetrySamples,
      };
      const text = JSON.stringify(payload, null, 2);
      void copyTextToClipboard(text).then((ok) => {
        if (ok) {
          ui.setStatus(`✓ Copied ${telemetrySamples.length} yearly telemetry samples to clipboard!`);
        } else {
          downloadJsonFile(`sistars-telemetry-${lastHud.mapSeed}.json`, text);
          ui.setStatus(`⬇ Clipboard blocked on HTTP — downloaded telemetry JSON (${telemetrySamples.length} yrs)!`);
        }
      });
    },
    onClearTelemetry: () => {
      resetTelemetry();
      ui.setStatus('Cleared telemetry log.');
    },
    onHistoryScrub: (index) => {
      if (historyBuffer.length === 0) return;
      const clamped = Math.max(0, Math.min(historyBuffer.length - 1, index));
      speed = 0;
      tickAccum = 0;
      ui.setActiveSpeedButton(0);
      renderer.clearFx();
      historyViewIndex = clamped;
      ui.update(activeFrame().hud, true);
      syncHistoryBar();
    },
    onHistoryStep: (delta) => {
      if (historyBuffer.length === 0) return;
      const current = historyViewIndex === null ? historyBuffer.length - 1 : historyViewIndex;
      const next = Math.max(0, Math.min(historyBuffer.length - 1, current + delta));
      speed = 0;
      tickAccum = 0;
      ui.setActiveSpeedButton(0);
      renderer.clearFx();
      historyViewIndex = next;
      ui.update(activeFrame().hud, true);
      syncHistoryBar();
    },
    onHistoryLive: () => {
      historyViewIndex = null;
      ui.update(lastHud, true);
      syncHistoryBar();
    },
    getPlanetDetail: (index) => engine.get_planet_detail(index) as PlanetView | null,
    getFleetById: (id) => engine.get_fleet_by_id(id) as FleetView | null,
    getDebrisDetail: (index) => engine.get_debris_detail(index) as DebrisFieldView | null,
    getReportsSnapshot: () => engine.get_reports_snapshot() as ReportsSnapshot,
    getPowerHistory: () => engine.get_power_history() as number[][],
    onLabAction: (action) => {
      if (action === 'copyjson') {
        const json = engine.export_state_json();
        void copyTextToClipboard(json).then((ok) => {
          if (ok) {
            ui.setStatus('✓ Copied state JSON to clipboard!');
          } else {
            downloadJsonFile(`sistars-state-${lastHud.mapSeed}.json`, json);
            ui.setStatus('⬇ Clipboard blocked on HTTP — downloaded state JSON!');
          }
        });
        return;
      }

      if (action === 'tournament') {
        const numGames = 30;
        ui.setStatus(`🏆 Running ${numGames}-map Monte Carlo tournament...`);
        setTimeout(() => {
          const res = engine.run_tournament(numGames, 1200, randomSeed()) as TournamentResult;
          refreshLiveState(true, true);
          ui.updateWinRateBar(res, lastHud);
          const summary = lastHud.empires
            .map((e, i) => `${e.name.slice(0, 4)} ${res.winRates[i] ?? 0}%`)
            .join(' | ');
          ui.setStatus(`✓ Tournament (${res.numGames} maps): ${summary}`);
        }, 25);
        return;
      }

      if (action === 'coevolve') {
        ui.setStatus('🧬 Co-evolution: training budgets, fleet classes & 6 tech fields...');
        setTimeout(() => {
          const res = engine.run_co_evolution(randomSeed(), randomSeed()) as CoEvolutionResult;
          refreshLiveState(true, true);
          ui.updateWinRateBar(res.tournament, lastHud);
          const summary = lastHud.empires
            .map((e, i) => `${e.name.slice(0, 4)} ${res.tournament.winRates[i] ?? 0}%`)
            .join(' | ');
          ui.setStatus(`✓ Generation #${res.dnaGeneration}! ${summary}`);
        }, 25);
      }
    },
  });

  // Camera zoom (wheel), pan (drag), reset (dblclick / Home / 0) & planet click
  let isDragging = false;
  let dragMoved = false;
  let dragStartX = 0;
  let dragStartY = 0;

  canvas.addEventListener(
    'wheel',
    (e) => {
      e.preventDefault();
      const factor = e.deltaY < 0 ? 1.15 : 1 / 1.15;
      renderer.zoomAt(e.clientX, e.clientY, factor);
    },
    { passive: false },
  );

  canvas.addEventListener('mousedown', (e) => {
    if (e.button !== 0) return;
    isDragging = true;
    dragMoved = false;
    dragStartX = e.clientX;
    dragStartY = e.clientY;
  });

  window.addEventListener('mousemove', (e) => {
    if (isDragging) {
      const dx = e.clientX - dragStartX;
      const dy = e.clientY - dragStartY;
      if (Math.hypot(dx, dy) > 3) dragMoved = true;
      if (dragMoved) {
        renderer.panBy(dx, dy);
        dragStartX = e.clientX;
        dragStartY = e.clientY;
        canvas.style.cursor = 'grabbing';
      }
      return;
    }
    hoveredTarget = renderer.hitTest(e.clientX, e.clientY, activeFrame());
    canvas.style.cursor = hoveredTarget !== null ? 'pointer' : 'crosshair';
  });

  window.addEventListener('mouseup', (e) => {
    if (!isDragging) return;
    isDragging = false;
    if (!dragMoved && e.target === canvas) {
      const hit = renderer.hitTest(e.clientX, e.clientY, activeFrame());
      if (!hit) {
        ui.setSelection(null);
      } else if (hit.kind === 'fleet') {
        const fl = engine.get_fleet_by_index(hit.index) as FleetView | null;
        ui.setSelection(fl ? { kind: 'fleet', id: fl.id } : null);
      } else {
        ui.setSelection(hit);
      }
    }
    canvas.style.cursor = hoveredTarget !== null ? 'pointer' : 'crosshair';
  });

  canvas.addEventListener('dblclick', () => {
    renderer.resetCamera();
  });

  window.addEventListener('keydown', (e) => {
    const tag = (e.target as HTMLElement | null)?.tagName;
    if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return;
    if (e.code === 'Home' || e.key === '0') {
      renderer.resetCamera();
      return;
    }
    if (e.code !== 'Space') return;
    e.preventDefault();
    if (speed > 0) {
      speed = 0;
      tickAccum = 0;
      ui.setActiveSpeedButton(0);
    } else {
      speed = lastNonZeroSpeed;
      tickAccum = 0;
      ui.setActiveSpeedButton(speed);
      if (historyViewIndex !== null) {
        historyViewIndex = null;
        ui.update(lastHud, true);
        syncHistoryBar();
      }
    }
  });

  ui.update(lastHud, true);

  const frame = (): void => {
    const measurePerf = debugPerfEnabled || debugTelemetryEnabled;
    let stepsDone = 0;
    let wasmMs = 0;
    let uiMs = 0;

    if (speed > 0 && historyViewIndex === null) {
      tickAccum += speed;
      const steps = Math.floor(tickAccum);
      if (steps > 0) {
        tickAccum -= steps;
        const t0 = measurePerf ? performance.now() : 0;
        engine.step(steps);
        lastHud = engine.take_hud_snapshot() as HudSnapshot;
        liveViews = buildZeroCopyViews();
        const t1 = measurePerf ? performance.now() : 0;
        if (measurePerf) wasmMs = t1 - t0;
        stepsDone = steps;
        renderer.ingestFx(lastHud);
        recordHistoryFrame(lastHud);
        recordYearlyTelemetry(lastHud);
        ui.update(lastHud, false);
        if (debugHistoryEnabled) syncHistoryBar();
        if (measurePerf) uiMs = performance.now() - t1;
      }
    }

    const cur = activeFrame();
    const sel = ui.getSelection();
    const selectedFleet =
      sel?.kind === 'fleet' ? (engine.get_fleet_by_id(sel.id) as FleetView | null) : null;
    const d0 = measurePerf ? performance.now() : 0;
    renderer.draw(cur, config.crustRichness, sel, hoveredTarget, selectedFleet);

    if (measurePerf) {
      const drawMs = performance.now() - d0;
      perfFrames += 1;
      perfTicks += stepsDone;
      perfWasmSum += wasmMs;
      perfUiSum += uiMs;
      perfDrawSum += drawMs;

      const now = performance.now();
      const elapsed = now - perfWindowStart;
      if (elapsed >= 250) {
        const sec = elapsed / 1000;
        latestPerf = {
          fps: perfFrames / sec,
          tps: perfTicks / sec,
          wasmMs: perfWasmSum / perfFrames,
          uiMs: perfUiSum / perfFrames,
          drawMs: perfDrawSum / perfFrames,
        };
        if (debugPerfEnabled) {
          ui.updatePerfHud(
            true,
            renderer.backendName,
            latestPerf.fps,
            latestPerf.tps,
            latestPerf.wasmMs,
            latestPerf.uiMs,
            latestPerf.drawMs,
            cur.hud.planetCount,
            cur.hud.fleetCount,
          );
        }
        perfFrames = 0;
        perfTicks = 0;
        perfWasmSum = 0;
        perfUiSum = 0;
        perfDrawSum = 0;
        perfWindowStart = now;
      }
    }

    requestAnimationFrame(frame);
  };

  requestAnimationFrame(frame);
}

void bootstrap();
