import type { EquilibriumMode, GalaxyShape, GalaxySize, RangeContourMode } from './types';

const SETTINGS_STORAGE_KEY = 'sistars.settings.v1';

const VALID_GALAXY_SIZES = new Set<GalaxySize>([
  'small',
  'medium',
  'large',
  'huge',
  'colossal',
  'galactic',
]);
const VALID_GALAXY_SHAPES = new Set<GalaxyShape>([
  'spiral',
  'random',
  'ring',
  'clusters',
  'elliptical',
]);
const VALID_EQ_MODES = new Set<EquilibriumMode>(['gap', 'none']);
const VALID_CONTOUR_MODES = new Set<RangeContourMode>(['hidden', 'outer', 'subtle', 'full']);
const VALID_SPEEDS = new Set<number>([0, 0.1, 0.25, 0.5, 1, 5, 20]);

export interface PersistedSettings {
  // Map & Scanner Field
  equilibriumMode: EquilibriumMode;
  contourMode: RangeContourMode;
  contourOpacity: number; // 0..80 (percentage)

  // World Rules & Galaxy Generator
  ignoreRacialTraits: boolean;
  galaxySize: GalaxySize;
  galaxyShape: GalaxyShape;
  wormholePairs: number;
  crustRichness: number;
  gateCost: number;
  interceptRad: number;
  salvageRate: number;
  aggroBias: number;
  sprawlPenalty: number;

  // Debug & UI
  showEventLog: boolean;
  debugPerf: boolean;
  debugHistory: boolean;
  debugTelemetry: boolean;
  speed: number;
}

export const DEFAULT_SETTINGS: PersistedSettings = {
  equilibriumMode: 'gap',
  contourMode: 'subtle',
  contourOpacity: 22,

  ignoreRacialTraits: false,
  galaxySize: 'small',
  galaxyShape: 'spiral',
  wormholePairs: 2,
  crustRichness: 800,
  gateCost: 45,
  interceptRad: 22,
  salvageRate: 65,
  aggroBias: 65,
  sprawlPenalty: 22,

  showEventLog: true,
  debugPerf: false,
  debugHistory: false,
  debugTelemetry: false,
  speed: 1,
};

function clampNum(val: unknown, fallback: number, min: number, max: number): number {
  if (typeof val !== 'number' || !Number.isFinite(val)) return fallback;
  return Math.max(min, Math.min(max, val));
}

export function loadPersistedSettings(): PersistedSettings {
  try {
    const raw = localStorage.getItem(SETTINGS_STORAGE_KEY);
    if (!raw) return { ...DEFAULT_SETTINGS };
    const parsed = JSON.parse(raw) as Partial<PersistedSettings>;
    if (!parsed || typeof parsed !== 'object') return { ...DEFAULT_SETTINGS };

    const eq = VALID_EQ_MODES.has(parsed.equilibriumMode as EquilibriumMode)
      ? (parsed.equilibriumMode as EquilibriumMode)
      : DEFAULT_SETTINGS.equilibriumMode;
    const cm = VALID_CONTOUR_MODES.has(parsed.contourMode as RangeContourMode)
      ? (parsed.contourMode as RangeContourMode)
      : DEFAULT_SETTINGS.contourMode;
    const gs = VALID_GALAXY_SIZES.has(parsed.galaxySize as GalaxySize)
      ? (parsed.galaxySize as GalaxySize)
      : DEFAULT_SETTINGS.galaxySize;
    const gsh = VALID_GALAXY_SHAPES.has(parsed.galaxyShape as GalaxyShape)
      ? (parsed.galaxyShape as GalaxyShape)
      : DEFAULT_SETTINGS.galaxyShape;
    const spd =
      typeof parsed.speed === 'number' && VALID_SPEEDS.has(parsed.speed)
        ? parsed.speed
        : DEFAULT_SETTINGS.speed;

    return {
      equilibriumMode: eq,
      contourMode: cm,
      contourOpacity: Math.round(clampNum(parsed.contourOpacity, DEFAULT_SETTINGS.contourOpacity, 0, 80)),

      ignoreRacialTraits:
        typeof parsed.ignoreRacialTraits === 'boolean'
          ? parsed.ignoreRacialTraits
          : DEFAULT_SETTINGS.ignoreRacialTraits,
      galaxySize: gs,
      galaxyShape: gsh,
      wormholePairs: Math.round(clampNum(parsed.wormholePairs, DEFAULT_SETTINGS.wormholePairs, 0, 5)),
      crustRichness: Math.round(clampNum(parsed.crustRichness, DEFAULT_SETTINGS.crustRichness, 300, 2500)),
      gateCost: Math.round(clampNum(parsed.gateCost, DEFAULT_SETTINGS.gateCost, 15, 120)),
      interceptRad: Math.round(clampNum(parsed.interceptRad, DEFAULT_SETTINGS.interceptRad, 8, 50)),
      salvageRate: Math.round(clampNum(parsed.salvageRate, DEFAULT_SETTINGS.salvageRate, 0, 100)),
      aggroBias: Math.round(clampNum(parsed.aggroBias, DEFAULT_SETTINGS.aggroBias, 0, 100)),
      sprawlPenalty: Math.round(clampNum(parsed.sprawlPenalty, DEFAULT_SETTINGS.sprawlPenalty, 0, 50)),

      showEventLog:
        typeof parsed.showEventLog === 'boolean' ? parsed.showEventLog : DEFAULT_SETTINGS.showEventLog,
      debugPerf: typeof parsed.debugPerf === 'boolean' ? parsed.debugPerf : DEFAULT_SETTINGS.debugPerf,
      debugHistory:
        typeof parsed.debugHistory === 'boolean' ? parsed.debugHistory : DEFAULT_SETTINGS.debugHistory,
      debugTelemetry:
        typeof parsed.debugTelemetry === 'boolean'
          ? parsed.debugTelemetry
          : DEFAULT_SETTINGS.debugTelemetry,
      speed: spd,
    };
  } catch {
    return { ...DEFAULT_SETTINGS };
  }
}

export function savePersistedSettings(settings: PersistedSettings): void {
  try {
    localStorage.setItem(SETTINGS_STORAGE_KEY, JSON.stringify(settings));
  } catch {
    // Ignore storage quota / private browsing errors
  }
}
