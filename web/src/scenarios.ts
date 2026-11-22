import type { GalaxyShape, GalaxySize, ScenarioModifiers } from './types';

export type ScenarioId =
  | 'standard'
  | 'wormhole_labyrinth'
  | 'dark_forest'
  | 'core_rush'
  | 'fallen_empire'
  | 'hyper_blitz'
  | 'graveyard'
  | 'finite_galaxy';

export const GALAXY_SIZE_ORDER: readonly GalaxySize[] = [
  'small',
  'medium',
  'large',
  'huge',
  'colossal',
  'galactic',
];

export const DEFAULT_SCENARIO_MODIFIERS: ScenarioModifiers = {
  scannerRangeMult: 1.0,
  ewStrengthMult: 1.0,
  techSpeedMult: 1.0,
  growthRateMult: 1.0,
  coreRichnessBias: 0.0,
  fallenEmpire: false,
  allowStargates: true,
  debrisTtl: 12.0,
  autoSalvage: true,
  allowAlchemy: true,
};

export interface ScenarioDefaults {
  galaxySize?: GalaxySize;
  galaxyShape?: GalaxyShape;
  wormholePairs?: number;
  crustRichness?: number;
  gateCost?: number;
  interceptRad?: number;
  salvageRate?: number;
  aggroBias?: number;
  sprawlPenalty?: number;
}

export interface ScenarioLocks {
  // Exact value locks
  galaxySize?: GalaxySize;
  galaxyShape?: GalaxyShape;
  wormholePairs?: number;
  crustRichness?: number;
  gateCost?: number;
  interceptRad?: number;
  salvageRate?: number;
  aggroBias?: number;
  sprawlPenalty?: number;
  // Minimum / range locks (smaller values disabled, larger remain open)
  minGalaxySize?: GalaxySize;
  minWormholePairs?: number;
}

export interface ScenarioDef {
  id: ScenarioId;
  name: string;
  shortTag: string;
  description: string;
  defaults: ScenarioDefaults;
  locks: ScenarioLocks;
  modifiers: ScenarioModifiers;
  highlights: string[];
}

export const SCENARIOS: readonly ScenarioDef[] = [
  {
    id: 'standard',
    name: 'Standard (Sandbox)',
    shortTag: 'Standard',
    description: 'Classic balanced 4X sandbox with zero locks and neutral simulation rules.',
    defaults: {},
    locks: {},
    modifiers: { ...DEFAULT_SCENARIO_MODIFIERS },
    highlights: ['All settings unlocked', 'Neutral modifiers'],
  },
  {
    id: 'wormhole_labyrinth',
    name: 'Wormhole Labyrinth',
    shortTag: 'Wormhole Labyrinth',
    description:
      'Stargate networks are disabled. Civilizations must fight over hundreds of natural wormholes.',
    defaults: {
      galaxySize: 'large',
      wormholePairs: 250,
    },
    locks: {
      minGalaxySize: 'large',
      minWormholePairs: 100,
    },
    modifiers: {
      ...DEFAULT_SCENARIO_MODIFIERS,
      allowStargates: false,
    },
    highlights: ['No Stargates', 'Wormholes ≥ 100', 'Size ≥ Large'],
  },
  {
    id: 'dark_forest',
    name: 'Dark Forest (EW Warfare)',
    shortTag: 'Dark Forest',
    description:
      'Reduced long-range sensors and amplified stealth/jamming turn the galaxy into a tense ambush arena.',
    defaults: {
      interceptRad: 12,
    },
    locks: {
      interceptRad: 12,
    },
    modifiers: {
      ...DEFAULT_SCENARIO_MODIFIERS,
      scannerRangeMult: 0.55,
      ewStrengthMult: 1.45,
    },
    highlights: ['Scanners ×0.55', 'EW Cloak/Jam ×1.45', 'Intercept = 12'],
  },
  {
    id: 'core_rush',
    name: 'Core Rush',
    shortTag: 'Core Rush',
    description:
      'The galactic core overflows with dense minerals and fertile worlds while the outer rim is barren.',
    defaults: {
      galaxySize: 'large',
      galaxyShape: 'spiral',
      crustRichness: 600,
    },
    locks: {
      minGalaxySize: 'large',
      galaxyShape: 'spiral',
    },
    modifiers: {
      ...DEFAULT_SCENARIO_MODIFIERS,
      coreRichnessBias: 0.85,
    },
    highlights: ['Core Bias +85%', 'Spiral only', 'Size ≥ Large'],
  },
  {
    id: 'fallen_empire',
    name: 'Fallen Empire',
    shortTag: 'Fallen Empire',
    description:
      'One ancient civilization starts with 11–12 fortified worlds and +4 tech across all fields. Unite or perish.',
    defaults: {
      galaxySize: 'large',
      aggroBias: 85,
    },
    locks: {
      minGalaxySize: 'large',
    },
    modifiers: {
      ...DEFAULT_SCENARIO_MODIFIERS,
      fallenEmpire: true,
    },
    highlights: ['1 Fallen Empire (+11 worlds, +4 Tech)', 'Size ≥ Large'],
  },
  {
    id: 'hyper_blitz',
    name: 'Hyper-Blitz',
    shortTag: 'Hyper-Blitz',
    description:
      'Accelerated research, rapid population growth, and high battle salvage create explosive fast-paced wars.',
    defaults: {
      salvageRate: 85,
    },
    locks: {},
    modifiers: {
      ...DEFAULT_SCENARIO_MODIFIERS,
      techSpeedMult: 2.5,
      growthRateMult: 2.0,
    },
    highlights: ['Tech Speed ×2.5', 'Growth ×2.0', 'No locks'],
  },
  {
    id: 'graveyard',
    name: 'Graveyard of Empires (Debris Lab)',
    shortTag: 'Graveyard',
    description:
      'Auto-salvage is disabled and battle wreckage persists for 1000 years, merging nearby wrecks into massive scrap fields.',
    defaults: {
      galaxySize: 'large',
      salvageRate: 100,
    },
    locks: {},
    modifiers: {
      ...DEFAULT_SCENARIO_MODIFIERS,
      debrisTtl: 1000.0,
      autoSalvage: false,
    },
    highlights: ['Debris TTL 1000 yr', 'Auto-Salvage OFF', 'Merged Graveyards'],
  },
  {
    id: 'finite_galaxy',
    name: 'Finite Galaxy (Zero Alchemy)',
    shortTag: 'Finite Galaxy',
    description:
      'Mineral Alchemy is disabled. Planetary crusts are strictly finite — once worlds are mined dry, only battle salvage and freighter logistics sustain the war.',
    defaults: {
      galaxySize: 'large',
      crustRichness: 550,
      salvageRate: 100,
    },
    locks: {},
    modifiers: {
      ...DEFAULT_SCENARIO_MODIFIERS,
      allowAlchemy: false,
      debrisTtl: 1000.0,
      autoSalvage: false,
    },
    highlights: ['No Alchemy', 'Finite Minerals', 'Debris TTL 1000 yr', 'Auto-Salvage OFF'],
  },
];

export const VALID_SCENARIO_IDS = new Set<ScenarioId>(SCENARIOS.map((s) => s.id));

export function getScenarioById(id: ScenarioId | string | undefined): ScenarioDef {
  return SCENARIOS.find((s) => s.id === id) ?? SCENARIOS[0];
}

export function galaxySizeRank(size: GalaxySize): number {
  const idx = GALAXY_SIZE_ORDER.indexOf(size);
  return idx >= 0 ? idx : 0;
}

export function clampGalaxySizeForScenario(size: GalaxySize, scenario: ScenarioDef): GalaxySize {
  if (scenario.locks.galaxySize) return scenario.locks.galaxySize;
  if (scenario.locks.minGalaxySize) {
    return galaxySizeRank(size) < galaxySizeRank(scenario.locks.minGalaxySize)
      ? scenario.locks.minGalaxySize
      : size;
  }
  return size;
}
