import type {
  CoEvolutionResult,
  DebrisFieldView,
  EngineMeta,
  FleetView,
  HudSnapshot,
  PlanetView,
  RaceDef,
  ReportsSnapshot,
  SimConfig,
  StarSystemView,
  TechCatalogView,
  TournamentResult,
} from './types';

export interface InitialSimSettings {
  ignoreRacialTraits: boolean;
  galaxySize: string;
  galaxyShape: string;
  wormholePairs: number;
  crustRichness: number;
  gateCost: number;
  interceptRad: number;
  salvageRate: number;
  aggroBias: number;
  sprawlPenalty: number;
  speed: number;
}

export type MainToWorkerMsg =
  | { type: 'init'; seed: number; initialSettings?: InitialSimSettings }
  | { type: 'setSpeed'; speed: number }
  | { type: 'setPaused'; paused: boolean }
  | { type: 'step'; ticks: number }
  | { type: 'newGalaxy'; seed: number; size?: string; shape?: string }
  | { type: 'updateConfig'; key: string; value: number; seed: number }
  | { type: 'setRaces'; races: RaceDef[]; seed: number }
  | { type: 'setSelectedFleet'; fleetId: number | null }
  | { type: 'setScrubIndex'; index: number | null }
  | {
      type: 'returnBuffers';
      planets: ArrayBuffer;
      fleets: ArrayBuffer;
      debris: ArrayBuffer;
    }
  | { type: 'getPlanetDetail'; reqId: number; index: number }
  | { type: 'getStarSystem'; reqId: number; index: number }
  | { type: 'getFleetById'; reqId: number; id: number }
  | { type: 'getFleetByIndex'; reqId: number; index: number }
  | { type: 'getDebrisDetail'; reqId: number; index: number }
  | { type: 'getReportsSnapshot'; reqId: number }
  | { type: 'getPowerHistory'; reqId: number }
  | { type: 'exportStateJson'; reqId: number }
  | { type: 'runTournament'; reqId: number; numGames: number; ticksPerGame: number; seed: number }
  | { type: 'trainAi'; reqId: number; evoSeed: number; resetSeed: number };

export type WorkerToMainMsg =
  | {
      type: 'ready';
      meta: EngineMeta;
      catalog: TechCatalogView;
      config: SimConfig;
    }
  | {
      type: 'frame';
      hud: HudSnapshot;
      config?: SimConfig;
      selectedFleet: FleetView | null;
      planets: ArrayBuffer;
      planetsLen: number;
      fleets: ArrayBuffer;
      fleetsLen: number;
      debris: ArrayBuffer;
      debrisLen: number;
      tps: number;
      wasmMs: number;
      stepsDone: number;
      resetVisuals: boolean;
      resetHistory: boolean;
    }
  | { type: 'planetDetail'; reqId: number; data: PlanetView | null }
  | { type: 'starSystem'; reqId: number; data: StarSystemView | null }
  | { type: 'fleetDetail'; reqId: number; data: FleetView | null }
  | { type: 'debrisDetail'; reqId: number; data: DebrisFieldView | null }
  | { type: 'reportsSnapshot'; reqId: number; data: ReportsSnapshot }
  | { type: 'powerHistory'; reqId: number; data: number[][] }
  | { type: 'exportStateJson'; reqId: number; data: string }
  | { type: 'tournamentResult'; reqId: number; data: TournamentResult }
  | { type: 'coEvolutionResult'; reqId: number; data: CoEvolutionResult };
