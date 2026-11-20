import './style.css';
import initWasm, { WasmEngine } from './wasm/sistars_engine';
import { GalaxyRenderer } from './renderer';
import { UiController } from './ui';
import type {
  CoEvolutionResult,
  EngineMeta,
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

async function bootstrap(): Promise<void> {
  const wasmExports = await initWasm();

  const engine = new WasmEngine(randomSeed());
  const meta = engine.get_metadata() as EngineMeta;
  const catalog = engine.get_tech_catalog() as TechCatalogView;
  let config = engine.get_config() as SimConfig;

  const canvas = document.getElementById('galaxyCanvas') as HTMLCanvasElement;
  const renderer = new GalaxyRenderer(canvas, meta.worldW, meta.worldH);

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
  let hoveredPlanet = -1;
  let lastHud = engine.take_hud_snapshot() as HudSnapshot;
  let liveViews = buildZeroCopyViews();
  renderer.setWorldSize(lastHud.worldW, lastHud.worldH);

  // Debug Performance HUD & History Scrubber state
  let debugPerfEnabled = false;
  let perfFrames = 0;
  let perfTicks = 0;
  let perfWasmSum = 0;
  let perfDrawSum = 0;
  let perfWindowStart = performance.now();

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
    } else if (historyViewIndex === null) {
      recordHistoryFrame(lastHud);
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
      ui.setSelectedPlanet(-1);
      engine.init(randomSeed());
      refreshLiveState(true, true);
    },
    onParamChange: (key, value) => {
      const reset = engine.update_param(key, value, randomSeed());
      config = engine.get_config() as SimConfig;
      ui.setConfig(config);
      if (reset) {
        ui.setSelectedPlanet(-1);
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
        ui.setSelectedPlanet(-1);
        refreshLiveState(true, true);
      }
    },
    onSelectPlanet: (index) => {
      ui.setSelectedPlanet(index);
    },
    onLaunchRaces: (races) => {
      const ok = engine.set_races(races, randomSeed());
      if (ok) {
        ui.setSelectedPlanet(-1);
        refreshLiveState(true, true);
      }
    },
    onDebugPerfToggle: (enabled) => {
      debugPerfEnabled = enabled;
      perfFrames = 0;
      perfTicks = 0;
      perfWasmSum = 0;
      perfDrawSum = 0;
      perfWindowStart = performance.now();
      const f = activeFrame();
      ui.updatePerfHud(enabled, 0, 0, 0, 0, f.hud.planetCount, f.hud.fleetCount);
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
    getReportsSnapshot: () => engine.get_reports_snapshot() as ReportsSnapshot,
    onLabAction: (action) => {
      if (action === 'copyjson') {
        const json = engine.export_state_json();
        void navigator.clipboard.writeText(json);
        ui.setStatus('✓ Copied state JSON to clipboard!');
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
    hoveredPlanet = renderer.findPlanetAt(e.clientX, e.clientY, activeFrame().planetsBuf, 20);
    canvas.style.cursor = hoveredPlanet !== -1 ? 'pointer' : 'crosshair';
  });

  window.addEventListener('mouseup', (e) => {
    if (!isDragging) return;
    isDragging = false;
    if (!dragMoved && e.target === canvas) {
      const closest = renderer.findPlanetAt(e.clientX, e.clientY, activeFrame().planetsBuf, 22);
      ui.setSelectedPlanet(closest);
    }
    canvas.style.cursor = hoveredPlanet !== -1 ? 'pointer' : 'crosshair';
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
    let stepsDone = 0;
    let wasmMs = 0;

    if (speed > 0 && historyViewIndex === null) {
      tickAccum += speed;
      const steps = Math.floor(tickAccum);
      if (steps > 0) {
        tickAccum -= steps;
        const t0 = debugPerfEnabled ? performance.now() : 0;
        engine.step(steps);
        lastHud = engine.take_hud_snapshot() as HudSnapshot;
        liveViews = buildZeroCopyViews();
        if (debugPerfEnabled) wasmMs = performance.now() - t0;
        stepsDone = steps;
        renderer.ingestFx(lastHud);
        recordHistoryFrame(lastHud);
        ui.update(lastHud, false);
        if (debugHistoryEnabled) syncHistoryBar();
      }
    }

    const cur = activeFrame();
    const d0 = debugPerfEnabled ? performance.now() : 0;
    renderer.draw(cur, config.crustRichness, ui.getSelectedPlanet(), hoveredPlanet);

    if (debugPerfEnabled) {
      const drawMs = performance.now() - d0;
      perfFrames += 1;
      perfTicks += stepsDone;
      perfWasmSum += wasmMs;
      perfDrawSum += drawMs;

      const now = performance.now();
      const elapsed = now - perfWindowStart;
      if (elapsed >= 250) {
        const sec = elapsed / 1000;
        ui.updatePerfHud(
          true,
          perfFrames / sec,
          perfTicks / sec,
          perfWasmSum / perfFrames,
          perfDrawSum / perfFrames,
          cur.hud.planetCount,
          cur.hud.fleetCount,
        );
        perfFrames = 0;
        perfTicks = 0;
        perfWasmSum = 0;
        perfDrawSum = 0;
        perfWindowStart = now;
      }
    }

    requestAnimationFrame(frame);
  };

  requestAnimationFrame(frame);
}

void bootstrap();
