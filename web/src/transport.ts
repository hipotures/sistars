import type {
  ArenaFeedEvent,
  ArenaMatchStatus,
  ArenaMatchSummary,
  CoEvolutionResult,
  CreateArenaMatchResponse,
  DebrisFieldView,
  EngineMeta,
  FleetView,
  HudSnapshot,
  PlanetView,
  RaceDef,
  ReportsSnapshot,
  ScenarioModifiers,
  SimConfig,
  StarSystemView,
  TechCatalogView,
  TournamentResult,
  TransportMode,
} from './types';
import type { InitialSimSettings, MainToWorkerMsg, WorkerToMainMsg } from './workerProtocol';

export interface TransportFrame {
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

export interface TransportInitResult {
  meta: EngineMeta;
  catalog: TechCatalogView;
  config: SimConfig;
  firstFrame: TransportFrame;
}

export interface ApplyScenarioPayload {
  seed: number;
  modifiers: ScenarioModifiers;
  galaxySize: string;
  galaxyShape: string;
  wormholePairs: number;
  crustRichness: number;
  gateCost: number;
  colCostMult: number;
  interceptRad: number;
  salvageRate: number;
  aggroBias: number;
  sprawlPenalty: number;
}

export type ArenaConnectionState = 'connecting' | 'connected' | 'disconnected' | 'error';

export interface SimTransport {
  readonly mode: TransportMode;
  init(seed: number, initialSettings?: InitialSimSettings): Promise<TransportInitResult>;
  onFrame(handler: (frame: TransportFrame) => void): void;
  onArenaFeed?(handler: (event: ArenaFeedEvent) => void): void;
  onArenaStatus?(handler: (status: ArenaMatchSummary) => void): void;
  onConnectionState?(handler: (state: ArenaConnectionState, message?: string) => void): void;
  returnBuffers(planets: ArrayBuffer, fleets: ArrayBuffer, debris: ArrayBuffer): void;

  // Local simulation controls (also bridged to POST /control in Remote Arena Spectator Mode)
  setSpeed(speed: number): void;
  setPaused(paused: boolean): void;
  setExecutionMode?(mode: 'realtimeAsync' | 'lockstepEpoch'): void;
  step(ticks: number): void;
  newGalaxy(seed: number, size?: string, shape?: string): void;
  applyScenario(payload: ApplyScenarioPayload): void;
  updateConfig(key: string, value: number, seed: number): void;
  setRaces(races: RaceDef[], seed: number): void;
  setSelectedFleet(fleetId: number | null): void;
  setScrubIndex(index: number | null): void;
  setProfiling(enabled: boolean): void;

  // On-demand queries (implemented in both Local WASM and Remote Arena)
  getPlanetDetail(index: number): Promise<PlanetView | null>;
  getStarSystem(index: number): Promise<StarSystemView | null>;
  getFleetById(id: number): Promise<FleetView | null>;
  getFleetByIndex(index: number): Promise<FleetView | null>;
  getDebrisDetail(index: number): Promise<DebrisFieldView | null>;
  getPlanetFleets(index: number): Promise<FleetView[]>;
  getReportsSnapshot(): Promise<ReportsSnapshot>;
  getPowerHistory(): Promise<number[][]>;
  exportStateJson(): Promise<string>;
  runTournament(numGames: number, ticksPerGame: number, seed: number): Promise<TournamentResult | null>;
  trainAi(evoSeed: number, resetSeed: number): Promise<CoEvolutionResult | null>;
  dispose(): void;
}

/**
 * Local offline WASM transport wrapping `sim.worker.ts` with zero regressions.
 */
export class LocalWasmTransport implements SimTransport {
  public readonly mode: TransportMode = 'local';
  private readonly worker: Worker;
  private nextReqId = 1;
  private readonly pending = new Map<number, (val: unknown) => void>();
  private frameHandler: ((frame: TransportFrame) => void) | null = null;
  private disposed = false;

  constructor() {
    this.worker = new Worker(new URL('./sim.worker.ts', import.meta.url), {
      type: 'module',
    });
  }

  private post(msg: MainToWorkerMsg, transfer?: Transferable[]): void {
    if (this.disposed) return;
    if (transfer && transfer.length > 0) {
      this.worker.postMessage(msg, transfer);
    } else {
      this.worker.postMessage(msg);
    }
  }

  private request<T>(buildMsg: (reqId: number) => MainToWorkerMsg): Promise<T> {
    const reqId = this.nextReqId++;
    return new Promise<T>((resolve) => {
      this.pending.set(reqId, (val) => resolve(val as T));
      this.post(buildMsg(reqId));
    });
  }

  public async init(seed: number, initialSettings?: InitialSimSettings): Promise<TransportInitResult> {
    let resolveReady!: (init: { meta: EngineMeta; catalog: TechCatalogView; config: SimConfig }) => void;
    const readyPromise = new Promise<{ meta: EngineMeta; catalog: TechCatalogView; config: SimConfig }>((res) => {
      resolveReady = res;
    });

    let resolveFirstFrame!: (msg: TransportFrame) => void;
    const firstFramePromise = new Promise<TransportFrame>((res) => {
      resolveFirstFrame = res;
    });
    let gotFirstFrame = false;

    this.worker.onmessage = (event: MessageEvent<WorkerToMainMsg>): void => {
      const msg = event.data;
      switch (msg.type) {
        case 'ready':
          resolveReady({ meta: msg.meta, catalog: msg.catalog, config: msg.config });
          break;
        case 'frame':
          if (!gotFirstFrame) {
            gotFirstFrame = true;
            resolveFirstFrame(msg);
          } else if (this.frameHandler) {
            this.frameHandler(msg);
          } else {
            this.returnBuffers(msg.planets, msg.fleets, msg.debris);
          }
          break;
        case 'planetDetail':
        case 'starSystem':
        case 'fleetDetail':
        case 'debrisDetail':
        case 'planetFleets':
        case 'reportsSnapshot':
        case 'powerHistory':
        case 'exportStateJson':
        case 'tournamentResult':
        case 'coEvolutionResult': {
          const resolver = this.pending.get(msg.reqId);
          if (resolver) {
            this.pending.delete(msg.reqId);
            resolver(msg.data);
          }
          break;
        }
      }
    };

    this.post({ type: 'init', seed, initialSettings });
    const [{ meta, catalog, config }, firstFrame] = await Promise.all([readyPromise, firstFramePromise]);
    return { meta, catalog, config, firstFrame };
  }

  public onFrame(handler: (frame: TransportFrame) => void): void {
    this.frameHandler = handler;
  }

  public returnBuffers(planets: ArrayBuffer, fleets: ArrayBuffer, debris: ArrayBuffer): void {
    this.post({ type: 'returnBuffers', planets, fleets, debris }, [planets, fleets, debris]);
  }

  public setSpeed(speed: number): void {
    this.post({ type: 'setSpeed', speed });
  }

  public setPaused(paused: boolean): void {
    this.post({ type: 'setPaused', paused });
  }

  public step(ticks: number): void {
    this.post({ type: 'step', ticks });
  }

  public newGalaxy(seed: number, size?: string, shape?: string): void {
    this.post({ type: 'newGalaxy', seed, size, shape });
  }

  public applyScenario(payload: ApplyScenarioPayload): void {
    this.post({ type: 'applyScenario', ...payload });
  }

  public updateConfig(key: string, value: number, seed: number): void {
    this.post({ type: 'updateConfig', key, value, seed });
  }

  public setRaces(races: RaceDef[], seed: number): void {
    this.post({ type: 'setRaces', races, seed });
  }

  public setSelectedFleet(fleetId: number | null): void {
    this.post({ type: 'setSelectedFleet', fleetId });
  }

  public setScrubIndex(index: number | null): void {
    this.post({ type: 'setScrubIndex', index });
  }

  public setProfiling(enabled: boolean): void {
    this.post({ type: 'setProfiling', enabled });
  }

  public getPlanetDetail(index: number): Promise<PlanetView | null> {
    return this.request((reqId) => ({ type: 'getPlanetDetail', reqId, index }));
  }

  public getStarSystem(index: number): Promise<StarSystemView | null> {
    return this.request((reqId) => ({ type: 'getStarSystem', reqId, index }));
  }

  public getFleetById(id: number): Promise<FleetView | null> {
    return this.request((reqId) => ({ type: 'getFleetById', reqId, id }));
  }

  public getFleetByIndex(index: number): Promise<FleetView | null> {
    return this.request((reqId) => ({ type: 'getFleetByIndex', reqId, index }));
  }

  public getDebrisDetail(index: number): Promise<DebrisFieldView | null> {
    return this.request((reqId) => ({ type: 'getDebrisDetail', reqId, index }));
  }

  public getPlanetFleets(index: number): Promise<FleetView[]> {
    return this.request((reqId) => ({ type: 'getPlanetFleets', reqId, index }));
  }

  public getReportsSnapshot(): Promise<ReportsSnapshot> {
    return this.request((reqId) => ({ type: 'getReportsSnapshot', reqId }));
  }

  public getPowerHistory(): Promise<number[][]> {
    return this.request((reqId) => ({ type: 'getPowerHistory', reqId }));
  }

  public exportStateJson(): Promise<string> {
    return this.request((reqId) => ({ type: 'exportStateJson', reqId }));
  }

  public runTournament(numGames: number, ticksPerGame: number, seed: number): Promise<TournamentResult> {
    return this.request((reqId) => ({ type: 'runTournament', reqId, numGames, ticksPerGame, seed }));
  }

  public trainAi(evoSeed: number, resetSeed: number): Promise<CoEvolutionResult> {
    return this.request((reqId) => ({ type: 'trainAi', reqId, evoSeed, resetSeed }));
  }

  public dispose(): void {
    this.disposed = true;
    this.pending.clear();
    this.worker.terminate();
  }
}

interface WsInitPayload {
  type: 'init';
  matchId: string;
  meta: EngineMeta;
  catalog: TechCatalogView;
  config: SimConfig;
  summary: ArenaMatchSummary;
  hud: HudSnapshot;
  planetsBuf: number[];
  fleetsBuf: number[];
  debrisBuf: number[];
  feedHistory: ArenaFeedEvent[];
}

interface WsFramePayload {
  type: 'frame';
  epoch: number;
  tick: number;
  status: ArenaMatchStatus;
  winner: number | null;
  waitingForSeats: number[];
  deadlineMs: number | null;
  hud: HudSnapshot;
  planetsBuf: number[];
  fleetsBuf: number[];
  debrisBuf: number[];
  tps: number;
}

interface WsFeedPayload {
  type: 'feed';
  event: ArenaFeedEvent;
}

interface WsStatusPayload {
  type: 'status';
  summary: ArenaMatchSummary;
}

interface WsControlPayload {
  type: 'control';
  paused: boolean;
  speedMult: number;
  executionMode: 'realtimeAsync' | 'lockstepEpoch';
}

type WsMessage = WsInitPayload | WsFramePayload | WsFeedPayload | WsStatusPayload | WsControlPayload;

function toFloat32ArrayBuffer(arr: number[]): ArrayBuffer {
  const f32 = Float32Array.from(arr);
  return f32.buffer;
}

export function normalizeServerUrl(url: string): string {
  return url.trim().replace(/\/+$/, '') || 'http://localhost:8080';
}

export function httpToWsUrl(httpUrl: string): string {
  const clean = normalizeServerUrl(httpUrl);
  if (clean.startsWith('https://')) return 'wss://' + clean.slice(8);
  if (clean.startsWith('http://')) return 'ws://' + clean.slice(7);
  if (clean.startsWith('ws://') || clean.startsWith('wss://')) return clean;
  return 'ws://' + clean;
}

export async function fetchArenaMatches(serverUrl: string): Promise<ArenaMatchSummary[]> {
  const base = normalizeServerUrl(serverUrl);
  const resp = await fetch(`${base}/api/v1/matches`);
  if (!resp.ok) {
    throw new Error(`HTTP ${resp.status} listing matches`);
  }
  return (await resp.json()) as ArenaMatchSummary[];
}

export async function fetchMcpSeats(serverUrl: string): Promise<import('./types').McpSeatPublicInfo[]> {
  const base = normalizeServerUrl(serverUrl);
  try {
    const resp = await fetch(`${base}/api/v1/mcp/seats`);
    if (!resp.ok) return [];
    return (await resp.json()) as import('./types').McpSeatPublicInfo[];
  } catch {
    return [];
  }
}

export async function bindMcpSeat(
  serverUrl: string,
  seatName: string,
  matchId: string | null,
  empireIdx?: number,
): Promise<boolean> {
  const base = normalizeServerUrl(serverUrl);
  const resp = await fetch(`${base}/api/v1/mcp/seats/${encodeURIComponent(seatName)}/bind`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      matchId,
      empireIdx: matchId === null ? null : (empireIdx ?? 0),
    }),
  });
  return resp.ok;
}

export async function deleteArenaMatch(serverUrl: string, matchId: string): Promise<boolean> {
  const base = normalizeServerUrl(serverUrl);
  const resp = await fetch(`${base}/api/v1/matches/${encodeURIComponent(matchId)}`, {
    method: 'DELETE',
  });
  return resp.ok || resp.status === 204;
}

export async function createArenaMatch(
  serverUrl: string,
  options?: {
    seed?: number;
    scenarioName?: string;
    simConfig?: SimConfig;
    scenarioModifiers?: ScenarioModifiers;
    galaxySize?: string;
    galaxyShape?: string;
    executionMode?: 'realtimeAsync' | 'lockstepEpoch';
    speedMult?: number;
    ticksPerEpoch?: number;
    epochTimeoutMs?: number;
    agentSeats?: number[];
    debugAudit?: boolean;
    frameStrideTicks?: number;
    tickDelayMs?: number;
    startPaused?: boolean;
  },
): Promise<CreateArenaMatchResponse> {
  const base = normalizeServerUrl(serverUrl);
  const body: Record<string, unknown> = {
    galaxySize: options?.galaxySize ?? 'small',
    galaxyShape: options?.galaxyShape ?? 'spiral',
    executionMode: options?.executionMode ?? 'realtimeAsync',
    speedMult: options?.speedMult ?? 1.0,
    ticksPerEpoch: options?.ticksPerEpoch ?? 20,
    epochTimeoutMs: options?.epochTimeoutMs ?? 20000,
    agentSeats: options?.agentSeats ?? [],
    debugAudit: options?.debugAudit ?? true,
    startPaused: options?.startPaused ?? false,
  };
  if (options?.scenarioName) {
    body.scenarioName = options.scenarioName;
  }
  if (options?.frameStrideTicks !== undefined) {
    body.frameStrideTicks = options.frameStrideTicks;
  }
  if (options?.tickDelayMs !== undefined) {
    body.tickDelayMs = options.tickDelayMs;
  }
  if (options?.simConfig) {
    body.simConfig = options.simConfig;
  }
  if (options?.scenarioModifiers) {
    body.scenarioModifiers = options.scenarioModifiers;
  }
  if (options?.seed !== undefined && Number.isFinite(options.seed)) {
    body.seed = Math.floor(options.seed);
  }
  const resp = await fetch(`${base}/api/v1/matches`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
  if (!resp.ok) {
    throw new Error(`HTTP ${resp.status} creating match`);
  }
  return (await resp.json()) as CreateArenaMatchResponse;
}

export function speedToArenaControl(speed: number): {
  paused: boolean;
  tickDelayMs: number;
  frameStrideTicks: number;
} {
  if (speed <= 0) return { paused: true, tickDelayMs: 25, frameStrideTicks: 1 };
  if (speed <= 0.1) return { paused: false, tickDelayMs: 250, frameStrideTicks: 1 };
  if (speed <= 0.25) return { paused: false, tickDelayMs: 120, frameStrideTicks: 1 };
  if (speed <= 0.5) return { paused: false, tickDelayMs: 60, frameStrideTicks: 1 };
  if (speed <= 1) return { paused: false, tickDelayMs: 25, frameStrideTicks: 1 };
  if (speed <= 5) return { paused: false, tickDelayMs: 5, frameStrideTicks: 2 };
  return { paused: false, tickDelayMs: 0, frameStrideTicks: 5 };
}

/**
 * Remote Spectator Mode transport backed by `crates/server` WebSocket + REST API.
 */
export class RemoteArenaTransport implements SimTransport {
  public readonly mode: TransportMode = 'remote';
  private readonly serverUrl: string;
  private readonly matchId: string;
  private ws: WebSocket | null = null;
  private frameHandler: ((frame: TransportFrame) => void) | null = null;
  private feedHandler: ((event: ArenaFeedEvent) => void) | null = null;
  private statusHandler: ((status: ArenaMatchSummary) => void) | null = null;
  private connHandler: ((state: ArenaConnectionState, message?: string) => void) | null = null;
  private selectedFleetId: number | null = null;
  private lastSummary: ArenaMatchSummary | null = null;
  private pendingFeedHistory: ArenaFeedEvent[] = [];
  private disposed = false;

  constructor(serverUrl: string, matchId: string) {
    this.serverUrl = normalizeServerUrl(serverUrl);
    this.matchId = matchId;
  }

  public init(): Promise<TransportInitResult> {
    const wsUrl = `${httpToWsUrl(this.serverUrl)}/ws/matches/${encodeURIComponent(this.matchId)}/spectate`;
    this.connHandler?.('connecting', `Connecting to ${this.matchId}...`);

    return new Promise<TransportInitResult>((resolve, reject) => {
      let settled = false;
      const ws = new WebSocket(wsUrl);
      this.ws = ws;
      const initTimer = window.setTimeout(() => {
        if (!settled) {
          settled = true;
          try {
            ws.close();
          } catch {
            // ignore
          }
          this.connHandler?.('error', `Timed out connecting to ${this.matchId}`);
          reject(new Error(`Timed out waiting for Spectator WebSocket init (${wsUrl})`));
        }
      }, 10000);
      const clearInitTimer = () => window.clearTimeout(initTimer);

      ws.onopen = () => {
        this.connHandler?.('connected', `Live Spectator: ${this.matchId}`);
      };

      ws.onerror = () => {
        if (!settled) {
          settled = true;
          clearInitTimer();
          this.connHandler?.('error', `WebSocket error connecting to ${wsUrl}`);
          reject(new Error(`Failed to connect to Spectator WebSocket at ${wsUrl}`));
        } else {
          this.connHandler?.('error', 'WebSocket stream error');
        }
      };

      ws.onclose = () => {
        if (!settled) {
          settled = true;
          clearInitTimer();
          this.connHandler?.('disconnected', 'Connection closed before init');
          reject(new Error('WebSocket closed before receiving initial arena state'));
        } else if (!this.disposed) {
          this.connHandler?.('disconnected', 'Spectator stream disconnected');
        }
      };

      ws.onmessage = (ev: MessageEvent<string>) => {
        if (this.disposed) return;
        let msg: WsMessage;
        try {
          msg = JSON.parse(ev.data) as WsMessage;
        } catch {
          return;
        }

        if (msg.type === 'init') {
          this.lastSummary = msg.summary;
          this.statusHandler?.(msg.summary);
          if (this.feedHandler) {
            for (const item of msg.feedHistory) {
              this.feedHandler(item);
            }
          } else {
            this.pendingFeedHistory = msg.feedHistory.slice();
          }

          const firstFrame: TransportFrame = {
            hud: msg.hud,
            config: msg.config,
            selectedFleet: null,
            planets: toFloat32ArrayBuffer(msg.planetsBuf),
            planetsLen: msg.planetsBuf.length,
            fleets: toFloat32ArrayBuffer(msg.fleetsBuf),
            fleetsLen: msg.fleetsBuf.length,
            debris: toFloat32ArrayBuffer(msg.debrisBuf),
            debrisLen: msg.debrisBuf.length,
            tps: 0,
            wasmMs: 0,
            stepsDone: 0,
            resetVisuals: true,
            resetHistory: true,
          };

          if (!settled) {
            settled = true;
            clearInitTimer();
            resolve({
              meta: msg.meta,
              catalog: msg.catalog,
              config: msg.config,
              firstFrame,
            });
          } else if (this.frameHandler) {
            this.frameHandler(firstFrame);
          }
          return;
        }

        if (msg.type === 'frame') {
          if (this.lastSummary) {
            this.lastSummary = {
              ...this.lastSummary,
              epoch: msg.epoch,
              tick: msg.tick,
              year: msg.hud.year,
              status: msg.status,
              winner: msg.winner,
              waitingForSeats: msg.waitingForSeats,
              epochDeadlineMs: msg.deadlineMs,
            };
            this.statusHandler?.(this.lastSummary);
          }

          const frame: TransportFrame = {
            hud: msg.hud,
            selectedFleet: null,
            planets: toFloat32ArrayBuffer(msg.planetsBuf),
            planetsLen: msg.planetsBuf.length,
            fleets: toFloat32ArrayBuffer(msg.fleetsBuf),
            fleetsLen: msg.fleetsBuf.length,
            debris: toFloat32ArrayBuffer(msg.debrisBuf),
            debrisLen: msg.debrisBuf.length,
            tps: msg.tps,
            wasmMs: 0,
            stepsDone: 1,
            resetVisuals: false,
            resetHistory: false,
          };

          if (this.selectedFleetId !== null) {
            const targetId = this.selectedFleetId;
            void this.getFleetById(targetId).then((fl) => {
              if (!this.disposed && this.frameHandler) {
                this.frameHandler({ ...frame, selectedFleet: fl });
              }
            });
          } else {
            this.frameHandler?.(frame);
          }
          return;
        }

        if (msg.type === 'feed') {
          this.feedHandler?.(msg.event);
          return;
        }

        if (msg.type === 'status') {
          this.lastSummary = msg.summary;
          this.statusHandler?.(msg.summary);
          return;
        }

        if (msg.type === 'control') {
          if (this.lastSummary) {
            this.lastSummary = {
              ...this.lastSummary,
              paused: msg.paused,
              speedMult: msg.speedMult,
              executionMode: msg.executionMode,
            };
            this.statusHandler?.(this.lastSummary);
          }
        }
      };
    });
  }

  public onFrame(handler: (frame: TransportFrame) => void): void {
    this.frameHandler = handler;
  }

  public onArenaFeed(handler: (event: ArenaFeedEvent) => void): void {
    this.feedHandler = handler;
    if (this.pendingFeedHistory.length > 0) {
      const queued = this.pendingFeedHistory.splice(0);
      for (const item of queued) {
        handler(item);
      }
    }
  }

  public onArenaStatus(handler: (status: ArenaMatchSummary) => void): void {
    this.statusHandler = handler;
    if (this.lastSummary) {
      handler(this.lastSummary);
    }
  }

  public onConnectionState(handler: (state: ArenaConnectionState, message?: string) => void): void {
    this.connHandler = handler;
  }

  public returnBuffers(): void {
    // Remote frames allocate fresh buffers from WebSocket JSON; no worker pool to recycle
  }

  private speedDebounceTimer: ReturnType<typeof setTimeout> | null = null;

  private postControl(payload: {
    paused?: boolean;
    speedMult?: number;
    executionMode?: 'realtimeAsync' | 'lockstepEpoch';
    tickDelayMs?: number;
    frameStrideTicks?: number;
    stepTicks?: number;
  }): void {
    if (this.disposed) return;
    void fetch(`${this.serverUrl}/api/v1/matches/${encodeURIComponent(this.matchId)}/control`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    }).catch(() => {
      // ignore transient network errors
    });
  }

  public setSpeed(speed: number): void {
    if (this.speedDebounceTimer !== null) {
      clearTimeout(this.speedDebounceTimer);
      this.speedDebounceTimer = null;
    }
    if (speed <= 0) {
      this.postControl({ paused: true });
      return;
    }
    this.speedDebounceTimer = setTimeout(() => {
      this.speedDebounceTimer = null;
      this.postControl({ paused: false, speedMult: speed });
    }, 60);
  }

  public setPaused(paused: boolean): void {
    if (this.speedDebounceTimer !== null) {
      clearTimeout(this.speedDebounceTimer);
      this.speedDebounceTimer = null;
    }
    this.postControl({ paused });
  }

  public setExecutionMode(mode: 'realtimeAsync' | 'lockstepEpoch'): void {
    this.postControl({ executionMode: mode });
  }

  public step(ticks: number): void {
    this.postControl({ paused: true, stepTicks: ticks });
  }

  public newGalaxy(): void {}
  public applyScenario(): void {}
  public updateConfig(): void {}
  public setRaces(): void {}
  public setScrubIndex(): void {}
  public setProfiling(): void {}

  public setSelectedFleet(fleetId: number | null): void {
    this.selectedFleetId = fleetId;
  }

  private async getJson<T>(path: string, fallback: T): Promise<T> {
    try {
      const resp = await fetch(`${this.serverUrl}/api/v1/matches/${encodeURIComponent(this.matchId)}${path}`);
      if (!resp.ok) return fallback;
      return (await resp.json()) as T;
    } catch {
      return fallback;
    }
  }

  public getPlanetDetail(index: number): Promise<PlanetView | null> {
    return this.getJson<PlanetView | null>(`/planet/${index}`, null);
  }

  public getStarSystem(index: number): Promise<StarSystemView | null> {
    return this.getJson<StarSystemView | null>(`/system/${index}`, null);
  }

  public getFleetById(id: number): Promise<FleetView | null> {
    return this.getJson<FleetView | null>(`/fleet/by-id/${id}`, null);
  }

  public getFleetByIndex(index: number): Promise<FleetView | null> {
    return this.getJson<FleetView | null>(`/fleet/by-index/${index}`, null);
  }

  public getDebrisDetail(index: number): Promise<DebrisFieldView | null> {
    return this.getJson<DebrisFieldView | null>(`/debris/${index}`, null);
  }

  public getPlanetFleets(index: number): Promise<FleetView[]> {
    return this.getJson<FleetView[]>(`/planet/${index}/fleets`, []);
  }

  public getReportsSnapshot(): Promise<ReportsSnapshot> {
    return this.getJson<ReportsSnapshot>('/reports', { planets: [], fleets: [], debrisFields: [] });
  }

  public getPowerHistory(): Promise<number[][]> {
    return this.getJson<number[][]>('/power-history', []);
  }

  public async exportStateJson(): Promise<string> {
    try {
      const resp = await fetch(
        `${this.serverUrl}/api/v1/matches/${encodeURIComponent(this.matchId)}/export-state`,
      );
      if (!resp.ok) return '{}';
      return await resp.text();
    } catch {
      return '{}';
    }
  }

  public async runTournament(): Promise<TournamentResult | null> {
    return null;
  }

  public async trainAi(): Promise<CoEvolutionResult | null> {
    return null;
  }

  public dispose(): void {
    this.disposed = true;
    if (this.ws) {
      try {
        this.ws.close();
      } catch {
        // ignore close errors
      }
      this.ws = null;
    }
  }
}
