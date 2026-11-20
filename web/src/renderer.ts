import type { CombatFxView, HudSnapshot, JumpFxView, RenderFrame } from './types';

export const PLANET_STRIDE = 7; // [x, y, owner, factories, hab, hasGate, crustSum]
export const FLEET_STRIDE = 7;  // [x, y, targetX, targetY, emp, typeCode, flags]
export const DEBRIS_STRIDE = 2; // [x, y]

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
  private readonly ctx: CanvasRenderingContext2D;
  private worldW: number;
  private worldH: number;
  private readonly bgStars: BgStar[];
  private readonly glowSpriteCache = new Map<string, HTMLCanvasElement>();
  private combatFx: CombatFxView[] = [];
  private jumpFx: JumpFxView[] = [];
  private zoom = 1.0;
  private panX = 0;
  private panY = 0;

  constructor(canvas: HTMLCanvasElement, worldW: number, worldH: number) {
    this.canvas = canvas;
    const ctx = canvas.getContext('2d');
    if (!ctx) {
      throw new Error('Failed to acquire 2D canvas context');
    }
    this.ctx = ctx;
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
  }

  public resize(): void {
    this.canvas.width = window.innerWidth;
    this.canvas.height = window.innerHeight;
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

    this.zoom = Math.max(0.6, Math.min(6.0, this.zoom * factor));

    const base = this.getBaseTransform();
    const newScale = base.scale * this.zoom;
    this.panX = mx - wx * newScale - base.offX;
    this.panY = my - wy * newScale - base.offY;
  }

  private getBaseTransform(): ViewportTransform {
    const padTop = 64;
    const padBot = 34;
    const padX = 24;
    const availW = this.canvas.width - padX * 2;
    const availH = this.canvas.height - padTop - padBot;
    const scale = Math.min(availW / this.worldW, availH / this.worldH);
    return {
      scale,
      offX: (this.canvas.width - this.worldW * scale) / 2,
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

  public findPlanetAt(
    clientX: number,
    clientY: number,
    planetsBuf: Float32Array,
    maxDistPx: number,
  ): number {
    const rect = this.canvas.getBoundingClientRect();
    const { scale, offX, offY } = this.getTransform();
    const wx = (clientX - rect.left - offX) / scale;
    const wy = (clientY - rect.top - offY) / scale;
    const maxWorldDistSq = (maxDistPx / scale) ** 2;

    let closest = -1;
    let minDistSq = maxWorldDistSq;
    const count = Math.floor(planetsBuf.length / PLANET_STRIDE);
    for (let i = 0; i < count; i++) {
      const off = i * PLANET_STRIDE;
      const dx = planetsBuf[off] - wx;
      const dy = planetsBuf[off + 1] - wy;
      const dSq = dx * dx + dy * dy;
      if (dSq < minDistSq) {
        minDistSq = dSq;
        closest = i;
      }
    }
    return closest;
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

  public draw(
    frame: RenderFrame,
    crustRichness: number,
    selectedPlanet: number,
    hoveredPlanet: number,
  ): void {
    const { hud, planetsBuf, fleetsBuf, debrisBuf } = frame;
    const ctx = this.ctx;
    const W = this.canvas.width;
    const H = this.canvas.height;
    ctx.clearRect(0, 0, W, H);

    for (const s of this.bgStars) {
      ctx.fillStyle = `rgba(180, 210, 255, ${s.a})`;
      ctx.fillRect(s.x * W, s.y * H, s.r, s.r);
    }

    const { scale, offX, offY } = this.getTransform();
    const sx = (x: number) => offX + x * scale;
    const sy = (y: number) => offY + y * scale;
    const rgbCache = hud.empires.map((emp) => hexToRgbTuple(emp.color));
    const glowSprites = rgbCache.map((rgb) => this.getGlowSprite(rgb));
    const empColors = hud.empires.map((emp) => emp.color);
    const planetCount = Math.floor(planetsBuf.length / PLANET_STRIDE);

    // Empire territory glows (pre-rendered offscreen sprite blits — zero per-frame createRadialGradient!)
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

    // Wormholes
    for (const wh of hud.wormholes) {
      const x1 = sx(wh.x1);
      const y1 = sy(wh.y1);
      const x2 = sx(wh.x2);
      const y2 = sy(wh.y2);
      ctx.beginPath();
      ctx.moveTo(x1, y1);
      ctx.lineTo(x2, y2);
      ctx.strokeStyle = 'rgba(217, 70, 239, 0.25)';
      ctx.setLineDash([5, 5]);
      ctx.stroke();
      ctx.setLineDash([]);

      for (const [wx, wy] of [
        [x1, y1],
        [x2, y2],
      ]) {
        ctx.beginPath();
        ctx.arc(wx, wy, 9, 0, Math.PI * 2);
        ctx.strokeStyle = '#d946ef';
        ctx.lineWidth = 1.6;
        ctx.stroke();
        ctx.beginPath();
        ctx.arc(wx, wy, 4, 0, Math.PI * 2);
        ctx.fillStyle = '#f0abfc';
        ctx.fill();
      }
      ctx.lineWidth = 1.0;
    }

    // Debris fields
    const debrisCount = Math.floor(debrisBuf.length / DEBRIS_STRIDE);
    if (debrisCount > 0) {
      ctx.strokeStyle = 'rgba(192, 132, 252, 0.75)';
      ctx.setLineDash([2, 2]);
      for (let i = 0; i < debrisCount; i++) {
        const off = i * DEBRIS_STRIDE;
        ctx.beginPath();
        ctx.arc(sx(debrisBuf[off]), sy(debrisBuf[off + 1]), 5, 0, Math.PI * 2);
        ctx.stroke();
      }
      ctx.setLineDash([]);
    }

    // Planets
    const denseFactor = planetCount >= 450 ? 0.72 : planetCount >= 220 ? 0.85 : 1.0;
    const invCrustNorm = 1.0 / (crustRichness * 4);
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

      const r =
        (owner > 0 ? Math.min(13, 4 + Math.sqrt(factories) * 0.45) : 3 + hab * 2.5) * denseFactor;
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

      if (this.zoom >= 1.65) {
        ctx.font = '10px monospace';
        ctx.fillStyle = 'rgba(203, 213, 225, 0.72)';
        ctx.fillText(`#${i + 1}`, px + r + 4, py + 3);
      }

      if (i === selectedPlanet || i === hoveredPlanet) {
        ctx.beginPath();
        ctx.arc(px, py, r + 7, 0, Math.PI * 2);
        ctx.strokeStyle = i === selectedPlanet ? '#ffffff' : 'rgba(255,255,255,0.45)';
        ctx.lineWidth = 1.5;
        ctx.stroke();
        ctx.lineWidth = 1.0;

        if (owner > 0) {
          const emp = hud.empires[owner - 1];
          if (emp) {
            const rgb = rgbCache[owner - 1] ?? '56,189,248';
            const colRad = emp.loadout.rangeCol * scale;
            ctx.beginPath();
            ctx.arc(px, py, colRad, 0, Math.PI * 2);
            ctx.strokeStyle = `rgba(${rgb}, ${i === selectedPlanet ? 0.55 : 0.28})`;
            ctx.setLineDash([6, 4]);
            ctx.lineWidth = 1.2;
            ctx.stroke();

            const capRad = emp.loadout.rangeBattleship * scale;
            if (Math.abs(capRad - colRad) > 12) {
              ctx.beginPath();
              ctx.arc(px, py, capRad, 0, Math.PI * 2);
              ctx.strokeStyle = `rgba(${rgb}, ${i === selectedPlanet ? 0.30 : 0.14})`;
              ctx.setLineDash([3, 5]);
              ctx.lineWidth = 1.0;
              ctx.stroke();
            }
            ctx.setLineDash([]);
          }
        }
      }
    }

    // Jump FX
    for (let i = this.jumpFx.length - 1; i >= 0; i--) {
      const jx = this.jumpFx[i];
      ctx.beginPath();
      ctx.moveTo(sx(jx.x1), sy(jx.y1));
      ctx.lineTo(sx(jx.x2), sy(jx.y2));
      ctx.strokeStyle = jx.col;
      ctx.lineWidth = 2.0;
      ctx.stroke();
      ctx.lineWidth = 1.0;
      jx.ttl--;
      if (jx.ttl <= 0) this.jumpFx.splice(i, 1);
    }

    // Fleets (flat Float32Array zero-copy loop)
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
        // Battleship
        ctx.beginPath();
        ctx.moveTo(fx, fy - 5.5);
        ctx.lineTo(fx + 5.5, fy);
        ctx.lineTo(fx, fy + 5.5);
        ctx.lineTo(fx - 5.5, fy);
        ctx.closePath();
        ctx.fill();
      } else if (fType === 3) {
        // Bomber
        ctx.beginPath();
        ctx.arc(fx, fy, 4.5, 0, Math.PI * 2);
        ctx.fill();
        ctx.strokeStyle = '#f87171';
        ctx.stroke();
      } else if (fType === 1) {
        // Cruiser
        ctx.beginPath();
        ctx.moveTo(fx, fy - 5);
        ctx.lineTo(fx - 4.2, fy + 3.5);
        ctx.lineTo(fx + 4.2, fy + 3.5);
        ctx.closePath();
        ctx.fill();
      } else if (fType === 4) {
        // Freighter
        ctx.fillRect(fx - 2.8, fy - 2.8, 5.6, 5.6);
      } else {
        // Colonizer
        ctx.beginPath();
        ctx.arc(fx, fy, 3, 0, Math.PI * 2);
        ctx.fill();
      }
    }

    // Combat FX
    for (let i = this.combatFx.length - 1; i >= 0; i--) {
      const fx = this.combatFx[i];
      ctx.beginPath();
      ctx.moveTo(sx(fx.x1), sy(fx.y1));
      ctx.lineTo(sx(fx.x2), sy(fx.y2));
      ctx.strokeStyle = '#ffffff';
      ctx.lineWidth = 2.0;
      ctx.stroke();

      ctx.beginPath();
      ctx.arc(sx(fx.mx), sy(fx.my), (11 - fx.ttl) * 2.1, 0, Math.PI * 2);
      ctx.strokeStyle = fx.col;
      ctx.lineWidth = 1.6;
      ctx.stroke();
      ctx.lineWidth = 1.0;

      fx.ttl--;
      if (fx.ttl <= 0) this.combatFx.splice(i, 1);
    }

    this.drawScaleBar(scale);
  }

  private drawScaleBar(scale: number): void {
    const ctx = this.ctx;
    const candidates = [10, 20, 25, 50, 100, 150, 200, 250, 500];
    let ly = 100;
    for (const c of candidates) {
      const px = c * scale;
      if (px >= 75 && px <= 175) {
        ly = c;
        break;
      }
    }
    const barW = Math.round(ly * scale);
    const x = 16;
    const y = 68;
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
