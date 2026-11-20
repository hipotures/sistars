import { drawPowerChart } from './renderer';
import type {
  Doctrine,
  EngineMeta,
  FleetClass,
  RaceDef,
  SimConfig,
  SimSnapshot,
  TechCatalogView,
  TournamentResult,
} from './types';

type SortKey = 'id' | 'owner' | 'gate' | 'hab' | 'pop' | 'fac' | 'iron' | 'bora' | 'germ' | 'res' | 'def';
type LabAction = 'tournament' | 'coevolve' | 'copyjson';

export interface UiCallbacks {
  onSpeedChange: (speed: number) => void;
  onStepTick: () => void;
  onReset: () => void;
  onParamChange: (key: string, value: number) => void;
  onGalaxyChange: (size: string, shape: string) => void;
  onLabAction: (action: LabAction) => void;
  onSelectPlanet: (index: number) => void;
  onLaunchRaces: (races: RaceDef[]) => void;
  onDebugPerfToggle: (enabled: boolean) => void;
  onDebugHistoryToggle: (enabled: boolean) => void;
  onHistoryScrub: (index: number) => void;
  onHistoryStep: (delta: number) => void;
  onHistoryLive: () => void;
}

const CUSTOM_RACES_STORAGE_KEY = 'sistars.customRaces.v1';

const FLEET_LABELS: Record<FleetClass, string> = {
  col: '● Colonizer',
  cruiser: '▲ Cruiser',
  battleship: '◆ Battleship',
  bomber: '★ Bomber',
  freight: '■ Freighter',
};

const DRAWER_IDS = ['drawerRaces', 'drawerTech', 'drawerStats', 'drawerReports', 'drawerSettings'] as const;
const DRAWER_TAB_MAP: Record<(typeof DRAWER_IDS)[number], string> = {
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

  private sortKey: SortKey = 'res';
  private sortAsc = false;
  private selectedPlanet = -1;
  private lastSnap: SimSnapshot | null = null;
  private lastConfig: SimConfig;
  private lastHudSignature = '';

  private customRaces: RaceDef[] = [];
  private selectedRaceIds: Set<string>;
  private activeTechTab: 'overview' | 'browser' = 'overview';

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

    this.initStaticControls();
    this.bindEvents();
    this.renderRaceRoster();
  }

  public setConfig(cfg: SimConfig): void {
    this.lastConfig = cfg;
    getEl<HTMLInputElement>('p_ignoreRacialTraits').checked = cfg.ignoreRacialTraits;
    getEl<HTMLSelectElement>('p_galaxySize').value = cfg.galaxySize;
    getEl<HTMLSelectElement>('p_galaxyShape').value = cfg.galaxyShape;
  }

  public setSelectedPlanet(index: number): void {
    this.selectedPlanet = index;
    if (index === -1) {
      getEl('planetInspector').style.display = 'none';
    } else if (this.lastSnap) {
      this.updateInspector(this.lastSnap);
    }
  }

  public getSelectedPlanet(): number {
    return this.selectedPlanet;
  }

  public setStatus(text: string): void {
    getEl('statusMsg').textContent = text;
    getEl('statusMsgStats').textContent = text;
  }

  public setActiveSpeedButton(speed: number): void {
    document.querySelectorAll<HTMLButtonElement>('[data-speed]').forEach((b) => {
      b.classList.toggle('active', Number(b.dataset.speed) === speed);
    });
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
    fps: number,
    tps: number,
    wasmMs: number,
    drawMs: number,
    planets: number,
    fleets: number,
  ): void {
    const hud = getEl('perfHud');
    hud.classList.toggle('visible', enabled);
    if (!enabled) return;

    const fpsEl = getEl('perfFps');
    fpsEl.textContent = String(Math.round(fps));
    fpsEl.style.color = fps >= 50 ? 'var(--green)' : fps >= 28 ? 'var(--yellow)' : 'var(--red)';

    getEl('perfTps').textContent = String(Math.round(tps));
    getEl('perfWasm').textContent = `${wasmMs.toFixed(2)}ms`;
    getEl('perfDraw').textContent = `${drawMs.toFixed(2)}ms`;
    getEl('perfEnt').textContent = `${planets}p / ${fleets}f`;
  }

  public updateWinRateBar(res: TournamentResult, snap: SimSnapshot): void {
    getEl('tourneyLabel').textContent = `🏆 Win-Rate (${res.numGames} maps):`;
    const bar = getEl('winrateBar');
    bar.innerHTML = snap.empires
      .map((emp, idx) => {
        const w = res.winRates[idx] ?? 0;
        const label = w >= 9 ? `${emp.name.slice(0, 3)} ${w}%` : w >= 5 ? `${w}%` : '';
        return `<div class="wr-seg" style="width:${w}%;background:${emp.color};" title="${emp.name}: ${w}% (${res.wins[idx] ?? 0} wins)">${label}</div>`;
      })
      .join('');
  }

  public update(snap: SimSnapshot, forceDrawers = false): void {
    this.lastSnap = snap;
    getEl('hudYear').textContent = snap.year.toFixed(1);
    this.updateEmpireHud(snap);
    this.renderEvents(snap);

    if (this.selectedPlanet !== -1) {
      this.updateInspector(snap);
    }

    if (forceDrawers || snap.ticks % 15 === 0) {
      this.renderOpenDrawers(snap);
    }
  }

  public renderOpenDrawers(snap: SimSnapshot): void {
    this.lastSnap = snap;
    if (getEl('drawerTech').classList.contains('open')) {
      if (this.activeTechTab === 'overview') this.renderTechOverview(snap);
      else this.renderTechBrowser(snap);
    }
    if (getEl('drawerStats').classList.contains('open')) {
      this.renderDNATable(snap);
      this.renderPowerChart(snap);
    }
    if (getEl('drawerReports').classList.contains('open')) {
      this.renderPlanetTable(snap);
      this.renderFleetTable(snap);
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
    // Speed buttons & single-tick step
    document.querySelectorAll<HTMLButtonElement>('[data-speed]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const speed = Number(btn.dataset.speed ?? 1);
        this.setActiveSpeedButton(speed);
        this.callbacks.onSpeedChange(speed);
      });
    });

    getEl('btnStep').addEventListener('click', () => this.callbacks.onStepTick());

    // Debug Performance HUD & History Scrubber controls
    getEl<HTMLInputElement>('p_debugPerf').addEventListener('change', (e) => {
      this.callbacks.onDebugPerfToggle((e.target as HTMLInputElement).checked);
    });
    getEl<HTMLInputElement>('p_debugHistory').addEventListener('change', (e) => {
      this.callbacks.onDebugHistoryToggle((e.target as HTMLInputElement).checked);
    });
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
    getEl('btnCloseInspector').addEventListener('click', () => this.callbacks.onSelectPlanet(-1));

    // Tech drawer tabs
    document.querySelectorAll<HTMLButtonElement>('[data-tech-tab]').forEach((btn) => {
      btn.addEventListener('click', () => {
        this.activeTechTab = btn.dataset.techTab === 'browser' ? 'browser' : 'overview';
        getEl('techTabOverview').classList.toggle('active', this.activeTechTab === 'overview');
        getEl('techTabBrowser').classList.toggle('active', this.activeTechTab === 'browser');
        getEl('techOverviewPane').style.display = this.activeTechTab === 'overview' ? 'flex' : 'none';
        getEl('techBrowserPane').style.display = this.activeTechTab === 'browser' ? 'flex' : 'none';
        if (this.lastSnap) this.renderOpenDrawers(this.lastSnap);
      });
    });

    // Tech browser filters
    const reRenderBrowser = () => {
      if (this.lastSnap) this.renderTechBrowser(this.lastSnap);
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
        const which = btn.dataset.report === 'fleets' ? 'fleets' : 'planets';
        getEl('repPlanetsWrap').style.display = which === 'planets' ? 'block' : 'none';
        getEl('repFleetsWrap').style.display = which === 'fleets' ? 'block' : 'none';
        getEl('repBtnPlanets').classList.toggle('active', which === 'planets');
        getEl('repBtnFleets').classList.toggle('active', which === 'fleets');
      });
    });

    document.querySelectorAll<HTMLTableCellElement>('th[data-sort]').forEach((th) => {
      th.addEventListener('click', () => {
        const key = th.dataset.sort as SortKey;
        if (this.sortKey === key) this.sortAsc = !this.sortAsc;
        else {
          this.sortKey = key;
          this.sortAsc = false;
        }
        if (this.lastSnap) this.renderPlanetTable(this.lastSnap);
      });
    });

    getEl('planetTableBody').addEventListener('click', (e) => {
      const row = (e.target as HTMLElement).closest('tr');
      if (!row || !row.dataset.planetIdx) return;
      const idx = Number(row.dataset.planetIdx);
      if (!Number.isNaN(idx)) this.callbacks.onSelectPlanet(idx);
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

    getEl<HTMLInputElement>('p_ignoreRacialTraits').addEventListener('change', (e) => {
      const checked = (e.target as HTMLInputElement).checked;
      this.callbacks.onParamChange('ignoreRacialTraits', checked ? 1 : 0);
    });

    document.querySelectorAll<HTMLInputElement>('input[data-param]').forEach((input) => {
      input.addEventListener('input', () => {
        const key = input.dataset.param!;
        const num = parseFloat(input.value);
        const label = getEl(`v_${key}`);
        if (key === 'sprawlPenalty' || key === 'aggroBias' || key === 'salvageRate') {
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
      getEl(DRAWER_TAB_MAP[id]).classList.toggle('active', el.classList.contains('open'));
    }
    if (this.lastSnap) this.renderOpenDrawers(this.lastSnap);
  }

  private formatTechBadges(perkMask: number): string {
    return this.meta.breakthroughs
      .filter((bp) => (perkMask & bp.bit) !== 0)
      .map((bp) => bp.icon)
      .join('');
  }

  private updateEmpireHud(snap: SimSnapshot): void {
    const bar = getEl('empiresBar');
    const sig = snap.empires.map((e) => `${e.id}:${e.color}`).join('|');
    if (sig !== this.lastHudSignature) {
      this.lastHudSignature = sig;
      bar.innerHTML = snap.empires
        .map(
          (emp, idx) => `<div class="emp-pill" style="border-top-color:${emp.color};">
            <div class="emp-row1">
              <span style="color:${emp.color};">■ ${emp.name}</span>
              <span id="hudTechB_${idx}"></span>
            </div>
            <div class="emp-row2">
              <span id="hudStat_${idx}">1 pl | 25 Res</span>
              <span id="hudKill_${idx}">💥0 🌀0</span>
            </div>
          </div>`,
        )
        .join('');
    }

    for (let i = 0; i < snap.empires.length; i++) {
      const emp = snap.empires[i];
      const statEl = document.getElementById(`hudStat_${i}`);
      const killEl = document.getElementById(`hudKill_${i}`);
      const badgeEl = document.getElementById(`hudTechB_${i}`);
      if (statEl) statEl.textContent = `${emp.stats.planets} pl | ${Math.round(emp.stats.res)} Res`;
      if (killEl) killEl.textContent = `💥${emp.doctrine.kills} 🌀${emp.stats.gates}`;
      if (badgeEl) badgeEl.textContent = this.formatTechBadges(emp.perks);
    }
  }

  private renderEvents(snap: SimSnapshot): void {
    const container = getEl('eventLog');
    container.innerHTML = snap.events
      .map(
        (ev) =>
          `<div class="log-item" style="border-right-color:${ev.color}"><b style="font-family:monospace;color:${ev.color}">[${ev.year.toFixed(0)}]</b> ${ev.text}</div>`,
      )
      .join('');
  }

  private updateInspector(snap: SimSnapshot): void {
    const i = this.selectedPlanet;
    if (i < 0 || i >= snap.planets.length) {
      getEl('planetInspector').style.display = 'none';
      return;
    }
    const pl = snap.planets[i];
    const insp = getEl('planetInspector');
    insp.style.display = 'flex';
    const emp = pl.owner > 0 ? snap.empires[pl.owner - 1] : null;
    insp.style.borderLeftColor = emp ? emp.color : '#64748b';
    getEl('inspTitle').textContent = `System #${pl.id}`;
    const ownerEl = getEl('inspOwner');
    ownerEl.textContent = emp ? `${emp.name} (${shortPrt(emp.prt)})` : 'Uninhabited';
    ownerEl.style.color = emp ? emp.color : '#94a3b8';
    getEl('inspHab').textContent = `${Math.round(pl.hab * 100)}%`;
    getEl('inspPop').textContent = `${Math.round(pl.pop)}k / ${Math.round(pl.maxPop)}k`;
    getEl('inspRes').textContent = `${Math.round(pl.res)} Res/yr`;
    getEl('inspFac').textContent = `${Math.round(pl.factories)} fac / ${Math.round(pl.mines)} mine`;
    getEl('inspDef').textContent = `${Math.round(pl.defense)} | ${pl.hasGate ? '🌀 Active' : 'None'}`;

    const maxC = this.lastConfig.crustRichness * 1.6;
    getEl('barIron').style.width = `${Math.min(100, (pl.ironCrust / maxC) * 100)}%`;
    getEl('valIron').textContent = `${Math.round(pl.ironSurf)} (${Math.round(pl.ironCrust)})`;
    getEl('barBora').style.width = `${Math.min(100, (pl.boraCrust / maxC) * 100)}%`;
    getEl('valBora').textContent = `${Math.round(pl.boraSurf)} (${Math.round(pl.boraCrust)})`;
    getEl('barGerm').style.width = `${Math.min(100, (pl.germCrust / maxC) * 100)}%`;
    getEl('valGerm').textContent = `${Math.round(pl.germSurf)} (${Math.round(pl.germCrust)})`;
  }

  private renderTechOverview(snap: SimSnapshot): void {
    const maxLv = this.meta.maxTechLevel || 26;
    getEl('techCardsContainer').innerHTML = snap.empires
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
        const badges = snap.empires
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

  private renderTechBrowser(snap: SimSnapshot): void {
    const q = getEl<HTMLInputElement>('techSearch').value.trim().toLowerCase();
    const cat = getEl<HTMLSelectElement>('techCatFilter').value;
    const status = getEl<HTMLSelectElement>('techStatusFilter').value;

    const filtered = this.catalog.entries.filter((entry) => {
      if (cat !== 'all' && entry.categoryLabel !== cat) return false;
      if (q && !entry.name.toLowerCase().includes(q) && !entry.statsSummary.toLowerCase().includes(q)) {
        return false;
      }
      const unlockedByAny = snap.empires.some((emp) => isTechUnlocked(emp.loadout.unlockedBits, entry.id));
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
        const dots = snap.empires
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

  private renderDNATable(snap: SimSnapshot): void {
    getEl('dnaGenInfo').textContent = `DNA Generation: #${snap.dnaGeneration}`;
    getEl('dnaTableBody').innerHTML = snap.empires
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

  private renderPowerChart(snap: SimSnapshot): void {
    const canvas = getEl<HTMLCanvasElement>('powerCanvas');
    const colors = snap.empires.map((e) => e.color);
    const maxVal = drawPowerChart(canvas, snap.powerHistory, colors);
    getEl('maxResInfo').textContent = `Max: ${Math.round(maxVal)} Res/yr`;
  }

  private renderPlanetTable(snap: SimSnapshot): void {
    const n = snap.planets.length;
    getEl('repBtnPlanets').textContent = `Planets (${n})`;
    const rows = snap.planets.map((pl, idx) => ({
      idx,
      id: pl.id,
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

    rows.sort((a, b) => (this.sortAsc ? a[this.sortKey] - b[this.sortKey] : b[this.sortKey] - a[this.sortKey]));
    getEl('planetTableBody').innerHTML = rows
      .map((r) => {
        const col = r.owner > 0 ? (snap.empires[r.owner - 1]?.color ?? '#94a3b8') : '#475569';
        return `<tr data-planet-idx="${r.idx}">
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

  private renderFleetTable(snap: SimSnapshot): void {
    getEl('repBtnFleets').textContent = `Fleets in Flight (${snap.fleets.length})`;
    const minLetters = ['I', 'B', 'G'];
    const minCols = ['#60a5fa', '#34d399', '#fbbf24'];

    const slice = snap.fleets.slice(-80).reverse();
    getEl('fleetTableBody').innerHTML = slice
      .map((fl) => {
        const target = snap.planets[fl.target];
        const dist = target ? Math.hypot(target.x - fl.x, target.y - fl.y) : 0;
        const eta = dist / this.lastConfig.fleetSpeed;
        const empCol = snap.empires[fl.emp - 1]?.color ?? '#94a3b8';
        const label =
          fl.type === 'freight'
            ? `■ Freight (${minLetters[fl.minType] ?? 'I'})`
            : `${FLEET_LABELS[fl.type]}${fl.hunting ? ' 🎯' : ''}${fl.usedWh ? ' ⚡' : ''}`;
        const col =
          fl.type === 'freight'
            ? minCols[fl.minType] ?? '#60a5fa'
            : fl.type === 'bomber'
              ? '#34d399'
              : fl.type === 'battleship'
                ? '#f87171'
                : '#93c5fd';
        const hpPct = fl.maxPower > 0 ? Math.round((fl.power / fl.maxPower) * 100) : 0;
        return `<tr>
          <td>F${fl.id}</td>
          <td><span class="dot" style="background:${empCol}"></span></td>
          <td style="color:${col};font-weight:bold;">${label}</td>
          <td>#${fl.from + 1}→#${fl.target + 1}</td>
          <td>${Math.round(fl.power)} (${hpPct}%)</td>
          <td>${Math.round(dist)}ly</td>
          <td>${eta.toFixed(1)}yr</td>
        </tr>`;
      })
      .join('');
  }
}
