import type { BodyType, StarSystemView, StarView } from './types';

function rgbTuple(rgb: [number, number, number]): string {
  return `${Math.round(rgb[0] * 255)},${Math.round(rgb[1] * 255)},${Math.round(rgb[2] * 255)}`;
}

function bodyDiscColor(bodyType: BodyType, atmoRgb: [number, number, number]): string {
  switch (bodyType) {
    case 'Oceanic':
      return '#38bdf8';
    case 'Terrestrial':
      return '#4ade80';
    case 'Desert':
      return '#e2a35a';
    case 'Ice':
      return '#bae6fd';
    case 'Volcanic':
      return '#f97316';
    case 'Rock':
      return '#94a3b8';
    case 'Dwarf':
      return '#cbd5e1';
    case 'GasGiant':
      return `rgb(${rgbTuple(atmoRgb)})`;
    case 'IceGiant':
      return `rgb(${rgbTuple(atmoRgb)})`;
    case 'Comet':
      return '#7dd3fc';
    default:
      return '#94a3b8';
  }
}

interface HitCircle {
  target: 'star' | 'star2' | number;
  x: number;
  y: number;
  r: number;
}

/**
 * Interactive top-down animated Orrery renderer for StarSystemView.
 * Displays primary + binary stars, Keplerian orbiting planets & moons,
 * Asteroid & Kuiper particle belts, eccentric comets with anti-solar tails,
 * and live orbital infrastructure around the anchored Primary World.
 */
export class SystemOrreryRenderer {
  private readonly canvas: HTMLCanvasElement;
  private readonly ctx: CanvasRenderingContext2D | null;
  private sys: StarSystemView | null = null;
  private rafId: number | null = null;
  private startMs = performance.now();
  private active = false;
  private hits: HitCircle[] = [];
  private hovered: 'star' | 'star2' | number | null = null;

  constructor(canvas: HTMLCanvasElement, onSelect: (which: 'star' | 'star2' | number) => void) {
    this.canvas = canvas;
    this.ctx = canvas.getContext('2d');

    canvas.addEventListener('pointermove', (e) => {
      const hit = this.hitTest(e.clientX, e.clientY);
      this.hovered = hit;
      canvas.style.cursor = hit !== null ? 'pointer' : 'default';
    });

    canvas.addEventListener('click', (e) => {
      const hit = this.hitTest(e.clientX, e.clientY);
      if (hit !== null) {
        onSelect(hit);
      }
    });
  }

  public setSystem(sys: StarSystemView | null, animate = true): void {
    this.sys = sys;
    this.hits = [];
    this.hovered = null;
    if (!sys || !animate) {
      this.stop();
      return;
    }
    this.active = true;
    if (this.rafId === null) {
      this.startMs = performance.now();
      this.rafId = requestAnimationFrame(this.tick);
    }
  }

  public stop(): void {
    this.active = false;
    if (this.rafId !== null) {
      cancelAnimationFrame(this.rafId);
      this.rafId = null;
    }
  }

  private hitTest(clientX: number, clientY: number): 'star' | 'star2' | number | null {
    const rect = this.canvas.getBoundingClientRect();
    const mx = clientX - rect.left;
    const my = clientY - rect.top;
    let best: 'star' | 'star2' | number | null = null;
    let bestD = Infinity;
    for (const h of this.hits) {
      const d = Math.hypot(mx - h.x, my - h.y);
      if (d <= Math.max(12, h.r + 6) && d < bestD) {
        bestD = d;
        best = h.target;
      }
    }
    return best;
  }

  private readonly tick = (now: number): void => {
    this.rafId = null;
    if (!this.active || !this.sys) return;
    this.draw((now - this.startMs) * 0.001);
    this.rafId = requestAnimationFrame(this.tick);
  };

  private drawStar(
    ctx: CanvasRenderingContext2D,
    x: number,
    y: number,
    star: StarView,
    rad: number,
    t: number,
  ): void {
    const rgb = rgbTuple(star.colorRgb);
    if (star.starKind === 'BlackHole') {
      // Swirling accretion glow + black event horizon + photon ring
      const g = ctx.createRadialGradient(x, y, rad * 0.7, x, y, rad * 2.8);
      g.addColorStop(0, 'rgba(255, 196, 110, 0.95)');
      g.addColorStop(0.38, 'rgba(249, 115, 22, 0.45)');
      g.addColorStop(1, 'rgba(249, 115, 22, 0)');
      ctx.fillStyle = g;
      ctx.beginPath();
      ctx.arc(x, y, rad * 2.8, 0, Math.PI * 2);
      ctx.fill();

      ctx.fillStyle = '#000000';
      ctx.beginPath();
      ctx.arc(x, y, rad * 0.72, 0, Math.PI * 2);
      ctx.fill();
      ctx.strokeStyle = 'rgba(255, 224, 160, 0.95)';
      ctx.lineWidth = 1.4;
      ctx.stroke();
      return;
    }

    if (star.starKind === 'Pulsar') {
      // Sweeping relativistic jet beams
      const jetAng = t * 2.8;
      ctx.save();
      ctx.translate(x, y);
      ctx.rotate(jetAng);
      for (const dir of [-1, 1]) {
        const g = ctx.createLinearGradient(0, 0, dir * rad * 4.2, 0);
        g.addColorStop(0, 'rgba(125, 211, 252, 0.85)');
        g.addColorStop(1, 'rgba(56, 189, 248, 0)');
        ctx.fillStyle = g;
        ctx.beginPath();
        ctx.moveTo(0, 0);
        ctx.lineTo(dir * rad * 4.2, -rad * 0.75);
        ctx.lineTo(dir * rad * 4.2, rad * 0.75);
        ctx.closePath();
        ctx.fill();
      }
      ctx.restore();
    }

    const corona = ctx.createRadialGradient(x, y, rad * 0.4, x, y, rad * 2.5);
    corona.addColorStop(0, `rgba(${rgb}, 0.95)`);
    corona.addColorStop(0.45, `rgba(${rgb}, 0.32)`);
    corona.addColorStop(1, `rgba(${rgb}, 0)`);
    ctx.fillStyle = corona;
    ctx.beginPath();
    ctx.arc(x, y, rad * 2.5, 0, Math.PI * 2);
    ctx.fill();

    ctx.fillStyle = star.starKind === 'BrownDwarf' ? '#7a1f34' : `rgb(${rgb})`;
    ctx.beginPath();
    ctx.arc(x, y, rad, 0, Math.PI * 2);
    ctx.fill();
  }

  private draw(t: number): void {
    const sys = this.sys;
    const ctx = this.ctx;
    if (!sys || !ctx) return;

    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const cssW = Math.max(1, this.canvas.clientWidth || 780);
    const cssH = Math.max(1, this.canvas.clientHeight || 390);
    const bufW = Math.round(cssW * dpr);
    const bufH = Math.round(cssH * dpr);
    if (this.canvas.width !== bufW || this.canvas.height !== bufH) {
      this.canvas.width = bufW;
      this.canvas.height = bufH;
    }

    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.fillStyle = '#05080f';
    ctx.fillRect(0, 0, cssW, cssH);

    // Subtle background starfield
    ctx.fillStyle = 'rgba(180, 210, 255, 0.25)';
    for (let i = 0; i < 65; i++) {
      const h1 = Math.sin((i + 1) * 127.1 + sys.systemSeed * 0.01) * 43758.5453;
      const h2 = Math.sin((i + 1) * 269.5 + sys.systemSeed * 0.02) * 43758.5453;
      const sx = (h1 - Math.floor(h1)) * cssW;
      const sy = (h2 - Math.floor(h2)) * cssH;
      ctx.fillRect(sx, sy, 1.2, 1.2);
    }

    const cx = cssW * 0.5;
    const cy = cssH * 0.5;
    const maxR = Math.min(cssW, cssH) * 0.455;
    const minR = sys.companionStar ? 38 : 28;
    const count = Math.max(1, sys.bodies.length);
    const stepR = (maxR - minR) / count;

    this.hits = [];

    // 1. Center Star (or tight Binary pair)
    if (sys.companionStar) {
      const binR = 13;
      const binAng = t * 0.65;
      const x1 = cx + Math.cos(binAng) * binR * 0.65;
      const y1 = cy + Math.sin(binAng) * binR * 0.65;
      const x2 = cx - Math.cos(binAng) * binR * 1.15;
      const y2 = cy - Math.sin(binAng) * binR * 1.15;
      this.drawStar(ctx, x1, y1, sys.star, 11, t);
      this.drawStar(ctx, x2, y2, sys.companionStar, 7.5, t);
      this.hits.push({ target: 'star', x: x1, y: y1, r: 12 });
      this.hits.push({ target: 'star2', x: x2, y: y2, r: 9 });
    } else {
      const sRad =
        sys.star.starKind === 'WhiteDwarf' || sys.star.starKind === 'Pulsar' ? 8.5 : 13;
      this.drawStar(ctx, cx, cy, sys.star, sRad, t);
      this.hits.push({ target: 'star', x: cx, y: cy, r: 14 });
    }

    // 2. Orbits & Bodies
    for (let i = 0; i < sys.bodies.length; i++) {
      const body = sys.bodies[i];
      const orbitR = minR + (i + 0.65) * stepR;
      const isPrimary = i === sys.primaryIndex;
      const isHovered = this.hovered === i;

      if (body.bodyType === 'AsteroidBelt' || body.bodyType === 'KuiperBelt') {
        const isKuiper = body.bodyType === 'KuiperBelt';
        const bandW = Math.max(8, stepR * 0.42);
        ctx.beginPath();
        ctx.arc(cx, cy, orbitR, 0, Math.PI * 2);
        ctx.strokeStyle = isKuiper ? 'rgba(125, 211, 252, 0.14)' : 'rgba(203, 184, 152, 0.15)';
        ctx.lineWidth = bandW;
        ctx.stroke();
        ctx.lineWidth = 1;

        const speckCount = isKuiper ? 95 : 75;
        const baseSpeed = isKuiper ? 0.035 : 0.085;
        ctx.fillStyle = isKuiper ? 'rgba(186, 230, 253, 0.68)' : 'rgba(214, 198, 172, 0.68)';
        for (let k = 0; k < speckCount; k++) {
          const s1 = Math.sin(k * 78.233 + body.seed * 0.01) * 43758.5453;
          const s2 = Math.sin(k * 45.164 + body.seed * 0.02) * 43758.5453;
          const f1 = s1 - Math.floor(s1);
          const f2 = s2 - Math.floor(s2);
          const ang = f1 * Math.PI * 2 + t * baseSpeed * (0.75 + f2 * 0.5);
          const rr = orbitR + (f2 - 0.5) * bandW;
          const px = cx + Math.cos(ang) * rr;
          const py = cy + Math.sin(ang) * rr;
          ctx.fillRect(px - 0.8, py - 0.8, 1.6, 1.6);
        }

        // Clickable label marker on belt
        const labelX = cx + Math.cos(body.orbitAngleRad) * orbitR;
        const labelY = cy + Math.sin(body.orbitAngleRad) * orbitR;
        this.hits.push({ target: i, x: labelX, y: labelY, r: 10 });
        ctx.font = '9px monospace';
        ctx.fillStyle = isHovered ? '#ffffff' : isKuiper ? 'rgba(125, 211, 252, 0.72)' : 'rgba(214, 198, 172, 0.72)';
        ctx.fillText(isKuiper ? 'Kuiper Belt' : 'Main Belt', labelX + 5, labelY + 3);
        continue;
      }

      if (body.bodyType === 'Comet') {
        // Eccentric elliptical orbit + animated dual anti-solar tail
        const a = orbitR * 0.98;
        const e = 0.54;
        const b = a * Math.sqrt(1 - e * e);
        const focusOff = a * e;
        const tilt = body.orbitAngleRad;
        ctx.save();
        ctx.translate(cx, cy);
        ctx.rotate(tilt);
        ctx.beginPath();
        ctx.ellipse(-focusOff, 0, a, b, 0, 0, Math.PI * 2);
        ctx.strokeStyle = 'rgba(125, 211, 252, 0.25)';
        ctx.setLineDash([4, 4]);
        ctx.stroke();
        ctx.setLineDash([]);
        ctx.restore();

        const ang = body.orbitAngleRad + t * 0.28;
        const lx = Math.cos(ang) * a - focusOff;
        const ly = Math.sin(ang) * b;
        const px = cx + lx * Math.cos(tilt) - ly * Math.sin(tilt);
        const py = cy + lx * Math.sin(tilt) + ly * Math.cos(tilt);

        // Tail always points radially AWAY from star (cx, cy)
        const outAng = Math.atan2(py - cy, px - cx);
        const tailLen = 28;
        // Blue ion tail
        const gIon = ctx.createLinearGradient(px, py, px + Math.cos(outAng) * tailLen, py + Math.sin(outAng) * tailLen);
        gIon.addColorStop(0, 'rgba(56, 189, 248, 0.92)');
        gIon.addColorStop(1, 'rgba(56, 189, 248, 0)');
        ctx.strokeStyle = gIon;
        ctx.lineWidth = 2.2;
        ctx.beginPath();
        ctx.moveTo(px, py);
        ctx.lineTo(px + Math.cos(outAng) * tailLen, py + Math.sin(outAng) * tailLen);
        ctx.stroke();

        // Warm dust tail (slightly angled)
        const dustAng = outAng + 0.18;
        const gDust = ctx.createLinearGradient(px, py, px + Math.cos(dustAng) * (tailLen * 0.8), py + Math.sin(dustAng) * (tailLen * 0.8));
        gDust.addColorStop(0, 'rgba(254, 240, 199, 0.72)');
        gDust.addColorStop(1, 'rgba(254, 240, 199, 0)');
        ctx.strokeStyle = gDust;
        ctx.lineWidth = 3.0;
        ctx.beginPath();
        ctx.moveTo(px, py);
        ctx.lineTo(px + Math.cos(dustAng) * (tailLen * 0.8), py + Math.sin(dustAng) * (tailLen * 0.8));
        ctx.stroke();
        ctx.lineWidth = 1;

        ctx.fillStyle = '#e0f2fe';
        ctx.beginPath();
        ctx.arc(px, py, 2.6, 0, Math.PI * 2);
        ctx.fill();
        this.hits.push({ target: i, x: px, y: py, r: 8 });
        continue;
      }

      // Standard planet orbit track
      ctx.beginPath();
      ctx.arc(cx, cy, orbitR, 0, Math.PI * 2);
      ctx.strokeStyle = isPrimary
        ? 'rgba(74, 222, 128, 0.38)'
        : isHovered
          ? 'rgba(56, 189, 248, 0.45)'
          : 'rgba(148, 163, 184, 0.16)';
      ctx.lineWidth = isPrimary ? 1.4 : 1.0;
      ctx.stroke();

      // Keplerian angular speed (inner planets orbit faster)
      const speed = 0.55 / Math.pow(orbitR / 35, 1.15);
      const ang = body.orbitAngleRad + t * speed;
      const px = cx + Math.cos(ang) * orbitR;
      const py = cy + Math.sin(ang) * orbitR;
      const discR =
        body.bodyType === 'GasGiant'
          ? 7.2
          : body.bodyType === 'IceGiant'
            ? 5.8
            : body.bodyType === 'Dwarf'
              ? 3.0
              : Math.max(3.6, Math.min(5.6, 3.2 + body.radiusEarth * 1.4));

      // Planetary rings
      if (body.hasRings) {
        ctx.beginPath();
        ctx.ellipse(px, py, discR * 1.85, discR * 0.72, 0.35, 0, Math.PI * 2);
        ctx.strokeStyle = `rgba(${rgbTuple(body.ringColorRgb)}, 0.75)`;
        ctx.lineWidth = 1.6;
        ctx.stroke();
        ctx.lineWidth = 1;
      }

      // Active ECM Jammer pulsing aura around Primary World
      const ls = body.liveState;
      if (ls?.hasJammer) {
        const pulse = 0.55 + 0.45 * Math.sin(t * 4.0);
        ctx.beginPath();
        ctx.arc(px, py, discR + 6 + pulse * 3, 0, Math.PI * 2);
        ctx.strokeStyle = `rgba(192, 132, 252, ${0.3 + pulse * 0.35})`;
        ctx.setLineDash([3, 3]);
        ctx.stroke();
        ctx.setLineDash([]);
      }

      // Active Stargate cyan ring around Primary World
      if (ls?.hasStargate) {
        ctx.beginPath();
        ctx.arc(px, py, discR + 4.2, 0, Math.PI * 2);
        ctx.strokeStyle = '#22d3ee';
        ctx.lineWidth = 1.4;
        ctx.stroke();
        ctx.lineWidth = 1;
      }

      // Planet disc
      ctx.fillStyle = bodyDiscColor(body.bodyType, body.atmoColorRgb);
      ctx.beginPath();
      ctx.arc(px, py, discR, 0, Math.PI * 2);
      ctx.fill();

      if (isPrimary) {
        ctx.strokeStyle = ls?.owner ? ls.ownerColor : '#4ade80';
        ctx.lineWidth = 1.5;
        ctx.beginPath();
        ctx.arc(px, py, discR + 2.2, 0, Math.PI * 2);
        ctx.stroke();
        ctx.lineWidth = 1;
      }

      // Orbiting moons
      const moons = body.moons ?? [];
      for (let m = 0; m < Math.min(4, moons.length); m++) {
        const mn = moons[m];
        const mr = discR + 4.5 + m * 3.0;
        const mang = mn.orbitAngleRad + t * mn.orbitSpeed * 3.2;
        const mx = px + Math.cos(mang) * mr;
        const my = py + Math.sin(mang) * mr;
        ctx.fillStyle = mn.bodyType === 'Ice' ? '#bae6fd' : mn.bodyType === 'Volcanic' ? '#fb923c' : '#cbd5e1';
        ctx.beginPath();
        ctx.arc(mx, my, 1.5, 0, Math.PI * 2);
        ctx.fill();
      }

      // Label
      const shortName = body.name.replace(/^System #\d+\s*/, '');
      ctx.font = isPrimary ? 'bold 10px monospace' : '9px monospace';
      ctx.fillStyle = isPrimary ? '#4ade80' : isHovered ? '#ffffff' : 'rgba(203, 213, 225, 0.78)';
      ctx.fillText(isPrimary ? `★ ${shortName}` : shortName, px + discR + 5, py + 3);

      this.hits.push({ target: i, x: px, y: py, r: discR + 4 });
    }
  }
}
