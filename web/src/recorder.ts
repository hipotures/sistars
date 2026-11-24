/**
 * "Live REC": records what is shown in the browser to a video file, in real time.
 *
 * Two sources:
 *  - 'map': the galaxy map only. The WebGPU base canvas and the 2D overlay canvas are
 *    composited into an offscreen canvas right after a render (a WebGPU canvas can only be
 *    copied in the same task it was drawn in), with a small caption. Frames are encoded with
 *    WebCodecs at the requested bitrate (MediaRecorder ignored it: ~2 Mb/s of 64 requested,
 *    with large blocks) and muxed into MP4 (H.264) or WebM (VP9). If WebCodecs is missing,
 *    MediaRecorder is used as a fallback.
 *  - 'tab': the whole browser tab including HUD and panels, via getDisplayMedia + MediaRecorder.
 *
 * The video shows exactly what the screen showed, including the speed the match ran at.
 * The file is built in memory and downloaded on Stop.
 */
import { ArrayBufferTarget as Mp4Target, Muxer as Mp4Muxer } from 'mp4-muxer';
import { ArrayBufferTarget as WebmTarget, Muxer as WebmMuxer } from 'webm-muxer';

export type RecordSource = 'map' | 'tab';
/** 'webm' = VP9, 'mp4' = H.264 (plays everywhere). */
export type RecordFormat = 'webm' | 'mp4';

export interface RecorderOptions {
  source: RecordSource;
  /** Output height in pixels for 'map' (width follows the map aspect); 0 = native size. */
  height: number;
  fps: number;
  bitsPerSecond: number;
  format: RecordFormat;
  /**
   * Constant-quality mode ('high' / 'max'): every region of every frame gets the same precision,
   * so dark flat space does not turn into blotches the way bitrate modes starve it. Falls back to
   * bitsPerSecond when the encoder has no quantizer mode.
   */
  constantQuality?: 'high' | 'max';
}

/** Quantizer per codec (lower = better). H.264 0–51, VP9 0–63. */
const QP: Record<RecordFormat, Record<'high' | 'max', number>> = {
  mp4: { high: 20, max: 14 },
  webm: { high: 24, max: 14 },
};

export interface RecorderStatus {
  recording: boolean;
  /** True while the last frames are being encoded and the file is assembled. */
  finishing: boolean;
  elapsedMs: number;
  bytes: number;
  mimeType: string;
  frames: number;
  dropped: number;
}

const MP4_TYPES = ['video/mp4;codecs=avc1.640028', 'video/mp4;codecs=avc1', 'video/mp4'];
const WEBM_TYPES = ['video/webm;codecs=vp9', 'video/webm;codecs=vp8', 'video/webm'];

/** WebCodecs codec strings to try, best first, with the muxer codec id each one maps to. */
const AVC_CODECS = ['avc1.640034', 'avc1.64002a', 'avc1.4d0034', 'avc1.42003e'];
const VP9_CODECS = ['vp09.00.51.08', 'vp09.00.41.08', 'vp09.00.40.08'];

/** Frames waiting in the encoder before new ones are dropped (keeps the page responsive). */
const MAX_ENCODE_QUEUE = 6;

export function pickMimeType(format: RecordFormat = 'webm'): string | null {
  if (typeof MediaRecorder === 'undefined') return null;
  const order = format === 'mp4' ? [...MP4_TYPES, ...WEBM_TYPES] : [...WEBM_TYPES, ...MP4_TYPES];
  for (const m of order) {
    if (MediaRecorder.isTypeSupported(m)) return m;
  }
  return null;
}

export function recordingSupported(): boolean {
  return typeof VideoEncoder !== 'undefined' || pickMimeType() !== null;
}

interface CodecSession {
  encoder: VideoEncoder;
  /** Extra per-frame encode options (the quantizer in constant-quality mode). */
  encodeExtra: Record<string, unknown>;
  finalize: () => Blob;
  mime: string;
}

async function findEncoderConfig(
  format: RecordFormat,
  width: number,
  height: number,
  fps: number,
  bitrate: number,
  tryQuantizer: boolean,
): Promise<VideoEncoderConfig | null> {
  const codecs = format === 'mp4' ? AVC_CODECS : VP9_CODECS;
  const modes = (tryQuantizer ? ['quantizer', 'constant', 'variable'] : ['constant', 'variable']) as (
    | 'quantizer'
    | 'constant'
    | 'variable'
  )[];
  for (const bitrateMode of modes) {
    for (const codec of codecs) {
      const cfg: VideoEncoderConfig = {
        codec,
        width,
        height,
        bitrate,
        bitrateMode: bitrateMode as VideoEncoderBitrateMode,
        framerate: fps,
        latencyMode: 'quality',
        ...(format === 'mp4' ? { avc: { format: 'avc' as const } } : {}),
      };
      try {
        const res = await VideoEncoder.isConfigSupported(cfg);
        if (res.supported) return res.config ?? cfg;
      } catch {
        // try the next one
      }
    }
  }
  return null;
}

export class LiveRecorder {
  // MediaRecorder path ('tab' source, or no WebCodecs)
  private mediaRecorder: MediaRecorder | null = null;
  private stream: MediaStream | null = null;
  private chunks: Blob[] = [];
  // WebCodecs path ('map' source)
  private codec: CodecSession | null = null;

  private active = false;
  private finishing = false;
  private bytes = 0;
  private frames = 0;
  private dropped = 0;
  private startedAt = 0;
  private mimeType = '';
  private fps = 30;
  private composite: HTMLCanvasElement | null = null;
  private compositeCtx: CanvasRenderingContext2D | null = null;
  private fileBase = 'sistars';
  private tickTimer: number | null = null;
  private frameIntervalMs = 1000 / 30;
  private lastCaptureMs = 0;

  constructor(
    private readonly baseCanvas: HTMLCanvasElement,
    private readonly overlayCanvas: HTMLCanvasElement,
    /** Caption burned into 'map' recordings (e.g. "Year 2411.2 · Humanoids vs Rabbitoids"). */
    private readonly caption: () => string,
    private readonly onStatus: (s: RecorderStatus) => void,
    private readonly onError: (message: string) => void = () => {},
  ) {}

  public get recording(): boolean {
    return this.active;
  }

  public async start(opts: RecorderOptions, fileBase: string): Promise<void> {
    if (this.active || this.finishing) return;
    this.fileBase = fileBase;
    this.fps = Math.max(1, opts.fps);
    this.frameIntervalMs = 1000 / this.fps;
    this.lastCaptureMs = 0;
    this.bytes = 0;
    this.frames = 0;
    this.dropped = 0;

    if (opts.source === 'map') {
      this.setupComposite(opts.height);
      if (typeof VideoEncoder !== 'undefined' && typeof VideoFrame !== 'undefined') {
        const session = await this.createCodecSession(opts);
        if (session) {
          this.codec = session;
          this.mimeType = session.mime;
          this.begin();
          this.captureFrame();
          return;
        }
      }
      // Fallback: MediaRecorder on the composite canvas.
      const c = this.composite;
      if (!c) throw new Error('Could not create the recording canvas.');
      this.captureFrame();
      this.startMediaRecorder(c.captureStream(this.fps), opts);
      return;
    }

    const md = navigator.mediaDevices as MediaDevices & {
      getDisplayMedia?: (c: DisplayMediaStreamOptions & Record<string, unknown>) => Promise<MediaStream>;
    };
    if (!md?.getDisplayMedia) throw new Error('Screen capture is not available in this browser.');
    const stream = await md.getDisplayMedia({
      video: { frameRate: opts.fps },
      audio: false,
      preferCurrentTab: true,
      selfBrowserSurface: 'include',
    });
    this.startMediaRecorder(stream, opts);
  }

  /** Stops recording; the file is downloaded once the last frames are encoded. */
  public stop(): void {
    if (!this.active) return;
    if (this.codec) {
      void this.finishCodec();
      return;
    }
    const rec = this.mediaRecorder;
    if (rec && rec.state !== 'inactive') rec.stop();
  }

  /** True when a 'map' recording wants its next frame (throttled to the chosen FPS). */
  public frameDue(): boolean {
    // Half a 60 Hz display frame of slack, so 30 fps lands on every 2nd animation frame.
    return (
      this.active &&
      this.compositeCtx !== null &&
      performance.now() - this.lastCaptureMs >= this.frameIntervalMs - 8
    );
  }

  /**
   * Call right after the map has been drawn (same task), to copy it into the recording.
   * Copying a WebGPU canvas is a synchronous GPU readback, so it is throttled to the FPS.
   */
  public captureFrame(): void {
    const c = this.composite;
    const ctx = this.compositeCtx;
    if (!c || !ctx || !this.frameDue()) return;
    const now = performance.now();
    this.lastCaptureMs = now;

    const codec = this.codec;
    if (codec && codec.encoder.encodeQueueSize > MAX_ENCODE_QUEUE) {
      this.dropped += 1;
      return;
    }

    this.drawComposite(c, ctx);

    if (codec) {
      const frame = new VideoFrame(c, {
        timestamp: Math.round((now - this.startedAt) * 1000),
        duration: Math.round(1e6 / this.fps),
      });
      // A keyframe every 2 s keeps seeking in players fast.
      const keyFrame = this.frames % (this.fps * 2) === 0;
      try {
        codec.encoder.encode(frame, { keyFrame, ...codec.encodeExtra } as VideoEncoderEncodeOptions);
        this.frames += 1;
      } finally {
        frame.close();
      }
    } else {
      this.frames += 1;
    }
  }

  private setupComposite(height: number): void {
    const c = document.createElement('canvas');
    const srcW = Math.max(2, this.baseCanvas.width || this.overlayCanvas.width);
    const srcH = Math.max(2, this.baseCanvas.height || this.overlayCanvas.height);
    const h = height > 0 ? Math.min(height, srcH) : srcH;
    // Encoders want even dimensions.
    c.height = Math.floor(h / 2) * 2;
    c.width = Math.floor((srcW * (c.height / srcH)) / 2) * 2;
    const ctx = c.getContext('2d', { alpha: false });
    if (!ctx) throw new Error('Could not create the recording canvas.');
    this.composite = c;
    this.compositeCtx = ctx;
  }

  private drawComposite(c: HTMLCanvasElement, ctx: CanvasRenderingContext2D): void {
    ctx.fillStyle = '#05070d';
    ctx.fillRect(0, 0, c.width, c.height);
    if (this.baseCanvas.style.visibility !== 'hidden' && this.baseCanvas.width > 0) {
      ctx.drawImage(this.baseCanvas, 0, 0, c.width, c.height);
    }
    if (this.overlayCanvas.width > 0) {
      ctx.drawImage(this.overlayCanvas, 0, 0, c.width, c.height);
    }
    // Small caption in the bottom-left corner (the map's scale bar sits top-left).
    const text = this.caption();
    if (text) {
      const fs = Math.max(11, Math.round(c.height / 60));
      // Monospace so changing digits (the year) do not make the caption jitter.
      ctx.font = `600 ${fs}px ui-monospace, SFMono-Regular, Menlo, Consolas, monospace`;
      const pad = Math.round(fs * 0.5);
      const w = ctx.measureText(text).width + pad * 2;
      const h = fs + pad * 1.2;
      const y = c.height - h - pad * 2;
      ctx.fillStyle = 'rgba(5, 7, 13, 0.65)';
      ctx.fillRect(pad * 2, y, w, h);
      ctx.fillStyle = '#e2e8f0';
      ctx.textBaseline = 'middle';
      ctx.fillText(text, pad * 3, y + h / 2);
    }
  }

  private async createCodecSession(opts: RecorderOptions): Promise<CodecSession | null> {
    const c = this.composite;
    if (!c) return null;
    const cfg = await findEncoderConfig(
      opts.format,
      c.width,
      c.height,
      this.fps,
      opts.bitsPerSecond,
      opts.constantQuality !== undefined,
    );
    if (!cfg) return null;

    let addChunk: (chunk: EncodedVideoChunk, meta?: EncodedVideoChunkMetadata) => void;
    let finalize: () => Blob;
    let mime: string;
    if (opts.format === 'mp4') {
      const target = new Mp4Target();
      const muxer = new Mp4Muxer({
        target,
        video: { codec: 'avc', width: c.width, height: c.height, frameRate: this.fps },
        fastStart: 'in-memory',
        firstTimestampBehavior: 'offset',
      });
      addChunk = (chunk, meta) => muxer.addVideoChunk(chunk, meta);
      finalize = () => {
        muxer.finalize();
        return new Blob([target.buffer], { type: 'video/mp4' });
      };
      mime = `video/mp4;codecs=${cfg.codec}`;
    } else {
      const target = new WebmTarget();
      const muxer = new WebmMuxer({
        target,
        video: { codec: 'V_VP9', width: c.width, height: c.height, frameRate: this.fps },
        firstTimestampBehavior: 'offset',
      });
      addChunk = (chunk, meta) => muxer.addVideoChunk(chunk, meta);
      finalize = () => {
        muxer.finalize();
        return new Blob([target.buffer], { type: 'video/webm' });
      };
      mime = `video/webm;codecs=${cfg.codec}`;
    }

    const encoder = new VideoEncoder({
      output: (chunk, meta) => {
        this.bytes += chunk.byteLength;
        addChunk(chunk, meta);
      },
      error: (e) => {
        this.onError(`Video encoder failed: ${e.message}`);
        this.stop();
      },
    });
    encoder.configure(cfg);
    const mode = String(cfg.bitrateMode ?? 'variable');
    let encodeExtra: Record<string, unknown> = {};
    let modeLabel = mode;
    if (mode === 'quantizer' && opts.constantQuality) {
      const q = QP[opts.format][opts.constantQuality];
      encodeExtra = opts.format === 'mp4' ? { avc: { quantizer: q } } : { vp9: { quantizer: q } };
      modeLabel = `constant quality q${q}`;
    }
    return { encoder, encodeExtra, finalize, mime: `${mime} · ${modeLabel}` };
  }

  private async finishCodec(): Promise<void> {
    const codec = this.codec;
    if (!codec || this.finishing) return;
    this.active = false;
    this.finishing = true;
    this.emitStatus();
    try {
      if (codec.encoder.state === 'configured') await codec.encoder.flush();
      if (codec.encoder.state !== 'closed') codec.encoder.close();
      if (this.frames > 0) this.download(codec.finalize());
    } catch (err) {
      this.onError(`Could not finish the video: ${err instanceof Error ? err.message : String(err)}`);
    } finally {
      this.codec = null;
      this.cleanup();
    }
  }

  private startMediaRecorder(stream: MediaStream, opts: RecorderOptions): void {
    const mime = pickMimeType(opts.format);
    if (!mime) {
      stream.getTracks().forEach((t) => t.stop());
      throw new Error('This browser cannot record video.');
    }
    this.mimeType = mime;
    const recOpts: MediaRecorderOptions & { videoBitrateMode?: 'constant' | 'variable' } = {
      mimeType: mime,
      videoBitsPerSecond: opts.bitsPerSecond,
      videoBitrateMode: 'constant',
    };
    const rec = new MediaRecorder(stream, recOpts);
    this.chunks = [];
    rec.ondataavailable = (ev) => {
      if (ev.data.size > 0) {
        this.chunks.push(ev.data);
        this.bytes += ev.data.size;
      }
    };
    rec.onstop = () => {
      this.active = false;
      this.download(new Blob(this.chunks, { type: this.mimeType.split(';')[0] }));
      this.stream?.getTracks().forEach((t) => t.stop());
      this.stream = null;
      this.mediaRecorder = null;
      this.chunks = [];
      this.cleanup();
    };
    // The user can end a tab capture from the browser's own "Stop sharing" bar.
    stream.getVideoTracks().forEach((t) => t.addEventListener('ended', () => this.stop()));
    this.stream = stream;
    this.mediaRecorder = rec;
    rec.start(1000);
    this.begin();
  }

  private begin(): void {
    this.active = true;
    this.startedAt = performance.now();
    this.tickTimer = window.setInterval(() => this.emitStatus(), 500);
    this.emitStatus();
  }

  private download(blob: Blob): void {
    const ext = blob.type.startsWith('video/mp4') ? 'mp4' : 'webm';
    const stamp = new Date().toISOString().replace(/[:T]/g, '-').slice(0, 19);
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${this.fileBase}_${stamp}.${ext}`;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 60_000);
  }

  private cleanup(): void {
    this.active = false;
    this.finishing = false;
    this.composite = null;
    this.compositeCtx = null;
    if (this.tickTimer !== null) window.clearInterval(this.tickTimer);
    this.tickTimer = null;
    this.emitStatus();
  }

  private emitStatus(): void {
    this.onStatus({
      recording: this.active,
      finishing: this.finishing,
      elapsedMs: this.active ? performance.now() - this.startedAt : 0,
      bytes: this.bytes,
      mimeType: this.mimeType,
      frames: this.frames,
      dropped: this.dropped,
    });
  }
}
