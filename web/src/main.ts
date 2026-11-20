import './style.css';
import initWasm, { WasmEngine } from './wasm/sistars_engine';
import { GalaxyRenderer } from './renderer';
import { UiController } from './ui';
import type {
  CoEvolutionResult,
  EngineMeta,
  SimConfig,
  SimSnapshot,
  TechCatalogView,
  TournamentResult,
} from './types';

const MAX_HISTORY_SNAPSHOTS = 600;
const HISTORY_SAMPLE_EVERY_TICKS = 2;

function randomSeed(): number {
  return Math.floor(Math.random() * 900000 + 100000);
}

async function bootstrap(): Promise<void> {
  await initWasm();

  const engine = new WasmEngine(randomSeed());
  const meta = engine.get_metadata() as EngineMeta;
  const catalog = engine.get_tech_catalog() as TechCatalogView;
  let config = engine.get_config() as SimConfig;

  const canvas = document.getElementById('galaxyCanvas') as HTMLCanvasElement;
  const renderer = new GalaxyRenderer(canvas, meta.worldW, meta.worldH);

  let speed = 1;
  let tickAccum = 0;
  let hoveredPlanet = -1;
  let lastSnap = engine.take_snapshot() as SimSnapshot;

  // Debug History Scrubber state
  let debugHistoryEnabled = false;
  let historyBuffer: SimSnapshot[] = [];
  let historyViewIndex: number | null = null; // null = Live
  let lastRecordedTick = -1;

  const activeSnap = (): SimSnapshot => {
    if (historyViewIndex !== null && historyBuffer[historyViewIndex]) {
      return historyBuffer[historyViewIndex];
    }
    return lastSnap;
  };

  const syncHistoryBar = (): void => {
    const total = historyBuffer.length;
    const isLive = historyViewIndex === null;
    const idx = isLive ? Math.max(0, total - 1) : historyViewIndex!;
    const snap = activeSnap();
    ui.updateHistoryBar(debugHistoryEnabled, idx, total, snap.year, isLive);
  };

  const recordHistorySnapshot = (snap: SimSnapshot, force = false): void => {
    if (!debugHistoryEnabled) return;
    if (!force && snap.ticks - lastRecordedTick < HISTORY_SAMPLE_EVERY_TICKS) return;
    lastRecordedTick = snap.ticks;
    // Strip transient FX so replaying snapshots stays clean
    const archived: SimSnapshot = { ...snap, newCombatFx: [], newJumpFx: [] };
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
      recordHistorySnapshot(lastSnap, true);
    }
    syncHistoryBar();
  };

  const refreshSnapshot = (clearVisualFx = false, resetHist = false): void => {
    if (clearVisualFx) renderer.clearFx();
    lastSnap = engine.take_snapshot() as SimSnapshot;
    renderer.ingestFx(lastSnap);
    if (resetHist) {
      resetHistory();
    } else if (historyViewIndex === null) {
      recordHistorySnapshot(lastSnap);
      syncHistoryBar();
    }
    ui.update(activeSnap(), true);
  };

  const ui = new UiController(meta, catalog, config, {
    onSpeedChange: (newSpeed) => {
      speed = newSpeed;
      tickAccum = 0;
      if (newSpeed > 0 && historyViewIndex !== null) {
        historyViewIndex = null;
        ui.update(lastSnap, true);
        syncHistoryBar();
      }
    },
    onStepTick: () => {
      speed = 0;
      tickAccum = 0;
      ui.setActiveSpeedButton(0);
      historyViewIndex = null;
      engine.step(1);
      lastSnap = engine.take_snapshot() as SimSnapshot;
      renderer.ingestFx(lastSnap);
      recordHistorySnapshot(lastSnap, true);
      ui.update(lastSnap, true);
      syncHistoryBar();
    },
    onReset: () => {
      ui.setSelectedPlanet(-1);
      engine.init(randomSeed());
      refreshSnapshot(true, true);
    },
    onParamChange: (key, value) => {
      const reset = engine.update_param(key, value, randomSeed());
      config = engine.get_config() as SimConfig;
      ui.setConfig(config);
      if (reset) {
        ui.setSelectedPlanet(-1);
        refreshSnapshot(true, true);
      } else {
        refreshSnapshot(false, false);
      }
    },
    onSelectPlanet: (index) => {
      ui.setSelectedPlanet(index);
    },
    onLaunchRaces: (races) => {
      const ok = engine.set_races(races, randomSeed());
      if (ok) {
        ui.setSelectedPlanet(-1);
        refreshSnapshot(true, true);
      }
    },
    onDebugHistoryToggle: (enabled) => {
      debugHistoryEnabled = enabled;
      if (enabled) {
        historyBuffer = [];
        historyViewIndex = null;
        lastRecordedTick = -1;
        recordHistorySnapshot(lastSnap, true);
      } else {
        historyBuffer = [];
        historyViewIndex = null;
        ui.update(lastSnap, true);
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
      ui.update(activeSnap(), true);
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
      ui.update(activeSnap(), true);
      syncHistoryBar();
    },
    onHistoryLive: () => {
      historyViewIndex = null;
      ui.update(lastSnap, true);
      syncHistoryBar();
    },
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
          refreshSnapshot(true, true);
          ui.updateWinRateBar(res, lastSnap);
          const summary = lastSnap.empires
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
          refreshSnapshot(true, true);
          ui.updateWinRateBar(res.tournament, lastSnap);
          const summary = lastSnap.empires
            .map((e, i) => `${e.name.slice(0, 4)} ${res.tournament.winRates[i] ?? 0}%`)
            .join(' | ');
          ui.setStatus(`✓ Generation #${res.dnaGeneration}! ${summary}`);
        }, 25);
      }
    },
  });

  canvas.addEventListener('click', (e) => {
    const closest = renderer.findPlanetAt(e.clientX, e.clientY, activeSnap().planets, 22);
    ui.setSelectedPlanet(closest);
  });

  canvas.addEventListener('mousemove', (e) => {
    hoveredPlanet = renderer.findPlanetAt(e.clientX, e.clientY, activeSnap().planets, 20);
    canvas.style.cursor = hoveredPlanet !== -1 ? 'pointer' : 'crosshair';
  });

  ui.update(lastSnap, true);

  const frame = (): void => {
    if (speed > 0 && historyViewIndex === null) {
      tickAccum += speed;
      const steps = Math.floor(tickAccum);
      if (steps > 0) {
        tickAccum -= steps;
        engine.step(steps);
        lastSnap = engine.take_snapshot() as SimSnapshot;
        renderer.ingestFx(lastSnap);
        recordHistorySnapshot(lastSnap);
        ui.update(lastSnap, false);
        if (debugHistoryEnabled) syncHistoryBar();
      }
    }
    renderer.draw(activeSnap(), config.crustRichness, ui.getSelectedPlanet(), hoveredPlanet);
    requestAnimationFrame(frame);
  };

  requestAnimationFrame(frame);
}

void bootstrap();
