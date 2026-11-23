import type { ArenaMatchSummary, ReplayFileInfo } from './types';

/** Actions the Replay panel asks the app to perform. */
export interface ReplayPanelCallbacks {
  onRefresh: (serverUrl: string) => void;
  onPlay: (serverUrl: string, sourceMatchId: string) => void;
  onDownload: (serverUrl: string, sourceMatchId: string) => void;
  onPause: () => void;
  onResume: (speed: number) => void;
  onStep: () => void;
  onSpeed: (speed: number) => void;
  onSeek: (tick: number) => void;
  onStop: () => void;
}

const MIN_SPEED = 0.1;
const MAX_SPEED = 100;

function el<T extends HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new Error(`Replay panel element #${id} missing`);
  return node as T;
}

function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
}

function formatSize(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

function formatDate(ms: number): string {
  if (!ms) return '—';
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/**
 * Dedicated drawer for recorded arena matches: lists replays on the server and drives a
 * server-side playback (pause / step / speed 0.1–100x / jump forward / stop).
 */
export class ReplayPanel {
  private playback: ArenaMatchSummary | null = null;
  private speed = 1;
  private paused = true;
  private lastTick = 0;
  private lastYear = 0;

  constructor(private readonly cb: ReplayPanelCallbacks) {
    el('btnReplayRefresh').addEventListener('click', () => this.cb.onRefresh(this.serverUrl()));
    el('replayListBody').addEventListener('click', (ev) => {
      const btn = (ev.target as HTMLElement).closest<HTMLButtonElement>('button[data-replay-action]');
      if (!btn) return;
      const id = btn.dataset.matchId ?? '';
      if (btn.dataset.replayAction === 'play') this.cb.onPlay(this.serverUrl(), id);
      else if (btn.dataset.replayAction === 'download') this.cb.onDownload(this.serverUrl(), id);
    });
    el('btnReplayPlayPause').addEventListener('click', () => {
      if (this.paused) this.cb.onResume(this.speed);
      else this.cb.onPause();
    });
    el('btnReplayStep').addEventListener('click', () => this.cb.onStep());
    document.querySelectorAll<HTMLButtonElement>('[data-replay-speed]').forEach((b) => {
      b.addEventListener('click', () => this.applySpeed(Number(b.dataset.replaySpeed)));
    });
    const customInput = el<HTMLInputElement>('replaySpeedCustom');
    el('btnReplaySpeedApply').addEventListener('click', () => this.applySpeed(Number(customInput.value)));
    customInput.addEventListener('keydown', (ev) => {
      if (ev.key === 'Enter') this.applySpeed(Number(customInput.value));
    });
    const seekInput = el<HTMLInputElement>('replaySeekTick');
    const doSeek = () => {
      const target = Math.floor(Number(seekInput.value));
      const total = this.playback?.replayTotalTicks ?? 0;
      if (!Number.isFinite(target) || target <= this.lastTick || target > total) {
        this.setHint(`Enter a tick between ${this.lastTick + 1} and ${total}.`);
        return;
      }
      this.setHint('');
      this.cb.onSeek(target);
    };
    el('btnReplaySeek').addEventListener('click', doSeek);
    seekInput.addEventListener('keydown', (ev) => {
      if (ev.key === 'Enter') doSeek();
    });
    el('btnReplayStop').addEventListener('click', () => this.cb.onStop());
    this.renderPlayback();
  }

  public serverUrl(): string {
    return el<HTMLInputElement>('replayServerInput').value;
  }

  public setServerUrl(url: string): void {
    const input = el<HTMLInputElement>('replayServerInput');
    if (!input.value || document.activeElement !== input) input.value = url;
  }

  public setListMessage(msg: string): void {
    el('replayListBody').innerHTML = `<div class="replay-empty">${escapeHtml(msg)}</div>`;
  }

  public setReplays(list: ReplayFileInfo[]): void {
    if (list.length === 0) {
      this.setListMessage('No recorded matches on this server yet. A replay is saved when a match ends.');
      return;
    }
    el('replayListBody').innerHTML = list
      .map((r) => {
        const id = escapeHtml(r.matchId);
        const head = `<div class="replay-item-head">
            <span class="replay-item-title" title="seed ${r.seed ?? '?'} · engine ${escapeHtml(r.engineCommit ?? '?')}">${formatDate(r.modifiedMs)} · ${id}</span>
            ${r.error ? '' : `<button class="hud-btn" data-replay-action="play" data-match-id="${id}" title="Play this match on the server">▶</button>`}
            <button class="hud-btn" data-replay-action="download" data-match-id="${id}" title="Download .replay.jsonl.gz">⬇</button>
          </div>`;
        if (r.error) {
          return `<div class="replay-item">${head}<div class="replay-error">⚠ ${escapeHtml(r.error)}</div></div>`;
        }
        const winner = r.winnerName ? escapeHtml(r.winnerName) : r.winner != null ? `#${r.winner + 1}` : 'no winner';
        const ticks = r.finalTick != null ? `tick ${r.finalTick} · yr ${(r.finalYear ?? 0).toFixed(1)}` : '';
        return `<div class="replay-item">${head}
          <div class="replay-item-meta">${escapeHtml(r.scenarioName ?? '?')} · ${escapeHtml(r.galaxySize ?? '?')} · ${r.numEmpires ?? '?'} races</div>
          <div class="replay-item-meta">${escapeHtml(r.endReason ?? '?')} · ${winner} · ${ticks} · ${formatSize(r.fileSize)}</div>
        </div>`;
      })
      .join('');
  }

  /** Called with every summary of the connected match; non-playback matches hide the controls. */
  public updateSummary(summary: ArenaMatchSummary | null): void {
    this.playback = summary && summary.replayOf ? summary : null;
    if (this.playback) {
      this.paused = this.playback.paused ?? true;
      if (!this.paused && this.playback.speedMult) this.speed = this.playback.speedMult;
      this.lastTick = Math.max(this.lastTick, this.playback.tick);
      this.lastYear = this.playback.year;
    } else {
      this.lastTick = 0;
    }
    this.renderPlayback();
  }

  /** Called with every frame of the connected match (progress). */
  public updateTick(tick: number, year: number): void {
    if (!this.playback) return;
    this.lastTick = tick;
    this.lastYear = year;
    this.renderProgress();
  }

  public setHint(msg: string): void {
    el('replayHint').textContent = msg;
  }

  private applySpeed(value: number): void {
    if (!Number.isFinite(value) || value < MIN_SPEED || value > MAX_SPEED) {
      this.setHint(`Speed must be between ${MIN_SPEED}x and ${MAX_SPEED}x.`);
      return;
    }
    this.setHint('');
    this.speed = Math.round(value * 100) / 100;
    // While paused only remember the speed; it is applied on ▶ Play.
    if (!this.paused) this.cb.onSpeed(this.speed);
    this.renderPlayback();
  }

  private renderPlayback(): void {
    const pb = this.playback;
    el('replayPlaybackSection').style.display = pb ? '' : 'none';
    el('replayIdleHint').style.display = pb ? 'none' : '';
    if (!pb) return;
    const ended = pb.status === 'Finished';
    el('replaySource').textContent = `Replay of ${pb.replayOf} · recorded with ${(pb.replayRecordedCommit ?? '?').slice(0, 12)}`;
    const playBtn = el<HTMLButtonElement>('btnReplayPlayPause');
    playBtn.textContent = this.paused ? '▶ Play' : '⏸ Pause';
    playBtn.disabled = ended;
    el<HTMLButtonElement>('btnReplayStep').disabled = ended;
    el<HTMLButtonElement>('btnReplaySeek').disabled = ended;
    document.querySelectorAll<HTMLButtonElement>('[data-replay-speed]').forEach((b) => {
      b.classList.toggle('active', Number(b.dataset.replaySpeed) === this.speed);
    });
    const custom = el<HTMLInputElement>('replaySpeedCustom');
    if (document.activeElement !== custom) custom.value = String(this.speed);
    const div = el('replayDivergence');
    div.style.display = pb.replayDivergence ? '' : 'none';
    div.textContent = pb.replayDivergence
      ? `⚠ Diverged from the recording: ${pb.replayDivergence}. Playback continues as a what-if run (details in the server anomaly log).`
      : '';
    el('replayEnded').style.display = ended ? '' : 'none';
    this.renderProgress();
  }

  private renderProgress(): void {
    const pb = this.playback;
    if (!pb) return;
    const total = Math.max(1, pb.replayTotalTicks ?? 1);
    const pct = Math.min(100, (this.lastTick / total) * 100);
    el('replayProgressFill').style.width = `${pct.toFixed(2)}%`;
    el('replayProgressText').textContent =
      `Tick ${this.lastTick} / ${total} · Year ${this.lastYear.toFixed(1)} · ${this.paused ? 'paused' : `${this.speed}x`}`;
  }
}
