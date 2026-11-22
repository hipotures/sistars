import { drawPowerChart } from './renderer';
import { SystemOrreryRenderer } from './orreryRenderer';
import {
  galaxySizeRank,
  getScenarioById,
  SCENARIOS,
  type ScenarioDef,
  type ScenarioId,
} from './scenarios';
import { ShipPreviewRenderer, type ShipViewMode } from './shipRenderer';
import type { PersistedSettings } from './settingsStorage';
import {
  exportPlanetTexturePng,
  ProceduralBodyRenderer,
  renderProceduralThumbnailTo2d,
} from './systemRenderer';
import type {
  ArenaFeedEvent,
  ArenaMatchSummary,
  CreateArenaMatchOptions,
  CreateArenaMatchResponse,
  DebrisFieldView,
  Doctrine,
  EngineMeta,
  EquilibriumMode,
  FleetClass,
  FleetView,
  HudSnapshot,
  MapSelection,
  PlanetView,
  RaceDef,
  RangeContourMode,
  ReportsSnapshot,
  SimConfig,
  StarSystemView,
  TechCatalogView,
  TournamentResult,
  TransportMode,
} from './types';

type ReportTab = 'planets' | 'fleets' | 'starbases' | 'stargates' | 'wormholes' | 'debris';
type PlanetSortKey = 'id' | 'owner' | 'gate' | 'hab' | 'pop' | 'fac' | 'iron' | 'bora' | 'germ' | 'res' | 'def';
type FleetSortKey = 'id' | 'emp' | 'label' | 'route' | 'power' | 'dist' | 'eta';
type StarbaseSortKey = 'id' | 'owner' | 'tier' | 'hp' | 'dock' | 'orbiting';
type StargateSortKey = 'id' | 'owner' | 'tech' | 'def' | 'res';
type WormholeSortKey = 'id' | 'span';
type DebrisSortKey = 'id' | 'total' | 'iron' | 'bora' | 'germ' | 'ttl';
type LabAction = 'tournament' | 'coevolve' | 'copyjson';

const ASC_DEFAULT_SORT_KEYS = new Set(['id', 'owner', 'emp', 'label', 'tech', 'route']);

export interface UiCallbacks {
  onSpeedChange: (speed: number) => void;
  onStepTick: () => void;
  onReset: () => void;
  onParamChange: (key: string, value: number) => void;
  onScenarioChange: (scenarioId: ScenarioId) => void;
  onGalaxyChange: (size: string, shape: string) => void;
  onLabAction: (action: LabAction) => void;
  onSelectObject: (sel: MapSelection) => void;
  onFocusWorldRegion: (
    minX: number,
    minY: number,
    maxX: number,
    maxY: number,
    rightInsetPx: number,
  ) => void;
  onLaunchRaces: (races: RaceDef[]) => void;
  onToggleEmpireLens: (empIndex: number) => void;
  onEquilibriumModeChange: (mode: EquilibriumMode) => void;
  onContourModeChange: (mode: RangeContourMode) => void;
  onContourOpacityChange: (opacity: number) => void;
  onShowEventLogToggle: (enabled: boolean) => void;
  onDebugPerfToggle: (enabled: boolean) => void;
  onDebugHistoryToggle: (enabled: boolean) => void;
  onDebugTelemetryToggle: (enabled: boolean) => void;
  onCopyTelemetry: () => void;
  onClearTelemetry: () => void;
  onHistoryScrub: (index: number) => void;
  onHistoryStep: (delta: number) => void;
  onHistoryLive: () => void;
  getPlanetDetail: (index: number) => Promise<PlanetView | null>;
  getStarSystem: (index: number) => Promise<StarSystemView | null>;
  getFleetById: (id: number) => Promise<FleetView | null>;
  getDebrisDetail: (index: number) => Promise<DebrisFieldView | null>;
  getPlanetFleets: (index: number) => Promise<FleetView[]>;
  getReportsSnapshot: () => Promise<ReportsSnapshot>;
  getPowerHistory: () => Promise<number[][]>;
  onTransportModeChange?: (mode: TransportMode) => void;
  onArenaRefreshMatches?: (serverUrl: string) => void;
  onArenaConnect?: (serverUrl: string, matchId: string) => void;
  onArenaCreateMatch?: (
    serverUrl: string,
    opts: CreateArenaMatchOptions,
    initialSpeed: number,
  ) => Promise<CreateArenaMatchResponse | null>;
}

const CUSTOM_RACES_STORAGE_KEY = 'sistars.customRaces.v1';

const FLEET_LABELS: Record<FleetClass, string> = {
  col: '● Colonizer',
  cruiser: '▲ Cruiser',
  battleship: '◆ Battleship',
  bomber: '★ Bomber',
  freight: '■ Freighter',
};

const DRAWER_IDS = [
  'drawerRaces',
  'drawerTech',
  'drawerStats',
  'drawerReports',
  'drawerSettings',
  'drawerArenaNew',
] as const;
const DRAWER_TAB_MAP: Partial<Record<(typeof DRAWER_IDS)[number], string>> = {
  drawerRaces: 'tabRaces',
  drawerTech: 'tabTech',
  drawerStats: 'tabStats',
  drawerReports: 'tabReports',
  drawerSettings: 'tabSettings',
};

function getEl<T extends HTMLElement>(id: string): T {
  const el = document.getElementById(id);
  if (!el) throw new Error(`Missing DOM element #${id}`);
  return el as T;
}

function shortPrt(prt: string): string {
  return prt.replace(/^PRT_/, '');
}

function formatCompact(n: number): string {
  const v = Math.round(n);
  if (v < 1000) return String(v);
  if (v < 10_000) return `${(v / 1000).toFixed(1)}k`;
  if (v < 1_000_000) return `${Math.round(v / 1000)}k`;
  if (v < 10_000_000) return `${(v / 1_000_000).toFixed(1)}M`;
  return `${Math.round(v / 1_000_000)}M`;
}

function doctrinePreset(focus: string): Doctrine {
  const base: Doctrine = {
    facShare: 0.48,
    shipShare: 0.24,
    techShare: 0.16,
    colPopReq: 0.32,
    distExp: 1.5,
    qualExp: 1.5,
    cruiserShare: 0.4,
    bshipShare: 0.35,
    bomberShare: 0.25,
    interceptBias: 0.55,
    techAlloc: [0.17, 0.17, 0.17, 0.17, 0.16, 0.16],
    kills: 0,
    gates: 0,
    lastFit: 0,
    winRate: 0,
  };
  if (focus === 'industry') {
    return { ...base, facShare: 0.54, shipShare: 0.20, techShare: 0.14, bshipShare: 0.45, cruiserShare: 0.30, bomberShare: 0.25, techAlloc: [0.22, 0.14, 0.14, 0.24, 0.10, 0.16] };
  }
  if (focus === 'expansion') {
    return { ...base, facShare: 0.46, shipShare: 0.28, techShare: 0.14, colPopReq: 0.25, distExp: 1.9, cruiserShare: 0.50, bshipShare: 0.30, bomberShare: 0.20, techAlloc: [0.14, 0.14, 0.28, 0.18, 0.14, 0.12] };
  }
  if (focus === 'warfare') {
    return { ...base, facShare: 0.42, shipShare: 0.34, techShare: 0.14, cruiserShare: 0.30, bshipShare: 0.45, bomberShare: 0.25, interceptBias: 0.70, techAlloc: [0.16, 0.28, 0.16, 0.22, 0.10, 0.08] };
  }
  if (focus === 'science') {
    return { ...base, facShare: 0.44, shipShare: 0.20, techShare: 0.26, cruiserShare: 0.30, bshipShare: 0.25, bomberShare: 0.45, techAlloc: [0.22, 0.20, 0.16, 0.14, 0.16, 0.12] };
  }
  return base;
}

function isTechUnlocked(bits: number[], techId: number): boolean {
  const word = techId >>> 5;
  const bit = techId & 31;
  return ((bits[word] ?? 0) & (1 << bit)) !== 0;
}

export class UiController {
  private readonly meta: EngineMeta;
  private readonly catalog: TechCatalogView;
  private readonly callbacks: UiCallbacks;

  private reportSort: {
    planets: { key: PlanetSortKey; asc: boolean };
    fleets: { key: FleetSortKey; asc: boolean };
    starbases: { key: StarbaseSortKey; asc: boolean };
    stargates: { key: StargateSortKey; asc: boolean };
    wormholes: { key: WormholeSortKey; asc: boolean };
    debris: { key: DebrisSortKey; asc: boolean };
  } = {
    planets: { key: 'res', asc: false },
    fleets: { key: 'id', asc: false },
    starbases: { key: 'tier', asc: false },
    stargates: { key: 'res', asc: false },
    wormholes: { key: 'id', asc: true },
    debris: { key: 'total', asc: false },
  };
  private reportPage: Record<ReportTab, number> = {
    planets: 0,
    fleets: 0,
    starbases: 0,
    stargates: 0,
    wormholes: 0,
    debris: 0,
  };
  private reportPageSize = 100;
  private reportTotalRows = 0;
  private fleetReportFilter: 'all' | 'flight' | 'orbiting' = 'all';
  private selection: MapSelection = null;
  private debrisWrecksExpanded = false;
  private planetFleetsExpanded = false;
  private lastHud: HudSnapshot | null = null;
  private lastReports: ReportsSnapshot | null = null;
  private lastConfig: SimConfig;
  private lastHudSignature = '';
  private lastEventsSignature = '';
  private lastHudUpdateMs = 0;
  private lastDrawerUpdateMs = 0;
  private lastDrawerFleetCount = -1;
  private inspectorReqSeq = 0;
  private reportsInFlight = false;
  private powerHistoryInFlight = false;
  private showEventLog = true;
  private currentSpeed = 1;
  private lastNonZeroSpeed = 1;

  private customRaces: RaceDef[] = [];
  private selectedRaceIds: Set<string>;
  private activeTechTab: 'overview' | 'browser' = 'overview';
  private activeReportTab: ReportTab = 'planets';
  private activeLensEmpires: Set<number> | null = null;
  private activeContourMode: RangeContourMode = 'subtle';
  private lastNonZeroContourOpacity = 22;

  // Procedural 3D globe (Inspector) & Star System View modal state
  private readonly inspGlobeRenderer: ProceduralBodyRenderer;
  private readonly sysPreviewRenderer: ProceduralBodyRenderer;
  private readonly sysOrreryRenderer: SystemOrreryRenderer;
  private activeModalSystem: StarSystemView | null = null;
  private activeModalSelection: 'star' | 'star2' | { bodyIdx: number; moonIdx: number | null } = {
    bodyIdx: 0,
    moonIdx: null,
  };
  private modalReqSeq = 0;

  // 3D Spaceship preview (Fleet Inspector) & Fleet View modal state
  private readonly inspShipRenderer: ShipPreviewRenderer;
  private readonly modalShipRenderer: ShipPreviewRenderer;
  private activeModalFleetId: number | null = null;
  private fleetModalReqSeq = 0;

  constructor(
    meta: EngineMeta,
    catalog: TechCatalogView,
    initialConfig: SimConfig,
    callbacks: UiCallbacks,
  ) {
    this.meta = meta;
    this.catalog = catalog;
    this.lastConfig = initialConfig;
    this.callbacks = callbacks;

    this.customRaces = this.loadCustomRaces();
    this.selectedRaceIds = new Set(this.catalog.classicRaces.map((r) => r.id));
    this.inspGlobeRenderer = new ProceduralBodyRenderer(
      getEl<HTMLCanvasElement>('inspPlanetCanvas'),
    );
    this.sysPreviewRenderer = new ProceduralBodyRenderer(
      getEl<HTMLCanvasElement>('sysPreviewCanvas'),
    );
    this.sysOrreryRenderer = new SystemOrreryRenderer(
      getEl<HTMLCanvasElement>('sysOrreryCanvas'),
      (which) => {
        this.setSystemModalTab('bodies');
        if (which === 'star' || which === 'star2') {
          this.selectModalItem(which);
        } else {
          this.selectModalItem({ bodyIdx: which, moonIdx: null });
        }
      },
    );
    this.inspShipRenderer = new ShipPreviewRenderer(
      getEl<HTMLCanvasElement>('inspShipCanvas'),
      false,
    );
    this.modalShipRenderer = new ShipPreviewRenderer(
      getEl<HTMLCanvasElement>('fleetPreviewCanvas'),
      true,
    );

    this.initStaticControls();
    this.bindEvents();
    this.bindArenaEvents();
    this.renderRaceRoster();
  }

  public setConfig(cfg: SimConfig): void {
    this.lastConfig = cfg;
    getEl<HTMLInputElement>('p_ignoreRacialTraits').checked = cfg.ignoreRacialTraits;
    getEl<HTMLSelectElement>('p_galaxySize').value = cfg.galaxySize;
    getEl<HTMLSelectElement>('p_galaxyShape').value = cfg.galaxyShape;
  }

  public applyScenarioLocks(scenario: ScenarioDef): void {
    getEl<HTMLSelectElement>('p_scenario').value = scenario.id;
    getEl('scenarioDesc').textContent = scenario.description;
    getEl('scenarioBadges').innerHTML = scenario.highlights
      .map((h) => `<span class="scenario-chip">${h}</span>`)
      .join('');

    const hudTag = getEl('hudScenarioTag');
    if (scenario.id === 'standard') {
      hudTag.style.display = 'none';
      hudTag.textContent = '';
    } else {
      hudTag.style.display = 'inline-block';
      hudTag.textContent = `⚑ ${scenario.shortTag}`;
      hudTag.title = scenario.description;
    }

    const setBadge = (id: string, text: string | null) => {
      const el = document.getElementById(`lock_${id}`);
      if (!el) return;
      if (text) {
        el.textContent = text;
        el.classList.add('visible');
      } else {
        el.textContent = '';
        el.classList.remove('visible');
      }
    };

    const locks = scenario.locks;

    // Galaxy size lock (exact or min)
    const sizeSelect = getEl<HTMLSelectElement>('p_galaxySize');
    sizeSelect.disabled = locks.galaxySize !== undefined;
    const minRank = locks.minGalaxySize ? galaxySizeRank(locks.minGalaxySize) : -1;
    Array.from(sizeSelect.options).forEach((opt) => {
      const r = galaxySizeRank(opt.value as typeof scenario.locks.galaxySize & string);
      opt.disabled = minRank >= 0 && r < minRank;
    });
    setBadge(
      'galaxySize',
      locks.galaxySize
        ? '🔒 Locked'
        : locks.minGalaxySize
          ? `🔒 ≥ ${locks.minGalaxySize[0].toUpperCase()}${locks.minGalaxySize.slice(1)}`
          : null,
    );

    // Galaxy shape lock
    const shapeSelect = getEl<HTMLSelectElement>('p_galaxyShape');
    shapeSelect.disabled = locks.galaxyShape !== undefined;
    setBadge('galaxyShape', locks.galaxyShape ? '🔒 Locked' : null);

    // Wormhole pairs lock (exact or min)
    const whSlider = getEl<HTMLInputElement>('p_wormholePairs');
    const minWh = locks.minWormholePairs ?? 0;
    whSlider.min = String(minWh);
    whSlider.disabled = locks.wormholePairs !== undefined;
    document.querySelectorAll<HTMLButtonElement>('[data-wh-preset]').forEach((btn) => {
      const val = Number(btn.dataset.whPreset ?? 0);
      btn.disabled = locks.wormholePairs !== undefined || val < minWh;
    });
    setBadge(
      'wormholePairs',
      locks.wormholePairs !== undefined
        ? '🔒 Locked'
        : locks.minWormholePairs !== undefined
          ? `🔒 ≥ ${locks.minWormholePairs}`
          : null,
    );

    // Simple numeric slider locks
    const sliderLocks: [string, number | undefined, string | null][] = [
      ['crustRichness', locks.crustRichness, null],
      [
        'gateCost',
        locks.gateCost,
        !scenario.modifiers.allowStargates ? '🔒 Stargates off' : null,
      ],
      ['interceptRad', locks.interceptRad, null],
      ['salvageRate', locks.salvageRate, null],
      ['aggroBias', locks.aggroBias, null],
      ['sprawlPenalty', locks.sprawlPenalty, null],
    ];
    for (const [key, exactVal, extraBadge] of sliderLocks) {
      const inp = document.getElementById(`p_${key}`) as HTMLInputElement | null;
      const locked = exactVal !== undefined || extraBadge !== null;
      if (inp) inp.disabled = locked;
      setBadge(key, extraBadge ?? (exactVal !== undefined ? '🔒 Locked' : null));
    }
  }

  public applyPersistedSettings(s: PersistedSettings): void {
    // Map & Scanner Field
    document.querySelectorAll<HTMLButtonElement>('[data-eq-mode]').forEach((b) => {
      b.classList.toggle('active', b.dataset.eqMode === s.equilibriumMode);
    });
    this.activeContourMode = s.contourMode;
    document.querySelectorAll<HTMLButtonElement>('[data-contour-mode]').forEach((b) => {
      b.classList.toggle('active', b.dataset.contourMode === s.contourMode);
    });
    const pct = Math.max(0, Math.min(80, Math.round(s.contourOpacity)));
    if (pct > 0) this.lastNonZeroContourOpacity = pct;
    getEl<HTMLInputElement>('p_contourOpacity').value = String(pct);
    getEl('v_contourOpacity').textContent = `${pct}%`;

    // World Rules & Galaxy Generator
    const scen = getScenarioById(s.scenarioId);
    this.applyScenarioLocks(scen);
    getEl<HTMLInputElement>('p_ignoreRacialTraits').checked = s.ignoreRacialTraits;
    getEl<HTMLSelectElement>('p_galaxySize').value = s.galaxySize;
    getEl<HTMLSelectElement>('p_galaxyShape').value = s.galaxyShape;

    const setSlider = (key: string, val: number, suffix = ''): void => {
      const inp = document.getElementById(`p_${key}`) as HTMLInputElement | null;
      const lbl = document.getElementById(`v_${key}`);
      if (inp) inp.value = String(val);
      if (lbl) lbl.textContent = `${val}${suffix}`;
    };
    setSlider('wormholePairs', s.wormholePairs);
    setSlider('crustRichness', s.crustRichness);
    setSlider('gateCost', s.gateCost);
    setSlider('colCostMult', s.colCostMult, '%');
    setSlider('interceptRad', s.interceptRad);
    setSlider('salvageRate', s.salvageRate, '%');
    setSlider('aggroBias', s.aggroBias, '%');
    setSlider('sprawlPenalty', s.sprawlPenalty, '%');

    // Debug & UI
    this.showEventLog = s.showEventLog;
    getEl<HTMLInputElement>('p_showEventLog').checked = s.showEventLog;
    getEl<HTMLInputElement>('p_debugPerf').checked = s.debugPerf;
    getEl<HTMLInputElement>('p_debugHistory').checked = s.debugHistory;
    getEl<HTMLInputElement>('p_debugTelemetry').checked = s.debugTelemetry;
    this.setActiveSpeedButton(s.speed);
  }

  public setSelection(sel: MapSelection): void {
    const sameDebris =
      sel?.kind === 'debris' &&
      this.selection?.kind === 'debris' &&
      sel.index === this.selection.index;
    const samePlanet =
      sel?.kind === 'planet' &&
      this.selection?.kind === 'planet' &&
      sel.index === this.selection.index;
    if (!sameDebris) {
      this.debrisWrecksExpanded = false;
    }
    if (!samePlanet) {
      this.planetFleetsExpanded = false;
    }
    this.selection = sel;
    this.syncReportRowHighlight();
    if (!sel) {
      getEl('planetInspector').style.display = 'none';
      getEl('inspGlobeWrap').style.display = 'none';
      getEl('inspShipWrap').style.display = 'none';
      getEl('inspDebrisHistoryWrap').style.display = 'none';
      getEl('inspPlanetFleetsWrap').style.display = 'none';
      this.inspGlobeRenderer.stop();
      this.inspShipRenderer.stop();
    } else if (this.lastHud) {
      void this.updateInspector(this.lastHud);
    }
  }

  public getSelection(): MapSelection {
    return this.selection;
  }

  public setStatus(text: string): void {
    getEl('statusMsg').textContent = text;
    getEl('statusMsgStats').textContent = text;
  }

  public setActiveSpeedButton(speed: number): void {
    this.currentSpeed = speed;
    if (speed > 0) {
      this.lastNonZeroSpeed = speed;
    }
    const paused = speed === 0;
    const pauseBtn = getEl<HTMLButtonElement>('btnPause');
    pauseBtn.textContent = paused ? '▶' : '⏸';
    pauseBtn.title = paused ? `Resume (${this.lastNonZeroSpeed}x)` : 'Pause';
    pauseBtn.classList.toggle('active', paused);

    document.querySelectorAll<HTMLButtonElement>('[data-speed]').forEach((b) => {
      if (b.id === 'btnPause') return;
      b.classList.toggle('active', !paused && Number(b.dataset.speed) === speed);
    });
    this.syncEventLogVisibility();
  }

  private syncEventLogVisibility(): void {
    const visible = this.showEventLog && this.currentSpeed <= 1;
    getEl('eventLog').style.display = visible ? 'flex' : 'none';
  }

  public updateHistoryBar(
    enabled: boolean,
    index: number,
    total: number,
    year: number,
    isLive: boolean,
  ): void {
    const bar = getEl('historyBar');
    bar.classList.toggle('visible', enabled);
    if (!enabled) return;

    const maxIdx = Math.max(0, total - 1);
    const slider = getEl<HTMLInputElement>('historySlider');
    slider.max = String(maxIdx);
    slider.value = String(Math.min(index, maxIdx));

    const badge = getEl('historyBadge');
    badge.textContent = isLive ? 'DEBUG REC' : 'REPLAY';
    badge.classList.toggle('paused', !isLive);

    getEl('historyLabel').textContent = `Yr ${year.toFixed(1)} (${total > 0 ? index + 1 : 0}/${total})`;
    getEl('btnHistLive').classList.toggle('active', isLive);
  }

  public updatePerfHud(
    enabled: boolean,
    backend: 'WebGPU' | 'Canvas2D',
    fps: number,
    tps: number,
    wasmMs: number,
    uiMs: number,
    drawMs: number,
    planets: number,
    fleets: number,
    stepProfile?: HudSnapshot['stepProfile'],
  ): void {
    const hud = getEl('perfHud');
    hud.classList.toggle('visible', enabled);
    if (!enabled) return;

    const backendEl = getEl('perfBackend');
    backendEl.textContent = backend;
    backendEl.style.color = backend === 'WebGPU' ? 'var(--accent)' : 'var(--yellow)';

    const fpsEl = getEl('perfFps');
    fpsEl.textContent = String(Math.round(fps));
    fpsEl.style.color = fps >= 50 ? 'var(--green)' : fps >= 28 ? 'var(--yellow)' : 'var(--red)';

    getEl('perfTps').textContent = String(Math.round(tps));
    const wasmEl = getEl('perfWasm');
    wasmEl.textContent = `${wasmMs.toFixed(2)}ms`;
    if (stepProfile) {
      wasmEl.title = `Step phases (ms): econ ${stepProfile.econMs.toFixed(3)} | ai ${stepProfile.aiMs.toFixed(3)} | move ${stepProfile.moveMs.toFixed(3)} | ew ${stepProfile.ewScanMs.toFixed(3)} | combat ${stepProfile.combatMs.toFixed(3)} (total ${stepProfile.totalMs.toFixed(3)})`;
    }
    getEl('perfUi').textContent = `${uiMs.toFixed(2)}ms`;
    getEl('perfDraw').textContent = `${drawMs.toFixed(2)}ms`;
    getEl('perfEnt').textContent = `${planets}p / ${fleets}f`;
  }

  public updateTelemetryInfo(count: number, lastYear: number | null): void {
    getEl('telemetryInfo').textContent =
      count > 0 && lastYear !== null
        ? `Recorded: ${count} years (last: ${lastYear})`
        : 'Recorded: 0 years';
  }

  public updateWinRateBar(res: TournamentResult, hud: HudSnapshot): void {
    getEl('tourneyLabel').textContent = `🏆 Win-Rate (${res.numGames} maps):`;
    const bar = getEl('winrateBar');
    bar.innerHTML = hud.empires
      .map((emp, idx) => {
        const w = res.winRates[idx] ?? 0;
        const label = w >= 9 ? `${emp.name.slice(0, 3)} ${w}%` : w >= 5 ? `${w}%` : '';
        return `<div class="wr-seg" style="width:${w}%;background:${emp.color};" title="${emp.name}: ${w}% (${res.wins[idx] ?? 0} wins)">${label}</div>`;
      })
      .join('');
  }

  public update(hud: HudSnapshot, forceDrawers = false): void {
    this.lastHud = hud;
    const now = performance.now();

    if (forceDrawers || now - this.lastHudUpdateMs >= 100) {
      this.lastHudUpdateMs = now;
      getEl('hudYear').textContent = hud.year.toFixed(1);
      this.updateEmpireHud(hud);
      this.renderEvents(hud);
      if (this.selection !== null) {
        void this.updateInspector(hud);
      }
      if (this.activeModalFleetId !== null) {
        void this.refreshFleetModal(hud, false);
      }
    }

    const fleetCountChanged = hud.fleetCount !== this.lastDrawerFleetCount;
    if (forceDrawers || fleetCountChanged || now - this.lastDrawerUpdateMs >= 150) {
      this.lastDrawerUpdateMs = now;
      this.lastDrawerFleetCount = hud.fleetCount;
      this.renderOpenDrawers(hud);
    }
  }

  public renderOpenDrawers(hud: HudSnapshot): void {
    this.lastHud = hud;
    if (getEl('drawerTech').classList.contains('open')) {
      if (this.activeTechTab === 'overview') this.renderTechOverview(hud);
      else this.renderTechBrowser(hud);
    }
    if (getEl('drawerStats').classList.contains('open')) {
      this.renderDNATable(hud);
      void this.renderPowerChart(hud);
    }
    if (getEl('drawerReports').classList.contains('open') && !this.reportsInFlight) {
      this.reportsInFlight = true;
      void this.callbacks
        .getReportsSnapshot()
        .then((rep) => {
          this.lastReports = rep;
          const gateCount = rep.planets.reduce((acc, p) => acc + (p.hasGate ? 1 : 0), 0);
          const baseCount = rep.planets.reduce((acc, p) => acc + ((p.starbaseTier ?? 0) > 0 ? 1 : 0), 0);
          getEl('repBtnPlanets').textContent = `Planets (${rep.planets.length})`;
          getEl('repBtnFleets').textContent = `Fleets (${rep.fleets.length})`;
          getEl('repBtnStarbases').textContent = `🛡 Bases (${baseCount})`;
          getEl('repBtnStargates').textContent = `🌀 Gates (${gateCount})`;
          getEl('repBtnWormholes').textContent = `🟣 Wormholes (${hud.wormholes.length})`;
          getEl('repBtnDebris').textContent = `✦ Debris (${rep.debrisFields.length})`;

          this.renderActiveReportTab(hud, rep);
        })
        .finally(() => {
          this.reportsInFlight = false;
        });
    }
  }

  private loadCustomRaces(): RaceDef[] {
    try {
      const raw = localStorage.getItem(CUSTOM_RACES_STORAGE_KEY);
      if (!raw) return [];
      const parsed = JSON.parse(raw);
      return Array.isArray(parsed) ? (parsed as RaceDef[]) : [];
    } catch {
      return [];
    }
  }

  private saveCustomRaces(): void {
    try {
      localStorage.setItem(CUSTOM_RACES_STORAGE_KEY, JSON.stringify(this.customRaces));
    } catch {
      // Ignore storage quota errors
    }
  }

  private allAvailableRaces(): RaceDef[] {
    return [...this.catalog.classicRaces, ...this.customRaces];
  }

  private formatLrtCodes(mask: number): string {
    if (mask === 0) return '—';
    return this.catalog.lrts
      .filter((l) => (mask & l.bit) !== 0)
      .map((l) => l.code)
      .join(',');
  }

  private initStaticControls(): void {
    // Populate Scenario select
    const scenSelect = getEl<HTMLSelectElement>('p_scenario');
    scenSelect.innerHTML = SCENARIOS.map(
      (s) => `<option value="${s.id}">${s.name}</option>`,
    ).join('');

    // Populate PRT select
    const prtSelect = getEl<HTMLSelectElement>('crPrt');
    prtSelect.innerHTML = this.catalog.prts
      .map((p) => `<option value="${p.code}">${p.shortName} – ${p.name}</option>`)
      .join('');
    this.updatePrtDescription();

    // Populate 14 LRT checkboxes
    const lrtGrid = getEl('crLrtGrid');
    lrtGrid.innerHTML = this.catalog.lrts
      .map(
        (l) => `<label class="lrt-check" title="${l.desc}">
          <input type="checkbox" data-lrt-bit="${l.bit}" />
          <span><b>${l.code}</b> – ${l.name}</span>
        </label>`,
      )
      .join('');

    // Populate Tech Browser categories
    const cats = Array.from(new Set(this.catalog.entries.map((e) => e.categoryLabel))).sort();
    const catSelect = getEl<HTMLSelectElement>('techCatFilter');
    catSelect.innerHTML =
      `<option value="all">All Categories (${this.catalog.entries.length})</option>` +
      cats.map((c) => `<option value="${c}">${c}</option>`).join('');
  }

  private updatePrtDescription(): void {
    const code = getEl<HTMLSelectElement>('crPrt').value;
    const found = this.catalog.prts.find((p) => p.code === code);
    getEl('crPrtDesc').textContent = found ? `${found.name}: ${found.desc}` : '';
  }

  private bindEvents(): void {
    // Speed buttons & Play/Pause toggle + single-tick step
    document.querySelectorAll<HTMLButtonElement>('[data-speed]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const rawSpeed = Number(btn.dataset.speed ?? 1);
        const speed =
          btn.id === 'btnPause'
            ? this.currentSpeed > 0
              ? 0
              : this.lastNonZeroSpeed > 0
                ? this.lastNonZeroSpeed
                : 1
            : rawSpeed;
        this.setActiveSpeedButton(speed);
        this.callbacks.onSpeedChange(speed);
      });
    });

    getEl('btnStep').addEventListener('click', () => this.callbacks.onStepTick());

    // Interactive HUD empire lens multi-select
    getEl('empiresBar').addEventListener('click', (e) => {
      const pill = (e.target as HTMLElement).closest<HTMLElement>('.emp-pill');
      if (!pill || pill.dataset.empIdx === undefined) return;
      const idx = Number(pill.dataset.empIdx);
      if (!Number.isNaN(idx)) {
        this.callbacks.onToggleEmpireLens(idx);
      }
    });

    // Scanner field visual controls (Equilibrium Line, Range Contours, Contour Opacity)
    const syncContourOpacityUi = (pct: number): void => {
      const clamped = Math.max(0, Math.min(80, Math.round(pct)));
      getEl<HTMLInputElement>('p_contourOpacity').value = String(clamped);
      getEl('v_contourOpacity').textContent = `${clamped}%`;
    };

    document.querySelectorAll<HTMLButtonElement>('[data-eq-mode]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const mode = (btn.dataset.eqMode === 'none' ? 'none' : 'gap') as EquilibriumMode;
        document.querySelectorAll<HTMLButtonElement>('[data-eq-mode]').forEach((b) => {
          b.classList.toggle('active', b.dataset.eqMode === mode);
        });
        this.callbacks.onEquilibriumModeChange(mode);
      });
    });

    document.querySelectorAll<HTMLButtonElement>('[data-contour-mode]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const mode = (btn.dataset.contourMode ?? 'subtle') as RangeContourMode;
        this.activeContourMode = mode;
        document.querySelectorAll<HTMLButtonElement>('[data-contour-mode]').forEach((b) => {
          b.classList.toggle('active', b.dataset.contourMode === mode);
        });
        let pct = this.lastNonZeroContourOpacity;
        if (mode === 'hidden') pct = 0;
        else if (mode === 'subtle') pct = 22;
        else if (mode === 'full') pct = 58;
        else if (mode === 'outer') pct = pct > 0 ? pct : 45;
        if (pct > 0) this.lastNonZeroContourOpacity = pct;
        syncContourOpacityUi(pct);
        this.callbacks.onContourModeChange(mode);
        this.callbacks.onContourOpacityChange(pct / 100);
      });
    });

    getEl<HTMLInputElement>('p_contourOpacity').addEventListener('input', (e) => {
      const pct = Math.max(0, Math.min(80, Math.round(Number((e.target as HTMLInputElement).value))));
      getEl('v_contourOpacity').textContent = `${pct}%`;
      if (pct > 0) {
        this.lastNonZeroContourOpacity = pct;
        if (this.activeContourMode === 'hidden') {
          this.activeContourMode = 'subtle';
          document.querySelectorAll<HTMLButtonElement>('[data-contour-mode]').forEach((b) => {
            b.classList.toggle('active', b.dataset.contourMode === 'subtle');
          });
          this.callbacks.onContourModeChange('subtle');
        }
      }
      this.callbacks.onContourOpacityChange(pct / 100);
    });

    // Debug Performance HUD, Event Log, Telemetry & History Scrubber controls
    getEl<HTMLInputElement>('p_showEventLog').addEventListener('change', (e) => {
      this.showEventLog = (e.target as HTMLInputElement).checked;
      this.syncEventLogVisibility();
      if (this.showEventLog && this.currentSpeed <= 1 && this.lastHud) {
        this.renderEvents(this.lastHud);
      }
      this.callbacks.onShowEventLogToggle(this.showEventLog);
    });
    getEl<HTMLInputElement>('p_debugPerf').addEventListener('change', (e) => {
      this.callbacks.onDebugPerfToggle((e.target as HTMLInputElement).checked);
    });
    getEl<HTMLInputElement>('p_debugHistory').addEventListener('change', (e) => {
      this.callbacks.onDebugHistoryToggle((e.target as HTMLInputElement).checked);
    });
    getEl<HTMLInputElement>('p_debugTelemetry').addEventListener('change', (e) => {
      this.callbacks.onDebugTelemetryToggle((e.target as HTMLInputElement).checked);
    });
    getEl('btnCopyTelemetry').addEventListener('click', () => this.callbacks.onCopyTelemetry());
    getEl('btnClearTelemetry').addEventListener('click', () => this.callbacks.onClearTelemetry());
    getEl<HTMLInputElement>('historySlider').addEventListener('input', (e) => {
      this.callbacks.onHistoryScrub(Number((e.target as HTMLInputElement).value));
    });
    getEl('btnHistPrev').addEventListener('click', () => this.callbacks.onHistoryStep(-1));
    getEl('btnHistNext').addEventListener('click', () => this.callbacks.onHistoryStep(1));
    getEl('btnHistLive').addEventListener('click', () => this.callbacks.onHistoryLive());

    // Drawers
    document.querySelectorAll<HTMLButtonElement>('[data-drawer]').forEach((btn) => {
      btn.addEventListener('click', () => {
        this.toggleDrawer(btn.dataset.drawer as (typeof DRAWER_IDS)[number]);
      });
    });
    document.querySelectorAll<HTMLButtonElement>('[data-close-drawer]').forEach((btn) => {
      btn.addEventListener('click', () => {
        this.toggleDrawer(btn.dataset.closeDrawer as (typeof DRAWER_IDS)[number]);
      });
    });

    getEl('btnReset').addEventListener('click', () => this.callbacks.onReset());
    getEl('btnCloseInspector').addEventListener('click', () => this.callbacks.onSelectObject(null));
    getEl('btnToggleDebrisWrecks').addEventListener('click', () => {
      this.debrisWrecksExpanded = !this.debrisWrecksExpanded;
      const body = getEl('inspDebrisWrecksBody');
      body.style.display = this.debrisWrecksExpanded ? 'block' : 'none';
      const hint = getEl('btnToggleDebrisWrecks').querySelector('.insp-wreck-hint');
      if (hint) hint.textContent = this.debrisWrecksExpanded ? 'Hide' : 'Show';
      const lbl = getEl('inspWreckToggleLabel');
      if (lbl.textContent) {
        lbl.textContent = lbl.textContent.replace(/^[▸▾]/, this.debrisWrecksExpanded ? '▾' : '▸');
      }
    });

    getEl('btnTogglePlanetFleets').addEventListener('click', () => {
      this.planetFleetsExpanded = !this.planetFleetsExpanded;
      const body = getEl('inspPlanetFleetsBody');
      body.style.display = this.planetFleetsExpanded ? 'block' : 'none';
      const hint = getEl('btnTogglePlanetFleets').querySelector('.insp-wreck-hint');
      if (hint) hint.textContent = this.planetFleetsExpanded ? 'Hide' : 'Show';
      const lbl = getEl('inspOrbitToggleLabel');
      if (lbl.textContent) {
        lbl.textContent = lbl.textContent.replace(/^[▸▾]/, this.planetFleetsExpanded ? '▾' : '▸');
      }
    });

    const handleStationedFleetClick = (e: Event): void => {
      const row = (e.target as HTMLElement).closest<HTMLElement>('tr[data-fleet-id]');
      if (!row || !row.dataset.fleetId) return;
      const id = Number(row.dataset.fleetId);
      if (!Number.isNaN(id)) {
        void this.openFleetView(id);
      }
    };
    getEl('inspPlanetFleetsTbody').addEventListener('click', handleStationedFleetClick);
    getEl('sysOrbitFleetsTbody').addEventListener('click', handleStationedFleetClick);

    // Procedural Planet Inspector -> System View modal events
    getEl('btnOpenSystemView').addEventListener('click', () => {
      if (this.selection?.kind === 'planet') {
        void this.openSystemView(this.selection.index);
      }
    });
    getEl('btnCloseSystemModal').addEventListener('click', () => this.closeSystemView());
    getEl('systemModal').addEventListener('click', (e) => {
      if (e.target === getEl('systemModal')) {
        if (getEl('sysPreviewBox').classList.contains('expanded')) {
          this.setSysPreviewExpanded(false);
        } else {
          this.closeSystemView();
        }
      }
    });
    getEl('btnToggleSysExpand').addEventListener('click', () => {
      const isExp = getEl('sysPreviewBox').classList.contains('expanded');
      this.setSysPreviewExpanded(!isExp);
    });
    getEl('sysPreviewCanvas').addEventListener('dblclick', () => {
      const isExp = getEl('sysPreviewBox').classList.contains('expanded');
      this.setSysPreviewExpanded(!isExp);
    });
    window.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') {
        if (getEl('fleetPreviewBox').classList.contains('expanded')) {
          this.setFleetPreviewExpanded(false);
        } else if (getEl('fleetModal').style.display !== 'none') {
          this.closeFleetView();
        } else if (getEl('sysPreviewBox').classList.contains('expanded')) {
          this.setSysPreviewExpanded(false);
        } else if (getEl('systemModal').style.display !== 'none') {
          this.closeSystemView();
        }
      }
    });

    // Fleet Inspector -> Fleet 3D View modal events
    getEl('btnOpenFleetView').addEventListener('click', () => {
      if (this.selection?.kind === 'fleet') {
        void this.openFleetView(this.selection.id);
      }
    });
    getEl('btnCloseFleetModal').addEventListener('click', () => this.closeFleetView());
    getEl('fleetModal').addEventListener('click', (e) => {
      if (e.target === getEl('fleetModal')) {
        if (getEl('fleetPreviewBox').classList.contains('expanded')) {
          this.setFleetPreviewExpanded(false);
        } else {
          this.closeFleetView();
        }
      }
    });
    getEl('btnResetFleetCam').addEventListener('click', () => {
      this.modalShipRenderer.resetCamera();
    });
    getEl('btnToggleFleetExpand').addEventListener('click', () => {
      const isExp = getEl('fleetPreviewBox').classList.contains('expanded');
      this.setFleetPreviewExpanded(!isExp);
    });
    getEl('fleetPreviewCanvas').addEventListener('dblclick', () => {
      const isExp = getEl('fleetPreviewBox').classList.contains('expanded');
      this.setFleetPreviewExpanded(!isExp);
    });
    document.querySelectorAll<HTMLButtonElement>('[data-fleet-mode]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const mode: ShipViewMode = btn.dataset.fleetMode === 'blueprint' ? 'blueprint' : 'solid';
        this.setFleetModalMode(mode);
      });
    });
    getEl('btnExportPlanetPng').addEventListener('click', () => {
      const sys = this.activeModalSystem;
      if (!sys || this.activeModalSelection === 'star' || this.activeModalSelection === 'star2') return;
      const body = sys.bodies[this.activeModalSelection.bodyIdx];
      if (!body) return;
      if (this.activeModalSelection.moonIdx !== null) {
        const moon = body.moons[this.activeModalSelection.moonIdx];
        if (moon) exportPlanetTexturePng(moon, sys.star);
      } else {
        exportPlanetTexturePng(body, sys.star);
      }
    });
    getEl('btnBackToParentPlanet').addEventListener('click', () => {
      if (this.activeModalSelection !== 'star' && this.activeModalSelection !== 'star2') {
        this.selectModalItem({ bodyIdx: this.activeModalSelection.bodyIdx, moonIdx: null });
      }
    });
    document.querySelectorAll<HTMLButtonElement>('[data-sys-tab]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const tab = btn.dataset.sysTab === 'orrery' ? 'orrery' : 'bodies';
        this.setSystemModalTab(tab);
      });
    });

    // Tech drawer tabs
    document.querySelectorAll<HTMLButtonElement>('[data-tech-tab]').forEach((btn) => {
      btn.addEventListener('click', () => {
        this.activeTechTab = btn.dataset.techTab === 'browser' ? 'browser' : 'overview';
        getEl('techTabOverview').classList.toggle('active', this.activeTechTab === 'overview');
        getEl('techTabBrowser').classList.toggle('active', this.activeTechTab === 'browser');
        getEl('techOverviewPane').style.display = this.activeTechTab === 'overview' ? 'flex' : 'none';
        getEl('techBrowserPane').style.display = this.activeTechTab === 'browser' ? 'flex' : 'none';
        if (this.lastHud) this.renderOpenDrawers(this.lastHud);
      });
    });

    // Tech browser filters
    const reRenderBrowser = () => {
      if (this.lastHud) this.renderTechBrowser(this.lastHud);
    };
    getEl('techSearch').addEventListener('input', reRenderBrowser);
    getEl('techCatFilter').addEventListener('change', reRenderBrowser);
    getEl('techStatusFilter').addEventListener('change', reRenderBrowser);

    // Races drawer events
    getEl('crPrt').addEventListener('change', () => this.updatePrtDescription());
    getEl('btnSelectClassic6').addEventListener('click', () => {
      this.selectedRaceIds = new Set(this.catalog.classicRaces.map((r) => r.id));
      this.renderRaceRoster();
    });
    getEl('btnSelectAllRaces').addEventListener('click', () => {
      this.selectedRaceIds = new Set(this.allAvailableRaces().slice(0, 16).map((r) => r.id));
      this.renderRaceRoster();
    });
    getEl('btnLaunchRoster').addEventListener('click', () => {
      const chosen = this.allAvailableRaces().filter((r) => this.selectedRaceIds.has(r.id));
      if (chosen.length < 2 || chosen.length > 16) {
        getEl('crStatus').textContent = 'Select between 2 and 16 races to launch.';
        return;
      }
      getEl('crStatus').textContent = `Launched galaxy with ${chosen.length} empires!`;
      this.callbacks.onLaunchRaces(chosen);
    });

    getEl('btnCreateRace').addEventListener('click', () => {
      const name = getEl<HTMLInputElement>('crName').value.trim();
      if (!name) {
        getEl('crStatus').textContent = 'Please enter a race name.';
        return;
      }
      const color = getEl<HTMLInputElement>('crColor').value;
      const prt = getEl<HTMLSelectElement>('crPrt').value;
      const focus = getEl<HTMLSelectElement>('crFocus').value;
      let lrtMask = 0;
      document.querySelectorAll<HTMLInputElement>('input[data-lrt-bit]').forEach((cb) => {
        if (cb.checked) lrtMask |= Number(cb.dataset.lrtBit ?? 0);
      });
      const newRace: RaceDef = {
        id: `custom_${Date.now()}`,
        name,
        color,
        prt,
        lrtMask,
        isCustom: true,
        doctrine: doctrinePreset(focus),
      };
      this.customRaces.push(newRace);
      this.saveCustomRaces();
      if (this.selectedRaceIds.size < 16) {
        this.selectedRaceIds.add(newRace.id);
      }
      getEl<HTMLInputElement>('crName').value = '';
      getEl('crStatus').textContent = `✓ Created "${name}" (${shortPrt(prt)}). Click Launch Galaxy to play!`;
      this.renderRaceRoster();
    });

    getEl('raceRosterList').addEventListener('change', (e) => {
      const target = e.target as HTMLInputElement;
      if (!target.dataset.raceId) return;
      const id = target.dataset.raceId;
      if (target.checked) {
        if (this.selectedRaceIds.size >= 16) {
          target.checked = false;
          getEl('crStatus').textContent = 'Maximum 16 empires per galaxy.';
          return;
        }
        this.selectedRaceIds.add(id);
      } else {
        this.selectedRaceIds.delete(id);
      }
      this.updateRosterCount();
    });

    getEl('raceRosterList').addEventListener('click', (e) => {
      const btn = (e.target as HTMLElement).closest<HTMLButtonElement>('[data-delete-race]');
      if (!btn || !btn.dataset.deleteRace) return;
      const id = btn.dataset.deleteRace;
      this.customRaces = this.customRaces.filter((r) => r.id !== id);
      this.selectedRaceIds.delete(id);
      this.saveCustomRaces();
      this.renderRaceRoster();
    });

    // Report tabs & sorting
    document.querySelectorAll<HTMLButtonElement>('[data-report]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const which = (btn.dataset.report ?? 'planets') as ReportTab;
        this.activeReportTab = which;
        getEl('repPlanetsWrap').style.display = which === 'planets' ? 'block' : 'none';
        getEl('repFleetsWrap').style.display = which === 'fleets' ? 'block' : 'none';
        getEl('repStarbasesWrap').style.display = which === 'starbases' ? 'block' : 'none';
        getEl('repStargatesWrap').style.display = which === 'stargates' ? 'block' : 'none';
        getEl('repWormholesWrap').style.display = which === 'wormholes' ? 'block' : 'none';
        getEl('repDebrisWrap').style.display = which === 'debris' ? 'block' : 'none';
        getEl('repBtnPlanets').classList.toggle('active', which === 'planets');
        getEl('repBtnFleets').classList.toggle('active', which === 'fleets');
        getEl('repBtnStarbases').classList.toggle('active', which === 'starbases');
        getEl('repBtnStargates').classList.toggle('active', which === 'stargates');
        getEl('repBtnWormholes').classList.toggle('active', which === 'wormholes');
        getEl('repBtnDebris').classList.toggle('active', which === 'debris');
        if (this.lastHud) this.renderOpenDrawers(this.lastHud);
      });
    });

    this.updateReportSortIndicators();
    document.querySelectorAll<HTMLTableCellElement>('table[data-sort-table] th[data-sort]').forEach((th) => {
      th.addEventListener('click', () => {
        const table = th.closest('table[data-sort-table]') as HTMLTableElement | null;
        const tab = table?.dataset.sortTable as ReportTab | undefined;
        const key = th.dataset.sort;
        if (!tab || !key) return;

        const state = this.reportSort[tab] as { key: string; asc: boolean };
        if (state.key === key) {
          state.asc = !state.asc;
        } else {
          state.key = key;
          state.asc = ASC_DEFAULT_SORT_KEYS.has(key);
        }
        this.reportPage[tab] = 0;
        this.updateReportSortIndicators(tab);
        this.rerenderCurrentReportTab(true);
      });
    });

    const goReportPage = (nextPage: number): void => {
      const tab = this.activeReportTab;
      const maxPage = Math.max(0, Math.ceil(this.reportTotalRows / this.reportPageSize) - 1);
      const clamped = Math.max(0, Math.min(maxPage, nextPage));
      if (clamped === this.reportPage[tab]) return;
      this.reportPage[tab] = clamped;
      this.rerenderCurrentReportTab(true);
    };

    getEl('repPageFirst').addEventListener('click', () => goReportPage(0));
    getEl('repPagePrev').addEventListener('click', () => goReportPage(this.reportPage[this.activeReportTab] - 1));
    getEl('repPageNext').addEventListener('click', () => goReportPage(this.reportPage[this.activeReportTab] + 1));
    getEl('repPageLast').addEventListener('click', () => {
      const maxPage = Math.max(0, Math.ceil(this.reportTotalRows / this.reportPageSize) - 1);
      goReportPage(maxPage);
    });
    getEl<HTMLSelectElement>('repPageSize').addEventListener('change', (e) => {
      const val = Number((e.target as HTMLSelectElement).value);
      if (val > 0) {
        this.reportPageSize = val;
        this.reportPage[this.activeReportTab] = 0;
        this.rerenderCurrentReportTab(true);
      }
    });

    document.querySelectorAll<HTMLButtonElement>('[data-fleet-filter]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const next = (btn.dataset.fleetFilter ?? 'all') as 'all' | 'flight' | 'orbiting';
        this.fleetReportFilter = next;
        this.reportPage.fleets = 0;
        document.querySelectorAll<HTMLButtonElement>('[data-fleet-filter]').forEach((b) => {
          b.classList.toggle('active', b.dataset.fleetFilter === next);
        });
        this.rerenderCurrentReportTab(true);
      });
    });

    getEl('planetTableBody').addEventListener('click', (e) => {
      const row = (e.target as HTMLElement).closest('tr');
      if (!row || !row.dataset.planetIdx) return;
      const idx = Number(row.dataset.planetIdx);
      const x = Number(row.dataset.x);
      const y = Number(row.dataset.y);
      if (Number.isNaN(idx)) return;
      this.callbacks.onSelectObject({ kind: 'planet', index: idx });
      if (!Number.isNaN(x) && !Number.isNaN(y)) {
        this.focusWorldPointOnMap(x, y, 55);
      }
      this.syncReportRowHighlight();
    });

    getEl('starbaseTableBody').addEventListener('click', (e) => {
      const row = (e.target as HTMLElement).closest('tr');
      if (!row || !row.dataset.planetIdx) return;
      const idx = Number(row.dataset.planetIdx);
      const x = Number(row.dataset.x);
      const y = Number(row.dataset.y);
      if (Number.isNaN(idx)) return;
      this.callbacks.onSelectObject({ kind: 'planet', index: idx });
      if (!Number.isNaN(x) && !Number.isNaN(y)) {
        this.focusWorldPointOnMap(x, y, 55);
      }
      this.syncReportRowHighlight();
    });

    getEl('fleetTableBody').addEventListener('click', (e) => {
      const target = e.target as HTMLElement;
      const row = target.closest('tr');
      if (!row || !row.dataset.fleetId) return;
      const id = Number(row.dataset.fleetId);
      if (Number.isNaN(id)) return;

      const isOrbiting = row.dataset.orbiting === '1';
      const targetIdx = Number(row.dataset.targetIdx);
      const routeCell = target.closest<HTMLElement>('[data-route-focus]');

      if (isOrbiting && !Number.isNaN(targetIdx)) {
        // Orbiting fleet: focus host system on map and open System View with this fleet highlighted
        this.callbacks.onSelectObject({ kind: 'planet', index: targetIdx });
        void this.callbacks.getPlanetDetail(targetIdx).then((pl) => {
          if (pl) this.focusWorldPointOnMap(pl.x, pl.y, 55);
        });
        if (!routeCell) {
          void this.openSystemView(targetIdx, id);
        }
        return;
      }

      if (routeCell) {
        const fromIdx = Number(row.dataset.fromIdx);
        this.callbacks.onSelectObject({ kind: 'fleet', id });
        void this.focusFleetRouteOnMap(
          id,
          Number.isNaN(fromIdx) ? null : fromIdx,
          Number.isNaN(targetIdx) ? null : targetIdx,
        );
        return;
      }

      this.callbacks.onSelectObject({ kind: 'fleet', id });
    });

    getEl('stargateTableBody').addEventListener('click', (e) => {
      const row = (e.target as HTMLElement).closest('tr');
      if (!row || !row.dataset.planetIdx) return;
      const idx = Number(row.dataset.planetIdx);
      const x = Number(row.dataset.x);
      const y = Number(row.dataset.y);
      if (Number.isNaN(idx)) return;
      this.callbacks.onSelectObject({ kind: 'planet', index: idx });
      if (!Number.isNaN(x) && !Number.isNaN(y)) {
        this.focusWorldPointOnMap(x, y, 55);
      }
    });

    getEl('wormholeTableBody').addEventListener('click', (e) => {
      const target = e.target as HTMLElement;
      const row = target.closest('tr');
      if (!row || !row.dataset.whIdx) return;
      const idx = Number(row.dataset.whIdx);
      if (Number.isNaN(idx) || !this.lastHud) return;
      const wh = this.lastHud.wormholes[idx];
      if (!wh) return;
      this.callbacks.onSelectObject({ kind: 'wormhole', index: idx });
      const ep = target.closest<HTMLElement>('[data-wh-end]')?.dataset.whEnd;
      if (ep === 'a') {
        this.focusWorldPointOnMap(wh.x1, wh.y1, 55);
      } else if (ep === 'b') {
        this.focusWorldPointOnMap(wh.x2, wh.y2, 55);
      } else {
        this.focusWorldBoxOnMap(
          Math.min(wh.x1, wh.x2),
          Math.min(wh.y1, wh.y2),
          Math.max(wh.x1, wh.x2),
          Math.max(wh.y1, wh.y2),
        );
      }
    });

    getEl('debrisTableBody').addEventListener('click', (e) => {
      const row = (e.target as HTMLElement).closest('tr');
      if (!row || !row.dataset.debrisIdx) return;
      const idx = Number(row.dataset.debrisIdx);
      const x = Number(row.dataset.x);
      const y = Number(row.dataset.y);
      if (Number.isNaN(idx)) return;
      this.callbacks.onSelectObject({ kind: 'debris', index: idx });
      if (!Number.isNaN(x) && !Number.isNaN(y)) {
        this.focusWorldPointOnMap(x, y, 45);
      }
    });

    // Lab actions
    document.querySelectorAll<HTMLButtonElement>('[data-action]').forEach((btn) => {
      btn.addEventListener('click', () => {
        this.callbacks.onLabAction(btn.dataset.action as LabAction);
      });
    });

    // Settings sliders, galaxy generator & sandbox checkbox
    const onGalaxySelect = () => {
      const size = getEl<HTMLSelectElement>('p_galaxySize').value;
      const shape = getEl<HTMLSelectElement>('p_galaxyShape').value;
      this.callbacks.onGalaxyChange(size, shape);
    };
    getEl('p_galaxySize').addEventListener('change', onGalaxySelect);
    getEl('p_galaxyShape').addEventListener('change', onGalaxySelect);

    getEl<HTMLSelectElement>('p_scenario').addEventListener('change', (e) => {
      const id = (e.target as HTMLSelectElement).value as ScenarioId;
      this.callbacks.onScenarioChange(id);
    });

    getEl<HTMLInputElement>('p_ignoreRacialTraits').addEventListener('change', (e) => {
      const checked = (e.target as HTMLInputElement).checked;
      this.callbacks.onParamChange('ignoreRacialTraits', checked ? 1 : 0);
    });

    document.querySelectorAll<HTMLButtonElement>('[data-wh-preset]').forEach((btn) => {
      btn.addEventListener('click', () => {
        if (btn.disabled) return;
        const val = Number(btn.dataset.whPreset ?? 2);
        if (Number.isNaN(val)) return;
        getEl<HTMLInputElement>('p_wormholePairs').value = String(val);
        getEl('v_wormholePairs').textContent = String(val);
        this.callbacks.onParamChange('wormholePairs', val);
      });
    });

    document.querySelectorAll<HTMLInputElement>('input[data-param]').forEach((input) => {
      input.addEventListener('input', () => {
        const key = input.dataset.param!;
        const num = parseFloat(input.value);
        const label = getEl(`v_${key}`);
        if (key === 'sprawlPenalty' || key === 'aggroBias' || key === 'salvageRate' || key === 'colCostMult') {
          label.textContent = `${num}%`;
        } else {
          label.textContent = String(num);
        }
        this.callbacks.onParamChange(key, num);
      });
    });
  }

  private renderRaceRoster(): void {
    const list = getEl('raceRosterList');
    const races = this.allAvailableRaces();
    list.innerHTML = races
      .map((r) => {
        const checked = this.selectedRaceIds.has(r.id) ? 'checked' : '';
        const lrtTxt = this.formatLrtCodes(r.lrtMask);
        const delBtn = r.isCustom
          ? `<button class="insp-close" data-delete-race="${r.id}" title="Delete custom race">✕</button>`
          : `<span style="font-size:0.6rem;color:var(--muted);">Classic</span>`;
        return `<div class="race-item" style="border-left-color:${r.color};">
          <label class="race-item-left">
            <input type="checkbox" data-race-id="${r.id}" ${checked} />
            <div>
              <div style="color:${r.color};font-weight:700;">${r.name}</div>
              <div class="race-meta">${shortPrt(r.prt)} | LRT: ${lrtTxt}</div>
            </div>
          </label>
          ${delBtn}
        </div>`;
      })
      .join('');
    this.updateRosterCount();
  }

  private updateRosterCount(): void {
    const count = this.selectedRaceIds.size;
    getEl('rosterCountInfo').textContent = `Selected: ${count} / 16 races (min 2)`;
  }

  private toggleDrawer(targetId: (typeof DRAWER_IDS)[number]): void {
    for (const id of DRAWER_IDS) {
      const el = getEl(id);
      if (id === targetId) el.classList.toggle('open');
      else el.classList.remove('open');
      const tabId = DRAWER_TAB_MAP[id];
      if (tabId) {
        getEl(tabId).classList.toggle('active', el.classList.contains('open'));
      }
    }
    if (this.lastHud) this.renderOpenDrawers(this.lastHud);
  }

  private closeAllDrawers(): void {
    for (const id of DRAWER_IDS) {
      getEl(id).classList.remove('open');
      const tabId = DRAWER_TAB_MAP[id];
      if (tabId) {
        getEl(tabId).classList.remove('active');
      }
    }
  }

  private formatTechBadges(perkMask: number): string {
    return this.meta.breakthroughs
      .filter((bp) => (perkMask & bp.bit) !== 0)
      .map((bp) => bp.icon)
      .join('');
  }

  public setEmpireLensState(activeEmpires: Set<number> | null): void {
    this.activeLensEmpires = activeEmpires ? new Set(activeEmpires) : null;
    this.applyEmpireLensClasses();
  }

  private applyEmpireLensClasses(): void {
    const bar = getEl('empiresBar');
    const pills = bar.querySelectorAll<HTMLElement>('.emp-pill');
    const hasFilter = this.activeLensEmpires !== null && this.activeLensEmpires.size > 0;
    pills.forEach((pill) => {
      const idx = Number(pill.dataset.empIdx ?? -1);
      const isSelected = hasFilter && this.activeLensEmpires!.has(idx);
      pill.classList.toggle('lens-active', isSelected);
      pill.classList.toggle('lens-dimmed', hasFilter && !isSelected);
    });
  }

  private updateEmpireHud(hud: HudSnapshot): void {
    const bar = getEl('empiresBar');
    const sig = hud.empires.map((e) => `${e.id}:${e.color}`).join('|');
    if (sig !== this.lastHudSignature) {
      this.lastHudSignature = sig;
      bar.innerHTML = hud.empires
        .map(
          (emp, idx) => `<div class="emp-pill" id="hudPill_${idx}" data-emp-idx="${idx}" style="border-top-color:${emp.color};--emp-color:${emp.color};">
            <div class="emp-row1">
              <span style="color:${emp.color};">■ ${emp.name}</span>
              <span id="hudTechB_${idx}"></span>
            </div>
            <div class="emp-row2">
              <span id="hudStat_${idx}">1p · 25R</span>
              <span id="hudKill_${idx}">💥0 🌀0</span>
            </div>
          </div>`,
        )
        .join('');
      this.applyEmpireLensClasses();
    }

    for (let i = 0; i < hud.empires.length; i++) {
      const emp = hud.empires[i];
      const pillEl = document.getElementById(`hudPill_${i}`);
      const statEl = document.getElementById(`hudStat_${i}`);
      const killEl = document.getElementById(`hudKill_${i}`);
      const badgeEl = document.getElementById(`hudTechB_${i}`);
      if (statEl) {
        statEl.textContent = `${formatCompact(emp.stats.planets)}p · ${formatCompact(emp.stats.res)}R`;
      }
      if (killEl) {
        killEl.textContent = `💥${formatCompact(emp.doctrine.kills)} 🌀${formatCompact(emp.stats.gates)}`;
      }
      if (badgeEl) badgeEl.textContent = this.formatTechBadges(emp.perks);
      if (pillEl) {
        const isExtinct = emp.stats.planets === 0;
        pillEl.classList.toggle('extinct', isExtinct);
        const upkeep = emp.stats.fleetUpkeep ?? 0;
        const upkeepPct = emp.stats.res > 0 ? Math.round((upkeep / emp.stats.res) * 100) : 0;
        const fCount = emp.stats.fleetCount ?? 0;
        const orbCount = emp.stats.orbitingCount ?? 0;
        const ewTxt = `Cloak ${Math.round(emp.loadout.cloakArmedPct * 100)}% · Tachyon ${Math.round(emp.loadout.tachyonReductionPct * 100)}% · Jammer ${Math.round(emp.loadout.torpedoJammingPct * 100)}%`;
        const prefix = isExtinct ? '[EXTINCT] ' : '';
        pillEl.title = `${prefix}${emp.name}: ${emp.stats.planets.toLocaleString()} planets · ${Math.round(emp.stats.res).toLocaleString()} Res/yr · Upkeep ${upkeep.toFixed(1)} Res/yr (${upkeepPct}%) · Fleets ${fCount} (${orbCount} orbiting) · ${emp.doctrine.kills.toLocaleString()} kills · ${emp.stats.gates.toLocaleString()} stargates | EW: ${ewTxt}`;
      }
    }
  }

  private renderEvents(hud: HudSnapshot): void {
    if (!this.showEventLog || this.currentSpeed > 1) return;
    const sig = hud.events.map((ev) => `${ev.year.toFixed(0)}:${ev.text}`).join('|');
    if (sig === this.lastEventsSignature) return;
    this.lastEventsSignature = sig;
    const container = getEl('eventLog');
    container.innerHTML = hud.events
      .map(
        (ev) =>
          `<div class="log-item" style="border-right-color:${ev.color}"><b style="font-family:monospace;color:${ev.color}">[${ev.year.toFixed(0)}]</b> ${ev.text}</div>`,
      )
      .join('');
  }

  private setInspCell(idx: number, label: string, value: string, color = ''): void {
    getEl(`inspLbl${idx}`).textContent = label;
    const v = getEl(`inspVal${idx}`);
    v.textContent = value;
    v.style.color = color;
  }

  public async openSystemView(planetIdx: number, highlightFleetId?: number): Promise<void> {
    const seq = ++this.modalReqSeq;
    const [sys, pl, orbitFleets] = await Promise.all([
      this.callbacks.getStarSystem(planetIdx),
      this.callbacks.getPlanetDetail(planetIdx),
      this.callbacks.getPlanetFleets(planetIdx),
    ]);
    if (seq !== this.modalReqSeq || !sys) return;

    this.activeModalSystem = sys;
    this.activeModalSelection = { bodyIdx: sys.primaryIndex, moonIdx: null };
    getEl('systemModal').style.display = 'flex';
    const binaryTag = sys.companionStar ? ' • Binary' : '';
    getEl('sysModalTitle').textContent = `🌌 System #${sys.planetId} (${sys.bodies.length} bodies${binaryTag})`;
    const star2Meta = sys.companionStar ? ` + ${sys.companionStar.spectralClass}` : '';
    getEl('sysStarMeta').innerHTML =
      `${sys.star.spectralClass}${star2Meta} • ${Math.round(sys.star.tempK)} K • ${sys.star.massSolar.toFixed(2)} M<sub class="astro-sub">☉</sub> • ${sys.star.luminositySolar.toFixed(2)} L<sub class="astro-sub">☉</sub>`;

    const sbWrap = getEl('sysStarbaseWrap');
    const sbTier = pl?.starbaseTier ?? 0;
    if (pl && sbTier > 0) {
      const dockMults = [0.75, 1.0, 1.25, 1.45, 1.65, 1.9];
      const repRates = [3, 10, 14, 18, 22, 28];
      const upkeeps = [0, 0.8, 1.5, 2.5, 4.0, 6.0];
      const hp = Math.round(pl.starbaseHp ?? 0);
      const maxHp = Math.max(1, Math.round(pl.starbaseMaxHp ?? 1));
      const pct = Math.round((hp / maxHp) * 100);
      sbWrap.style.display = 'flex';
      getEl('sysStarbaseTitle').textContent = `🛡 ${pl.starbaseName ?? 'Starbase'} (Tier ${sbTier})`;
      getEl('sysStarbaseHpBadge').textContent = `${hp} / ${maxHp} HP (${pct}%)`;
      getEl('sysStarbaseStats').innerHTML = `
        <div class="insp-stat"><span>Shipyard Dock</span><b style="color:var(--accent)">×${(dockMults[sbTier] ?? 1.0).toFixed(2)} build speed</b></div>
        <div class="insp-stat"><span>Fleet Repair</span><b style="color:#34d399">${repRates[sbTier] ?? 10}% HP/yr</b></div>
        <div class="insp-stat"><span>Orbital Firepower</span><b>${Math.round(hp * 0.22)} power</b></div>
        <div class="insp-stat"><span>Upkeep</span><b style="color:var(--muted)">${(upkeeps[sbTier] ?? 1.0).toFixed(1)} Res/yr</b></div>
      `;
    } else {
      sbWrap.style.display = 'none';
      getEl('sysStarbaseStats').innerHTML = '';
    }

    const fleetWrap = getEl('sysOrbitFleetsWrap');
    if (orbitFleets.length > 0) {
      fleetWrap.style.display = 'flex';
      getEl('sysOrbitFleetsTitle').textContent = `🛰 Stationed Fleets in Orbit (${orbitFleets.length})`;
      const classColors: Record<FleetClass, string> = {
        battleship: '#f87171',
        cruiser: '#93c5fd',
        bomber: '#34d399',
        freight: '#c084fc',
        col: '#fbbf24',
      };
      getEl('sysOrbitFleetsTbody').innerHTML = orbitFleets
        .map((fl) => {
          const hpPct = fl.maxPower > 0 ? Math.round((fl.power / fl.maxPower) * 100) : 100;
          const col = classColors[fl.type] ?? '#e2e8f0';
          const statTxt =
            fl.type === 'freight'
              ? `Cap ${Math.round(fl.maxPower)} kT`
              : `${Math.round(fl.power)} (${hpPct}%)`;
          const upkeepTxt = fl.upkeep !== undefined ? `${fl.upkeep.toFixed(1)} R/yr` : '—';
          const hl = highlightFleetId !== undefined && fl.id === highlightFleetId ? ' class="fleet-row-highlight"' : '';
          return `<tr data-fleet-id="${fl.id}"${hl} title="Open 3D Fleet View">
            <td>F#${fl.id}</td>
            <td style="color:${col};font-weight:bold;">${FLEET_LABELS[fl.type]}</td>
            <td>${statTxt}</td>
            <td style="color:var(--muted);">${upkeepTxt}</td>
          </tr>`;
        })
        .join('');
    } else {
      fleetWrap.style.display = 'none';
      getEl('sysOrbitFleetsTbody').innerHTML = '';
    }

    const strip = getEl('sysOrbitStrip');
    strip.innerHTML = '';

    // 1. Primary Star card
    const starLabel =
      sys.star.starKind === 'BlackHole'
        ? 'Black Hole'
        : sys.star.starKind === 'Pulsar'
          ? 'Pulsar'
          : sys.star.starKind === 'WhiteDwarf'
            ? `${sys.star.spectralClass} WD`
            : sys.star.starKind === 'BrownDwarf'
              ? `${sys.star.spectralClass} BD`
              : `${sys.star.spectralClass} Star`;
    const starCard = document.createElement('div');
    starCard.className = 'sys-body-card';
    starCard.dataset.sysItem = 'star';
    starCard.innerHTML = `
      <canvas class="sys-body-thumb" width="96" height="96"></canvas>
      <div class="sys-body-meta">
        <div class="sys-body-name">${starLabel}</div>
        <div class="sys-body-sub">${sys.star.starKind === 'BlackHole' ? `${sys.star.massSolar.toFixed(1)} M☉` : `${Math.round(sys.star.tempK)} K`}</div>
      </div>
    `;
    const starCanvas = starCard.querySelector<HTMLCanvasElement>('canvas');
    if (starCanvas) {
      renderProceduralThumbnailTo2d(starCanvas, { kind: 'star', star: sys.star });
    }
    starCard.addEventListener('click', () => this.selectModalItem('star'));
    strip.appendChild(starCard);

    // 1b. Optional Binary Companion Star card
    if (sys.companionStar) {
      const st2 = sys.companionStar;
      const st2Card = document.createElement('div');
      st2Card.className = 'sys-body-card';
      st2Card.dataset.sysItem = 'star2';
      st2Card.innerHTML = `
        <canvas class="sys-body-thumb" width="96" height="96"></canvas>
        <div class="sys-body-meta">
          <div class="sys-body-name">${st2.spectralClass} B</div>
          <div class="sys-body-sub">${Math.round(st2.tempK)} K</div>
        </div>
      `;
      const st2Canvas = st2Card.querySelector<HTMLCanvasElement>('canvas');
      if (st2Canvas) {
        renderProceduralThumbnailTo2d(st2Canvas, { kind: 'star', star: st2 });
      }
      st2Card.addEventListener('click', () => this.selectModalItem('star2'));
      strip.appendChild(st2Card);
    }

    // 2. Orbiting planets in ascending AU order
    sys.bodies.forEach((body, idx) => {
      const isPrimary = idx === sys.primaryIndex;
      const card = document.createElement('div');
      card.className = `sys-body-card${isPrimary ? ' primary' : ''}`;
      card.dataset.sysItem = String(idx);
      const ls = body.liveState;
      const icons =
        (ls?.hasStargate ? '🌀' : '') +
        (ls?.hasJammer ? '📡' : '') +
        (ls?.hasStarbase ? '🛡' : '');
      const moonTag = body.moonsCount > 0 ? ` · ${body.moonsCount}m` : '';
      card.innerHTML = `
        <canvas class="sys-body-thumb" width="96" height="96"></canvas>
        <div class="sys-body-meta">
          <div class="sys-body-name" title="${body.name}">${isPrimary ? '★ ' : ''}${body.name.replace(/^System #\d+\s*/, '')}</div>
          <div class="sys-body-sub">${body.orbitAu.toFixed(2)} AU${moonTag} ${icons}</div>
        </div>
      `;
      const c = card.querySelector<HTMLCanvasElement>('canvas');
      if (c) {
        renderProceduralThumbnailTo2d(c, {
          kind: 'body',
          body,
          star: sys.star,
          companionStar: sys.companionStar,
        });
      }
      card.addEventListener('click', () => this.selectModalItem({ bodyIdx: idx, moonIdx: null }));
      strip.appendChild(card);
    });

    this.setSystemModalTab('bodies');
    this.selectModalItem({ bodyIdx: sys.primaryIndex, moonIdx: null });
  }

  private setSysPreviewExpanded(expanded: boolean): void {
    const box = getEl('sysPreviewBox');
    box.classList.toggle('expanded', expanded);
    getEl('btnToggleSysExpand').textContent = expanded ? '↙ Collapse' : '⛶ Expand';
  }

  private setSystemModalTab(tab: 'bodies' | 'orrery'): void {
    if (tab === 'orrery') {
      this.setSysPreviewExpanded(false);
    }
    getEl('sysTabBodies').classList.toggle('active', tab === 'bodies');
    getEl('sysTabOrrery').classList.toggle('active', tab === 'orrery');
    getEl('sysBodiesPane').style.display = tab === 'bodies' ? 'flex' : 'none';
    getEl('sysOrreryPane').style.display = tab === 'orrery' ? 'flex' : 'none';
    if (tab === 'orrery' && this.activeModalSystem) {
      this.sysPreviewRenderer.stop();
      this.sysOrreryRenderer.setSystem(this.activeModalSystem, true);
    } else {
      this.sysOrreryRenderer.stop();
      if (this.activeModalSystem) {
        this.selectModalItem(this.activeModalSelection);
      }
    }
  }

  public closeSystemView(): void {
    this.setSysPreviewExpanded(false);
    getEl('systemModal').style.display = 'none';
    this.sysPreviewRenderer.stop();
    this.sysOrreryRenderer.stop();
    this.activeModalSystem = null;
  }

  private selectModalItem(which: 'star' | 'star2' | { bodyIdx: number; moonIdx: number | null }): void {
    const sys = this.activeModalSystem;
    if (!sys) return;
    this.activeModalSelection = which;

    const selectedOrbitKey =
      which === 'star' || which === 'star2' ? which : String(which.bodyIdx);
    getEl('sysOrbitStrip')
      .querySelectorAll<HTMLElement>('.sys-body-card')
      .forEach((el) => {
        el.classList.toggle('selected', el.dataset.sysItem === selectedOrbitKey);
      });

    const statsEl = getEl('sysBodyStats');
    const exportBtn = getEl('btnExportPlanetPng');
    const moonsWrap = getEl('sysMoonsWrap');
    const backBtn = getEl('btnBackToParentPlanet');

    if (which === 'star' || which === 'star2') {
      const st = which === 'star2' && sys.companionStar ? sys.companionStar : sys.star;
      const kindLabel =
        st.starKind === 'BlackHole'
          ? 'Stellar Black Hole'
          : st.starKind === 'Pulsar'
            ? 'Pulsar / Neutron Star'
            : st.starKind === 'WhiteDwarf'
              ? 'White Dwarf'
              : st.starKind === 'BrownDwarf'
                ? 'Brown Dwarf'
                : 'Main Sequence Star';
      getEl('sysSelectedLabel').textContent =
        `${which === 'star2' ? 'Companion Star B' : 'Primary Host'} • ${st.spectralClass}`;
      exportBtn.style.display = 'none';
      moonsWrap.style.display = 'none';
      this.sysPreviewRenderer.setTarget({ kind: 'star', star: st }, true);
      statsEl.innerHTML = `
        <div class="insp-stat"><span>Object Type</span><b>${kindLabel}</b></div>
        <div class="insp-stat"><span>Spectral Class</span><b>${st.spectralClass}</b></div>
        <div class="insp-stat"><span>${st.starKind === 'BlackHole' ? 'Disk Temp' : 'Surface Temp'}</span><b>${Math.round(st.tempK)} K</b></div>
        <div class="insp-stat"><span>Mass</span><b>${st.massSolar.toFixed(2)} M<sub class="astro-sub">☉</sub></b></div>
        <div class="insp-stat"><span>Radius</span><b>${st.radiusSolar < 0.01 ? `${(st.radiusSolar * 696340).toFixed(0)} km` : `${st.radiusSolar.toFixed(2)} R<sub class="astro-sub">☉</sub>`}</b></div>
        <div class="insp-stat"><span>Luminosity</span><b>${st.luminositySolar.toFixed(3)} L<sub class="astro-sub">☉</sub></b></div>
      `;
      return;
    }

    const body = sys.bodies[which.bodyIdx];
    if (!body) return;
    const isSpecialNonSphere =
      body.bodyType === 'AsteroidBelt' ||
      body.bodyType === 'KuiperBelt' ||
      body.bodyType === 'Comet';
    exportBtn.style.display = isSpecialNonSphere ? 'none' : 'inline-block';

    // Populate Moons strip for the parent planet
    const moons = body.moons ?? [];
    if (moons.length > 0) {
      moonsWrap.style.display = 'flex';
      getEl('sysMoonsTitle').textContent = `Moons of ${body.name.replace(/^System #\d+\s*/, '')} (${moons.length})`;
      backBtn.style.display = which.moonIdx !== null ? 'inline-block' : 'none';
      const moonsStrip = getEl('sysMoonsStrip');
      moonsStrip.innerHTML = '';
      moons.forEach((moon, mIdx) => {
        const card = document.createElement('div');
        card.className = `sys-moon-card${which.moonIdx === mIdx ? ' selected' : ''}`;
        const shortName = moon.name.replace(/^System #\d+\s*/, '');
        card.innerHTML = `
          <canvas class="sys-moon-thumb" width="64" height="64"></canvas>
          <div class="sys-body-name">${shortName}</div>
          <div class="sys-body-sub">${moon.bodyType}</div>
        `;
        const mc = card.querySelector<HTMLCanvasElement>('canvas');
        if (mc) {
          renderProceduralThumbnailTo2d(mc, {
            kind: 'moon',
            moon,
            star: sys.star,
            companionStar: sys.companionStar,
          });
        }
        card.addEventListener('click', () =>
          this.selectModalItem({ bodyIdx: which.bodyIdx, moonIdx: mIdx }),
        );
        moonsStrip.appendChild(card);
      });
    } else {
      moonsWrap.style.display = 'none';
    }

    if (which.moonIdx !== null) {
      const moon = moons[which.moonIdx];
      if (!moon) return;
      getEl('sysSelectedLabel').textContent = `${moon.name} (Moon)`;
      this.sysPreviewRenderer.setTarget(
        { kind: 'moon', moon, star: sys.star, companionStar: sys.companionStar },
        true,
      );
      statsEl.innerHTML = `
        <div class="insp-stat"><span>Classification</span><b>${moon.bodyType} Moon</b></div>
        <div class="insp-stat"><span>Parent Orbit</span><b>${moon.orbitRadiusPlanet.toFixed(2)} R<sub class="astro-sub">p</sub></b></div>
        <div class="insp-stat"><span>Radius</span><b>${moon.radiusEarth.toFixed(2)} R<sub class="astro-sub">⊕</sub></b></div>
        <div class="insp-stat"><span>Surface Temp</span><b>${Math.round(moon.tempK)} K (${Math.round(moon.tempK - 273.15)}°C)</b></div>
        <div class="insp-stat"><span>Ice / Water</span><b>${Math.round(moon.icePct * 100)}% ice · ${Math.round(moon.waterPct * 100)}% water</b></div>
        <div class="insp-stat"><span>Atmo / Clouds</span><b>${Math.round(moon.atmoDensity * 100)}% atmo · ${Math.round(moon.cloudPct * 100)}% clouds</b></div>
      `;
      return;
    }

    const isPrimary = which.bodyIdx === sys.primaryIndex;
    const ls = body.liveState;
    getEl('sysSelectedLabel').textContent = `${body.name}${isPrimary ? ' (Primary World)' : ''}`;
    this.sysPreviewRenderer.setTarget(
      { kind: 'body', body, star: sys.star, companionStar: sys.companionStar },
      true,
    );

    if (body.bodyType === 'AsteroidBelt' || body.bodyType === 'KuiperBelt') {
      const isKuiper = body.bodyType === 'KuiperBelt';
      statsEl.innerHTML = `
        <div class="insp-stat"><span>Classification</span><b>${isKuiper ? 'Kuiper Belt' : 'Main Asteroid Belt'}</b></div>
        <div class="insp-stat"><span>Mean Orbit</span><b>${body.orbitAu.toFixed(2)} AU</b></div>
        <div class="insp-stat"><span>Composition</span><b>${isKuiper ? 'Water/Methane Ice & Rock' : 'Iron-Nickel & Silicates'}</b></div>
        <div class="insp-stat"><span>Mean Temp</span><b>${Math.round(body.tempK)} K (${Math.round(body.tempK - 273.15)}°C)</b></div>
        <div class="insp-stat"><span>Ice Fraction</span><b>${Math.round(body.icePct * 100)}%</b></div>
        <div class="insp-stat"><span>Mining Yield</span><b>${isKuiper ? 'Volatiles / Low Metals' : 'Rich Fe / Bo / Ge'}</b></div>
      `;
      return;
    }

    if (body.bodyType === 'Comet') {
      statsEl.innerHTML = `
        <div class="insp-stat"><span>Classification</span><b>Eccentric Comet</b></div>
        <div class="insp-stat"><span>Orbit Zone</span><b>${body.orbitAu.toFixed(2)} AU</b></div>
        <div class="insp-stat"><span>Nucleus Radius</span><b>~${Math.round(body.radiusEarth * 6371)} km</b></div>
        <div class="insp-stat"><span>Surface Temp</span><b>${Math.round(body.tempK)} K (${Math.round(body.tempK - 273.15)}°C)</b></div>
        <div class="insp-stat"><span>Volatile Ice</span><b>${Math.round(body.icePct * 100)}%</b></div>
        <div class="insp-stat"><span>Activity</span><b>Dual Dust + Ion Tail</b></div>
      `;
      return;
    }

    const liveCells = ls
      ? `
        <div class="insp-stat"><span>Gravity / Rad</span><b>${ls.gravG.toFixed(2)} g • ${Math.round(ls.radMr)} mR</b></div>
        <div class="insp-stat"><span>Colony / Terra</span><b style="color:${ls.ownerColor}">${ls.owner > 0 ? `${Math.round(ls.popNorm * 100)}% pop` : 'Uninhabited'}${ls.terraPct > 0.5 ? ` · +${Math.round(ls.terraPct)}% terra` : ''}</b></div>
        <div class="insp-stat"><span>Orbital Infra</span><b>${ls.hasStargate ? '🌀 Gate ' : ''}${ls.hasJammer ? '📡 ECM ' : ''}${ls.hasStarbase ? '🛡 Base' : !ls.hasStargate && !ls.hasJammer ? 'None' : ''}</b></div>
      `
      : '';

    statsEl.innerHTML = `
      <div class="insp-stat"><span>Classification</span><b>${body.bodyType}</b></div>
      <div class="insp-stat"><span>Orbit Distance</span><b>${body.orbitAu.toFixed(2)} AU</b></div>
      <div class="insp-stat"><span>Radius</span><b>${body.radiusEarth.toFixed(2)} R<sub class="astro-sub">⊕</sub></b></div>
      <div class="insp-stat"><span>Surface Temp</span><b>${Math.round(body.tempK)} K (${Math.round(body.tempK - 273.15)}°C)</b></div>
      <div class="insp-stat"><span>Hydrosphere / Ice</span><b>${Math.round(body.waterPct * 100)}% water · ${Math.round(body.icePct * 100)}% ice</b></div>
      <div class="insp-stat"><span>Atmo / Clouds</span><b>${Math.round(body.atmoDensity * 100)}% atmo · ${Math.round(body.cloudPct * 100)}% clouds</b></div>
      <div class="insp-stat"><span>Moons / Rings</span><b>${body.moonsCount} moons · ${body.hasRings ? 'Ringed' : 'No rings'}</b></div>
      ${liveCells}
    `;
  }

  private setFleetModalMode(mode: ShipViewMode): void {
    this.modalShipRenderer.setMode(mode);
    getEl('fleetModeSolid').classList.toggle('active', mode === 'solid');
    getEl('fleetModeBlueprint').classList.toggle('active', mode === 'blueprint');
  }

  private getReportsRightInset(): number {
    const drawer = getEl('drawerReports');
    return drawer.classList.contains('open') && window.innerWidth > 768
      ? drawer.getBoundingClientRect().width
      : 0;
  }

  private focusWorldPointOnMap(x: number, y: number, pad = 55): void {
    this.callbacks.onFocusWorldRegion(x - pad, y - pad, x + pad, y + pad, this.getReportsRightInset());
  }

  private focusWorldBoxOnMap(minX: number, minY: number, maxX: number, maxY: number): void {
    this.callbacks.onFocusWorldRegion(minX, minY, maxX, maxY, this.getReportsRightInset());
  }

  private async focusFleetRouteOnMap(
    fleetId: number,
    fallbackFromIdx: number | null,
    fallbackTargetIdx: number | null,
  ): Promise<void> {
    const fl = await this.callbacks.getFleetById(fleetId);
    const fromIdx = fl ? fl.from : fallbackFromIdx;
    const targetIdx = fl ? fl.target : fallbackTargetIdx;
    const [fromPl, targetPl] = await Promise.all([
      fromIdx !== null ? this.callbacks.getPlanetDetail(fromIdx) : Promise.resolve(null),
      targetIdx !== null ? this.callbacks.getPlanetDetail(targetIdx) : Promise.resolve(null),
    ]);
    const xs: number[] = [];
    const ys: number[] = [];
    if (fl) {
      xs.push(fl.x);
      ys.push(fl.y);
    }
    if (fromPl) {
      xs.push(fromPl.x);
      ys.push(fromPl.y);
    }
    if (targetPl) {
      xs.push(targetPl.x);
      ys.push(targetPl.y);
    }
    if (xs.length === 0) return;

    // If the fleet already arrived at its destination, select the destination planet & refresh table
    if (!fl && targetIdx !== null) {
      this.callbacks.onSelectObject({ kind: 'planet', index: targetIdx });
      if (this.lastHud) this.renderOpenDrawers(this.lastHud);
    }

    const minX = Math.min(...xs);
    const maxX = Math.max(...xs);
    const minY = Math.min(...ys);
    const maxY = Math.max(...ys);

    const drawer = getEl('drawerReports');
    const rightInset =
      drawer.classList.contains('open') && window.innerWidth > 768
        ? drawer.getBoundingClientRect().width
        : 0;
    this.callbacks.onFocusWorldRegion(minX, minY, maxX, maxY, rightInset);
  }

  private setFleetPreviewExpanded(expanded: boolean): void {
    const box = getEl('fleetPreviewBox');
    box.classList.toggle('expanded', expanded);
    getEl('btnToggleFleetExpand').textContent = expanded ? '↙ Collapse' : '⛶ Expand';
    getEl('fleetPreviewHint').textContent = expanded
      ? 'Drag to orbit · Scroll to zoom · Dbl-click or Esc to collapse'
      : 'Drag to orbit · Scroll to zoom · Dbl-click to expand';
  }

  public async openFleetView(fleetId: number): Promise<void> {
    this.activeModalFleetId = fleetId;
    this.setFleetPreviewExpanded(false);
    this.modalShipRenderer.resetCamera();
    this.setFleetModalMode('solid');
    getEl('planetInspector').style.display = 'none';
    getEl('inspShipWrap').style.display = 'none';
    this.inspShipRenderer.stop();
    getEl('fleetModal').style.display = 'flex';
    this.modalShipRenderer.start();
    if (this.lastHud) {
      await this.refreshFleetModal(this.lastHud, true);
    }
  }

  public closeFleetView(): void {
    this.activeModalFleetId = null;
    this.fleetModalReqSeq++;
    this.setFleetPreviewExpanded(false);
    getEl('fleetModal').style.display = 'none';
    this.modalShipRenderer.stop();
    if (this.selection && this.lastHud) {
      void this.updateInspector(this.lastHud);
    }
  }

  private async refreshFleetModal(hud: HudSnapshot, isInitial: boolean): Promise<void> {
    const fleetId = this.activeModalFleetId;
    if (fleetId === null) return;
    const seq = ++this.fleetModalReqSeq;
    const fl = await this.callbacks.getFleetById(fleetId);
    if (seq !== this.fleetModalReqSeq || this.activeModalFleetId !== fleetId) return;

    if (!fl) {
      if (isInitial) {
        this.closeFleetView();
      } else {
        getEl('fleetModalSub').textContent = 'Fleet arrived at destination or destroyed in combat';
      }
      return;
    }

    const emp = hud.empires[fl.emp - 1];
    const minNames = ['Ironium', 'Boranium', 'Germanium'];
    const isSalvage = fl.type === 'freight' && Boolean(fl.salvageMode);
    const salvageCargo = Math.round((fl.cargoIron ?? 0) + (fl.cargoBora ?? 0) + (fl.cargoGerm ?? 0));
    const hullName = emp
      ? fl.type === 'battleship'
        ? emp.loadout.battleshipHull
        : fl.type === 'cruiser'
          ? emp.loadout.cruiserHull
          : fl.type === 'bomber'
            ? emp.loadout.bomberHull
            : fl.type === 'freight'
              ? isSalvage
                ? 'Salvage Freighter'
                : `Freighter (${minNames[fl.minType] ?? 'Mineral'})`
              : 'Colony Ship'
      : FLEET_LABELS[fl.type];

    const hpRatio = fl.maxPower > 0 ? Math.max(0, Math.min(1, fl.power / fl.maxPower)) : 1;
    const hpPct = Math.round(hpRatio * 100);
    const isArmed = fl.type === 'cruiser' || fl.type === 'battleship' || fl.type === 'bomber';
    const cloakFrac = emp ? (isArmed ? emp.loadout.cloakArmedPct : emp.loadout.cloakUnarmedPct) : 0;
    const tachFrac = emp ? emp.loadout.tachyonReductionPct : 0;
    const jamFrac = emp ? emp.loadout.torpedoJammingPct : 0;

    this.modalShipRenderer.setState({
      fleetClass: fl.type,
      empireColor: emp?.color ?? '#38bdf8',
      hpRatio,
      cloakPct: cloakFrac,
      jammerPct: jamFrac,
      cargoMineralType: fl.type === 'freight' ? fl.minType : undefined,
    });

    const targetPl = await this.callbacks.getPlanetDetail(fl.target);
    if (seq !== this.fleetModalReqSeq || this.activeModalFleetId !== fleetId) return;
    const dist = fl.orbiting
      ? 0
      : fl.salvageMode === 1
        ? Math.hypot(fl.steerX - fl.x, fl.steerY - fl.y)
        : targetPl
          ? Math.hypot(targetPl.x - fl.x, targetPl.y - fl.y)
          : 0;
    const eta = dist / Math.max(1, this.lastConfig.fleetSpeed);
    const missionTxt = fl.orbiting
      ? `🛰 Stationed in Orbit (#${fl.target + 1})`
      : fl.salvageMode === 1
        ? '✦ Salvage Outbound'
        : fl.salvageMode === 2
          ? '✦ Salvage Return'
          : fl.type === 'freight'
            ? `■ Transport ${minNames[fl.minType] ?? 'Minerals'}`
            : fl.type === 'col'
              ? targetPl && targetPl.owner === fl.emp
                ? '● Reinforce Colony'
                : '● Colonize'
              : fl.hunting
                ? '🎯 Intercept Fleet'
                : fl.type === 'bomber'
                  ? '★ Orbital Bombardment'
                  : '⚔ Planetary Assault';
    const routeTag = !fl.orbiting && fl.whRoute >= 0 ? ' (🌀 WH)' : !fl.orbiting && fl.usedWh ? ' (⚡ Jump)' : '';
    const routeTxt = fl.orbiting
      ? `Orbit #${fl.target + 1} (Docked)`
      : fl.salvageMode === 1
        ? `#${fl.from + 1} → (${Math.round(fl.steerX)}, ${Math.round(fl.steerY)}) (${eta.toFixed(1)} yr)`
        : fl.salvageMode === 2
          ? `Wreck → #${fl.target + 1} (${eta.toFixed(1)} yr)`
          : `#${fl.from + 1} → #${fl.target + 1} (${eta.toFixed(1)} yr)`;
    const cargoTxt = isSalvage
      ? `${salvageCargo} / ${Math.round(fl.maxPower)} kT (I:${Math.round(fl.cargoIron ?? 0)} B:${Math.round(fl.cargoBora ?? 0)} G:${Math.round(fl.cargoGerm ?? 0)})`
      : fl.type === 'freight'
        ? fl.orbiting
          ? `0 / ${Math.round(fl.maxPower)} kT (Idle)`
          : `${Math.round(fl.power)} kT ${minNames[fl.minType] ?? ''}`
        : `${Math.round(fl.power)} / ${Math.round(fl.maxPower)} (${hpPct}%)`;
    const upkeepTxt = fl.upkeep !== undefined ? ` · Upkeep ${fl.upkeep.toFixed(1)} Res/yr` : '';

    getEl('fleetModalTitle').textContent = `🚀 Fleet F#${fl.id} • ${isSalvage ? '✦ Salvage Freighter' : FLEET_LABELS[fl.type]}`;
    getEl('fleetModalSub').textContent = `${hullName} • ${emp ? emp.name : `Empire #${fl.emp}`} • ${missionTxt}${routeTag}${upkeepTxt}`;

    getEl('fleetModalStats').innerHTML = `
      <div class="insp-stat"><span>Owner</span><b style="color:${emp?.color ?? '#e2e8f0'}">${emp ? `${emp.name} (${shortPrt(emp.prt)})` : `Empire #${fl.emp}`}</b></div>
      <div class="insp-stat"><span>Hull / Upkeep</span><b>${hullName}${fl.upkeep !== undefined ? ` (${fl.upkeep.toFixed(1)} R/yr)` : ''}</b></div>
      <div class="insp-stat"><span>Route &amp; ETA</span><b>${routeTxt}</b></div>
      <div class="insp-stat"><span>Mission / Dist</span><b>${missionTxt}${routeTag}${fl.orbiting ? '' : ` · ${Math.round(dist)} ly`}</b></div>
      <div class="insp-stat"><span>${fl.type === 'freight' ? 'Cargo Hold' : 'Power / HP'}</span><b style="color:${isSalvage ? '#c084fc' : 'var(--accent)'}">${cargoTxt}</b></div>
      <div class="insp-stat"><span>EW (Cloak / Tach / Jam)</span><b>${Math.round(cloakFrac * 100)}% / ${Math.round(tachFrac * 100)}% / ${Math.round(jamFrac * 100)}%</b></div>
    `;

    if (emp) {
      const lo = emp.loadout;
      const rangeVal =
        fl.type === 'battleship'
          ? lo.rangeBattleship
          : fl.type === 'cruiser'
            ? lo.rangeCruiser
            : fl.type === 'bomber'
              ? lo.rangeBomber
              : lo.rangeCol;
      getEl('fleetModalLoadout').innerHTML = `
        <div class="insp-stat"><span>Propulsion</span><b>${lo.engine}</b></div>
        <div class="insp-stat"><span>${fl.type === 'bomber' ? 'Ordnance / Bombs' : 'Weapons'}</span><b>${fl.type === 'bomber' ? lo.bomb : lo.weapon}</b></div>
        <div class="insp-stat"><span>Armor / Shields</span><b>${lo.armor} / ${lo.shield}</b></div>
        <div class="insp-stat"><span>Scanner</span><b>${lo.scanner}</b></div>
        <div class="insp-stat"><span>Fuel Range</span><b>${Math.round(rangeVal)} ly</b></div>
        <div class="insp-stat"><span>Stargate Network</span><b>${lo.stargate}</b></div>
      `;
    } else {
      getEl('fleetModalLoadout').innerHTML = '';
    }
  }

  private async updateInspector(hud: HudSnapshot): Promise<void> {
    const sel = this.selection;
    const reqSeq = ++this.inspectorReqSeq;
    const insp = getEl('planetInspector');
    const minWrap = getEl('inspMineralsWrap');
    const debrisHistWrap = getEl('inspDebrisHistoryWrap');
    const planetFleetsWrap = getEl('inspPlanetFleetsWrap');
    if (!sel || this.activeModalFleetId !== null) {
      insp.style.display = 'none';
      getEl('inspGlobeWrap').style.display = 'none';
      getEl('inspShipWrap').style.display = 'none';
      debrisHistWrap.style.display = 'none';
      planetFleetsWrap.style.display = 'none';
      this.inspGlobeRenderer.stop();
      this.inspShipRenderer.stop();
      return;
    }

    if (sel.kind !== 'debris') {
      debrisHistWrap.style.display = 'none';
    }
    if (sel.kind !== 'planet') {
      planetFleetsWrap.style.display = 'none';
    }

    if (sel.kind === 'planet') {
      getEl('inspShipWrap').style.display = 'none';
      this.inspShipRenderer.stop();
      if (sel.index < 0 || sel.index >= hud.planetCount) {
        insp.style.display = 'none';
        getEl('inspGlobeWrap').style.display = 'none';
        this.inspGlobeRenderer.stop();
        return;
      }
      const [pl, sys, orbitFleets] = await Promise.all([
        this.callbacks.getPlanetDetail(sel.index),
        this.callbacks.getStarSystem(sel.index),
        this.callbacks.getPlanetFleets(sel.index),
      ]);
      if (reqSeq !== this.inspectorReqSeq || !this.selection) return;
      if (!pl) {
        insp.style.display = 'none';
        getEl('inspGlobeWrap').style.display = 'none';
        this.inspGlobeRenderer.stop();
        return;
      }
      insp.style.display = 'flex';
      minWrap.style.display = 'flex';
      if (sys) {
        const primary = sys.bodies[sys.primaryIndex];
        getEl('inspGlobeWrap').style.display = 'block';
        const compTag = sys.companionStar ? ` + ${sys.companionStar.spectralClass}` : '';
        getEl('inspStarBadge').textContent =
          sys.star.starKind === 'BlackHole'
            ? `BH • ${sys.star.massSolar.toFixed(1)} M☉`
            : `${sys.star.spectralClass}${compTag} • ${Math.round(sys.star.tempK)}K`;
        getEl('inspBodyTypeBadge').textContent = primary ? `${primary.bodyType} • ${Math.round(primary.tempK)}K` : '';
        const ls = primary?.liveState;
        const badges: string[] = [];
        if (ls?.hasStargate) badges.push('<span class="insp-badge" title="Active Stargate">🌀</span>');
        if (ls?.hasJammer) badges.push('<span class="insp-badge" title="Active ECM Jammer">📡</span>');
        if (ls?.hasStarbase) badges.push('<span class="insp-badge" title="Orbital Starbase">🛡</span>');
        getEl('inspOrbitBadges').innerHTML = badges.join('');
        if (primary) {
          this.inspGlobeRenderer.setTarget(
            { kind: 'body', body: primary, star: sys.star, companionStar: sys.companionStar },
            true,
          );
        }
      } else {
        getEl('inspGlobeWrap').style.display = 'none';
        this.inspGlobeRenderer.stop();
      }
      const emp = pl.owner > 0 ? hud.empires[pl.owner - 1] : null;
      insp.style.borderLeftColor = emp ? emp.color : '#64748b';
      getEl('inspTitle').textContent = `System #${pl.id}`;
      this.setInspCell(0, 'Owner', emp ? `${emp.name} (${shortPrt(emp.prt)})` : 'Uninhabited', emp ? emp.color : '#94a3b8');
      this.setInspCell(1, 'Habitability', `${Math.round(pl.hab * 100)}%`);
      this.setInspCell(2, 'Population', `${Math.round(pl.pop)}k / ${Math.round(pl.maxPop)}k`);
      this.setInspCell(3, 'Production', `${Math.round(pl.res)} Res/yr`, 'var(--accent)');
      this.setInspCell(4, 'Factories / Mines', `${Math.round(pl.factories)} fac / ${Math.round(pl.mines)} mine`);
      const gateTxt = pl.hasGate ? '🌀 Gate' : 'No Gate';
      const sbTxt =
        (pl.starbaseTier ?? 0) > 0
          ? ` · 🛡 ${pl.starbaseName} (${Math.round(((pl.starbaseHp ?? 0) / Math.max(1, pl.starbaseMaxHp ?? 1)) * 100)}%)`
          : '';
      const orbTxt = (pl.orbitingFleets ?? 0) > 0 ? ` · 🛰 ${pl.orbitingFleets}` : '';
      const ecmTxt = pl.hasEcmJammer && emp
        ? ` · 📡 ECM ${Math.round(emp.loadout.ecmJammerStrength * 100)}% (${Math.round(emp.loadout.ecmJammerRange)}ly)`
        : '';
      this.setInspCell(5, 'Defense / Garrison', `${Math.round(pl.defense)}${sbTxt}${orbTxt} | ${gateTxt}${ecmTxt}`);

      const maxC = this.lastConfig.crustRichness * 1.6;
      getEl('barIron').style.width = `${Math.min(100, (pl.ironCrust / maxC) * 100)}%`;
      getEl('valIron').textContent = `${Math.round(pl.ironSurf)} (${Math.round(pl.ironCrust)})`;
      getEl('barBora').style.width = `${Math.min(100, (pl.boraCrust / maxC) * 100)}%`;
      getEl('valBora').textContent = `${Math.round(pl.boraSurf)} (${Math.round(pl.boraCrust)})`;
      getEl('barGerm').style.width = `${Math.min(100, (pl.germCrust / maxC) * 100)}%`;
      getEl('valGerm').textContent = `${Math.round(pl.germSurf)} (${Math.round(pl.germCrust)})`;

      if (orbitFleets.length > 0) {
        planetFleetsWrap.style.display = 'flex';
        const arrow = this.planetFleetsExpanded ? '▾' : '▸';
        getEl('inspOrbitToggleLabel').textContent = `${arrow} Orbiting Fleets (${orbitFleets.length})`;
        const hint = getEl('btnTogglePlanetFleets').querySelector('.insp-wreck-hint');
        if (hint) hint.textContent = this.planetFleetsExpanded ? 'Hide' : 'Show';
        getEl('inspPlanetFleetsBody').style.display = this.planetFleetsExpanded ? 'block' : 'none';
        const classColors: Record<FleetClass, string> = {
          battleship: '#f87171',
          cruiser: '#93c5fd',
          bomber: '#34d399',
          freight: '#c084fc',
          col: '#fbbf24',
        };
        getEl('inspPlanetFleetsTbody').innerHTML = orbitFleets
          .map((fl) => {
            const hpPct = fl.maxPower > 0 ? Math.round((fl.power / fl.maxPower) * 100) : 100;
            const col = classColors[fl.type] ?? '#e2e8f0';
            const statTxt =
              fl.type === 'freight'
                ? `Cap ${Math.round(fl.maxPower)} kT`
                : `${Math.round(fl.power)} (${hpPct}%)`;
            return `<tr data-fleet-id="${fl.id}" title="Open 3D Fleet View">
              <td>F#${fl.id}</td>
              <td style="color:${col};font-weight:bold;">${FLEET_LABELS[fl.type]}</td>
              <td>${statTxt}</td>
            </tr>`;
          })
          .join('');
      } else {
        planetFleetsWrap.style.display = 'none';
      }
      return;
    }

    if (sel.kind === 'fleet') {
      getEl('inspGlobeWrap').style.display = 'none';
      this.inspGlobeRenderer.stop();
      const fl = await this.callbacks.getFleetById(sel.id);
      if (reqSeq !== this.inspectorReqSeq || !this.selection) return;
      if (!fl) {
        this.selection = null;
        insp.style.display = 'none';
        getEl('inspShipWrap').style.display = 'none';
        this.inspShipRenderer.stop();
        return;
      }
      insp.style.display = 'flex';
      minWrap.style.display = 'none';
      const emp = hud.empires[fl.emp - 1];
      insp.style.borderLeftColor = emp ? emp.color : '#38bdf8';
      const minNames = ['Ironium', 'Boranium', 'Germanium'];
      const hullName = emp
        ? fl.type === 'battleship'
          ? emp.loadout.battleshipHull
          : fl.type === 'cruiser'
            ? emp.loadout.cruiserHull
            : fl.type === 'bomber'
              ? emp.loadout.bomberHull
              : fl.type === 'freight'
                ? fl.salvageMode
                  ? 'Salvage Freighter'
                  : `Freighter (${minNames[fl.minType] ?? 'Mineral'})`
                : 'Colony Ship'
        : FLEET_LABELS[fl.type];
      const hpRatio = fl.maxPower > 0 ? Math.max(0, Math.min(1, fl.power / fl.maxPower)) : 1;
      const hpPct = Math.round(hpRatio * 100);
      const isArmed = fl.type === 'cruiser' || fl.type === 'battleship' || fl.type === 'bomber';
      const cloakFrac = emp ? (isArmed ? emp.loadout.cloakArmedPct : emp.loadout.cloakUnarmedPct) : 0;
      const tachFrac = emp ? emp.loadout.tachyonReductionPct : 0;
      const jamFrac = emp ? emp.loadout.torpedoJammingPct : 0;
      const cloakPct = Math.round(cloakFrac * 100);
      const tachPct = Math.round(tachFrac * 100);
      const jamPct = Math.round(jamFrac * 100);

      getEl('inspShipWrap').style.display = 'block';
      getEl('inspShipClassBadge').textContent = FLEET_LABELS[fl.type];
      getEl('inspShipHullBadge').textContent = hullName;
      const ewBadges: string[] = [];
      if (cloakPct > 0) ewBadges.push(`Cloak ${cloakPct}%`);
      if (jamPct > 0) ewBadges.push(`Jam ${jamPct}%`);
      ewBadges.push(`HP ${hpPct}%`);
      getEl('inspShipEwBadge').textContent = ewBadges.join(' · ');
      this.inspShipRenderer.setState({
        fleetClass: fl.type,
        empireColor: emp?.color ?? '#38bdf8',
        hpRatio,
        cloakPct: cloakFrac,
        jammerPct: jamFrac,
        cargoMineralType: fl.type === 'freight' ? fl.minType : undefined,
      });
      this.inspShipRenderer.start();

      const isSalvage = fl.type === 'freight' && Boolean(fl.salvageMode);
      const salvageCargo = Math.round((fl.cargoIron ?? 0) + (fl.cargoBora ?? 0) + (fl.cargoGerm ?? 0));
      getEl('inspTitle').textContent = `Fleet F#${fl.id} • ${isSalvage ? '✦ Salvage Freighter' : FLEET_LABELS[fl.type]}`;
      this.setInspCell(0, 'Owner', emp ? `${emp.name} (${shortPrt(emp.prt)})` : `Empire #${fl.emp}`, emp ? emp.color : '#e2e8f0');
      this.setInspCell(1, 'Hull / Upkeep', `${hullName}${fl.upkeep !== undefined ? ` (${fl.upkeep.toFixed(1)}R/yr)` : ''}`);
      this.setInspCell(
        2,
        fl.type === 'freight' ? 'Cargo' : 'Power / HP',
        isSalvage
          ? `${salvageCargo} / ${Math.round(fl.maxPower)} kT`
          : fl.type === 'freight'
            ? fl.orbiting
              ? `0 / ${Math.round(fl.maxPower)} kT (Docked)`
              : `${Math.round(fl.power)} kT (${minNames[fl.minType] ?? 'Mineral'})`
            : `${Math.round(fl.power)} / ${Math.round(fl.maxPower)} (${hpPct}%)`,
        isSalvage ? '#c084fc' : 'var(--accent)',
      );
      const targetPl = await this.callbacks.getPlanetDetail(fl.target);
      if (reqSeq !== this.inspectorReqSeq || !this.selection) return;
      const dist = fl.orbiting
        ? 0
        : fl.salvageMode === 1
          ? Math.hypot(fl.steerX - fl.x, fl.steerY - fl.y)
          : targetPl
            ? Math.hypot(targetPl.x - fl.x, targetPl.y - fl.y)
            : 0;
      const eta = dist / Math.max(1, this.lastConfig.fleetSpeed);
      const routeTxt = fl.orbiting
        ? `Orbit #${fl.target + 1} (Docked)`
        : fl.salvageMode === 1
          ? `#${fl.from + 1}→(${Math.round(fl.steerX)},${Math.round(fl.steerY)}) (${eta.toFixed(1)}yr)`
          : fl.salvageMode === 2
            ? `Wreck→#${fl.target + 1} (${eta.toFixed(1)}yr)`
            : `#${fl.from + 1}→#${fl.target + 1} (${eta.toFixed(1)}yr)`;
      this.setInspCell(3, 'Route / ETA', routeTxt);
      const missionTxt = fl.orbiting
        ? '🛰 Orbiting / Garrison'
        : fl.salvageMode === 1
          ? '✦ Salvage Outbound'
          : fl.salvageMode === 2
            ? '✦ Salvage Return'
            : fl.type === 'freight'
              ? `■ Transport ${minNames[fl.minType] ?? 'Minerals'}`
              : fl.type === 'col'
                ? targetPl && targetPl.owner === fl.emp
                  ? '● Reinforce Colony'
                  : '● Colonize'
                : fl.hunting
                  ? '🎯 Intercept Fleet'
                  : fl.type === 'bomber'
                    ? '★ Orbital Bombardment'
                    : '⚔ Planetary Assault';
      const routeTag = !fl.orbiting && fl.whRoute >= 0 ? ' (🌀 WH)' : !fl.orbiting && fl.usedWh ? ' (⚡ Jump)' : '';
      this.setInspCell(4, 'Mission / Dist', fl.orbiting ? missionTxt : `${missionTxt}${routeTag} • ${Math.round(dist)}ly`);
      this.setInspCell(5, 'EW (Cloak/Tach/Jam)', `${cloakPct}% / ${tachPct}% / ${jamPct}%`);
      return;
    }

    getEl('inspGlobeWrap').style.display = 'none';
    getEl('inspShipWrap').style.display = 'none';
    this.inspGlobeRenderer.stop();
    this.inspShipRenderer.stop();

    if (sel.kind === 'wormhole') {
      const wh = hud.wormholes[sel.index];
      if (!wh) {
        this.selection = null;
        insp.style.display = 'none';
        return;
      }
      insp.style.display = 'flex';
      minWrap.style.display = 'none';
      insp.style.borderLeftColor = '#d946ef';
      const span = Math.hypot(wh.x2 - wh.x1, wh.y2 - wh.y1);
      getEl('inspTitle').textContent = `🌀 Wormhole #${sel.index + 1}`;
      this.setInspCell(0, 'Type', 'Natural Wormhole', '#f0abfc');
      this.setInspCell(1, 'Span Distance', `${Math.round(span)} ly`, 'var(--accent)');
      this.setInspCell(2, 'Endpoint A', `(${Math.round(wh.x1)}, ${Math.round(wh.y1)})`);
      this.setInspCell(3, 'Endpoint B', `(${Math.round(wh.x2)}, ${Math.round(wh.y2)})`);
      this.setInspCell(4, 'Stability', 'Bidirectional');
      this.setInspCell(5, 'Trigger Radius', '28 ly (>22% shortcut)');
      return;
    }

    if (sel.kind === 'debris') {
      const deb = await this.callbacks.getDebrisDetail(sel.index);
      if (reqSeq !== this.inspectorReqSeq || !this.selection) return;
      if (!deb) {
        this.selection = null;
        insp.style.display = 'none';
        return;
      }
      insp.style.display = 'flex';
      minWrap.style.display = 'flex';
      insp.style.borderLeftColor = '#c084fc';
      const iron = Math.round(deb.iron);
      const bora = Math.round(deb.bora);
      const germ = Math.round(deb.germ);
      const total = iron + bora + germ;
      getEl('inspTitle').textContent = `✦ Salvage Debris #${sel.index + 1}`;
      this.setInspCell(0, 'Type', 'Battle Wreckage', '#c084fc');
      this.setInspCell(1, 'Total Salvage', `${total} kT`, 'var(--accent)');
      this.setInspCell(2, 'Position', `(${Math.round(deb.x)}, ${Math.round(deb.y)})`);
      this.setInspCell(3, 'Decay TTL', `${deb.ttl.toFixed(1)} yr`);
      const autoSalvage = this.lastConfig.scenarioModifiers?.autoSalvage !== false;
      this.setInspCell(4, 'Collection', autoSalvage ? 'Passing fleets & Freighters' : 'Salvage Freighters');
      this.setInspCell(5, 'Salvage Rate', `${Math.round(this.lastConfig.salvageRate * 100)}%`);

      const barMax = Math.max(25, total);
      getEl('barIron').style.width = `${Math.min(100, (iron / barMax) * 100)}%`;
      getEl('valIron').textContent = `${iron} kT`;
      getEl('barBora').style.width = `${Math.min(100, (bora / barMax) * 100)}%`;
      getEl('valBora').textContent = `${bora} kT`;
      getEl('barGerm').style.width = `${Math.min(100, (germ / barMax) * 100)}%`;
      getEl('valGerm').textContent = `${germ} kT`;

      const wrecks = deb.wrecks ?? [];
      const totalShips = wrecks.reduce((acc, w) => acc + w.count, 0);
      debrisHistWrap.style.display = 'flex';
      const arrow = this.debrisWrecksExpanded ? '▾' : '▸';
      getEl('inspWreckToggleLabel').textContent = `${arrow} Wreckage Manifest (${totalShips} ${totalShips === 1 ? 'ship' : 'ships'})`;
      const hint = getEl('btnToggleDebrisWrecks').querySelector('.insp-wreck-hint');
      if (hint) hint.textContent = this.debrisWrecksExpanded ? 'Hide' : 'Show';
      getEl('inspDebrisWrecksBody').style.display = this.debrisWrecksExpanded ? 'block' : 'none';

      if (wrecks.length === 0) {
        getEl('inspDebrisWrecksTbody').innerHTML =
          '<tr><td colspan="3" style="color:var(--muted);text-align:center;">No ship manifest recorded</td></tr>';
      } else {
        const sorted = [...wrecks].sort(
          (a, b) => b.count - a.count || a.emp - b.emp || a.shipType.localeCompare(b.shipType),
        );
        const classColors: Record<FleetClass, string> = {
          battleship: '#f87171',
          cruiser: '#93c5fd',
          bomber: '#34d399',
          freight: '#c084fc',
          col: '#fbbf24',
        };
        getEl('inspDebrisWrecksTbody').innerHTML = sorted
          .map((w) => {
            const emp = w.emp > 0 ? hud.empires[w.emp - 1] : null;
            const empName = emp ? `${emp.name}` : `Empire #${w.emp}`;
            const empCol = emp?.color ?? '#94a3b8';
            const clsLabel = FLEET_LABELS[w.shipType] ?? w.shipType;
            const clsCol = classColors[w.shipType] ?? '#cbd5e1';
            return `<tr>
              <td style="color:${empCol};"><span class="dot" style="background:${empCol}"></span>${empName}</td>
              <td style="color:${clsCol};font-weight:bold;">${clsLabel}</td>
              <td style="font-weight:bold;color:var(--text);">×${w.count}</td>
            </tr>`;
          })
          .join('');
      }
    }
  }

  private renderTechOverview(hud: HudSnapshot): void {
    const maxLv = this.meta.maxTechLevel || 26;
    getEl('techCardsContainer').innerHTML = hud.empires
      .map((emp) => {
        const rows = this.meta.techFields
          .map((tf) => {
            const lv = emp.techLv[tf.index] ?? 0;
            const pct = Math.min(100, (lv / maxLv) * 100);
            return `<div class="tech-row">
              <span style="width:28px;color:${tf.color};font-weight:bold;">${tf.key}</span>
              <div class="tech-bar"><div class="tech-fill" style="width:${pct}%;background:${tf.color};"></div></div>
              <span style="width:38px;text-align:right;">Lv ${lv.toFixed(1)}</span>
              <span style="width:28px;text-align:right;color:var(--muted);">${Math.round((emp.doctrine.techAlloc[tf.index] ?? 0) * 100)}%</span>
            </div>`;
          })
          .join('');
        const lo = emp.loadout;
        return `<div class="tech-card" style="border-top:2px solid ${emp.color}">
          <div class="tech-card-head">
            <span style="color:${emp.color}">${emp.name} <small style="color:var(--muted);">[${shortPrt(emp.prt)}]</small></span>
            <span style="color:var(--muted);font-family:monospace;">${lo.unlockedCount}/239</span>
          </div>
          ${rows}
          <div class="loadout-mini">
            <span>Eng: ${lo.engine}</span>
            <span>Wep: ${lo.weapon}</span>
            <span>Arm: ${lo.armor}</span>
            <span>Shd: ${lo.shield}</span>
            <span>Def: ${lo.defense}</span>
            <span>Gate: ${lo.stargate}</span>
            <span>Cru: ${lo.cruiserHull}</span>
            <span>Cap: ${lo.battleshipHull}</span>
            <span>Col Rng: ${Math.round(lo.rangeCol)}ly</span>
            <span>Cap Rng: ${Math.round(lo.rangeBattleship)}ly</span>
          </div>
        </div>`;
      })
      .join('');

    getEl('breakthroughsContainer').innerHTML = this.meta.breakthroughs
      .map((bp) => {
        const badges = hud.empires
          .map((emp) => {
            const has = (emp.perks & bp.bit) !== 0;
            return `<span title="${emp.name}" style="display:inline-block;width:9px;height:9px;margin-left:2px;border:1px solid ${emp.color};background:${has ? emp.color : 'transparent'};"></span>`;
          })
          .join('');
        return `<div class="bp-item">
          <div style="display:flex;justify-content:space-between;align-items:center;">
            <b>${bp.icon} ${bp.name}</b><div>${badges}</div>
          </div>
          <div class="bp-req">Requires: ${bp.reqTxt}</div>
          <div style="color:#94a3b8;margin-top:2px;">${bp.desc}</div>
        </div>`;
      })
      .join('');
  }

  private renderTechBrowser(hud: HudSnapshot): void {
    const q = getEl<HTMLInputElement>('techSearch').value.trim().toLowerCase();
    const cat = getEl<HTMLSelectElement>('techCatFilter').value;
    const status = getEl<HTMLSelectElement>('techStatusFilter').value;

    const filtered = this.catalog.entries.filter((entry) => {
      if (cat !== 'all' && entry.categoryLabel !== cat) return false;
      if (q && !entry.name.toLowerCase().includes(q) && !entry.statsSummary.toLowerCase().includes(q)) {
        return false;
      }
      const unlockedByAny = hud.empires.some((emp) => isTechUnlocked(emp.loadout.unlockedBits, entry.id));
      if (status === 'unlocked' && !unlockedByAny) return false;
      if (status === 'locked' && unlockedByAny) return false;
      return true;
    });

    getEl('techBrowserCount').textContent = `Showing ${filtered.length} of ${this.catalog.entries.length} craig-stars technologies`;

    getEl('techBrowserBody').innerHTML = filtered
      .map((entry) => {
        const reqStr = entry.reqLevels.join('/');
        const traits: string[] = [];
        if (entry.acquirable) traits.push('MT');
        if (entry.prtsRequired.length > 0) traits.push(entry.prtsRequired.map(shortPrt).join(','));
        if (entry.lrtsRequired !== 0) traits.push(this.formatLrtCodes(entry.lrtsRequired));
        const traitTxt = traits.length > 0 ? traits.join(' ') : 'All';

        const costTxt = `${entry.cost.ironium}/${entry.cost.boranium}/${entry.cost.germanium}/${entry.cost.resources}`;
        const dots = hud.empires
          .map((emp) => {
            const has = isTechUnlocked(emp.loadout.unlockedBits, entry.id);
            return `<span title="${emp.name}" style="display:inline-block;width:8px;height:8px;margin-left:2px;border:1px solid ${emp.color};background:${has ? emp.color : 'transparent'};"></span>`;
          })
          .join('');

        return `<tr>
          <td style="font-weight:600;color:#f8fafc;">${entry.name}</td>
          <td style="color:var(--muted);">${entry.categoryLabel}</td>
          <td style="text-align:center;">${reqStr}</td>
          <td style="color:var(--purple);">${traitTxt}</td>
          <td>${costTxt}</td>
          <td style="color:#94a3b8;text-align:left;">${entry.statsSummary}</td>
          <td>${dots}</td>
        </tr>`;
      })
      .join('');
  }

  private renderDNATable(hud: HudSnapshot): void {
    getEl('dnaGenInfo').textContent = `DNA Generation: #${hud.dnaGeneration}`;
    getEl('dnaTableBody').innerHTML = hud.empires
      .map((emp) => {
        const d = emp.doctrine;
        let topF = 0;
        for (let f = 1; f < 6; f++) {
          if ((d.techAlloc[f] ?? 0) > (d.techAlloc[topF] ?? 0)) topF = f;
        }
        const tf = this.meta.techFields[topF];
        return `<tr>
          <td style="color:${emp.color};font-weight:bold;">${emp.name}</td>
          <td style="color:var(--muted);">${shortPrt(emp.prt)}</td>
          <td>${Math.round(d.facShare * 100)}%</td>
          <td>${Math.round(d.shipShare * 100)}%</td>
          <td>${Math.round(d.techShare * 100)}%</td>
          <td style="color:#93c5fd;">${Math.round(d.cruiserShare * 100)}%</td>
          <td style="color:#60a5fa;">${Math.round(d.bshipShare * 100)}%</td>
          <td style="color:#34d399;">${Math.round(d.bomberShare * 100)}%</td>
          <td style="color:#c084fc;">${Math.round(d.interceptBias * 100)}%</td>
          <td style="color:${tf.color};font-weight:bold;">${tf.key} (${Math.round((d.techAlloc[topF] ?? 0) * 100)}%)</td>
          <td style="color:${emp.color};font-weight:bold;">${d.winRate}%</td>
          <td style="color:${emp.color};font-weight:bold;">${d.lastFit}</td>
        </tr>`;
      })
      .join('');
  }

  private async renderPowerChart(hud: HudSnapshot): Promise<void> {
    if (this.powerHistoryInFlight) return;
    this.powerHistoryInFlight = true;
    try {
      const canvas = getEl<HTMLCanvasElement>('powerCanvas');
      const colors = hud.empires.map((e) => e.color);
      const history = await this.callbacks.getPowerHistory();
      const maxVal = drawPowerChart(canvas, history, colors);
      getEl('maxResInfo').textContent = `Max: ${Math.round(maxVal)} Res/yr`;
    } finally {
      this.powerHistoryInFlight = false;
    }
  }

  private rerenderCurrentReportTab(scrollToTop: boolean): void {
    if (!this.lastHud) return;
    const hud = this.lastHud;
    const tab = this.activeReportTab;
    const scrollTopIfNeeded = () => {
      if (!scrollToTop) return;
      const wrapIds: Record<ReportTab, string> = {
        planets: 'repPlanetsWrap',
        fleets: 'repFleetsWrap',
        starbases: 'repStarbasesWrap',
        stargates: 'repStargatesWrap',
        wormholes: 'repWormholesWrap',
        debris: 'repDebrisWrap',
      };
      getEl(wrapIds[tab]).scrollTop = 0;
    };

    if (tab === 'wormholes') {
      this.renderWormholeTable(hud);
      scrollTopIfNeeded();
    } else if (this.lastReports) {
      this.renderActiveReportTab(hud, this.lastReports);
      scrollTopIfNeeded();
    } else {
      void this.callbacks.getReportsSnapshot().then((rep) => {
        this.lastReports = rep;
        this.renderActiveReportTab(hud, rep);
        scrollTopIfNeeded();
      });
    }
  }

  private paginateReportRows<T>(tab: ReportTab, rows: T[]): T[] {
    const total = rows.length;
    this.reportTotalRows = total;
    const pageSize = this.reportPageSize;
    const totalPages = Math.max(1, Math.ceil(total / pageSize));
    const page = Math.max(0, Math.min(totalPages - 1, this.reportPage[tab]));
    this.reportPage[tab] = page;

    const start = page * pageSize;
    const end = Math.min(total, start + pageSize);
    getEl('repPagerInfo').textContent =
      total === 0 ? 'Showing 0 of 0' : `Showing ${start + 1}–${end} of ${total}`;
    getEl('repPagerPage').textContent = `${page + 1} / ${totalPages}`;
    getEl<HTMLButtonElement>('repPageFirst').disabled = page <= 0;
    getEl<HTMLButtonElement>('repPagePrev').disabled = page <= 0;
    getEl<HTMLButtonElement>('repPageNext').disabled = page >= totalPages - 1;
    getEl<HTMLButtonElement>('repPageLast').disabled = page >= totalPages - 1;

    return rows.slice(start, end);
  }

  private syncReportRowHighlight(): void {
    const sel = this.selection;
    const bodies = [
      'planetTableBody',
      'fleetTableBody',
      'starbaseTableBody',
      'stargateTableBody',
      'wormholeTableBody',
      'debrisTableBody',
    ];
    for (const id of bodies) {
      const tbody = document.getElementById(id);
      if (!tbody) continue;
      tbody.querySelectorAll<HTMLTableRowElement>('tr').forEach((tr) => {
        let match = false;
        if (sel?.kind === 'planet' && tr.dataset.planetIdx !== undefined) {
          match = Number(tr.dataset.planetIdx) === sel.index;
        } else if (sel?.kind === 'fleet' && tr.dataset.fleetId !== undefined) {
          match = Number(tr.dataset.fleetId) === sel.id;
        } else if (sel?.kind === 'wormhole' && tr.dataset.whIdx !== undefined) {
          match = Number(tr.dataset.whIdx) === sel.index;
        } else if (sel?.kind === 'debris' && tr.dataset.debrisIdx !== undefined) {
          match = Number(tr.dataset.debrisIdx) === sel.index;
        }
        tr.classList.toggle('rep-row-selected', match);
      });
    }
  }

  private renderActiveReportTab(hud: HudSnapshot, rep: ReportsSnapshot): void {
    switch (this.activeReportTab) {
      case 'planets':
        this.renderPlanetTable(hud, rep);
        break;
      case 'fleets':
        this.renderFleetTable(hud, rep);
        break;
      case 'starbases':
        this.renderStarbaseTable(hud, rep);
        break;
      case 'stargates':
        this.renderStargateTable(hud, rep);
        break;
      case 'wormholes':
        this.renderWormholeTable(hud);
        break;
      case 'debris':
        this.renderDebrisTable(rep);
        break;
    }
    this.syncReportRowHighlight();
  }

  private updateReportSortIndicators(onlyTab?: ReportTab): void {
    const selector = onlyTab
      ? `table[data-sort-table="${onlyTab}"] th[data-sort]`
      : 'table[data-sort-table] th[data-sort]';
    document.querySelectorAll<HTMLTableCellElement>(selector).forEach((th) => {
      const table = th.closest('table[data-sort-table]') as HTMLTableElement | null;
      const tab = table?.dataset.sortTable as ReportTab | undefined;
      if (!tab) return;
      const state = this.reportSort[tab];
      const active = th.dataset.sort === state.key;
      th.classList.toggle('sort-asc', active && state.asc);
      th.classList.toggle('sort-desc', active && !state.asc);
    });
  }

  private renderPlanetTable(hud: HudSnapshot, rep: ReportsSnapshot): void {
    const n = rep.planets.length;
    getEl('repBtnPlanets').textContent = `Planets (${n})`;
    const rows = rep.planets.map((pl, idx) => ({
      idx,
      id: pl.id,
      x: pl.x,
      y: pl.y,
      owner: pl.owner,
      gate: pl.hasGate ? 1 : 0,
      hab: Math.round(pl.hab * 100),
      pop: Math.round(pl.pop),
      fac: Math.round(pl.factories),
      actF: Math.round(pl.actFac),
      maxB: Math.round(pl.maxBuildings),
      iron: Math.round(pl.ironSurf),
      ironC: Math.round(pl.ironCrust),
      bora: Math.round(pl.boraSurf),
      boraC: Math.round(pl.boraCrust),
      germ: Math.round(pl.germSurf),
      germC: Math.round(pl.germCrust),
      res: Math.round(pl.res),
      def: Math.round(pl.defense),
    }));

    const { key, asc } = this.reportSort.planets;
    rows.sort((a, b) => (asc ? a[key] - b[key] : b[key] - a[key]));
    const visibleRows = this.paginateReportRows('planets', rows);
    getEl('planetTableBody').innerHTML = visibleRows
      .map((r) => {
        const col = r.owner > 0 ? (hud.empires[r.owner - 1]?.color ?? '#94a3b8') : '#475569';
        return `<tr data-planet-idx="${r.idx}" data-x="${r.x}" data-y="${r.y}">
          <td>#${r.id}</td>
          <td><span class="dot" style="background:${col}"></span></td>
          <td style="color:#22d3ee;text-align:center;">${r.gate ? '🌀' : ''}</td>
          <td>${r.hab}%</td>
          <td>${r.pop}k</td>
          <td>${r.actF}/${r.maxB}</td>
          <td class="c-iron">${r.iron} <span style="color:#64748b">(${r.ironC})</span></td>
          <td class="c-bora">${r.bora} <span style="color:#64748b">(${r.boraC})</span></td>
          <td class="c-germ">${r.germ} <span style="color:#64748b">(${r.germC})</span></td>
          <td style="color:#38bdf8;font-weight:bold;">${r.res}</td>
          <td>${r.def}</td>
        </tr>`;
      })
      .join('');
  }

  private renderFleetTable(hud: HudSnapshot, rep: ReportsSnapshot): void {
    getEl('repBtnFleets').textContent = `Fleets (${rep.fleets.length})`;
    const minLetters = ['I', 'B', 'G'];
    const minCols = ['#60a5fa', '#34d399', '#fbbf24'];

    const filteredFleets = rep.fleets.filter((fl) => {
      if (this.fleetReportFilter === 'flight') return !fl.orbiting;
      if (this.fleetReportFilter === 'orbiting') return Boolean(fl.orbiting);
      return true;
    });

    const rows = filteredFleets.map((fl) => {
      const target = rep.planets[fl.target];
      const dist = fl.orbiting
        ? 0
        : fl.salvageMode === 1
          ? Math.hypot(fl.steerX - fl.x, fl.steerY - fl.y)
          : target
            ? Math.hypot(target.x - fl.x, target.y - fl.y)
            : 0;
      const eta = dist / this.lastConfig.fleetSpeed;
      const label = fl.orbiting
        ? `🛰 ${FLEET_LABELS[fl.type]} (Orbit)`
        : fl.type === 'freight'
          ? fl.salvageMode === 1
            ? '✦ Salvage → Wreck'
            : fl.salvageMode === 2
              ? '✦ Salvage ↩ Home'
              : `■ Transport (${minLetters[fl.minType] ?? 'I'})`
          : fl.type === 'col'
            ? target && target.owner === fl.emp
              ? '● Reinforce'
              : '● Colonize'
            : fl.type === 'bomber'
              ? `★ Bombard${fl.usedWh ? ' ⚡' : ''}`
              : `${FLEET_LABELS[fl.type]} · ${fl.hunting ? 'Intercept 🎯' : 'Assault'}${fl.usedWh ? ' ⚡' : ''}`;
      const cargoVal =
        fl.type === 'freight' && fl.salvageMode
          ? (fl.cargoIron ?? 0) + (fl.cargoBora ?? 0) + (fl.cargoGerm ?? 0)
          : fl.type === 'freight' && fl.orbiting
            ? 0
            : fl.power;
      return { fl, dist, eta, label, cargoVal };
    });

    const { key, asc } = this.reportSort.fleets;
    const dir = asc ? 1 : -1;
    rows.sort((a, b) => {
      let cmp = 0;
      switch (key) {
        case 'id':
          cmp = a.fl.id - b.fl.id;
          break;
        case 'emp':
          cmp = a.fl.emp - b.fl.emp;
          break;
        case 'label':
          cmp = a.label.localeCompare(b.label);
          break;
        case 'route':
          cmp = a.fl.from - b.fl.from || a.fl.target - b.fl.target;
          break;
        case 'power':
          cmp = a.cargoVal - b.cargoVal;
          break;
        case 'dist':
          cmp = a.dist - b.dist;
          break;
        case 'eta':
          cmp = a.eta - b.eta;
          break;
      }
      return (cmp || a.fl.id - b.fl.id) * dir;
    });

    const slice = this.paginateReportRows('fleets', rows);
    getEl('fleetTableBody').innerHTML = slice
      .map(({ fl, dist, eta, label, cargoVal }) => {
        const empCol = hud.empires[fl.emp - 1]?.color ?? '#94a3b8';
        const col =
          fl.type === 'freight'
            ? fl.salvageMode
              ? '#c084fc'
              : minCols[fl.minType] ?? '#60a5fa'
            : fl.type === 'bomber'
              ? '#34d399'
              : fl.type === 'battleship'
                ? '#f87171'
                : '#93c5fd';
        const hpPct = fl.maxPower > 0 ? Math.round((fl.power / fl.maxPower) * 100) : 0;
        const routeTxt = fl.orbiting
          ? `Orbit #${fl.target + 1}`
          : fl.salvageMode === 1
            ? `#${fl.from + 1}→(${Math.round(fl.steerX)},${Math.round(fl.steerY)})`
            : fl.salvageMode === 2
              ? `Wreck→#${fl.target + 1}`
              : `#${fl.from + 1}→#${fl.target + 1}`;
        const powerTxt =
          fl.type === 'freight' && (fl.salvageMode || fl.orbiting)
            ? `${Math.round(cargoVal)}/${Math.round(fl.maxPower)} kT`
            : fl.type === 'freight'
              ? `${Math.round(fl.power)} kT`
              : `${Math.round(fl.power)} (${hpPct}%)`;
        return `<tr data-fleet-id="${fl.id}" data-from-idx="${fl.from}" data-target-idx="${fl.target}" data-orbiting="${fl.orbiting ? '1' : '0'}">
          <td>F${fl.id}</td>
          <td><span class="dot" style="background:${empCol}"></span></td>
          <td style="color:${col};font-weight:bold;">${label}</td>
          <td class="fleet-route-cell" data-route-focus="1" title="Focus route on galaxy map">${routeTxt}</td>
          <td>${powerTxt}</td>
          <td>${fl.orbiting ? '—' : `${Math.round(dist)}ly`}</td>
          <td>${fl.orbiting ? 'Docked' : `${eta.toFixed(1)}yr`}</td>
        </tr>`;
      })
      .join('');
  }

  private renderStarbaseTable(hud: HudSnapshot, rep: ReportsSnapshot): void {
    const dockMults = [0.75, 1.0, 1.25, 1.45, 1.65, 1.9];
    const bases: {
      idx: number;
      pl: PlanetView;
      tier: number;
      hp: number;
      maxHp: number;
      dock: number;
      orbiting: number;
    }[] = [];
    for (let i = 0; i < rep.planets.length; i++) {
      const pl = rep.planets[i];
      const tier = pl.starbaseTier ?? 0;
      if (tier > 0) {
        bases.push({
          idx: i,
          pl,
          tier,
          hp: Math.round(pl.starbaseHp ?? 0),
          maxHp: Math.max(1, Math.round(pl.starbaseMaxHp ?? 1)),
          dock: dockMults[tier] ?? 1.0,
          orbiting: pl.orbitingFleets ?? 0,
        });
      }
    }

    const { key, asc } = this.reportSort.starbases;
    const dir = asc ? 1 : -1;
    bases.sort((a, b) => {
      let cmp = 0;
      switch (key) {
        case 'id':
          cmp = a.pl.id - b.pl.id;
          break;
        case 'owner':
          cmp = a.pl.owner - b.pl.owner;
          break;
        case 'tier':
          cmp = a.tier - b.tier || a.hp - b.hp;
          break;
        case 'hp':
          cmp = a.hp - b.hp;
          break;
        case 'dock':
          cmp = a.dock - b.dock;
          break;
        case 'orbiting':
          cmp = a.orbiting - b.orbiting;
          break;
      }
      return (cmp || a.pl.id - b.pl.id) * dir;
    });

    const slice = this.paginateReportRows('starbases', bases);
    getEl('starbaseTableBody').innerHTML = slice
      .map(({ idx, pl, tier, hp, maxHp, dock, orbiting }) => {
        const emp = pl.owner > 0 ? hud.empires[pl.owner - 1] : null;
        const col = emp?.color ?? '#94a3b8';
        const pct = Math.round((hp / maxHp) * 100);
        const tierCol =
          tier >= 5
            ? '#f87171'
            : tier >= 4
              ? '#c084fc'
              : tier >= 3
                ? '#38bdf8'
                : tier >= 2
                  ? '#34d399'
                  : '#cbd5e1';
        return `<tr data-planet-idx="${idx}" data-x="${pl.x}" data-y="${pl.y}">
          <td class="fleet-route-cell" title="Focus Starbase on map">🛡 #${pl.id}</td>
          <td><span class="dot" style="background:${col}"></span></td>
          <td style="color:${tierCol};font-weight:bold;">${pl.starbaseName ?? 'Starbase'}</td>
          <td>${hp}/${maxHp} (${pct}%)</td>
          <td style="color:var(--accent);">×${dock.toFixed(2)}</td>
          <td>${orbiting > 0 ? `🛰 ${orbiting}` : '—'}</td>
          <td style="color:var(--muted);">(${Math.round(pl.x)}, ${Math.round(pl.y)})</td>
        </tr>`;
      })
      .join('');
  }

  private renderStargateTable(hud: HudSnapshot, rep: ReportsSnapshot): void {
    const gates: { idx: number; pl: PlanetView; gateTech: string }[] = [];
    for (let i = 0; i < rep.planets.length; i++) {
      const pl = rep.planets[i];
      if (pl.hasGate) {
        const emp = pl.owner > 0 ? hud.empires[pl.owner - 1] : null;
        gates.push({ idx: i, pl, gateTech: emp?.loadout.stargate ?? 'Stargate' });
      }
    }

    const { key, asc } = this.reportSort.stargates;
    const dir = asc ? 1 : -1;
    gates.sort((a, b) => {
      let cmp = 0;
      switch (key) {
        case 'id':
          cmp = a.pl.id - b.pl.id;
          break;
        case 'owner':
          cmp = a.pl.owner - b.pl.owner;
          break;
        case 'tech':
          cmp = a.gateTech.localeCompare(b.gateTech);
          break;
        case 'def':
          cmp = a.pl.defense - b.pl.defense;
          break;
        case 'res':
          cmp = a.pl.res - b.pl.res;
          break;
      }
      return (cmp || a.pl.id - b.pl.id) * dir;
    });

    const slice = this.paginateReportRows('stargates', gates);
    getEl('stargateTableBody').innerHTML = slice
      .map(({ idx, pl, gateTech }) => {
        const emp = pl.owner > 0 ? hud.empires[pl.owner - 1] : null;
        const col = emp?.color ?? '#94a3b8';
        return `<tr data-planet-idx="${idx}" data-x="${pl.x}" data-y="${pl.y}">
          <td class="fleet-route-cell" title="Focus Stargate on map">🌀 #${pl.id}</td>
          <td><span class="dot" style="background:${col}"></span></td>
          <td style="color:#22d3ee;">${gateTech}</td>
          <td>${Math.round(pl.defense)}</td>
          <td style="color:#38bdf8;font-weight:bold;">${Math.round(pl.res)}</td>
          <td style="color:var(--muted);">(${Math.round(pl.x)}, ${Math.round(pl.y)})</td>
        </tr>`;
      })
      .join('');
  }

  private renderWormholeTable(hud: HudSnapshot): void {
    const rows = hud.wormholes.map((wh, idx) => ({
      wh,
      idx,
      span: Math.round(Math.hypot(wh.x2 - wh.x1, wh.y2 - wh.y1)),
    }));
    const { key, asc } = this.reportSort.wormholes;
    const dir = asc ? 1 : -1;
    rows.sort((a, b) => {
      const cmp = key === 'span' ? a.span - b.span : a.idx - b.idx;
      return (cmp || a.idx - b.idx) * dir;
    });

    const slice = this.paginateReportRows('wormholes', rows);
    getEl('wormholeTableBody').innerHTML = slice
      .map(({ wh, idx, span }) => {
        return `<tr data-wh-idx="${idx}">
          <td style="color:#e879f9;font-weight:bold;">WH#${idx + 1}</td>
          <td class="fleet-route-cell" data-wh-end="a" title="Focus Endpoint A">(${Math.round(wh.x1)}, ${Math.round(wh.y1)})</td>
          <td class="fleet-route-cell" data-wh-end="b" title="Focus Endpoint B">(${Math.round(wh.x2)}, ${Math.round(wh.y2)})</td>
          <td style="color:var(--accent);">${span} ly</td>
          <td class="fleet-route-cell" title="Frame both endpoints">Frame Pair</td>
        </tr>`;
      })
      .join('');
  }

  private renderDebrisTable(rep: ReportsSnapshot): void {
    const rows = rep.debrisFields.map((deb, idx) => {
      const iron = Math.round(deb.iron);
      const bora = Math.round(deb.bora);
      const germ = Math.round(deb.germ);
      return { idx, deb, iron, bora, germ, total: iron + bora + germ };
    });

    const { key, asc } = this.reportSort.debris;
    const dir = asc ? 1 : -1;
    rows.sort((a, b) => {
      let cmp = 0;
      switch (key) {
        case 'id':
          cmp = a.idx - b.idx;
          break;
        case 'total':
          cmp = a.total - b.total;
          break;
        case 'iron':
          cmp = a.iron - b.iron;
          break;
        case 'bora':
          cmp = a.bora - b.bora;
          break;
        case 'germ':
          cmp = a.germ - b.germ;
          break;
        case 'ttl':
          cmp = a.deb.ttl - b.deb.ttl;
          break;
      }
      return (cmp || a.idx - b.idx) * dir;
    });

    const slice = this.paginateReportRows('debris', rows);
    getEl('debrisTableBody').innerHTML = slice
      .map(({ idx, deb, iron, bora, germ, total }) => {
        return `<tr data-debris-idx="${idx}" data-x="${deb.x}" data-y="${deb.y}">
          <td class="fleet-route-cell" title="Focus Debris field on map">✦ D#${idx + 1}</td>
          <td style="color:#c084fc;font-weight:bold;">${total} kT</td>
          <td class="c-iron">${iron}</td>
          <td class="c-bora">${bora}</td>
          <td class="c-germ">${germ}</td>
          <td>${deb.ttl.toFixed(1)} yr</td>
          <td style="color:var(--muted);">(${Math.round(deb.x)}, ${Math.round(deb.y)})</td>
        </tr>`;
      })
      .join('');
  }

  private arenaFeedCount = 0;

  private bindArenaEvents(): void {
    const btnLocal = document.getElementById('btnModeLocal');
    const btnRemote = document.getElementById('btnModeRemote');
    const serverInput = document.getElementById('arenaServerInput') as HTMLInputElement | null;
    const matchSelect = document.getElementById('arenaMatchSelect') as HTMLSelectElement | null;
    const btnRefresh = document.getElementById('btnArenaRefresh');
    const btnConnect = document.getElementById('btnArenaConnect');
    const btnNewMatch = document.getElementById('btnArenaNewMatch');
    const btnToggleFeed = document.getElementById('btnToggleArenaFeed');
    const feedPanel = document.getElementById('arenaFeedPanel');

    btnLocal?.addEventListener('click', () => {
      this.callbacks.onTransportModeChange?.('local');
    });
    btnRemote?.addEventListener('click', () => {
      this.callbacks.onTransportModeChange?.('remote');
    });
    btnRefresh?.addEventListener('click', () => {
      this.callbacks.onArenaRefreshMatches?.(serverInput?.value ?? 'http://localhost:8080');
    });
    serverInput?.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        this.callbacks.onArenaRefreshMatches?.(serverInput.value);
      }
    });
    serverInput?.addEventListener('change', () => {
      this.callbacks.onArenaRefreshMatches?.(serverInput.value);
    });
    matchSelect?.addEventListener('change', () => {
      const srv = serverInput?.value ?? 'http://localhost:8080';
      const mid = matchSelect.value;
      if (mid) {
        this.callbacks.onArenaConnect?.(srv, mid);
      }
    });
    btnConnect?.addEventListener('click', () => {
      const srv = serverInput?.value ?? 'http://localhost:8080';
      const mid = matchSelect?.value ?? '';
      if (mid) {
        this.callbacks.onArenaConnect?.(srv, mid);
      } else {
        this.callbacks.onArenaRefreshMatches?.(srv);
      }
    });
    btnNewMatch?.addEventListener('click', () => {
      this.openArenaCreateDrawer();
    });
    btnToggleFeed?.addEventListener('click', () => {
      if (!feedPanel || !btnToggleFeed) return;
      const collapsed = feedPanel.classList.toggle('collapsed');
      btnToggleFeed.textContent = collapsed ? '▸ Show' : '▾ Hide';
    });

    const scenSel = document.getElementById('arenaNewScenario') as HTMLSelectElement | null;
    const renderArenaScenario = (scenId: string) => {
      const scen = getScenarioById(scenId);
      const desc = document.getElementById('arenaScenarioDesc');
      const badges = document.getElementById('arenaScenarioBadges');
      if (desc) desc.textContent = scen.description;
      if (badges) {
        badges.innerHTML = scen.highlights
          .map((h) => `<span class="scenario-chip">${h}</span>`)
          .join('');
      }
      const sizeSel = document.getElementById('arenaNewSize') as HTMLSelectElement | null;
      const shapeSel = document.getElementById('arenaNewShape') as HTMLSelectElement | null;
      if (sizeSel) {
        if (scen.locks.galaxySize) sizeSel.value = scen.locks.galaxySize;
        else if (scen.defaults.galaxySize) sizeSel.value = scen.defaults.galaxySize;
      }
      if (shapeSel) {
        if (scen.locks.galaxyShape) shapeSel.value = scen.locks.galaxyShape;
        else if (scen.defaults.galaxyShape) shapeSel.value = scen.defaults.galaxyShape;
      }
    };
    if (scenSel) {
      scenSel.innerHTML = SCENARIOS.map(
        (s) => `<option value="${s.id}">${s.name}</option>`,
      ).join('');
      scenSel.value = 'standard';
      renderArenaScenario('standard');
      scenSel.addEventListener('change', () => renderArenaScenario(scenSel.value));
    }

    const ticksSlider = document.getElementById('arenaNewTicksPerEpoch') as HTMLInputElement | null;
    const ticksVal = document.getElementById('v_arenaTicksPerEpoch');
    ticksSlider?.addEventListener('input', () => {
      if (ticksVal) ticksVal.textContent = ticksSlider.value;
    });

    const timeoutSlider = document.getElementById('arenaNewTimeoutSec') as HTMLInputElement | null;
    const timeoutVal = document.getElementById('v_arenaTimeoutSec');
    timeoutSlider?.addEventListener('input', () => {
      if (timeoutVal) timeoutVal.textContent = `${timeoutSlider.value}s`;
    });

    const btnLaunch = document.getElementById('btnArenaLaunchMatch') as HTMLButtonElement | null;
    btnLaunch?.addEventListener('click', async () => {
      const srv = serverInput?.value ?? 'http://localhost:8080';
      const scen = getScenarioById(scenSel?.value ?? 'standard');
      const size = (document.getElementById('arenaNewSize') as HTMLSelectElement | null)?.value ?? 'small';
      const shape = (document.getElementById('arenaNewShape') as HTMLSelectElement | null)?.value ?? 'spiral';
      const seedRaw = (document.getElementById('arenaNewSeed') as HTMLInputElement | null)?.value.trim() ?? '';
      const seed = seedRaw !== '' ? Number(seedRaw) : undefined;
      const ticksPerEpoch = Number(ticksSlider?.value ?? 20);
      const epochTimeoutMs = Number(timeoutSlider?.value ?? 20) * 1000;
      const speedSel = Number((document.getElementById('arenaNewSpeed') as HTMLSelectElement | null)?.value ?? 1);
      const debugAudit = (document.getElementById('arenaNewDebugAudit') as HTMLInputElement | null)?.checked ?? true;

      const simConfig: SimConfig = {
        ...this.lastConfig,
        galaxySize: size as SimConfig['galaxySize'],
        galaxyShape: shape as SimConfig['galaxyShape'],
        wormholePairs:
          scen.locks.wormholePairs ?? scen.defaults.wormholePairs ?? this.lastConfig.wormholePairs,
        crustRichness:
          scen.locks.crustRichness ?? scen.defaults.crustRichness ?? this.lastConfig.crustRichness,
        gateCost: scen.locks.gateCost ?? scen.defaults.gateCost ?? this.lastConfig.gateCost,
        interceptRad:
          scen.locks.interceptRad ?? scen.defaults.interceptRad ?? this.lastConfig.interceptRad,
        salvageRate:
          scen.locks.salvageRate !== undefined
            ? scen.locks.salvageRate / 100
            : scen.defaults.salvageRate !== undefined
              ? scen.defaults.salvageRate / 100
              : this.lastConfig.salvageRate,
        aggroBias:
          scen.locks.aggroBias !== undefined
            ? scen.locks.aggroBias / 100
            : scen.defaults.aggroBias !== undefined
              ? scen.defaults.aggroBias / 100
              : this.lastConfig.aggroBias,
        sprawlPenalty:
          scen.locks.sprawlPenalty !== undefined
            ? scen.locks.sprawlPenalty / 100
            : scen.defaults.sprawlPenalty !== undefined
              ? scen.defaults.sprawlPenalty / 100
              : this.lastConfig.sprawlPenalty,
        scenarioModifiers: scen.modifiers,
      };

      const agentSeats: number[] = [];
      document
        .querySelectorAll<HTMLInputElement>('#arenaNewSeatsGrid input[type="checkbox"]:checked')
        .forEach((cb) => {
          const seat = Number(cb.value);
          if (Number.isInteger(seat)) agentSeats.push(seat);
        });

      const tickDelayMs =
        speedSel <= 0.25 ? 120 : speedSel <= 0.5 ? 60 : speedSel <= 1 ? 25 : speedSel <= 5 ? 5 : 0;
      const frameStrideTicks = speedSel <= 1 ? 1 : speedSel <= 5 ? 2 : 5;

      const statusEl = document.getElementById('arenaNewStatus');
      if (statusEl) statusEl.textContent = 'Creating Arena match...';
      btnLaunch.disabled = true;

      try {
        const res = await this.callbacks.onArenaCreateMatch?.(
          srv,
          {
            seed,
            simConfig,
            scenarioModifiers: scen.modifiers,
            galaxySize: size,
            galaxyShape: shape,
            ticksPerEpoch,
            epochTimeoutMs,
            agentSeats,
            debugAudit,
            frameStrideTicks,
            tickDelayMs,
          },
          speedSel,
        );
        if (res) {
          this.showArenaCreatedMatch(srv, res);
          if (statusEl) {
            statusEl.textContent =
              res.agentSeats.length > 0
                ? `✓ Match ${res.matchId} created & connected! Copy agent CLI commands below.`
                : `✓ Autonomous match ${res.matchId} live!`;
          }
          if (res.agentSeats.length === 0) {
            this.closeAllDrawers();
          }
        }
      } catch (err) {
        if (statusEl) {
          statusEl.textContent = `✗ Failed: ${err instanceof Error ? err.message : 'network error'}`;
        }
      } finally {
        btnLaunch.disabled = false;
      }
    });
  }

  private openArenaCreateDrawer(): void {
    const drawer = document.getElementById('drawerArenaNew');
    if (!drawer) return;
    const isOpen = drawer.classList.contains('open');
    this.closeAllDrawers();
    if (!isOpen) {
      // Pre-fill size/shape from current config
      const sizeSel = document.getElementById('arenaNewSize') as HTMLSelectElement | null;
      const shapeSel = document.getElementById('arenaNewShape') as HTMLSelectElement | null;
      if (sizeSel) {
        sizeSel.value = this.lastConfig.galaxySize;
      }
      if (shapeSel) {
        shapeSel.value = this.lastConfig.galaxyShape;
      }
      drawer.classList.add('open');
    }
  }

  private showArenaCreatedMatch(serverUrl: string, res: CreateArenaMatchResponse): void {
    const box = document.getElementById('arenaCreatedBox');
    const title = document.getElementById('arenaCreatedTitle');
    const list = document.getElementById('arenaTokensList');
    if (!box || !title || !list) return;

    box.style.display = 'flex';
    title.textContent = `✓ Match ${res.matchId} (${res.status})`;

    const seatsToShow =
      res.agentSeats.length > 0
        ? res.agentSeats
        : Object.keys(res.empireTokens)
            .map(Number)
            .sort((a, b) => a - b);

    const cliServer = serverUrl.startsWith('https://')
      ? `http://127.0.0.1:8080`
      : serverUrl;

    list.innerHTML = '';
    for (const seat of seatsToShow) {
      const token = res.empireTokens[String(seat)];
      if (!token) continue;
      const cmd = `python3 tools/llm_agent.py --server ${cliServer} --match ${res.matchId} --token ${token} --mock`;

      const row = document.createElement('div');
      row.className = 'arena-token-row';

      const top = document.createElement('div');
      top.className = 'arena-token-top';
      const label = document.createElement('span');
      label.textContent = `Seat #${seat} Token`;

      const copyBtn = document.createElement('button');
      copyBtn.className = 'hud-btn';
      copyBtn.style.padding = '2px 6px';
      copyBtn.style.fontSize = '0.6rem';
      copyBtn.textContent = '📋 Copy CLI';
      copyBtn.addEventListener('click', () => {
        const markCopied = () => {
          copyBtn.textContent = '✓ Copied';
          setTimeout(() => {
            copyBtn.textContent = '📋 Copy CLI';
          }, 1400);
        };
        const fallbackCopy = () => {
          const ta = document.createElement('textarea');
          ta.value = cmd;
          ta.setAttribute('readonly', '');
          ta.style.position = 'fixed';
          ta.style.left = '-9999px';
          document.body.appendChild(ta);
          ta.select();
          try {
            document.execCommand('copy');
          } catch {
            // ignore
          }
          document.body.removeChild(ta);
          markCopied();
        };
        if (navigator.clipboard && window.isSecureContext) {
          navigator.clipboard.writeText(cmd).then(markCopied).catch(fallbackCopy);
        } else {
          fallbackCopy();
        }
      });

      top.appendChild(label);
      top.appendChild(copyBtn);

      const cmdDiv = document.createElement('div');
      cmdDiv.className = 'arena-token-cmd';
      cmdDiv.textContent = cmd;

      row.appendChild(top);
      row.appendChild(cmdDiv);
      list.appendChild(row);
    }
  }

  public setTransportMode(mode: TransportMode): void {
    const isRemote = mode === 'remote';
    document.getElementById('btnModeLocal')?.classList.toggle('active', !isRemote);
    document.getElementById('btnModeRemote')?.classList.toggle('active', isRemote);

    const arenaBar = document.getElementById('arenaBar');
    if (arenaBar) arenaBar.style.display = isRemote ? 'flex' : 'none';

    const feedPanel = document.getElementById('arenaFeedPanel');
    if (feedPanel) feedPanel.style.display = isRemote ? 'flex' : 'none';

    // Keep speed & pause controls active in BOTH Local WASM and Remote Arena (controls server pacing!)
    const speedWrap = document.querySelector<HTMLElement>('.speed-controls');
    if (speedWrap) {
      speedWrap.style.opacity = '1';
      speedWrap.style.pointerEvents = 'auto';
    }
    const resetBtn = document.getElementById('btnReset') as HTMLButtonElement | null;
    if (resetBtn) {
      resetBtn.disabled = isRemote;
      resetBtn.style.opacity = isRemote ? '0.4' : '1';
    }
  }

  public setArenaServerUrl(url: string): void {
    const input = document.getElementById('arenaServerInput') as HTMLInputElement | null;
    if (input) input.value = url;
  }

  public setArenaMatchList(matches: ArenaMatchSummary[], selectedId?: string): void {
    const sel = document.getElementById('arenaMatchSelect') as HTMLSelectElement | null;
    if (!sel) return;
    const prev = selectedId ?? sel.value;
    if (matches.length === 0) {
      sel.innerHTML = '<option value="">-- No active matches --</option>';
      return;
    }
    sel.innerHTML = matches
      .map((m) => {
        const seats = m.agentSeats.length > 0 ? `${m.agentSeats.length} AI` : 'Auto';
        return `<option value="${m.matchId}">${m.matchId} • ${m.status} • Ep#${m.epoch} • Yr ${m.year.toFixed(1)} (${seats})</option>`;
      })
      .join('');
    if (prev && matches.some((m) => m.matchId === prev)) {
      sel.value = prev;
    } else if (matches.length > 0) {
      sel.value = matches[0].matchId;
    }
  }

  public setArenaConnectionState(
    state: 'connecting' | 'connected' | 'disconnected' | 'error',
    detail?: string,
  ): void {
    const badge = document.getElementById('arenaConnBadge');
    const info = document.getElementById('arenaEpochInfo');
    if (badge) {
      badge.className = 'arena-badge';
      if (state === 'connected') {
        badge.textContent = 'LIVE';
        badge.classList.add('live');
      } else if (state === 'connecting') {
        badge.textContent = 'SYNC';
        badge.classList.add('waiting');
      } else if (state === 'error') {
        badge.textContent = 'ERR';
        badge.classList.add('error');
      } else {
        badge.textContent = 'OFFLINE';
      }
    }
    if (info && detail) {
      info.textContent = detail;
    }
  }

  public updateArenaStatus(summary: ArenaMatchSummary): void {
    const badge = document.getElementById('arenaConnBadge');
    const info = document.getElementById('arenaEpochInfo');
    const waitPill = document.getElementById('arenaWaitPill');

    if (badge) {
      badge.className = 'arena-badge';
      if (summary.status === 'Finished') {
        badge.textContent = 'FINISHED';
        badge.classList.add('finished');
      } else if (summary.paused) {
        badge.textContent = 'PAUSED';
        badge.classList.add('waiting');
      } else if (summary.status === 'WaitingForAgents') {
        badge.textContent = 'BARRIER';
        badge.classList.add('waiting');
      } else {
        badge.textContent = 'LIVE';
        badge.classList.add('live');
      }
    }

    if (info) {
      const waitTxt =
        summary.status === 'WaitingForAgents' && summary.waitingForSeats.length > 0
          ? `Waiting seats [${summary.waitingForSeats.join(', ')}]`
          : summary.status === 'Finished'
            ? `Winner: Empire #${(summary.winner ?? 0) + 1}`
            : `Running (${summary.planetCount}p)`;
      info.textContent = `Match ${summary.matchId} • Epoch #${summary.epoch} • ${waitTxt}`;
    }

    if (waitPill) {
      if (summary.status === 'WaitingForAgents' && summary.waitingForSeats.length > 0) {
        waitPill.style.display = 'inline-block';
        waitPill.textContent = `WAIT [${summary.waitingForSeats.join(',')}]`;
      } else {
        waitPill.style.display = 'none';
      }
    }
  }

  public clearArenaFeed(): void {
    this.arenaFeedCount = 0;
    const body = document.getElementById('arenaFeedBody');
    if (body) {
      body.innerHTML =
        '<div class="arena-feed-empty">Waiting for AI reasoning, diplomatic cables &amp; sector directives...</div>';
    }
  }

  public appendArenaFeedEvent(ev: ArenaFeedEvent): void {
    const body = document.getElementById('arenaFeedBody');
    if (!body) return;
    if (this.arenaFeedCount === 0) {
      body.innerHTML = '';
    }
    this.arenaFeedCount += 1;

    const kindIcons: Record<ArenaFeedEvent['kind'], string> = {
      Reasoning: '🧠',
      DipMessage: '📨',
      DiplomacyChange: '🤝',
      SectorDirective: '🎯',
      BuildHub: '🌀',
      PolicyUpdate: '📜',
      EpochStatus: '⏱',
      MatchFinished: '🏆',
    };

    const item = document.createElement('div');
    item.className = 'arena-feed-item';
    item.style.borderLeftColor = ev.empireColor || '#38bdf8';

    const meta = document.createElement('div');
    meta.className = 'arena-feed-meta';

    const empSpan = document.createElement('span');
    empSpan.className = 'arena-feed-emp';
    empSpan.style.color = ev.empireColor || '#38bdf8';
    empSpan.textContent = `${kindIcons[ev.kind] ?? '•'} ${ev.empireName}`;

    const tagSpan = document.createElement('span');
    tagSpan.className = 'arena-feed-tag';
    tagSpan.textContent = `Ep#${ev.epoch} • Yr ${ev.year.toFixed(1)} • ${ev.kind}`;

    meta.appendChild(empSpan);
    meta.appendChild(tagSpan);

    const text = document.createElement('div');
    text.className = 'arena-feed-text';
    text.textContent = ev.summary;

    item.appendChild(meta);
    item.appendChild(text);

    body.prepend(item);
    while (body.children.length > 150) {
      body.lastElementChild?.remove();
    }
  }
}
