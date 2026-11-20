import { drawPowerChart } from './renderer';
import type {
  EngineMeta,
  FleetClass,
  SimConfig,
  SimSnapshot,
  TournamentResult,
} from './types';

type SortKey = 'id' | 'owner' | 'gate' | 'hab' | 'pop' | 'fac' | 'iron' | 'bora' | 'germ' | 'res' | 'def';
type LabAction = 'tournament' | 'coevolve' | 'autobalance' | 'copyjson';

export interface UiCallbacks {
  onSpeedChange: (speed: number) => void;
  onReset: () => void;
  onParamChange: (key: string, value: number) => void;
  onLabAction: (action: LabAction) => void;
  onSelectPlanet: (index: number) => void;
}

const FLEET_LABELS: Record<FleetClass, string> = {
  col: '● Colonizer',
  cruiser: '▲ Cruiser',
  battleship: '◆ Battleship',
  bomber: '★ Bomber',
  freight: '■ Freighter',
};

const DRAWER_IDS = ['drawerTech', 'drawerStats', 'drawerReports', 'drawerSettings'] as const;
const DRAWER_TAB_MAP: Record<(typeof DRAWER_IDS)[number], string> = {
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

export class UiController {
  private readonly meta: EngineMeta;
  private readonly callbacks: UiCallbacks;
  private sortKey: SortKey = 'res';
  private sortAsc = false;
  private selectedPlanet = -1;
  private lastSnap: SimSnapshot | null = null;
  private lastConfig: SimConfig;

  constructor(meta: EngineMeta, initialConfig: SimConfig, callbacks: UiCallbacks) {
    this.meta = meta;
    this.lastConfig = initialConfig;
    this.callbacks = callbacks;
    this.bindEvents();
  }

  public setConfig(cfg: SimConfig): void {
    this.lastConfig = cfg;
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

  public updateWinRateBar(res: TournamentResult): void {
    getEl('tourneyLabel').textContent = `🏆 Win-Rate (${res.numGames} maps):`;
    const [w1, w2, w3] = res.winRates;
    const b1 = getEl('wrBar1');
    const b2 = getEl('wrBar2');
    const b3 = getEl('wrBar3');
    b1.style.width = `${w1}%`;
    b1.textContent = w1 > 8 ? `🟢 ${w1}%` : '';
    b2.style.width = `${w2}%`;
    b2.textContent = w2 > 8 ? `🔵 ${w2}%` : '';
    b3.style.width = `${w3}%`;
    b3.textContent = w3 > 8 ? `🟡 ${w3}%` : '';
  }

  public syncWeightSliders(indWeight: number, expWeight: number, techWeight: number): void {
    const pairs: [string, number][] = [
      ['indWeight', indWeight],
      ['expWeight', expWeight],
      ['techWeight', techWeight],
    ];
    for (const [k, val] of pairs) {
      getEl<HTMLInputElement>(`p_${k}`).value = String(val);
      getEl(`v_${k}`).textContent = val.toFixed(2);
    }
  }

  public update(snap: SimSnapshot, forceDrawers = false): void {
    this.lastSnap = snap;

    getEl('hudYear').textContent = snap.year.toFixed(1);
    for (let e = 0; e < 3; e++) {
      const p = e + 1;
      const st = snap.stats[e];
      const doc = snap.doctrines[e];
      getEl(`hudE${p}`).textContent = `${st.planets} pl | ${Math.round(st.res)} Res`;
      getEl(`hudK${p}`).textContent = `💥${doc.kills} 🌀${st.gates}`;
      getEl(`techB${p}`).textContent = this.formatTechBadges(snap.perks[e]);
    }

    this.renderEvents(snap);

    if (this.selectedPlanet !== -1) {
      this.updateInspector(snap);
    }

    if (forceDrawers || snap.ticks % 15 === 0) {
      if (getEl('drawerTech').classList.contains('open')) this.renderTechDrawer(snap);
      if (getEl('drawerStats').classList.contains('open')) {
        this.renderDNATable(snap);
        this.renderPowerChart(snap);
      }
      if (getEl('drawerReports').classList.contains('open')) {
        this.renderPlanetTable(snap);
        this.renderFleetTable(snap);
      }
    }
  }

  public renderOpenDrawers(snap: SimSnapshot): void {
    this.lastSnap = snap;
    if (getEl('drawerTech').classList.contains('open')) this.renderTechDrawer(snap);
    if (getEl('drawerStats').classList.contains('open')) {
      this.renderDNATable(snap);
      this.renderPowerChart(snap);
    }
    if (getEl('drawerReports').classList.contains('open')) {
      this.renderPlanetTable(snap);
      this.renderFleetTable(snap);
    }
  }

  private bindEvents(): void {
    // Speed buttons
    document.querySelectorAll<HTMLButtonElement>('[data-speed]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const speed = Number(btn.dataset.speed ?? 1);
        document.querySelectorAll<HTMLButtonElement>('[data-speed]').forEach((b) => {
          b.classList.toggle('active', b === btn);
        });
        this.callbacks.onSpeedChange(speed);
      });
    });

    // Drawer nav buttons & close buttons
    document.querySelectorAll<HTMLButtonElement>('[data-drawer]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const target = btn.dataset.drawer as (typeof DRAWER_IDS)[number];
        this.toggleDrawer(target);
      });
    });

    document.querySelectorAll<HTMLButtonElement>('[data-close-drawer]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const target = btn.dataset.closeDrawer as (typeof DRAWER_IDS)[number];
        this.toggleDrawer(target);
      });
    });

    // Reset button
    getEl('btnReset').addEventListener('click', () => this.callbacks.onReset());

    // Inspector close
    getEl('btnCloseInspector').addEventListener('click', () => {
      this.callbacks.onSelectPlanet(-1);
    });

    // Report tabs
    document.querySelectorAll<HTMLButtonElement>('[data-report]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const which = btn.dataset.report === 'fleets' ? 'fleets' : 'planets';
        getEl('repPlanetsWrap').style.display = which === 'planets' ? 'block' : 'none';
        getEl('repFleetsWrap').style.display = which === 'fleets' ? 'block' : 'none';
        getEl('repBtnPlanets').classList.toggle('active', which === 'planets');
        getEl('repBtnFleets').classList.toggle('active', which === 'fleets');
      });
    });

    // Sortable planet table headers
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

    // Planet table row click -> inspect planet
    getEl('planetTableBody').addEventListener('click', (e) => {
      const row = (e.target as HTMLElement).closest('tr');
      if (!row || !row.dataset.planetIdx) return;
      const idx = Number(row.dataset.planetIdx);
      if (!Number.isNaN(idx)) this.callbacks.onSelectPlanet(idx);
    });

    // Lab actions
    document.querySelectorAll<HTMLButtonElement>('[data-action]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const action = btn.dataset.action as LabAction;
        this.callbacks.onLabAction(action);
      });
    });

    // Settings sliders
    document.querySelectorAll<HTMLInputElement>('input[data-param]').forEach((input) => {
      input.addEventListener('input', () => {
        const key = input.dataset.param!;
        const num = parseFloat(input.value);
        const label = getEl(`v_${key}`);
        if (key === 'sprawlPenalty' || key === 'aggroBias' || key === 'salvageRate') {
          label.textContent = `${num}%`;
        } else if (key === 'indWeight' || key === 'expWeight' || key === 'techWeight') {
          label.textContent = num.toFixed(2);
        } else {
          label.textContent = String(num);
        }
        this.callbacks.onParamChange(key, num);
      });
    });
  }

  private toggleDrawer(targetId: (typeof DRAWER_IDS)[number]): void {
    for (const id of DRAWER_IDS) {
      const el = getEl(id);
      if (id === targetId) el.classList.toggle('open');
      else el.classList.remove('open');
      getEl(DRAWER_TAB_MAP[id]).classList.toggle('active', el.classList.contains('open'));
    }

    if (this.lastSnap) {
      this.renderOpenDrawers(this.lastSnap);
    }
  }

  private formatTechBadges(perkMask: number): string {
    return this.meta.breakthroughs
      .filter((bp) => (perkMask & bp.bit) !== 0)
      .map((bp) => bp.icon)
      .join('');
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
    const doc = pl.owner > 0 ? snap.doctrines[pl.owner - 1] : null;
    insp.style.borderLeftColor = doc ? doc.color : '#64748b';
    getEl('inspTitle').textContent = `System #${pl.id}`;
    const ownerEl = getEl('inspOwner');
    ownerEl.textContent = doc ? doc.name : 'Uninhabited';
    ownerEl.style.color = doc ? doc.color : '#94a3b8';
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

  private renderTechDrawer(snap: SimSnapshot): void {
    let cardsHtml = '';
    for (let e = 0; e < 3; e++) {
      const d = snap.doctrines[e];
      const lv = snap.techLv[e];
      let rows = '';
      for (const tf of this.meta.techFields) {
        const pct = Math.min(100, (lv[tf.index] / 10.0) * 100);
        rows += `<div class="tech-row">
          <span style="width:32px;color:${tf.color};font-weight:bold;">${tf.key}</span>
          <div class="tech-bar"><div class="tech-fill" style="width:${pct}%;background:${tf.color};"></div></div>
          <span style="width:40px;text-align:right;">Lv ${lv[tf.index].toFixed(1)}</span>
          <span style="width:32px;text-align:right;color:var(--muted);">${Math.round(d.techAlloc[tf.index] * 100)}%</span>
        </div>`;
      }
      cardsHtml += `<div class="tech-card" style="border-top:2px solid ${d.color}">
        <div class="tech-card-head"><span style="color:${d.color}">${d.name}</span><span style="color:var(--muted);font-family:monospace;">Budget: ${Math.round(d.techShare * 100)}%</span></div>
        ${rows}
      </div>`;
    }
    getEl('techCardsContainer').innerHTML = cardsHtml;

    let bpHtml = '';
    for (const bp of this.meta.breakthroughs) {
      let badges = '';
      for (let e = 0; e < 3; e++) {
        const has = (snap.perks[e] & bp.bit) !== 0;
        const col = snap.doctrines[e].color;
        badges += `<span style="display:inline-block;width:10px;height:10px;margin-left:3px;border:1px solid ${col};background:${has ? col : 'transparent'};"></span>`;
      }
      bpHtml += `<div class="bp-item">
        <div style="display:flex;justify-content:space-between;align-items:center;">
          <b>${bp.icon} ${bp.name}</b><div>${badges}</div>
        </div>
        <div class="bp-req">Requires: ${bp.reqTxt}</div>
        <div style="color:#94a3b8;margin-top:2px;">${bp.desc}</div>
      </div>`;
    }
    getEl('breakthroughsContainer').innerHTML = bpHtml;
  }

  private renderDNATable(snap: SimSnapshot): void {
    getEl('dnaGenInfo').textContent = `DNA Generation: #${snap.dnaGeneration}`;
    let html = '';
    for (let e = 0; e < 3; e++) {
      const d = snap.doctrines[e];
      let topF = 0;
      for (let f = 1; f < 6; f++) {
        if (d.techAlloc[f] > d.techAlloc[topF]) topF = f;
      }
      const tf = this.meta.techFields[topF];
      html += `<tr>
        <td style="color:${d.color};font-weight:bold;">${d.name}</td>
        <td>${Math.round(d.facShare * 100)}%</td>
        <td>${Math.round(d.shipShare * 100)}%</td>
        <td>${Math.round(d.techShare * 100)}%</td>
        <td style="color:#93c5fd;">${Math.round(d.cruiserShare * 100)}%</td>
        <td style="color:#60a5fa;">${Math.round(d.bshipShare * 100)}%</td>
        <td style="color:#34d399;">${Math.round(d.bomberShare * 100)}%</td>
        <td style="color:#c084fc;">${Math.round(d.interceptBias * 100)}%</td>
        <td style="color:${tf.color};font-weight:bold;">${tf.key} (${Math.round(d.techAlloc[topF] * 100)}%)</td>
        <td style="color:${d.color};font-weight:bold;">${d.winRate}%</td>
        <td style="color:${d.color};font-weight:bold;">${d.lastFit}</td>
      </tr>`;
    }
    getEl('dnaTableBody').innerHTML = html;
  }

  private renderPowerChart(snap: SimSnapshot): void {
    const canvas = getEl<HTMLCanvasElement>('powerCanvas');
    const maxVal = drawPowerChart(canvas, snap.powerHistory);
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
    const colors = ['#475569', '#4ade80', '#38bdf8', '#facc15'];
    getEl('planetTableBody').innerHTML = rows
      .map(
        (r) => `<tr data-planet-idx="${r.idx}">
        <td>#${r.id}</td>
        <td><span class="dot" style="background:${colors[r.owner]}"></span></td>
        <td style="color:#22d3ee;text-align:center;">${r.gate ? '🌀' : ''}</td>
        <td>${r.hab}%</td>
        <td>${r.pop}k</td>
        <td>${r.actF}/${r.maxB}</td>
        <td class="c-iron">${r.iron} <span style="color:#64748b">(${r.ironC})</span></td>
        <td class="c-bora">${r.bora} <span style="color:#64748b">(${r.boraC})</span></td>
        <td class="c-germ">${r.germ} <span style="color:#64748b">(${r.germC})</span></td>
        <td style="color:#38bdf8;font-weight:bold;">${r.res}</td>
        <td>${r.def}</td>
      </tr>`,
      )
      .join('');
  }

  private renderFleetTable(snap: SimSnapshot): void {
    getEl('repBtnFleets').textContent = `Fleets in Flight (${snap.fleets.length})`;
    const colors = ['#475569', '#4ade80', '#38bdf8', '#facc15'];
    const minLetters = ['I', 'B', 'G'];
    const minCols = ['#60a5fa', '#34d399', '#fbbf24'];

    const slice = snap.fleets.slice(-80).reverse();
    getEl('fleetTableBody').innerHTML = slice
      .map((fl) => {
        const target = snap.planets[fl.target];
        const dist = target ? Math.hypot(target.x - fl.x, target.y - fl.y) : 0;
        const eta = dist / this.lastConfig.fleetSpeed;
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
          <td><span class="dot" style="background:${colors[fl.emp]}"></span></td>
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
