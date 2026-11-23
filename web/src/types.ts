export type FleetClass = 'col' | 'cruiser' | 'battleship' | 'bomber' | 'freight';

export interface TechFieldMeta {
  index: number;
  key: string;
  name: string;
  desc: string;
  color: string;
}

export interface BreakthroughMeta {
  id: string;
  bit: number;
  icon: string;
  name: string;
  reqTxt: string;
  desc: string;
}

export interface EngineMeta {
  worldW: number;
  worldH: number;
  maxTechLevel: number;
  techFields: TechFieldMeta[];
  breakthroughs: BreakthroughMeta[];
}

export interface TechCost {
  ironium: number;
  boranium: number;
  germanium: number;
  resources: number;
}

export interface CatalogEntry {
  id: number;
  name: string;
  category: string;
  categoryLabel: string;
  origin: string;
  cost: TechCost;
  reqLevels: number[];
  prtsRequired: string[];
  prtsDenied: string[];
  lrtsRequired: number;
  lrtsDenied: number;
  acquirable: boolean;
  statsSummary: string;
}

export interface PrtMeta {
  code: string;
  shortName: string;
  name: string;
  desc: string;
}

export interface LrtMeta {
  bit: number;
  code: string;
  name: string;
  desc: string;
}

export interface Doctrine {
  facShare: number;
  shipShare: number;
  techShare: number;
  colPopReq: number;
  distExp: number;
  qualExp: number;
  cruiserShare: number;
  bshipShare: number;
  bomberShare: number;
  interceptBias: number;
  techAlloc: number[];
  kills: number;
  gates: number;
  lastFit: number;
  winRate: number;
}

export interface RaceDef {
  id: string;
  name: string;
  color: string;
  prt: string;
  lrtMask: number;
  isCustom: boolean;
  doctrine: Doctrine;
}

export interface TechCatalogView {
  entries: CatalogEntry[];
  prts: PrtMeta[];
  lrts: LrtMeta[];
  classicRaces: RaceDef[];
}

export type GalaxySize = 'small' | 'medium' | 'large' | 'huge' | 'colossal' | 'galactic';
export type GalaxyShape = 'spiral' | 'random' | 'ring' | 'clusters' | 'elliptical';

export interface ScenarioModifiers {
  scannerRangeMult: number;
  ewStrengthMult: number;
  techSpeedMult: number;
  growthRateMult: number;
  coreRichnessBias: number;
  fallenEmpire: boolean;
  allowStargates: boolean;
  debrisTtl: number;
  autoSalvage: boolean;
  allowAlchemy: boolean;
}

export interface SimConfig {
  galaxySize: GalaxySize;
  galaxyShape: GalaxyShape;
  planets: number;
  wormholePairs: number;
  crustRichness: number;
  gateCost: number;
  interceptRad: number;
  scannerRange: number;
  salvageRate: number;
  aggroBias: number;
  sprawlPenalty: number;
  colShipCost: number;
  colCostMult?: number;
  bomberPen: number;
  facCostRes: number;
  facCostGerm: number;
  fleetSpeed: number;
  growth: number;
  opLimit: number;
  ignoreRacialTraits: boolean;
  scenarioModifiers: ScenarioModifiers;
}

export interface EmpireStats {
  planets: number;
  factories: number;
  res: number;
  gates: number;
  fleetUpkeep?: number;
  fleetCount?: number;
  orbitingCount?: number;
}

export interface EmpireLoadoutView {
  unlockedCount: number;
  unlockedBits: number[];
  engine: string;
  weapon: string;
  bomb: string;
  armor: string;
  shield: string;
  scanner: string;
  defense: string;
  terraform: string;
  stargate: string;
  cruiserHull: string;
  battleshipHull: string;
  bomberHull: string;
  rangeCol: number;
  rangeCruiser: number;
  rangeBattleship: number;
  rangeBomber: number;
  scannerRange: number;
  cloakArmedPct: number;
  cloakUnarmedPct: number;
  tachyonReductionPct: number;
  torpedoJammingPct: number;
  ecmJammerRange: number;
  ecmJammerStrength: number;
}

export interface EmpireSummaryView {
  id: string;
  name: string;
  color: string;
  prt: string;
  lrtMask: number;
  isCustom: boolean;
  stats: EmpireStats;
  techLv: number[];
  perks: number;
  doctrine: Doctrine;
  loadout: EmpireLoadoutView;
}

export interface PlanetView {
  id: number;
  x: number;
  y: number;
  owner: number;
  hasGate: boolean;
  hasEcmJammer: boolean;
  hab: number;
  pop: number;
  maxPop: number;
  mines: number;
  factories: number;
  actFac: number;
  maxBuildings: number;
  ironSurf: number;
  ironCrust: number;
  boraSurf: number;
  boraCrust: number;
  germSurf: number;
  germCrust: number;
  res: number;
  defense: number;
  orbitingFleets?: number;
  starbaseTier?: number;
  starbaseName?: string;
  starbaseHp?: number;
  starbaseMaxHp?: number;
}

export interface FleetView {
  id: number;
  x: number;
  y: number;
  from: number;
  target: number;
  emp: number;
  type: FleetClass;
  minType: number;
  power: number;
  maxPower: number;
  hunting: boolean;
  usedWh: boolean;
  steerX: number;
  steerY: number;
  whRoute: number;
  salvageMode?: number;
  cargoIron?: number;
  cargoBora?: number;
  cargoGerm?: number;
  orbiting?: boolean;
  upkeep?: number;
  shipCount?: number;
}

export interface WormholeView {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
}

export interface WreckEntryView {
  emp: number;
  shipType: FleetClass;
  count: number;
}

export interface DebrisFieldView {
  x: number;
  y: number;
  iron: number;
  bora: number;
  germ: number;
  ttl: number;
  wrecks?: WreckEntryView[];
}

export interface CombatFxView {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  mx: number;
  my: number;
  ttl: number;
  col: string;
}

export interface JumpFxView {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  ttl: number;
  col: string;
}

export interface SimEventView {
  year: number;
  text: string;
  color: string;
}

export interface StepProfile {
  econMs: number;
  aiMs: number;
  moveMs: number;
  ewScanMs: number;
  combatMs: number;
  totalMs: number;
}

export interface HudSnapshot {
  year: number;
  ticks: number;
  mapSeed: number;
  worldW: number;
  worldH: number;
  dnaGeneration: number;
  empires: EmpireSummaryView[];
  wormholes: WormholeView[];
  newCombatFx: CombatFxView[];
  newJumpFx: JumpFxView[];
  events: SimEventView[];
  planetCount: number;
  fleetCount: number;
  spaceBattlesCount: number;
  invasionsCount: number;
  totalSalvaged: number;
  gateJumpsCount: number;
  diplomacyMatrix?: string[][];
  stepProfile?: StepProfile;
}

export interface ReportsSnapshot {
  planets: PlanetView[];
  fleets: FleetView[];
  debrisFields: DebrisFieldView[];
}

export type MapSelection =
  | { kind: 'planet'; index: number }
  | { kind: 'fleet'; id: number }
  | { kind: 'wormhole'; index: number }
  | { kind: 'debris'; index: number }
  | null;

export type EquilibriumMode = 'gap' | 'none';
export type RangeContourMode = 'hidden' | 'outer' | 'subtle' | 'full';

export interface ScannerFieldSettings {
  equilibriumMode: EquilibriumMode;
  contourMode: RangeContourMode;
  contourOpacity: number; // 0.0 .. 0.80
  empireMask: number;     // bitmask: bit i => empire (i + 1) visible
}

export interface RenderFrame {
  hud: HudSnapshot;
  planetsBuf: Float32Array;
  fleetsBuf: Float32Array;
  debrisBuf: Float32Array;
}

export interface SimSnapshot {
  year: number;
  ticks: number;
  mapSeed: number;
  worldW: number;
  worldH: number;
  dnaGeneration: number;
  empires: EmpireSummaryView[];
  planets: PlanetView[];
  fleets: FleetView[];
  wormholes: WormholeView[];
  debrisFields: DebrisFieldView[];
  newCombatFx: CombatFxView[];
  newJumpFx: JumpFxView[];
  powerHistory: number[][];
  events: SimEventView[];
  spaceBattlesCount: number;
  invasionsCount: number;
  totalSalvaged: number;
  gateJumpsCount: number;
}

export interface TournamentResult {
  numGames: number;
  wins: number[];
  winRates: number[];
  totalRes: number[];
}

export interface CoEvolutionResult {
  dnaGeneration: number;
  tournament: TournamentResult;
}

export type BodyType =
  | 'Terrestrial'
  | 'Oceanic'
  | 'Desert'
  | 'Ice'
  | 'Volcanic'
  | 'Rock'
  | 'GasGiant'
  | 'IceGiant'
  | 'Dwarf'
  | 'AsteroidBelt'
  | 'KuiperBelt'
  | 'Comet';

export type StarKind = 'MainSequence' | 'BrownDwarf' | 'WhiteDwarf' | 'Pulsar' | 'BlackHole';

export interface StarView {
  starKind: StarKind;
  spectralClass: string;
  tempK: number;
  massSolar: number;
  radiusSolar: number;
  luminositySolar: number;
  colorRgb: [number, number, number];
  seed: number;
}

export interface PlanetLiveStateView {
  owner: number;
  ownerColor: string;
  popNorm: number;
  hasStarbase: boolean;
  hasStargate: boolean;
  hasJammer: boolean;
  mineralTint: [number, number, number];
  gravG: number;
  radMr: number;
  terraPct: number;
}

export interface MoonView {
  name: string;
  orbitRadiusPlanet: number;
  orbitAngleRad: number;
  orbitSpeed: number;
  radiusEarth: number;
  bodyType: BodyType;
  tempK: number;
  waterPct: number;
  icePct: number;
  atmoDensity: number;
  atmoColorRgb: [number, number, number];
  cloudPct: number;
  seed: number;
}

export interface SystemBodyView {
  name: string;
  orbitAu: number;
  orbitAngleRad: number;
  radiusEarth: number;
  bodyType: BodyType;
  tempK: number;
  waterPct: number;
  icePct: number;
  atmoDensity: number;
  atmoColorRgb: [number, number, number];
  cloudPct: number;
  hasRings: boolean;
  ringInner: number;
  ringOuter: number;
  ringColorRgb: [number, number, number];
  moonsCount: number;
  moons: MoonView[];
  seed: number;
  liveState: PlanetLiveStateView | null;
}

export interface StarSystemView {
  planetId: number;
  systemSeed: number;
  star: StarView;
  companionStar: StarView | null;
  primaryIndex: number;
  bodies: SystemBodyView[];
}

export type TransportMode = 'local' | 'remote';
export type ArenaMatchStatus = 'WaitingForAgents' | 'Running' | 'Finished';

export type ExecutionMode = 'realtimeAsync' | 'lockstepEpoch';

export interface McpSeatPublicInfo {
  seatName: string;
  status: 'idle' | 'bound';
  matchId: string | null;
  empireIdx: number | null;
  empireName: string | null;
}

export interface ArenaMatchSeatInfo {
  empireIdx: number;
  empireId: number;
  name: string;
  prt: string;
  prtSummary: string;
  alive: boolean;
  occupied: boolean;
  controlledBy: string;
  seatRole?: 'agent' | 'bot';
  joinState?: 'bot' | 'open' | 'occupied' | 'yours';
  joinedBy?: string | null;
}

export interface ArenaMatchSummary {
  matchId: string;
  scenarioName?: string;
  galaxySize?: string;
  totalPlanets?: number;
  numEmpires?: number;
  year: number;
  tick: number;
  epoch: number;
  status: ArenaMatchStatus;
  executionMode?: ExecutionMode;
  paused?: boolean;
  speedMult?: number;
  tickDelayMs?: number;
  winner: number | null;
  planetCount: number;
  empireCount: number;
  seats?: ArenaMatchSeatInfo[];
  agentSeats: number[];
  openAgentSeats?: number[];
  lobbyTimeoutSec?: number;
  lobbyRemainingSec?: number | null;
  lobbyDeadlineMs?: number | null;
  requiredAgentCount?: number;
  joinedAgentCount?: number;
  startReason?: string | null;
  submittedSeats: number[];
  waitingForSeats: number[];
  ticksPerEpoch: number;
  epochTimeoutMs: number;
  epochDeadlineMs: number | null;
  debugAudit: boolean;
  anomaliesCount: number;
  diplomacyMatrix?: string[][];
}

export interface CreateArenaMatchOptions {
  seed?: number;
  scenarioName?: string;
  simConfig?: SimConfig;
  scenarioModifiers?: ScenarioModifiers;
  galaxySize: string;
  galaxyShape: string;
  executionMode?: ExecutionMode;
  speedMult?: number;
  ticksPerEpoch: number;
  epochTimeoutMs: number;
  agentSeats: number[];
  openAgentSeats?: number[];
  openAgentCount?: number;
  lobbyTimeoutSec?: number;
  debugAudit: boolean;
  frameStrideTicks?: number;
  tickDelayMs?: number;
  startPaused?: boolean;
}

export interface CreateArenaMatchResponse {
  matchId: string;
  spectatorWsUrl: string;
  empireTokens: Record<string, string>;
  mcpUrls?: Record<string, string>;
  status: ArenaMatchStatus;
  executionMode?: ExecutionMode;
  paused?: boolean;
  speedMult?: number;
  epoch: number;
  ticksPerEpoch: number;
  epochTimeoutMs: number;
  agentSeats: number[];
  openAgentSeats?: number[];
  lobbyTimeoutSec?: number;
  lobbyRemainingSec?: number | null;
  lobbyDeadlineMs?: number | null;
  requiredAgentCount?: number;
  joinedAgentCount?: number;
  startReason?: string | null;
  debugAudit: boolean;
}

export interface StrategyMemory {
  phase: string;
  priorities: string[];
  rationale: string;
  diplomacyNotes: string;
  pivotTriggers: string[];
}

export type ArenaFeedKind =
  | 'StrategyUpdate'
  | 'Reasoning'
  | 'DipMessage'
  | 'DiplomacyChange'
  | 'SectorDirective'
  | 'ScoutSector'
  | 'ProbeWormhole'
  | 'BuildHub'
  | 'PolicyUpdate'
  | 'EpochStatus'
  | 'MatchFinished';

export interface ArenaFeedEvent {
  id: number;
  epoch: number;
  tick: number;
  year: number;
  empireIdx: number | null;
  empireName: string;
  empireColor: string;
  kind: ArenaFeedKind;
  summary: string;
  detail?: string;
  strategy?: StrategyMemory;
}
