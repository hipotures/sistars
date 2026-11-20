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
  req: [[number, number], [number, number]];
  reqTxt: string;
  desc: string;
}

export interface EngineMeta {
  worldW: number;
  worldH: number;
  techFields: TechFieldMeta[];
  breakthroughs: BreakthroughMeta[];
}

export interface SimConfig {
  planets: number;
  wormholePairs: number;
  crustRichness: number;
  gateCost: number;
  interceptRad: number;
  scannerRange: number;
  salvageRate: number;
  indWeight: number;
  expWeight: number;
  techWeight: number;
  aggroBias: number;
  sprawlPenalty: number;
  colShipCost: number;
  bomberPen: number;
  facCostRes: number;
  facCostGerm: number;
  fleetSpeed: number;
  growth: number;
  opLimit: number;
}

export interface Doctrine {
  name: string;
  color: string;
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

export interface EmpireStats {
  planets: number;
  factories: number;
  res: number;
  gates: number;
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

export interface SimSnapshot {
  year: number;
  ticks: number;
  mapSeed: number;
  dnaGeneration: number;
  stats: EmpireStats[];
  techLv: number[][];
  perks: number[];
  doctrines: Doctrine[];
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

export interface AutoBalanceResult {
  indWeight: number;
  expWeight: number;
  techWeight: number;
  tournament: TournamentResult;
}
