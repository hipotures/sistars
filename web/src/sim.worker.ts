import initWasm, { WasmEngine, type InitOutput } from './wasm/sistars_engine';
import type {
  CoEvolutionResult,
  DebrisFieldView,
  EngineMeta,
  FleetView,
  HudSnapshot,
  PlanetView,
  ReportsSnapshot,
  SimConfig,
  TechCatalogView,
  TournamentResult,
} from './types';
import type { MainToWorkerMsg, WorkerToMainMsg } from './workerProtocol';

interface BufferSlot {
  planets: ArrayBuffer;
  fleets: ArrayBuffer;
  debris: ArrayBuffer;
}

const TARGET_FRAME_MS = 16.667;

let wasmExports: InitOutput | null = null;
let engine: WasmEngine | null = null;

// Ping-pong transferable ArrayBuffer pool (2 slots -> zero per-frame GC)
const freeSlots: BufferSlot[] = [
  {
    planets: new ArrayBuffer(4096),
    fleets: new ArrayBuffer(4096),
    debris: new ArrayBuffer(1024),
  },
  {
    planets: new ArrayBuffer(4096),
    fleets: new ArrayBuffer(4096),
    debris: new ArrayBuffer(1024),
  },
];

let speed = 1;
let paused = false;
let scrubActive = false;
let tickAccum = 0;
let loopTimer: ReturnType<typeof setTimeout> | null = null;
let pendingFrameRequest = false;
let pendingResetVisuals = false;
let pendingResetHistory = false;
let pendingIncludeConfig = false;
let selectedFleetId: number | null = null;

// Worker-side performance metrics (TPS & WASM ms)
let perfTicks = 0;
let perfWasmSum = 0;
let perfSamples = 0;
let perfWindowStart = performance.now();
let reportedTps = 0;
let reportedWasmMs = 0;

function post(msg: WorkerToMainMsg, transfer?: Transferable[]): void {
  if (transfer && transfer.length > 0) {
    (self as unknown as Worker).postMessage(msg, transfer);
  } else {
    (self as unknown as Worker).postMessage(msg);
  }
}

function ensureCapacity(buf: ArrayBuffer, neededBytes: number): ArrayBuffer {
  if (buf.byteLength >= neededBytes) return buf;
  let nextCap = Math.max(4096, buf.byteLength);
  while (nextCap < neededBytes) {
    nextCap *= 2;
  }
  return new ArrayBuffer(nextCap);
}

function sendFrame(
  stepsDone: number,
  wasmMs: number,
  resetVisuals: boolean,
  resetHistory: boolean,
  includeConfig = false,
): boolean {
  if (!engine || !wasmExports) return false;
  const slot = freeSlots.pop();
  if (!slot) {
    // Backpressure: main thread hasn't returned a buffer slot yet
    pendingFrameRequest = true;
    pendingResetVisuals = pendingResetVisuals || resetVisuals;
    pendingResetHistory = pendingResetHistory || resetHistory;
    pendingIncludeConfig = pendingIncludeConfig || includeConfig;
    return false;
  }

  const t0 = performance.now();
  const hud = engine.take_hud_snapshot() as HudSnapshot;
  const mem = wasmExports.memory.buffer;

  const pLen = engine.planets_render_len();
  const fLen = engine.fleets_render_len();
  const dLen = engine.debris_render_len();

  const pBytes = pLen * 4;
  const fBytes = fLen * 4;
  const dBytes = dLen * 4;

  slot.planets = ensureCapacity(slot.planets, pBytes);
  slot.fleets = ensureCapacity(slot.fleets, fBytes);
  slot.debris = ensureCapacity(slot.debris, dBytes);

  if (pLen > 0) {
    const src = new Float32Array(mem, engine.planets_render_ptr(), pLen);
    new Float32Array(slot.planets, 0, pLen).set(src);
  }
  if (fLen > 0) {
    const src = new Float32Array(mem, engine.fleets_render_ptr(), fLen);
    new Float32Array(slot.fleets, 0, fLen).set(src);
  }
  if (dLen > 0) {
    const src = new Float32Array(mem, engine.debris_render_ptr(), dLen);
    new Float32Array(slot.debris, 0, dLen).set(src);
  }

  const selectedFleet =
    selectedFleetId !== null ? (engine.get_fleet_by_id(selectedFleetId) as FleetView | null) : null;

  const snapMs = performance.now() - t0;
  const totalWasmMs = wasmMs + snapMs;

  if (stepsDone > 0) {
    perfTicks += stepsDone;
    perfWasmSum += totalWasmMs;
    perfSamples += 1;
  }

  const now = performance.now();
  const elapsed = now - perfWindowStart;
  if (elapsed >= 250) {
    reportedTps = (perfTicks / elapsed) * 1000;
    reportedWasmMs = perfSamples > 0 ? perfWasmSum / perfSamples : 0;
    perfTicks = 0;
    perfWasmSum = 0;
    perfSamples = 0;
    perfWindowStart = now;
  }

  const msg: WorkerToMainMsg = {
    type: 'frame',
    hud,
    ...(includeConfig ? { config: engine.get_config() as SimConfig } : {}),
    selectedFleet,
    planets: slot.planets,
    planetsLen: pLen,
    fleets: slot.fleets,
    fleetsLen: fLen,
    debris: slot.debris,
    debrisLen: dLen,
    tps: speed > 0 && !paused && !scrubActive ? reportedTps : 0,
    wasmMs: speed > 0 && !paused && !scrubActive ? reportedWasmMs : 0,
    stepsDone,
    resetVisuals,
    resetHistory,
  };

  post(msg, [slot.planets, slot.fleets, slot.debris]);
  return true;
}

function scheduleLoop(delayMs = 0): void {
  if (loopTimer !== null) return;
  if (!engine || speed <= 0 || paused || scrubActive) return;
  loopTimer = setTimeout(runStepCycle, delayMs);
}

function stopLoop(): void {
  if (loopTimer !== null) {
    clearTimeout(loopTimer);
    loopTimer = null;
  }
}

function runStepCycle(): void {
  loopTimer = null;
  if (!engine || speed <= 0 || paused || scrubActive) return;

  // Backpressure: don't run ahead if main thread hasn't returned a buffer slot
  if (freeSlots.length === 0) {
    pendingFrameRequest = true;
    return;
  }

  const cycleStart = performance.now();
  tickAccum += speed;
  const steps = Math.floor(tickAccum);

  if (steps > 0) {
    tickAccum -= steps;
    const t0 = performance.now();
    engine.step(steps);
    const stepMs = performance.now() - t0;
    sendFrame(steps, stepMs, false, false, false);
  }

  const spent = performance.now() - cycleStart;
  scheduleLoop(Math.max(0, TARGET_FRAME_MS - spent));
}

self.onmessage = async (event: MessageEvent<MainToWorkerMsg>): Promise<void> => {
  const msg = event.data;

  if (msg.type === 'init') {
    if (!wasmExports) {
      wasmExports = await initWasm();
    }
    engine = new WasmEngine(msg.seed);
    if (msg.initialSettings) {
      const s = msg.initialSettings;
      if (s.ignoreRacialTraits) engine.update_param('ignoreRacialTraits', 1, msg.seed);
      if (s.wormholePairs !== 2) engine.update_param('wormholePairs', s.wormholePairs, msg.seed);
      if (s.crustRichness !== 800) engine.update_param('crustRichness', s.crustRichness, msg.seed);
      if (s.gateCost !== 45) engine.update_param('gateCost', s.gateCost, msg.seed);
      if (s.interceptRad !== 22) engine.update_param('interceptRad', s.interceptRad, msg.seed);
      if (s.salvageRate !== 65) engine.update_param('salvageRate', s.salvageRate, msg.seed);
      if (s.aggroBias !== 65) engine.update_param('aggroBias', s.aggroBias, msg.seed);
      if (s.sprawlPenalty !== 22) engine.update_param('sprawlPenalty', s.sprawlPenalty, msg.seed);
      if (s.galaxySize !== 'small' || s.galaxyShape !== 'spiral') {
        engine.set_galaxy_options(s.galaxySize, s.galaxyShape, msg.seed);
      }
      speed = s.speed;
      paused = speed <= 0;
    }
    const meta = engine.get_metadata() as EngineMeta;
    const catalog = engine.get_tech_catalog() as TechCatalogView;
    const config = engine.get_config() as SimConfig;

    post({ type: 'ready', meta, catalog, config });
    sendFrame(0, 0, true, true, true);
    if (speed > 0 && !paused) {
      scheduleLoop(TARGET_FRAME_MS);
    }
    return;
  }

  if (!engine) return;

  switch (msg.type) {
    case 'returnBuffers': {
      freeSlots.push({
        planets: msg.planets,
        fleets: msg.fleets,
        debris: msg.debris,
      });
      if (pendingFrameRequest) {
        const rv = pendingResetVisuals;
        const rh = pendingResetHistory;
        const ic = pendingIncludeConfig;
        pendingFrameRequest = false;
        pendingResetVisuals = false;
        pendingResetHistory = false;
        pendingIncludeConfig = false;
        if (rv || rh || ic) {
          sendFrame(0, 0, rv, rh, ic);
        }
      }
      if (speed > 0 && !paused && !scrubActive) {
        scheduleLoop(0);
      }
      break;
    }

    case 'setSpeed': {
      speed = msg.speed;
      tickAccum = 0;
      if (speed <= 0) {
        reportedTps = 0;
        reportedWasmMs = 0;
        stopLoop();
      } else {
        paused = false;
        scrubActive = false;
        perfWindowStart = performance.now();
        perfTicks = 0;
        perfWasmSum = 0;
        perfSamples = 0;
        scheduleLoop(0);
      }
      break;
    }

    case 'setPaused': {
      paused = msg.paused;
      tickAccum = 0;
      if (paused) {
        reportedTps = 0;
        reportedWasmMs = 0;
        stopLoop();
      } else if (speed > 0 && !scrubActive) {
        scheduleLoop(0);
      }
      break;
    }

    case 'setScrubIndex': {
      scrubActive = msg.index !== null;
      if (scrubActive) {
        stopLoop();
      } else if (speed > 0 && !paused) {
        scheduleLoop(0);
      }
      break;
    }

    case 'step': {
      speed = 0;
      paused = true;
      scrubActive = false;
      tickAccum = 0;
      stopLoop();
      const t0 = performance.now();
      engine.step(msg.ticks);
      const stepMs = performance.now() - t0;
      sendFrame(msg.ticks, stepMs, false, false, false);
      break;
    }

    case 'newGalaxy': {
      stopLoop();
      tickAccum = 0;
      if (msg.size && msg.shape) {
        engine.set_galaxy_options(msg.size, msg.shape, msg.seed);
      } else {
        engine.init(msg.seed);
      }
      sendFrame(0, 0, true, true, true);
      if (speed > 0 && !paused && !scrubActive) {
        scheduleLoop(TARGET_FRAME_MS);
      }
      break;
    }

    case 'updateConfig': {
      const needsReset = engine.update_param(msg.key, msg.value, msg.seed);
      if (needsReset) {
        stopLoop();
        tickAccum = 0;
      }
      sendFrame(0, 0, needsReset, needsReset, true);
      if (needsReset && speed > 0 && !paused && !scrubActive) {
        scheduleLoop(TARGET_FRAME_MS);
      }
      break;
    }

    case 'setRaces': {
      stopLoop();
      tickAccum = 0;
      const ok = engine.set_races(msg.races, msg.seed);
      if (ok) {
        sendFrame(0, 0, true, true, true);
      }
      if (speed > 0 && !paused && !scrubActive) {
        scheduleLoop(TARGET_FRAME_MS);
      }
      break;
    }

    case 'setSelectedFleet': {
      selectedFleetId = msg.fleetId;
      break;
    }

    case 'getPlanetDetail': {
      const data = engine.get_planet_detail(msg.index) as PlanetView | null;
      post({ type: 'planetDetail', reqId: msg.reqId, data });
      break;
    }

    case 'getFleetById': {
      const data = engine.get_fleet_by_id(msg.id) as FleetView | null;
      post({ type: 'fleetDetail', reqId: msg.reqId, data });
      break;
    }

    case 'getFleetByIndex': {
      const data = engine.get_fleet_by_index(msg.index) as FleetView | null;
      post({ type: 'fleetDetail', reqId: msg.reqId, data });
      break;
    }

    case 'getDebrisDetail': {
      const data = engine.get_debris_detail(msg.index) as DebrisFieldView | null;
      post({ type: 'debrisDetail', reqId: msg.reqId, data });
      break;
    }

    case 'getReportsSnapshot': {
      const data = engine.get_reports_snapshot() as ReportsSnapshot;
      post({ type: 'reportsSnapshot', reqId: msg.reqId, data });
      break;
    }

    case 'getPowerHistory': {
      const data = engine.get_power_history() as number[][];
      post({ type: 'powerHistory', reqId: msg.reqId, data });
      break;
    }

    case 'exportStateJson': {
      const data = engine.export_state_json();
      post({ type: 'exportStateJson', reqId: msg.reqId, data });
      break;
    }

    case 'runTournament': {
      stopLoop();
      const data = engine.run_tournament(msg.numGames, msg.ticksPerGame, msg.seed) as TournamentResult;
      sendFrame(0, 0, true, true, true);
      post({ type: 'tournamentResult', reqId: msg.reqId, data });
      if (speed > 0 && !paused && !scrubActive) {
        scheduleLoop(TARGET_FRAME_MS);
      }
      break;
    }

    case 'trainAi': {
      stopLoop();
      const data = engine.run_co_evolution(msg.evoSeed, msg.resetSeed) as CoEvolutionResult;
      sendFrame(0, 0, true, true, true);
      post({ type: 'coEvolutionResult', reqId: msg.reqId, data });
      if (speed > 0 && !paused && !scrubActive) {
        scheduleLoop(TARGET_FRAME_MS);
      }
      break;
    }
  }
};
