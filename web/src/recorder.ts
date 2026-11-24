/**
 * "Live REC": records what is shown in the browser to a video file with MediaRecorder.
 *
 * Two sources:
 *  - 'map': the galaxy map only. The WebGPU base canvas and the 2D overlay canvas are
 *    composited into an offscreen canvas right after every render (a WebGPU canvas can only be
 *    copied in the same task it was drawn in), with a small caption (year, match).
 *  - 'tab': the whole browser tab including HUD and panels, via getDisplayMedia (the browser
 *    asks the user which tab/screen to share).
 *
 * Recording is real time: the video shows exactly what the screen showed, including the speed
 * the match was played at. Chunks are kept in memory and saved as one file on Stop.
 */

export type RecordSource = 'map' | 'tab';

export interface RecorderOptions {
  source: RecordSource;
  /** Output height in pixels for 'map' (width follows the map aspect); 0 = native size. */
  height: number;
  fps: number;
  bitsPerSecond: number;
}

export interface RecorderStatus {
  recording: boolean;
  elapsedMs: number;
  bytes: number;
  mimeType: string;
}

const MIME_CANDIDATES = [
  'video/mp4;codecs=avc1.640028',
  'video/mp4;codecs=avc1',
  'video/mp4',
  'video/webm;codecs=vp9',
  'video/webm;codecs=vp8',
  'video/webm',
];

export function pickMimeType(): string | null {
  if (typeof MediaRecorder === 'undefined') return null;
  for (const m of MIME_CANDIDATES) {
    if (MediaRecorder.isTypeSupported(m)) return m;
  }
  return null;
}

export function recordingSupported(): boolean {
  return pickMimeType() !== null;
}

export class LiveRecorder {
  private mediaRecorder: MediaRecorder | null = null;
  private stream: MediaStream | null = null;
  private chunks: Blob[] = [];
  private bytes = 0;
  private startedAt = 0;
  private mimeType = '';
  private composite: HTMLCanvasElement | null = null;
  private compositeCtx: CanvasRenderingContext2D | null = null;
  private fileBase = 'sistars';
  private tickTimer: number | null = null;
  private frameIntervalMs = 1000 / 30;
  private lastCaptureMs = 0;

  constructor(
    private readonly baseCanvas: HTMLCanvasElement,
    private readonly overlayCanvas: HTMLCanvasElement,
    /** Caption burned into 'map' recordings (e.g. "Year 2411.2 · Replay of abc"). */
    private readonly caption: () => string,
    private readonly onStatus: (s: RecorderStatus) => void,
  ) {}

  public get recording(): boolean {
    return this.mediaRecorder !== null;
  }

  public async start(opts: RecorderOptions, fileBase: string): Promise<void> {
    if (this.mediaRecorder) return;
    const mime = pickMimeType();
    if (!mime) throw new Error('This browser cannot record video (MediaRecorder unsupported).');
    this.mimeType = mime;
    this.fileBase = fileBase;
    this.frameIntervalMs = 1000 / Math.max(1, opts.fps);
    this.lastCaptureMs = 0;

    let stream: MediaStream;
    if (opts.source === 'tab') {
      const md = navigator.mediaDevices as MediaDevices & {
        getDisplayMedia?: (c: DisplayMediaStreamOptions & Record<string, unknown>) => Promise<MediaStream>;
      };
      if (!md?.getDisplayMedia) throw new Error('Screen capture is not available in this browser.');
      stream = await md.getDisplayMedia({
        video: { frameRate: opts.fps },
        audio: false,
        preferCurrentTab: true,
        selfBrowserSurface: 'include',
      });
    } else {
      const c = document.createElement('canvas');
      const srcW = Math.max(2, this.baseCanvas.width || this.overlayCanvas.width);
      const srcH = Math.max(2, this.baseCanvas.height || this.overlayCanvas.height);
      const h = opts.height > 0 ? Math.min(opts.height, srcH) : srcH;
      // Encoders want even dimensions.
      c.height = Math.floor(h / 2) * 2;
      c.width = Math.floor((srcW * (c.height / srcH)) / 2) * 2;
      const ctx = c.getContext('2d', { alpha: false });
      if (!ctx) throw new Error('Could not create the recording canvas.');
      this.composite = c;
      this.compositeCtx = ctx;
      this.captureFrame();
      stream = c.captureStream(opts.fps);
    }

    const rec = new MediaRecorder(stream, { mimeType: mime, videoBitsPerSecond: opts.bitsPerSecond });
    this.chunks = [];
    this.bytes = 0;
    rec.ondataavailable = (ev) => {
      if (ev.data.size > 0) {
        this.chunks.push(ev.data);
        this.bytes += ev.data.size;
      }
    };
    rec.onstop = () => this.finish();
    // The user can end a tab capture from the browser's own "Stop sharing" bar.
    stream.getVideoTracks().forEach((t) => t.addEventListener('ended', () => this.stop()));
    this.stream = stream;
    this.mediaRecorder = rec;
    this.startedAt = performance.now();
    rec.start(1000);
    this.tickTimer = window.setInterval(() => this.emitStatus(), 500);
    this.emitStatus();
  }

  /** Stops recording; the file is downloaded once the recorder flushes its last chunk. */
  public stop(): void {
    const rec = this.mediaRecorder;
    if (!rec || rec.state === 'inactive') return;
    rec.stop();
  }

  /** True when a 'map' recording wants its next frame (throttled to the chosen FPS). */
  public frameDue(): boolean {
    return this.compositeCtx !== null && performance.now() - this.lastCaptureMs >= this.frameIntervalMs - 1;
  }

  /**
   * Call right after the map has been drawn (same task), to copy it into the recording.
   * Copying a WebGPU canvas is a synchronous GPU readback, so it is throttled to the FPS.
   */
  public captureFrame(): void {
    const c = this.composite;
    const ctx = this.compositeCtx;
    if (!c || !ctx || !this.frameDue()) return;
    this.lastCaptureMs = performance.now();
    ctx.fillStyle = '#05070d';
    ctx.fillRect(0, 0, c.width, c.height);
    if (this.baseCanvas.style.visibility !== 'hidden' && this.baseCanvas.width > 0) {
      ctx.drawImage(this.baseCanvas, 0, 0, c.width, c.height);
    }
    if (this.overlayCanvas.width > 0) {
      ctx.drawImage(this.overlayCanvas, 0, 0, c.width, c.height);
    }
    const text = this.caption();
    if (text) {
      const fs = Math.max(12, Math.round(c.height / 45));
      ctx.font = `600 ${fs}px system-ui, sans-serif`;
      const pad = Math.round(fs * 0.6);
      const w = ctx.measureText(text).width + pad * 2;
      ctx.fillStyle = 'rgba(5, 7, 13, 0.7)';
      ctx.fillRect(pad, pad, w, fs + pad * 1.4);
      ctx.fillStyle = '#e2e8f0';
      ctx.textBaseline = 'top';
      ctx.fillText(text, pad * 2, pad * 1.7);
    }
  }

  private finish(): void {
    const blob = new Blob(this.chunks, { type: this.mimeType.split(';')[0] });
    const ext = this.mimeType.startsWith('video/mp4') ? 'mp4' : 'webm';
    const stamp = new Date().toISOString().replace(/[:T]/g, '-').slice(0, 19);
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${this.fileBase}_${stamp}.${ext}`;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 60_000);

    this.stream?.getTracks().forEach((t) => t.stop());
    this.stream = null;
    this.mediaRecorder = null;
    this.composite = null;
    this.compositeCtx = null;
    this.chunks = [];
    if (this.tickTimer !== null) window.clearInterval(this.tickTimer);
    this.tickTimer = null;
    this.emitStatus();
  }

  private emitStatus(): void {
    this.onStatus({
      recording: this.mediaRecorder !== null,
      elapsedMs: this.mediaRecorder ? performance.now() - this.startedAt : 0,
      bytes: this.bytes,
      mimeType: this.mimeType,
    });
  }
}
