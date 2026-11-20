import type { CombatFxView, FleetView, HudSnapshot, JumpFxView, MapSelection, RenderFrame } from './types';
import { WebGpuGalaxyBackend } from './webgpu';

export type RenderBackendName = 'WebGPU' | 'Canvas2D';

export type HitTarget =
  | { kind: 'planet'; index: number }
  | { kind: 'fleet'; index: number }
  | { kind: 'wormhole'; index: number }
  | { kind: 'debris'; index: number }
  | null;

export const PLANET_STRIDE = 7; // [x, y, owner, factories, hab, hasGate, crustSum]
export const FLEET_STRIDE = 7;  // [x, y, targetX, targetY, emp, typeCode, flags]
export const DEBRIS_STRIDE = 2; // [x, y]

const MAX_VISIBLE_PLANET_LABELS = 250;

interface BgStar {
  x: number;
  y: number;
  r: number;
  a: number;
}

interface ViewportTransform {
  scale: number;
  offX: number;
  offY: number;
}

function hexToRgbTuple(hex: string): string {
  const clean = hex.replace('#', '');
  if (clean.length !== 6) return '56,189,248';
  const r = parseInt(clean.slice(0, 2), 16);
  const g = parseInt(clean.slice(2, 4), 16);
  const b = parseInt(clean.slice(4, 6), 16);
  if (Number.isNaN(r) || Number.isNaN(g) || Number.isNaN(b)) return '56,189,248';
  return `${r},${g},${b}`;
}

export class GalaxyRenderer {
  private readonly canvas: HTMLCanvasElement;
  private readonly overlayCanvas: HTMLCanvasElement;
  private readonly overlayCtx: CanvasRenderingContext2D;
  private fallbackCtx: CanvasRenderingContext2D | null = null;
  private gpuBackend: WebGpuGalaxyBackend | null = null;

  private worldW: number;
  private worldH: number;
  private cssW = window.innerWidth;
  private cssH = window.innerHeight;
  private dpr = 1;
  private readonly bgStars: BgStar[];
  private readonly glowSpriteCache = new Map<string, HTMLCanvasElement>();
  private combatFx: CombatFxView[] = [];
  private jumpFx: JumpFxView[] = [];
  private zoom = 1.0;
  private panX = 0;
  private panY = 0;

  // Reusable scratch buffer for visible planet label culling (avoids per-frame allocation)
  private readonly visiblePlanetIndices = new Int32Array(MAX_VISIBLE_PLANET_LABELS);

  public static async create(
    canvas: HTMLCanvasElement,
    overlayCanvas: HTMLCanvasElement,
    worldW: number,
    worldH: number,
  ): Promise<GalaxyRenderer> {
    const renderer = new GalaxyRenderer(canvas, overlayCanvas, worldW, worldH);
    await renderer.initBackend();
    return renderer;
  }

  private constructor(
    canvas: HTMLCanvasElement,
    overlayCanvas: HTMLCanvasElement,
    worldW: number,
    worldH: number,
  ) {
    this.canvas = canvas;
    this.overlayCanvas = overlayCanvas;
    const octx = overlayCanvas.getContext('2d');
    if (!octx) {
      throw new Error('Failed to acquire 2D overlay canvas context');
    }
    this.overlayCtx = octx;
    this.worldW = worldW;
    this.worldH = worldH;

    this.bgStars = Array.from({ length: 160 }, () => ({
      x: Math.random(),
      y: Math.random(),
      r: Math.random() * 1.3 + 0.3,
      a: Math.random() * 0.5 + 0.15,
    }));

    this.resize();
    window.addEventListener('resize', () => this.resize());
    window.visualViewport?.addEventListener('resize', () => this.resize());
  }

  private async initBackend(): Promise<void> {
    const gpu = await WebGpuGalaxyBackend.create(this.canvas, this.bgStars, () => {
      // Automatic fallback if WebGPU device is lost at runtime
      this.gpuBackend = null;
      this.ensureCanvas2dFallback();
    });
    if (gpu) {
      this.gpuBackend = gpu;
    } else {
      this.ensureCanvas2dFallback();
    }
  }

  private ensureCanvas2dFallback(): void {
    if (this.fallbackCtx) return;
    const ctx = this.canvas.getContext('2d');
    if (ctx) {
      this.fallbackCtx = ctx;
    }
  }

  public get backendName(): RenderBackendName {
    return this.gpuBackend ? 'WebGPU' : 'Canvas2D';
  }

  public get hasActiveFx(): boolean {
    return this.jumpFx.length > 0 || this.combatFx.length > 0;
  }

  public resize(): void {
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const cssW = Math.max(1, this.canvas.clientWidth || window.innerWidth);
    const cssH = Math.max(1, this.canvas.clientHeight || window.innerHeight);
    this.dpr = dpr;
    this.cssW = cssW;
    this.cssH = cssH;

    const bufW = Math.round(cssW * dpr);
    const bufH = Math.round(cssH * dpr);
    if (this.canvas.width !== bufW || this.canvas.height !== bufH) {
      this.canvas.width = bufW;
      this.canvas.height = bufH;
    }
    if (this.overlayCanvas.width !== bufW || this.overlayCanvas.height !== bufH) {
      this.overlayCanvas.width = bufW;
      this.overlayCanvas.height = bufH;
    }
    this.gpuBackend?.reconfigure();
  }

  public clearFx(): void {
    this.combatFx = [];
    this.jumpFx = [];
  }

  public ingestFx(hud: HudSnapshot): void {
    if (hud.newCombatFx.length > 0) {
      this.combatFx.push(...hud.newCombatFx);
      if (this.combatFx.length > 35) {
        this.combatFx.splice(0, this.combatFx.length - 35);
      }
    }
    if (hud.newJumpFx.length > 0) {
      this.jumpFx.push(...hud.newJumpFx);
      if (this.jumpFx.length > 25) {
        this.jumpFx.splice(0, this.jumpFx.length - 25);
      }
    }
  }

  public setWorldSize(worldW: number, worldH: number): void {
    if (this.worldW !== worldW || this.worldH !== worldH) {
      this.worldW = worldW;
      this.worldH = worldH;
      this.resetCamera();
    }
  }

  public resetCamera(): void {
    this.zoom = 1.0;
    this.panX = 0;
    this.panY = 0;
  }

  public panBy(dx: number, dy: number): void {
    this.panX += dx;
    this.panY += dy;
  }

  public zoomAt(clientX: number, clientY: number, factor: number): void {
    const rect = this.canvas.getBoundingClientRect();
    const mx = clientX - rect.left;
    const my = clientY - rect.top;
    const before = this.getTransform();
    const wx = (mx - before.offX) / before.scale;
    const wy = (my - before.offY) / before.scale;

    this.zoom = Math.max(0.6, Math.min(40.0, this.zoom * factor));

    const base = this.getBaseTransform();
    const newScale = base.scale * this.zoom;
    this.panX = mx - wx * newScale - base.offX;
    this.panY = my - wy * newScale - base.offY;
  }

  private getHudTopHeight(): number {
    const hudEl = document.querySelector('header.hud-top');
    return hudEl ? hudEl.getBoundingClientRect().height : 54;
  }

  private getBaseTransform(): ViewportTransform {
    const padTop = this.getHudTopHeight() + 10;
    const isNarrow = this.cssW <= 768;
    const padBot = isNarrow ? 20 : 34;
    const padX = isNarrow ? 12 : 24;
    const availW = Math.max(1, this.cssW - padX * 2);
    const availH = Math.max(1, this.cssH - padTop - padBot);
    const scale = Math.min(availW / this.worldW, availH / this.worldH);
    return {
      scale,
      offX: (this.cssW - this.worldW * scale) / 2,
      offY: padTop + (availH - this.worldH * scale) / 2,
    };
  }

  private getTransform(): ViewportTransform {
    const base = this.getBaseTransform();
    const scale = base.scale * this.zoom;
    return {
      scale,
      offX: base.offX + this.panX,
      offY: base.offY + this.panY,
    };
  }

  public hitTest(clientX: number, clientY: number, frame: RenderFrame): HitTarget {
    const rect = this.canvas.getBoundingClientRect();
    const mx = clientX - rect.left;
    const my = clientY - rect.top;
    const { scale, offX, offY } = this.getTransform();

    let best: HitTarget = null;
    let bestScore = Infinity;

    // 1. Planets (18px radius, fast AABB reject for 100k+ stars)
    const planetCount = Math.floor(frame.planetsBuf.length / PLANET_STRIDE);
    for (let i = 0; i < planetCount; i++) {
      const off = i * PLANET_STRIDE;
      const dx = offX + frame.planetsBuf[off] * scale - mx;
      if (dx < -18 || dx > 18) continue;
      const dy = offY + frame.planetsBuf[off + 1] * scale - my;
      if (dy < -18 || dy > 18) continue;
      const d = Math.hypot(dx, dy);
      if (d <= 18 && d < bestScore) {
        bestScore = d;
        best = { kind: 'planet', index: i };
      }
    }

    // 2. Fleets (14px radius, slight priority boost so moving fleets near planets are clickable)
    const fleetCount = Math.floor(frame.fleetsBuf.length / FLEET_STRIDE);
    for (let i = 0; i < fleetCount; i++) {
      const off = i * FLEET_STRIDE;
      const dx = offX + frame.fleetsBuf[off] * scale - mx;
      if (dx < -14 || dx > 14) continue;
      const dy = offY + frame.fleetsBuf[off + 1] * scale - my;
      if (dy < -14 || dy > 14) continue;
      const d = Math.hypot(dx, dy);
      if (d <= 14 && d - 3 < bestScore) {
        bestScore = d - 3;
        best = { kind: 'fleet', index: i };
      }
    }

    // 3. Wormholes (both endpoints, 16px radius)
    for (let i = 0; i < frame.hud.wormholes.length; i++) {
      const wh = frame.hud.wormholes[i];
      const d1 = Math.hypot(offX + wh.x1 * scale - mx, offY + wh.y1 * scale - my);
      const d2 = Math.hypot(offX + wh.x2 * scale - mx, offY + wh.y2 * scale - my);
      const d = Math.min(d1, d2);
      if (d <= 16 && d < bestScore) {
        bestScore = d;
        best = { kind: 'wormhole', index: i };
      }
    }

    // 4. Debris fields (12px radius)
    const debrisCount = Math.floor(frame.debrisBuf.length / DEBRIS_STRIDE);
    for (let i = 0; i < debrisCount; i++) {
      const off = i * DEBRIS_STRIDE;
      const dx = offX + frame.debrisBuf[off] * scale;
      const dy = offY + frame.debrisBuf[off + 1] * scale;
      const d = Math.hypot(dx - mx, dy - my);
      if (d <= 12 && d < bestScore) {
        bestScore = d;
        best = { kind: 'debris', index: i };
      }
    }

    return best;
  }

  private getGlowSprite(rgb: string): HTMLCanvasElement {
    let sprite = this.glowSpriteCache.get(rgb);
    if (sprite) return sprite;
    const size = 64;
    sprite = document.createElement('canvas');
    sprite.width = size;
    sprite.height = size;
    const sctx = sprite.getContext('2d');
    if (sctx) {
      const half = size * 0.5;
      const grad = sctx.createRadialGradient(half, half, 1, half, half, half);
      grad.addColorStop(0, `rgba(${rgb}, 0.16)`);
      grad.addColorStop(1, `rgba(${rgb}, 0)`);
      sctx.fillStyle = grad;
      sctx.fillRect(0, 0, size, size);
    }
    this.glowSpriteCache.set(rgb, sprite);
    return sprite;
  }

  private getDenseFactor(planetCount: number): number {
    const base =
      planetCount >= 5000
        ? 0.55
        : planetCount >= 450
          ? 0.72
          : planetCount >= 220
            ? 0.85
            : 1.0;
    const mobileScale = this.cssW < 640 ? 0.72 : 1.0;
    return base * mobileScale;
  }

  private planetRadius(owner: number, factories: number, hab: number, denseFactor: number): number {
    return (owner > 0 ? Math.min(13, 4 + Math.sqrt(factories) * 0.45) : 3 + hab * 2.5) * denseFactor;
  }

  public draw(
    frame: RenderFrame,
    crustRichness: number,
    selection: MapSelection,
    hovered: HitTarget,
    selectedFleet: FleetView | null,
    dataVersion = 0,
  ): void {
    const { hud, planetsBuf, fleetsBuf, debrisBuf } = frame;
    const transform = this.getTransform();
    const planetCount = Math.floor(planetsBuf.length / PLANET_STRIDE);
    const denseFactor = this.getDenseFactor(planetCount);
    const invCrustNorm = 1.0 / (crustRichness * 4);
    const empColors = hud.empires.map((emp) => emp.color);
    const empRanges: [number, number][] = hud.empires.map((emp) => [
      emp.loadout.scannerRange * (emp.prt === 'PRT_SS' ? 1.25 : 1.0),
      Math.max(emp.loadout.rangeCol, emp.loadout.rangeCruiser),
    ]);

    if (this.gpuBackend) {
      this.gpuBackend.render(
        {
          offX: transform.offX,
          offY: transform.offY,
          scale: transform.scale,
          zoom: this.zoom,
          viewportW: this.cssW,
          viewportH: this.cssH,
          denseFactor,
          invCrustNorm,
          planetCount,
        },
        empColors,
        empRanges,
        planetsBuf,
        fleetsBuf,
        debrisBuf,
        hud.wormholes,
        this.jumpFx,
        this.combatFx,
        dataVersion,
      );
    } else if (this.fallbackCtx) {
      this.drawCanvas2dBase(
        this.fallbackCtx,
        frame,
        transform,
        denseFactor,
        invCrustNorm,
        empColors,
      );
    }

    // Lightweight 2D overlay: #ID labels (<= 250 visible planets), selection/hover outlines,
    // fuel range circles, wormhole endpoints, combat rings, selected fleet trajectory, scale bar.
    this.drawOverlay(frame, transform, denseFactor, selection, hovered, selectedFleet);

    // Advance ephemeral FX TTLs once per frame
    for (let i = this.jumpFx.length - 1; i >= 0; i--) {
      this.jumpFx[i].ttl--;
      if (this.jumpFx[i].ttl <= 0) this.jumpFx.splice(i, 1);
    }
    for (let i = this.combatFx.length - 1; i >= 0; i--) {
      this.combatFx[i].ttl--;
      if (this.combatFx[i].ttl <= 0) this.combatFx.splice(i, 1);
    }
  }

  private drawCanvas2dBase(
    ctx: CanvasRenderingContext2D,
    frame: RenderFrame,
    transform: ViewportTransform,
    denseFactor: number,
    invCrustNorm: number,
    empColors: string[],
  ): void {
    const { hud, planetsBuf, fleetsBuf, debrisBuf } = frame;
    const W = this.cssW;
    const H = this.cssH;
    ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
    ctx.clearRect(0, 0, W, H);

    for (const s of this.bgStars) {
      ctx.fillStyle = `rgba(180, 210, 255, ${s.a})`;
      ctx.fillRect(s.x * W, s.y * H, s.r, s.r);
    }

    const { scale, offX, offY } = transform;
    const sx = (x: number) => offX + x * scale;
    const sy = (y: number) => offY + y * scale;
    const glowSprites = hud.empires.map((emp) => this.getGlowSprite(hexToRgbTuple(emp.color)));
    const planetCount = Math.floor(planetsBuf.length / PLANET_STRIDE);

    // Empire territory glows
    for (let i = 0; i < planetCount; i++) {
      const off = i * PLANET_STRIDE;
      const owner = planetsBuf[off + 2];
      if (owner === 0) continue;
      const px = sx(planetsBuf[off]);
      const py = sy(planetsBuf[off + 1]);
      if (px < -60 || px > W + 60 || py < -60 || py > H + 60) continue;
      const sprite = glowSprites[owner - 1];
      if (!sprite) continue;
      const rad = (24 + Math.sqrt(planetsBuf[off + 3]) * 2.2) * scale;
      ctx.drawImage(sprite, px - rad, py - rad, rad * 2, rad * 2);
    }

    // Wormhole links
    for (let i = 0; i < hud.wormholes.length; i++) {
      const wh = hud.wormholes[i];
      ctx.beginPath();
      ctx.moveTo(sx(wh.x1), sy(wh.y1));
      ctx.lineTo(sx(wh.x2), sy(wh.y2));
      ctx.strokeStyle = 'rgba(217, 70, 239, 0.25)';
      ctx.lineWidth = 1.0;
      ctx.setLineDash([5, 5]);
      ctx.stroke();
      ctx.setLineDash([]);
    }

    // Debris fields
    const debrisCount = Math.floor(debrisBuf.length / DEBRIS_STRIDE);
    if (debrisCount > 0) {
      ctx.setLineDash([2, 2]);
      ctx.strokeStyle = 'rgba(192, 132, 252, 0.75)';
      ctx.lineWidth = 1.0;
      for (let i = 0; i < debrisCount; i++) {
        const off = i * DEBRIS_STRIDE;
        ctx.beginPath();
        ctx.arc(sx(debrisBuf[off]), sy(debrisBuf[off + 1]), 5, 0, Math.PI * 2);
        ctx.stroke();
      }
      ctx.setLineDash([]);
    }

    // Planets
    const fastDotMode = planetCount >= 5000 && this.zoom < 4.0;
    for (let i = 0; i < planetCount; i++) {
      const off = i * PLANET_STRIDE;
      const px = sx(planetsBuf[off]);
      const py = sy(planetsBuf[off + 1]);
      if (px < -30 || px > W + 30 || py < -30 || py > H + 30) continue;

      const owner = planetsBuf[off + 2];
      const factories = planetsBuf[off + 3];
      const hab = planetsBuf[off + 4];
      const hasGate = planetsBuf[off + 5] > 0.5;
      const crustSum = planetsBuf[off + 6];

      if (fastDotMode && owner === 0) {
        ctx.fillStyle = 'rgba(148, 163, 184, 0.55)';
        ctx.fillRect(px - 0.75, py - 0.75, 1.5, 1.5);
        continue;
      }

      const r = this.planetRadius(owner, factories, hab, denseFactor);
      ctx.beginPath();
      ctx.arc(px, py, r, 0, Math.PI * 2);
      if (owner > 0) {
        ctx.fillStyle = empColors[owner - 1] ?? '#94a3b8';
      } else {
        const alpha = Math.min(1, 0.3 + crustSum * invCrustNorm * 0.7);
        ctx.fillStyle = `rgba(148, 163, 184, ${alpha})`;
      }
      ctx.fill();

      if (owner > 0 && hasGate) {
        ctx.beginPath();
        ctx.arc(px, py, r + 3.5, 0, Math.PI * 2);
        ctx.strokeStyle = '#22d3ee';
        ctx.lineWidth = 1.5;
        ctx.stroke();
        ctx.lineWidth = 1.0;
      }
    }

    // Jump FX
    for (let i = 0; i < this.jumpFx.length; i++) {
      const jx = this.jumpFx[i];
      ctx.beginPath();
      ctx.moveTo(sx(jx.x1), sy(jx.y1));
      ctx.lineTo(sx(jx.x2), sy(jx.y2));
      ctx.strokeStyle = jx.col;
      ctx.lineWidth = 2.0;
      ctx.stroke();
      ctx.lineWidth = 1.0;
    }

    // Fleets
    const fleetCount = Math.floor(fleetsBuf.length / FLEET_STRIDE);
    for (let i = 0; i < fleetCount; i++) {
      const off = i * FLEET_STRIDE;
      const fx = sx(fleetsBuf[off]);
      const fy = sy(fleetsBuf[off + 1]);
      if (fx < -20 || fx > W + 20 || fy < -20 || fy > H + 20) continue;

      const tx = sx(fleetsBuf[off + 2]);
      const ty = sy(fleetsBuf[off + 3]);
      const emp = fleetsBuf[off + 4];
      const fType = fleetsBuf[off + 5];
      const hunting = (fleetsBuf[off + 6] & 1) !== 0;

      ctx.beginPath();
      ctx.moveTo(fx, fy);
      ctx.lineTo(tx, ty);
      ctx.strokeStyle = hunting
        ? 'rgba(248, 113, 113, 0.45)'
        : fType === 4
          ? 'rgba(192,132,252,0.16)'
          : 'rgba(255,255,255,0.09)';
      ctx.stroke();

      ctx.fillStyle = empColors[emp - 1] ?? '#e2e8f0';
      if (fType === 2) {
        ctx.beginPath();
        ctx.moveTo(fx, fy - 5.5);
        ctx.lineTo(fx + 5.5, fy);
        ctx.lineTo(fx, fy + 5.5);
        ctx.lineTo(fx - 5.5, fy);
        ctx.closePath();
        ctx.fill();
      } else if (fType === 3) {
        ctx.beginPath();
        ctx.arc(fx, fy, 4.5, 0, Math.PI * 2);
        ctx.fill();
        ctx.strokeStyle = '#f87171';
        ctx.stroke();
      } else if (fType === 1) {
        ctx.beginPath();
        ctx.moveTo(fx, fy - 5);
        ctx.lineTo(fx - 4.2, fy + 3.5);
        ctx.lineTo(fx + 4.2, fy + 3.5);
        ctx.closePath();
        ctx.fill();
      } else if (fType === 4) {
        ctx.fillRect(fx - 2.8, fy - 2.8, 5.6, 5.6);
      } else {
        ctx.beginPath();
        ctx.arc(fx, fy, 3, 0, Math.PI * 2);
        ctx.fill();
      }
    }

    // Combat FX lasers
    for (let i = 0; i < this.combatFx.length; i++) {
      const fx = this.combatFx[i];
      ctx.beginPath();
      ctx.moveTo(sx(fx.x1), sy(fx.y1));
      ctx.lineTo(sx(fx.x2), sy(fx.y2));
      ctx.strokeStyle = '#ffffff';
      ctx.lineWidth = 2.0;
      ctx.stroke();
      ctx.lineWidth = 1.0;
    }
  }

  private drawOverlay(
    frame: RenderFrame,
    transform: ViewportTransform,
    denseFactor: number,
    selection: MapSelection,
    hovered: HitTarget,
    selectedFleet: FleetView | null,
  ): void {
    const ctx = this.overlayCtx;
    const W = this.cssW;
    const H = this.cssH;
    ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
    ctx.clearRect(0, 0, W, H);

    const { hud, planetsBuf, debrisBuf } = frame;
    const { scale, offX, offY } = transform;
    const sx = (x: number) => offX + x * scale;
    const sy = (y: number) => offY + y * scale;

    const selectedPlanet = selection?.kind === 'planet' ? selection.index : -1;
    const hoveredPlanet = hovered?.kind === 'planet' ? hovered.index : -1;
    const selectedWormhole = selection?.kind === 'wormhole' ? selection.index : -1;
    const hoveredWormhole = hovered?.kind === 'wormhole' ? hovered.index : -1;
    const selectedDebris = selection?.kind === 'debris' ? selection.index : -1;
    const hoveredDebris = hovered?.kind === 'debris' ? hovered.index : -1;

    // 1. Wormhole portal rings & selection/hover highlight (<= 5 pairs, negligible cost)
    for (let i = 0; i < hud.wormholes.length; i++) {
      const wh = hud.wormholes[i];
      const isSel = i === selectedWormhole;
      const isHov = i === hoveredWormhole;
      const x1 = sx(wh.x1);
      const y1 = sy(wh.y1);
      const x2 = sx(wh.x2);
      const y2 = sy(wh.y2);

      if (isSel || isHov) {
        ctx.beginPath();
        ctx.moveTo(x1, y1);
        ctx.lineTo(x2, y2);
        ctx.strokeStyle = isSel ? 'rgba(240, 171, 252, 0.85)' : 'rgba(240, 171, 252, 0.5)';
        ctx.lineWidth = isSel ? 2.2 : 1.2;
        ctx.setLineDash([5, 5]);
        ctx.stroke();
        ctx.setLineDash([]);
      }

      for (const [wx, wy] of [
        [x1, y1],
        [x2, y2],
      ]) {
        ctx.beginPath();
        ctx.arc(wx, wy, isSel ? 12 : 9, 0, Math.PI * 2);
        ctx.strokeStyle = isSel ? '#ffffff' : '#d946ef';
        ctx.lineWidth = isSel ? 2.0 : 1.6;
        ctx.stroke();
        ctx.beginPath();
        ctx.arc(wx, wy, 4, 0, Math.PI * 2);
        ctx.fillStyle = '#f0abfc';
        ctx.fill();
      }
      ctx.lineWidth = 1.0;
    }

    // 2. Combat explosion shockwave rings (<= 35 active)
    for (let i = 0; i < this.combatFx.length; i++) {
      const fx = this.combatFx[i];
      ctx.beginPath();
      ctx.arc(sx(fx.mx), sy(fx.my), (11 - fx.ttl) * 2.1, 0, Math.PI * 2);
      ctx.strokeStyle = fx.col;
      ctx.lineWidth = 1.6;
      ctx.stroke();
    }
    ctx.lineWidth = 1.0;

    // 3. Planet #ID labels ONLY when visible planets in viewport <= 250 (early-abort at 251!)
    const planetCount = Math.floor(planetsBuf.length / PLANET_STRIDE);
    let visibleCount = 0;
    let tooMany = false;
    for (let i = 0; i < planetCount; i++) {
      const off = i * PLANET_STRIDE;
      const px = offX + planetsBuf[off] * scale;
      if (px < 0 || px > W) continue;
      const py = offY + planetsBuf[off + 1] * scale;
      if (py < 0 || py > H) continue;
      if (visibleCount >= MAX_VISIBLE_PLANET_LABELS) {
        tooMany = true;
        break;
      }
      this.visiblePlanetIndices[visibleCount++] = i;
    }

    if (!tooMany && visibleCount > 0) {
      ctx.font = '10px monospace';
      ctx.fillStyle = 'rgba(203, 213, 225, 0.72)';
      for (let k = 0; k < visibleCount; k++) {
        const i = this.visiblePlanetIndices[k];
        const off = i * PLANET_STRIDE;
        const px = offX + planetsBuf[off] * scale;
        const py = offY + planetsBuf[off + 1] * scale;
        const r = this.planetRadius(
          planetsBuf[off + 2],
          planetsBuf[off + 3],
          planetsBuf[off + 4],
          denseFactor,
        );
        ctx.fillText(`#${i + 1}`, px + r + 4, py + 3);
      }
    }

    // 4. Selected & hovered planet rings + fuel range projection
    const drawPlanetHighlight = (idx: number, isSelected: boolean): void => {
      if (idx < 0 || idx >= planetCount) return;
      const off = idx * PLANET_STRIDE;
      const px = sx(planetsBuf[off]);
      const py = sy(planetsBuf[off + 1]);
      const owner = planetsBuf[off + 2];
      const r = this.planetRadius(owner, planetsBuf[off + 3], planetsBuf[off + 4], denseFactor);

      ctx.beginPath();
      ctx.arc(px, py, r + 7, 0, Math.PI * 2);
      ctx.strokeStyle = isSelected ? '#ffffff' : 'rgba(255,255,255,0.45)';
      ctx.lineWidth = 1.5;
      ctx.stroke();
      ctx.lineWidth = 1.0;

      if (owner > 0) {
        const emp = hud.empires[owner - 1];
        if (emp) {
          const rgb = hexToRgbTuple(emp.color);
          const colRad = emp.loadout.rangeCol * scale;
          ctx.beginPath();
          ctx.arc(px, py, colRad, 0, Math.PI * 2);
          ctx.strokeStyle = `rgba(${rgb}, ${isSelected ? 0.55 : 0.28})`;
          ctx.setLineDash([6, 4]);
          ctx.lineWidth = 1.2;
          ctx.stroke();

          const capRad = emp.loadout.rangeBattleship * scale;
          if (Math.abs(capRad - colRad) > 12) {
            ctx.beginPath();
            ctx.arc(px, py, capRad, 0, Math.PI * 2);
            ctx.strokeStyle = `rgba(${rgb}, ${isSelected ? 0.3 : 0.14})`;
            ctx.setLineDash([3, 5]);
            ctx.lineWidth = 1.0;
            ctx.stroke();
          }
          ctx.setLineDash([]);
        }
      }
    };

    if (hoveredPlanet >= 0 && hoveredPlanet !== selectedPlanet) {
      drawPlanetHighlight(hoveredPlanet, false);
    }
    if (selectedPlanet >= 0) {
      drawPlanetHighlight(selectedPlanet, true);
    }

    // 5. Selected / hovered debris highlight
    const debrisCount = Math.floor(debrisBuf.length / DEBRIS_STRIDE);
    const drawDebrisHighlight = (idx: number, isSelected: boolean): void => {
      if (idx < 0 || idx >= debrisCount) return;
      const off = idx * DEBRIS_STRIDE;
      ctx.beginPath();
      ctx.arc(sx(debrisBuf[off]), sy(debrisBuf[off + 1]), isSelected ? 8 : 6, 0, Math.PI * 2);
      ctx.strokeStyle = isSelected ? '#ffffff' : '#e9d5ff';
      ctx.lineWidth = isSelected ? 2.0 : 1.2;
      ctx.stroke();
      ctx.lineWidth = 1.0;
    };
    if (hoveredDebris >= 0 && hoveredDebris !== selectedDebris) {
      drawDebrisHighlight(hoveredDebris, false);
    }
    if (selectedDebris >= 0) {
      drawDebrisHighlight(selectedDebris, true);
    }

    // 6. Selected fleet live highlight & trajectory
    if (selectedFleet) {
      const fx = sx(selectedFleet.x);
      const fy = sy(selectedFleet.y);
      const tOff = selectedFleet.target * PLANET_STRIDE;
      const hasSteer =
        (selectedFleet.hunting || selectedFleet.whRoute >= 0) &&
        Number.isFinite(selectedFleet.steerX);
      if (hasSteer || tOff + 1 < planetsBuf.length) {
        const tx = sx(hasSteer ? selectedFleet.steerX : planetsBuf[tOff]);
        const ty = sy(hasSteer ? selectedFleet.steerY : planetsBuf[tOff + 1]);
        ctx.beginPath();
        ctx.moveTo(fx, fy);
        ctx.lineTo(tx, ty);
        ctx.strokeStyle = selectedFleet.hunting
          ? '#f87171'
          : selectedFleet.whRoute >= 0
            ? '#e879f9'
            : '#ffffff';
        ctx.lineWidth = 1.8;
        ctx.setLineDash([5, 4]);
        ctx.stroke();
        ctx.setLineDash([]);
      }
      ctx.beginPath();
      ctx.arc(fx, fy, 10, 0, Math.PI * 2);
      ctx.strokeStyle = '#ffffff';
      ctx.lineWidth = 1.8;
      ctx.stroke();
      ctx.lineWidth = 1.0;
    }

    // 7. Cartographic scale bar
    this.drawScaleBar(ctx, scale);
  }

  private drawScaleBar(ctx: CanvasRenderingContext2D, scale: number): void {
    const candidates = [10, 20, 25, 50, 100, 150, 200, 250, 500, 1000, 2000, 2500, 5000, 10000];
    let ly = 100;
    for (const c of candidates) {
      const px = c * scale;
      if (px >= 75 && px <= 175) {
        ly = c;
        break;
      }
    }
    const barW = Math.round(ly * scale);
    const x = this.cssW <= 768 ? 10 : 16;
    const y = Math.round(this.getHudTopHeight() + 10);
    const boxW = Math.max(barW + 24, 130);
    const boxH = 34;

    ctx.fillStyle = 'rgba(10, 15, 25, 0.82)';
    ctx.strokeStyle = '#233148';
    ctx.lineWidth = 1;
    ctx.fillRect(x, y, boxW, boxH);
    ctx.strokeRect(x, y, boxW, boxH);

    const bx = x + 12;
    const by = y + 22;
    ctx.strokeStyle = '#94a3b8';
    ctx.lineWidth = 1.4;
    ctx.beginPath();
    ctx.moveTo(bx, by - 5);
    ctx.lineTo(bx, by);
    ctx.lineTo(bx + barW, by);
    ctx.lineTo(bx + barW, by - 5);
    ctx.moveTo(bx + barW * 0.5, by - 3);
    ctx.lineTo(bx + barW * 0.5, by);
    ctx.stroke();

    ctx.font = '600 10px monospace';
    ctx.fillStyle = '#e2e8f0';
    ctx.fillText('0', bx - 2, y + 12);
    ctx.fillText(`${ly} ly`, bx + barW - 26, y + 12);

    ctx.fillStyle = '#7c8ea8';
    ctx.fillText(`${this.zoom.toFixed(1)}x`, x + boxW - 30, y + 31);
    ctx.lineWidth = 1.0;
  }
}

export function drawPowerChart(
  canvas: HTMLCanvasElement,
  powerHistory: number[][],
  colors: string[],
): number {
  const ctx = canvas.getContext('2d');
  if (!ctx) return 50;
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  if (powerHistory.length < 2) return 50;

  let maxVal = 50;
  for (const pt of powerHistory) {
    for (const val of pt) {
      if (val > maxVal) maxVal = val;
    }
  }

  for (let emp = 0; emp < colors.length; emp++) {
    ctx.beginPath();
    ctx.strokeStyle = colors[emp];
    ctx.lineWidth = 2;
    powerHistory.forEach((pt, idx) => {
      const val = pt[emp] ?? 0;
      const x = (idx / (powerHistory.length - 1)) * (canvas.width - 16) + 8;
      const y = canvas.height - 8 - (val / maxVal) * (canvas.height - 16);
      if (idx === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    });
    ctx.stroke();
  }
  return maxVal;
}
