import './style.css';
import { GalaxyRenderer, type HitTarget } from './renderer';
import { UiController } from './ui';
import type {
  CoEvolutionResult,
  DebrisFieldView,
  EngineMeta,
  FleetView,
  HudSnapshot,
  MapSelection,
  PlanetView,
  RenderFrame,
  ReportsSnapshot,
  SimConfig,
  TechCatalogView,
  TournamentResult,
} from './types';
import type { MainToWorkerMsg, WorkerToMainMsg } from './workerProtocol';

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

interface LiveBufferBacking {
  planets: ArrayBuffer;
  fleets: ArrayBuffer;
  debris: ArrayBuffer;
}

async function bootstrap(): Promise<void> {
  const worker = new Worker(new URL('./sim.worker.ts', import.meta.url), {
    type: 'module',
  });

  // Request/response Promise map for on-demand worker queries
  let nextReqId = 1;
  const pending = new Map<number, (val: unknown) => void>();

  const postToWorker = (msg: MainToWorkerMsg, transfer?: Transferable[]): void => {
    if (transfer && transfer.length > 0) {
      worker.postMessage(msg, transfer);
    } else {
      worker.postMessage(msg);
    }
  };

  const requestWorker = <T>(buildMsg: (reqId: number) => MainToWorkerMsg): Promise<T> => {
    const reqId = nextReqId++;
    return new Promise<T>((resolve) => {
      pending.set(reqId, (val) => resolve(val as T));
      postToWorker(buildMsg(reqId));
    });
  };

  // Wait for worker WASM initialization + initial metadata & first frame
  let resolveReady!: (init: {
    meta: EngineMeta;
    catalog: TechCatalogView;
    config: SimConfig;
  }) => void;
  const readyPromise = new Promise<{
    meta: EngineMeta;
    catalog: TechCatalogView;
    config: SimConfig;
  }>((res) => {
    resolveReady = res;
  });

  let resolveFirstFrame!: (msg: Extract<WorkerToMainMsg, { type: 'frame' }>) => void;
  const firstFramePromise = new Promise<Extract<WorkerToMainMsg, { type: 'frame' }>>((res) => {
    resolveFirstFrame = res;
  });
  let gotFirstFrame = false;

  let onFrameHandler: ((msg: Extract<WorkerToMainMsg, { type: 'frame' }>) => void) | null = null;

  worker.onmessage = (event: MessageEvent<WorkerToMainMsg>): void => {
    const msg = event.data;
    switch (msg.type) {
      case 'ready':
        resolveReady({ meta: msg.meta, catalog: msg.catalog, config: msg.config });
        break;
      case 'frame':
        if (!gotFirstFrame) {
          gotFirstFrame = true;
          resolveFirstFrame(msg);
        } else if (onFrameHandler) {
          onFrameHandler(msg);
        } else {
          // Return unused buffers if UI isn't ready yet
          postToWorker(
            {
              type: 'returnBuffers',
              planets: msg.planets,
              fleets: msg.fleets,
              debris: msg.debris,
            },
            [msg.planets, msg.fleets, msg.debris],
          );
        }
        break;
      case 'planetDetail':
      case 'fleetDetail':
      case 'debrisDetail':
      case 'reportsSnapshot':
      case 'powerHistory':
      case 'exportStateJson':
      case 'tournamentResult':
      case 'coEvolutionResult': {
        const resolver = pending.get(msg.reqId);
        if (resolver) {
          pending.delete(msg.reqId);
          resolver(msg.data);
        }
        break;
      }
    }
  };

  postToWorker({ type: 'init', seed: randomSeed() });

  const [{ meta, catalog, config: initialConfig }, firstFrame] = await Promise.all([
    readyPromise,
    firstFramePromise,
  ]);
  let config = initialConfig;

  const canvas = document.getElementById('galaxyCanvas') as HTMLCanvasElement;
  const overlayCanvas = document.getElementById('overlayCanvas') as HTMLCanvasElement;
  const renderer = await GalaxyRenderer.create(canvas, overlayCanvas, meta.worldW, meta.worldH);

  let speed = 1;
  let lastNonZeroSpeed = 1;
  let hoveredTarget: HitTarget = null;
  let lastHud: HudSnapshot = firstFrame.hud;
  let liveSelectedFleet: FleetView | null = firstFrame.selectedFleet;
  let dataVersion = 1;
  let rafId: number | null = null;
  let renderDirty = true;

  const scheduleRender = (): void => {
    renderDirty = true;
    if (rafId === null) {
      rafId = requestAnimationFrame(renderFrame);
    }
  };

  window.addEventListener('resize', () => scheduleRender());
  window.visualViewport?.addEventListener('resize', () => scheduleRender());
  let liveBacking: LiveBufferBacking = {
    planets: firstFrame.planets,
    fleets: firstFrame.fleets,
    debris: firstFrame.debris,
  };
  let liveViews = {
    planetsBuf: new Float32Array(firstFrame.planets, 0, firstFrame.planetsLen),
    fleetsBuf: new Float32Array(firstFrame.fleets, 0, firstFrame.fleetsLen),
    debrisBuf: new Float32Array(firstFrame.debris, 0, firstFrame.debrisLen),
  };
  renderer.setWorldSize(lastHud.worldW, lastHud.worldH);

  // Separate Main-thread (FPS, UI, DRAW) & Worker-thread (TPS, WASM) telemetry
  let debugPerfEnabled = false;
  let perfFrames = 0;
  let perfUiSum = 0;
  let perfDrawSum = 0;
  let perfWindowStart = performance.now();
  let workerTps = 0;
  let workerWasmMs = 0;
  let latestPerf = { fps: 60, tps: 0, wasmMs: 0, uiMs: 0, drawMs: 0 };

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
      tps: Number(workerTps.toFixed(1)),
      wasmMs: Number(workerWasmMs.toFixed(2)),
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
    postToWorker({ type: 'setScrubIndex', index: null });
    if (debugHistoryEnabled) {
      recordHistoryFrame(lastHud, true);
    }
    syncHistoryBar();
  };

  const applySelection = (sel: MapSelection): void => {
    ui.setSelection(sel);
    const fleetId = sel?.kind === 'fleet' ? sel.id : null;
    postToWorker({ type: 'setSelectedFleet', fleetId });
    if (fleetId === null) {
      liveSelectedFleet = null;
      scheduleRender();
    } else {
      void requestWorker<FleetView | null>((reqId) => ({
        type: 'getFleetById',
        reqId,
        id: fleetId,
      })).then((fl) => {
        if (ui.getSelection()?.kind === 'fleet') {
          liveSelectedFleet = fl;
          scheduleRender();
        }
      });
    }
  };

  const ui = new UiController(meta, catalog, config, {
    onSpeedChange: (newSpeed) => {
      speed = newSpeed;
      if (newSpeed > 0) lastNonZeroSpeed = newSpeed;
      if (newSpeed === 0) {
        workerTps = 0;
        workerWasmMs = 0;
        if (debugPerfEnabled) {
          const f = activeFrame();
          ui.updatePerfHud(true, renderer.backendName, 0, 0, 0, 0, 0, f.hud.planetCount, f.hud.fleetCount);
        }
      }
      if (newSpeed > 0 && historyViewIndex !== null) {
        historyViewIndex = null;
        dataVersion++;
        postToWorker({ type: 'setScrubIndex', index: null });
        ui.update(lastHud, true);
        syncHistoryBar();
      }
      postToWorker({ type: 'setSpeed', speed: newSpeed });
      scheduleRender();
    },
    onStepTick: () => {
      speed = 0;
      ui.setActiveSpeedButton(0);
      historyViewIndex = null;
      postToWorker({ type: 'setScrubIndex', index: null });
      postToWorker({ type: 'step', ticks: 1 });
    },
    onReset: () => {
      applySelection(null);
      postToWorker({ type: 'newGalaxy', seed: randomSeed() });
    },
    onParamChange: (key, value) => {
      postToWorker({ type: 'updateConfig', key, value, seed: randomSeed() });
    },
    onGalaxyChange: (size, shape) => {
      applySelection(null);
      postToWorker({ type: 'newGalaxy', seed: randomSeed(), size, shape });
    },
    onSelectObject: (sel) => {
      applySelection(sel);
    },
    onLaunchRaces: (races) => {
      applySelection(null);
      postToWorker({ type: 'setRaces', races, seed: randomSeed() });
    },
    onDebugPerfToggle: (enabled) => {
      debugPerfEnabled = enabled;
      perfFrames = 0;
      perfUiSum = 0;
      perfDrawSum = 0;
      perfWindowStart = performance.now();
      const f = activeFrame();
      ui.updatePerfHud(
        enabled,
        renderer.backendName,
        latestPerf.fps,
        workerTps,
        workerWasmMs,
        0,
        latestPerf.drawMs,
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
        postToWorker({ type: 'setScrubIndex', index: null });
        recordHistoryFrame(lastHud, true);
      } else {
        historyBuffer = [];
        historyViewIndex = null;
        postToWorker({ type: 'setScrubIndex', index: null });
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
      workerTps = 0;
      workerWasmMs = 0;
      ui.setActiveSpeedButton(0);
      postToWorker({ type: 'setPaused', paused: true });
      postToWorker({ type: 'setScrubIndex', index: clamped });
      renderer.clearFx();
      historyViewIndex = clamped;
      dataVersion++;
      ui.update(activeFrame().hud, true);
      syncHistoryBar();
      scheduleRender();
    },
    onHistoryStep: (delta) => {
      if (historyBuffer.length === 0) return;
      const current = historyViewIndex === null ? historyBuffer.length - 1 : historyViewIndex;
      const next = Math.max(0, Math.min(historyBuffer.length - 1, current + delta));
      speed = 0;
      workerTps = 0;
      workerWasmMs = 0;
      ui.setActiveSpeedButton(0);
      postToWorker({ type: 'setPaused', paused: true });
      postToWorker({ type: 'setScrubIndex', index: next });
      renderer.clearFx();
      historyViewIndex = next;
      dataVersion++;
      ui.update(activeFrame().hud, true);
      syncHistoryBar();
      scheduleRender();
    },
    onHistoryLive: () => {
      historyViewIndex = null;
      dataVersion++;
      postToWorker({ type: 'setScrubIndex', index: null });
      ui.update(lastHud, true);
      syncHistoryBar();
      scheduleRender();
    },
    getPlanetDetail: (index) =>
      requestWorker<PlanetView | null>((reqId) => ({ type: 'getPlanetDetail', reqId, index })),
    getFleetById: (id) =>
      requestWorker<FleetView | null>((reqId) => ({ type: 'getFleetById', reqId, id })),
    getDebrisDetail: (index) =>
      requestWorker<DebrisFieldView | null>((reqId) => ({ type: 'getDebrisDetail', reqId, index })),
    getReportsSnapshot: () =>
      requestWorker<ReportsSnapshot>((reqId) => ({ type: 'getReportsSnapshot', reqId })),
    getPowerHistory: () =>
      requestWorker<number[][]>((reqId) => ({ type: 'getPowerHistory', reqId })),
    onLabAction: (action) => {
      if (action === 'copyjson') {
        void requestWorker<string>((reqId) => ({ type: 'exportStateJson', reqId })).then((json) => {
          void copyTextToClipboard(json).then((ok) => {
            if (ok) {
              ui.setStatus('✓ Copied state JSON to clipboard!');
            } else {
              downloadJsonFile(`sistars-state-${lastHud.mapSeed}.json`, json);
              ui.setStatus('⬇ Clipboard blocked on HTTP — downloaded state JSON!');
            }
          });
        });
        return;
      }

      if (action === 'tournament') {
        const numGames = 30;
        ui.setStatus(`🏆 Running ${numGames}-map Monte Carlo tournament...`);
        void requestWorker<TournamentResult>((reqId) => ({
          type: 'runTournament',
          reqId,
          numGames,
          ticksPerGame: 1200,
          seed: randomSeed(),
        })).then((res) => {
          ui.updateWinRateBar(res, lastHud);
          const summary = lastHud.empires
            .map((e, i) => `${e.name.slice(0, 4)} ${res.winRates[i] ?? 0}%`)
            .join(' | ');
          ui.setStatus(`✓ Tournament (${res.numGames} maps): ${summary}`);
        });
        return;
      }

      if (action === 'coevolve') {
        ui.setStatus('🧬 Co-evolution: training budgets, fleet classes & 6 tech fields...');
        void requestWorker<CoEvolutionResult>((reqId) => ({
          type: 'trainAi',
          reqId,
          evoSeed: randomSeed(),
          resetSeed: randomSeed(),
        })).then((res) => {
          ui.updateWinRateBar(res.tournament, lastHud);
          const summary = lastHud.empires
            .map((e, i) => `${e.name.slice(0, 4)} ${res.tournament.winRates[i] ?? 0}%`)
            .join(' | ');
          ui.setStatus(`✓ Generation #${res.dnaGeneration}! ${summary}`);
        });
      }
    },
  });

  // Receive ping-pong frame buffers from Web Worker and immediately recycle the previous slot
  onFrameHandler = (msg) => {
    const prev = liveBacking;
    liveBacking = {
      planets: msg.planets,
      fleets: msg.fleets,
      debris: msg.debris,
    };
    liveViews = {
      planetsBuf: new Float32Array(msg.planets, 0, msg.planetsLen),
      fleetsBuf: new Float32Array(msg.fleets, 0, msg.fleetsLen),
      debrisBuf: new Float32Array(msg.debris, 0, msg.debrisLen),
    };
    // Return previous ArrayBuffers to worker for zero-GC reuse
    postToWorker(
      {
        type: 'returnBuffers',
        planets: prev.planets,
        fleets: prev.fleets,
        debris: prev.debris,
      },
      [prev.planets, prev.fleets, prev.debris],
    );

    if (msg.config) {
      config = msg.config;
      ui.setConfig(config);
    }
    if (msg.resetVisuals) {
      renderer.clearFx();
    }

    lastHud = msg.hud;
    liveSelectedFleet = msg.selectedFleet;
    workerTps = msg.tps;
    workerWasmMs = msg.wasmMs;
    dataVersion++;

    renderer.setWorldSize(lastHud.worldW, lastHud.worldH);
    renderer.ingestFx(lastHud);

    const u0 = debugPerfEnabled ? performance.now() : 0;
    if (msg.resetHistory) {
      resetHistory();
      resetTelemetry();
      ui.update(activeFrame().hud, true);
    } else if (historyViewIndex === null) {
      recordHistoryFrame(lastHud);
      recordYearlyTelemetry(lastHud);
      ui.update(lastHud, false);
      if (debugHistoryEnabled) syncHistoryBar();
    }
    if (debugPerfEnabled) {
      perfUiSum += performance.now() - u0;
    }
    scheduleRender();
  };

  // Camera zoom (wheel + 2-finger pinch), pan (mouse drag + 1-finger drag),
  // reset (dblclick / Home / 0) & object tap/click
  const activePointers = new Map<number, { x: number; y: number }>();
  let dragMoved = false;
  let wasPinching = false;
  let primaryPointerId: number | null = null;
  let dragStartX = 0;
  let dragStartY = 0;

  canvas.addEventListener(
    'wheel',
    (e) => {
      e.preventDefault();
      const factor = e.deltaY < 0 ? 1.15 : 1 / 1.15;
      renderer.zoomAt(e.clientX, e.clientY, factor);
      scheduleRender();
    },
    { passive: false },
  );

  canvas.addEventListener('pointerdown', (e) => {
    if (e.pointerType === 'mouse' && e.button !== 0) return;
    canvas.setPointerCapture(e.pointerId);
    activePointers.set(e.pointerId, { x: e.clientX, y: e.clientY });

    if (activePointers.size === 1) {
      primaryPointerId = e.pointerId;
      dragMoved = false;
      wasPinching = false;
      dragStartX = e.clientX;
      dragStartY = e.clientY;
    } else if (activePointers.size >= 2) {
      wasPinching = true;
      dragMoved = true;
    }
  });

  window.addEventListener('pointermove', (e) => {
    if (activePointers.has(e.pointerId)) {
      if (activePointers.size === 2) {
        const entries = Array.from(activePointers.entries());
        const other = entries[0][0] === e.pointerId ? entries[1][1] : entries[0][1];
        const prev = activePointers.get(e.pointerId)!;

        const prevMidX = (prev.x + other.x) * 0.5;
        const prevMidY = (prev.y + other.y) * 0.5;
        const prevDist = Math.hypot(prev.x - other.x, prev.y - other.y);

        const nextMidX = (e.clientX + other.x) * 0.5;
        const nextMidY = (e.clientY + other.y) * 0.5;
        const nextDist = Math.hypot(e.clientX - other.x, e.clientY - other.y);

        activePointers.set(e.pointerId, { x: e.clientX, y: e.clientY });

        renderer.panBy(nextMidX - prevMidX, nextMidY - prevMidY);
        if (prevDist > 8 && nextDist > 8) {
          renderer.zoomAt(nextMidX, nextMidY, nextDist / prevDist);
        }
        scheduleRender();
        return;
      }

      if (activePointers.size === 1 && primaryPointerId === e.pointerId) {
        const dx = e.clientX - dragStartX;
        const dy = e.clientY - dragStartY;
        const threshold = e.pointerType === 'touch' ? 6 : 3;
        if (!dragMoved && Math.hypot(dx, dy) > threshold) {
          dragMoved = true;
        }
        if (dragMoved) {
          renderer.panBy(dx, dy);
          dragStartX = e.clientX;
          dragStartY = e.clientY;
          canvas.style.cursor = 'grabbing';
          scheduleRender();
        }
        activePointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
        return;
      }

      activePointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
      return;
    }

    if (e.pointerType === 'mouse') {
      const nextHit = renderer.hitTest(e.clientX, e.clientY, activeFrame());
      const changed =
        hoveredTarget?.kind !== nextHit?.kind ||
        (hoveredTarget && nextHit && hoveredTarget.index !== nextHit.index);
      hoveredTarget = nextHit;
      canvas.style.cursor = hoveredTarget !== null ? 'pointer' : 'crosshair';
      if (changed) scheduleRender();
    }
  });

  const handlePointerEnd = (e: PointerEvent): void => {
    if (!activePointers.has(e.pointerId)) return;
    activePointers.delete(e.pointerId);

    if (activePointers.size === 1) {
      const [remId, remPos] = activePointers.entries().next().value as [
        number,
        { x: number; y: number },
      ];
      primaryPointerId = remId;
      dragStartX = remPos.x;
      dragStartY = remPos.y;
      return;
    }

    if (activePointers.size === 0) {
      if (!dragMoved && !wasPinching && e.type === 'pointerup') {
        const hit = renderer.hitTest(e.clientX, e.clientY, activeFrame());
        if (!hit) {
          applySelection(null);
        } else if (hit.kind === 'fleet') {
          void requestWorker<FleetView | null>((reqId) => ({
            type: 'getFleetByIndex',
            reqId,
            index: hit.index,
          })).then((fl) => {
            applySelection(fl ? { kind: 'fleet', id: fl.id } : null);
          });
        } else {
          applySelection(hit);
        }
      }
      primaryPointerId = null;
      wasPinching = false;
      dragMoved = false;
      canvas.style.cursor = hoveredTarget !== null ? 'pointer' : 'crosshair';
    }
  };

  window.addEventListener('pointerup', handlePointerEnd);
  window.addEventListener('pointercancel', handlePointerEnd);

  canvas.addEventListener('dblclick', () => {
    renderer.resetCamera();
    scheduleRender();
  });

  window.addEventListener('keydown', (e) => {
    const tag = (e.target as HTMLElement | null)?.tagName;
    if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return;
    if (e.code === 'Home' || e.key === '0') {
      renderer.resetCamera();
      scheduleRender();
      return;
    }
    if (e.code !== 'Space') return;
    e.preventDefault();
    if (speed > 0) {
      speed = 0;
      workerTps = 0;
      workerWasmMs = 0;
      ui.setActiveSpeedButton(0);
      postToWorker({ type: 'setSpeed', speed: 0 });
      if (debugPerfEnabled) {
        const f = activeFrame();
        ui.updatePerfHud(true, renderer.backendName, 0, 0, 0, 0, 0, f.hud.planetCount, f.hud.fleetCount);
      }
    } else {
      speed = lastNonZeroSpeed;
      ui.setActiveSpeedButton(speed);
      if (historyViewIndex !== null) {
        historyViewIndex = null;
        dataVersion++;
        postToWorker({ type: 'setScrubIndex', index: null });
        ui.update(lastHud, true);
        syncHistoryBar();
      }
      postToWorker({ type: 'setSpeed', speed });
      scheduleRender();
    }
  });

  ui.update(lastHud, true);

  // Demand-driven 60 FPS render loop on Main Thread:
  // Stops completely (0% CPU & 0% GPU) when paused and idle!
  const renderFrame = (): void => {
    rafId = null;
    if (!renderDirty && !renderer.hasActiveFx) {
      return;
    }
    renderDirty = false;

    const cur = activeFrame();
    const sel = ui.getSelection();
    const selectedFleet = sel?.kind === 'fleet' && historyViewIndex === null ? liveSelectedFleet : null;

    const measurePerf = debugPerfEnabled || debugTelemetryEnabled;
    const d0 = measurePerf ? performance.now() : 0;
    renderer.draw(cur, config.crustRichness, sel, hoveredTarget, selectedFleet, dataVersion);

    if (measurePerf) {
      perfDrawSum += performance.now() - d0;
      perfFrames += 1;

      const now = performance.now();
      const elapsed = now - perfWindowStart;
      if (elapsed >= 250) {
        const sec = elapsed / 1000;
        latestPerf = {
          fps: perfFrames / sec,
          tps: workerTps,
          wasmMs: workerWasmMs,
          uiMs: perfUiSum / perfFrames,
          drawMs: perfDrawSum / perfFrames,
        };
        if (debugPerfEnabled) {
          ui.updatePerfHud(
            true,
            renderer.backendName,
            latestPerf.fps,
            workerTps,
            workerWasmMs,
            latestPerf.uiMs,
            latestPerf.drawMs,
            cur.hud.planetCount,
            cur.hud.fleetCount,
          );
        }
        perfFrames = 0;
        perfUiSum = 0;
        perfDrawSum = 0;
        perfWindowStart = now;
      }
    }

    // Keep animating only while ephemeral FX are fading out or another render was requested
    if (renderer.hasActiveFx || renderDirty) {
      rafId = requestAnimationFrame(renderFrame);
    }
  };

  scheduleRender();
}

void bootstrap();
