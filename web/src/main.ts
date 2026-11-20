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
  let hoveredPlanet = -1;
  let lastSnap = engine.take_snapshot() as SimSnapshot;

  const refreshSnapshot = (clearVisualFx = false): void => {
    if (clearVisualFx) renderer.clearFx();
    lastSnap = engine.take_snapshot() as SimSnapshot;
    renderer.ingestFx(lastSnap);
    ui.update(lastSnap, true);
  };

  const ui = new UiController(meta, catalog, config, {
    onSpeedChange: (newSpeed) => {
      speed = newSpeed;
    },
    onReset: () => {
      ui.setSelectedPlanet(-1);
      engine.init(randomSeed());
      refreshSnapshot(true);
    },
    onParamChange: (key, value) => {
      const reset = engine.update_param(key, value, randomSeed());
      config = engine.get_config() as SimConfig;
      ui.setConfig(config);
      if (reset) {
        ui.setSelectedPlanet(-1);
        refreshSnapshot(true);
      } else {
        refreshSnapshot(false);
      }
    },
    onSelectPlanet: (index) => {
      ui.setSelectedPlanet(index);
    },
    onLaunchRaces: (races) => {
      const ok = engine.set_races(races, randomSeed());
      if (ok) {
        ui.setSelectedPlanet(-1);
        refreshSnapshot(true);
      }
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
          refreshSnapshot(true);
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
          refreshSnapshot(true);
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
    const closest = renderer.findPlanetAt(e.clientX, e.clientY, lastSnap.planets, 22);
    ui.setSelectedPlanet(closest);
  });

  canvas.addEventListener('mousemove', (e) => {
    hoveredPlanet = renderer.findPlanetAt(e.clientX, e.clientY, lastSnap.planets, 20);
    canvas.style.cursor = hoveredPlanet !== -1 ? 'pointer' : 'crosshair';
  });

  ui.update(lastSnap, true);

  const frame = (): void => {
    if (speed > 0) {
      engine.step(speed);
      lastSnap = engine.take_snapshot() as SimSnapshot;
      renderer.ingestFx(lastSnap);
      ui.update(lastSnap, false);
    }
    renderer.draw(lastSnap, config.crustRichness, ui.getSelectedPlanet(), hoveredPlanet);
    requestAnimationFrame(frame);
  };

  requestAnimationFrame(frame);
}

void bootstrap();
