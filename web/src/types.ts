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
  bomberPen: number;
  facCostRes: number;
  facCostGerm: number;
  fleetSpeed: number;
  growth: number;
  opLimit: number;
  ignoreRacialTraits: boolean;
}

export interface EmpireStats {
  planets: number;
  factories: number;
  res: number;
  gates: number;
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
}

export interface WormholeView {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
}

export interface DebrisFieldView {
  x: number;
  y: number;
  iron: number;
  bora: number;
  germ: number;
  ttl: number;
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
}

export interface ReportsSnapshot {
  planets: PlanetView[];
  fleets: FleetView[];
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
