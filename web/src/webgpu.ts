import type { CombatFxView, JumpFxView, ScannerFieldSettings, WormholeView } from './types';

export interface CameraUniformData {
  offX: number;
  offY: number;
  scale: number;
  zoom: number;
  viewportW: number;
  viewportH: number;
  denseFactor: number;
  invCrustNorm: number;
  planetCount: number;
}

class DynamicGpuBuffer {
  private buffer: GPUBuffer | null = null;
  private byteCapacity = 0;

  constructor(
    private readonly device: GPUDevice,
    private readonly usage: GPUBufferUsageFlags,
  ) {}

  public upload(data: Float32Array, byteLength = data.byteLength): GPUBuffer | null {
    if (byteLength === 0) return this.buffer;
    const aligned = (byteLength + 3) & ~3;
    if (!this.buffer || aligned > this.byteCapacity) {
      this.buffer?.destroy();
      let nextCap = Math.max(256, this.byteCapacity);
      while (nextCap < aligned) {
        nextCap = Math.ceil(nextCap * 1.5);
      }
      this.byteCapacity = (nextCap + 3) & ~3;
      this.buffer = this.device.createBuffer({
        size: this.byteCapacity,
        usage: this.usage | GPUBufferUsage.COPY_DST,
      });
    }
    this.device.queue.writeBuffer(this.buffer, 0, data.buffer, data.byteOffset, aligned);
    return this.buffer;
  }

  public get gpuBuffer(): GPUBuffer | null {
    return this.buffer;
  }

  public destroy(): void {
    this.buffer?.destroy();
    this.buffer = null;
    this.byteCapacity = 0;
  }
}

function parseHexRgba(hex: string, out: Float32Array, offset: number): void {
  const clean = hex.startsWith('#') ? hex.slice(1) : hex;
  if (clean.length !== 6) {
    out[offset] = 0.22;
    out[offset + 1] = 0.74;
    out[offset + 2] = 0.97;
    out[offset + 3] = 1.0;
    return;
  }
  const r = parseInt(clean.slice(0, 2), 16);
  const g = parseInt(clean.slice(2, 4), 16);
  const b = parseInt(clean.slice(4, 6), 16);
  out[offset] = Number.isNaN(r) ? 0.22 : r / 255;
  out[offset + 1] = Number.isNaN(g) ? 0.74 : g / 255;
  out[offset + 2] = Number.isNaN(b) ? 0.97 : b / 255;
  out[offset + 3] = 1.0;
}

const COMMON_WGSL = /* wgsl */ `
struct CameraUniforms {
  offX: f32,
  offY: f32,
  scale: f32,
  zoom: f32,
  viewportW: f32,
  viewportH: f32,
  denseFactor: f32,
  invCrustNorm: f32,
  planetCount: f32,
  _pad0: f32,
  _pad1: f32,
  _pad2: f32,
};

struct EmpirePalette {
  colors: array<vec4<f32>, 17>,
};

struct EmpireRanges {
  ranges: array<vec4<f32>, 17>, // .x = scannerRange, .y = flightRange, .z = ecmRange, .w = ecmStrength
};

struct FieldUniforms {
  empireMask: u32,     // bit i (0..15) => empire (i + 1) enabled
  eqMode: u32,         // 0 = gap, 1 = none
  contourMode: u32,    // 0 = hidden, 1 = outer, 2 = subtle, 3 = full
  contourOpacity: f32, // 0.0 .. 0.80
};

@group(0) @binding(0) var<uniform> cam: CameraUniforms;
@group(0) @binding(1) var<uniform> palette: EmpirePalette;
@group(0) @binding(2) var<uniform> empRanges: EmpireRanges;
@group(0) @binding(3) var<uniform> fieldCfg: FieldUniforms;

var<private> QUAD: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
  vec2<f32>(-1.0, -1.0),
  vec2<f32>( 1.0, -1.0),
  vec2<f32>(-1.0,  1.0),
  vec2<f32>(-1.0,  1.0),
  vec2<f32>( 1.0, -1.0),
  vec2<f32>( 1.0,  1.0),
);

fn screenToClip(px: vec2<f32>) -> vec4<f32> {
  let x = (px.x / max(1.0, cam.viewportW)) * 2.0 - 1.0;
  let y = 1.0 - (px.y / max(1.0, cam.viewportH)) * 2.0;
  return vec4<f32>(x, y, 0.0, 1.0);
}

fn worldToScreen(w: vec2<f32>) -> vec2<f32> {
  return vec2<f32>(cam.offX + w.x * cam.scale, cam.offY + w.y * cam.scale);
}

fn empireColor(idx: f32) -> vec3<f32> {
  let i = u32(clamp(round(idx), 0.0, 16.0));
  return palette.colors[i].rgb;
}
`;

const BG_STARS_WGSL =
  COMMON_WGSL +
  /* wgsl */ `
struct BgOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) alpha: f32,
};

@vertex
fn vs_main(
  @builtin(vertex_index) vid: u32,
  @location(0) star: vec4<f32>, // [normX, normY, radiusPx, alpha]
) -> BgOut {
  let corner = QUAD[vid];
  let center = vec2<f32>(star.x * cam.viewportW, star.y * cam.viewportH);
  let p = center + corner * (star.z * 0.5);
  var out: BgOut;
  out.pos = screenToClip(p);
  out.alpha = star.w;
  return out;
}

@fragment
fn fs_main(in: BgOut) -> @location(0) vec4<f32> {
  return vec4<f32>(0.706, 0.824, 1.0, in.alpha);
}
`;

const SIGNAL_FIELD_WGSL =
  COMMON_WGSL +
  /* wgsl */ `
struct SignalVertOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) uv: vec2<f32>,
  @location(1) empIdx: f32,
  @location(2) strength: f32,
};

struct SignalFragOut {
  @location(0) f0: vec4<f32>, // empires 0..3 scanner
  @location(1) f1: vec4<f32>, // empires 4..5 scanner (.rg), empires 4..5 jammer (.ba)
  @location(2) j0: vec4<f32>, // empires 0..3 jammer
};

@vertex
fn vs_signal(
  @builtin(vertex_index) vid: u32,
  @location(0) worldPos: vec2<f32>,
  @location(1) meta0: vec4<f32>, // [owner, factories, hab, flags(bit0=gate, bit1=ecm)]
) -> SignalVertOut {
  var out: SignalVertOut;
  let ownerU = u32(round(meta0.x));
  if (ownerU == 0u || ownerU > 6u) {
    out.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(0.0);
    out.empIdx = -1.0;
    out.strength = 0.0;
    return out;
  }

  let empIdx = ownerU - 1u;
  if ((fieldCfg.empireMask & (1u << empIdx)) == 0u) {
    out.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(0.0);
    out.empIdx = -1.0;
    out.strength = 0.0;
    return out;
  }

  let rad = empRanges.ranges[ownerU].x * cam.scale;
  let center = worldToScreen(worldPos);
  if (
    rad < 1.5 ||
    center.x + rad < 0.0 ||
    center.x - rad > cam.viewportW ||
    center.y + rad < 0.0 ||
    center.y - rad > cam.viewportH
  ) {
    out.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(0.0);
    out.empIdx = -1.0;
    out.strength = 0.0;
    return out;
  }

  let corner = QUAD[vid];
  out.pos = screenToClip(center + corner * rad);
  out.uv = corner;
  out.empIdx = f32(empIdx);
  out.strength = 1.0;
  return out;
}

@vertex
fn vs_jammer(
  @builtin(vertex_index) vid: u32,
  @location(0) worldPos: vec2<f32>,
  @location(1) meta0: vec4<f32>, // [owner, factories, hab, flags(bit0=gate, bit1=ecm)]
) -> SignalVertOut {
  var out: SignalVertOut;
  let ownerU = u32(round(meta0.x));
  let flags = u32(round(meta0.w));
  if (ownerU == 0u || ownerU > 6u || (flags & 2u) == 0u) {
    out.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(0.0);
    out.empIdx = -1.0;
    out.strength = 0.0;
    return out;
  }

  let empIdx = ownerU - 1u;
  let ecmRange = empRanges.ranges[ownerU].z;
  let ecmStrength = empRanges.ranges[ownerU].w;
  let rad = ecmRange * cam.scale;
  let center = worldToScreen(worldPos);
  if (
    ecmStrength <= 0.001 ||
    rad < 1.5 ||
    center.x + rad < 0.0 ||
    center.x - rad > cam.viewportW ||
    center.y + rad < 0.0 ||
    center.y - rad > cam.viewportH
  ) {
    out.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(0.0);
    out.empIdx = -1.0;
    out.strength = 0.0;
    return out;
  }

  let corner = QUAD[vid];
  out.pos = screenToClip(center + corner * rad);
  out.uv = corner;
  out.empIdx = f32(empIdx);
  out.strength = ecmStrength;
  return out;
}

@fragment
fn fs_signal(in: SignalVertOut) -> SignalFragOut {
  let d = length(in.uv);
  if (d >= 1.0 || in.empIdx < -0.5) {
    discard;
  }
  let s = max(0.0, 1.0 - d);
  let emp = u32(round(in.empIdx));

  var f0 = vec4<f32>(0.0);
  var f1 = vec4<f32>(0.0);
  if (emp == 0u) {
    f0.r = s;
  } else if (emp == 1u) {
    f0.g = s;
  } else if (emp == 2u) {
    f0.b = s;
  } else if (emp == 3u) {
    f0.a = s;
  } else if (emp == 4u) {
    f1.r = s;
  } else if (emp == 5u) {
    f1.g = s;
  }

  var out: SignalFragOut;
  out.f0 = f0;
  out.f1 = f1;
  out.j0 = vec4<f32>(0.0);
  return out;
}

@fragment
fn fs_jammer(in: SignalVertOut) -> SignalFragOut {
  let d = length(in.uv);
  if (d >= 1.0 || in.empIdx < -0.5) {
    discard;
  }
  let j = max(0.0, 1.0 - d) * in.strength;
  let emp = u32(round(in.empIdx));

  var f1 = vec4<f32>(0.0);
  var j0 = vec4<f32>(0.0);
  if (emp == 0u) {
    j0.r = j;
  } else if (emp == 1u) {
    j0.g = j;
  } else if (emp == 2u) {
    j0.b = j;
  } else if (emp == 3u) {
    j0.a = j;
  } else if (emp == 4u) {
    f1.b = j;
  } else if (emp == 5u) {
    f1.a = j;
  }

  var out: SignalFragOut;
  out.f0 = vec4<f32>(0.0);
  out.f1 = f1;
  out.j0 = j0;
  return out;
}
`;

const SIGNAL_COMPOSITE_WGSL =
  COMMON_WGSL +
  /* wgsl */ `
@group(1) @binding(0) var signalTex0: texture_2d<f32>;
@group(1) @binding(1) var signalTex1: texture_2d<f32>;
@group(1) @binding(2) var fieldSampler: sampler;
@group(1) @binding(3) var jamTex0: texture_2d<f32>;

struct FullscreenOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) vid: u32) -> FullscreenOut {
  var pos = array<vec2<f32>, 3>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>( 3.0, -1.0),
    vec2<f32>(-1.0,  3.0),
  );
  let p = pos[vid];
  var out: FullscreenOut;
  out.pos = vec4<f32>(p, 0.0, 1.0);
  out.uv = vec2<f32>(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
  return out;
}

fn sampleSignals(uv: vec2<f32>) -> array<f32, 6> {
  let raw0 = textureSampleLevel(signalTex0, fieldSampler, uv, 0.0);
  let raw1 = textureSampleLevel(signalTex1, fieldSampler, uv, 0.0);
  let jam0 = textureSampleLevel(jamTex0, fieldSampler, uv, 0.0);
  var raw = array<f32, 6>(raw0.r, raw0.g, raw0.b, raw0.a, raw1.r, raw1.g);
  var jam = array<f32, 6>(jam0.r, jam0.g, jam0.b, jam0.a, raw1.b, raw1.a);

  var out: array<f32, 6>;
  for (var i: u32 = 0u; i < 6u; i = i + 1u) {
    let enabled = (fieldCfg.empireMask & (1u << i)) != 0u;
    // Faction-aware ECM: jammer of empire E never suppresses E, only enemy empires
    var enemyJam: f32 = 0.0;
    for (var j: u32 = 0u; j < 6u; j = j + 1u) {
      if (j != i) {
        enemyJam = max(enemyJam, jam[j]);
      }
    }
    out[i] = select(0.0, max(0.0, clamp(raw[i], 0.0, 1.0) - enemyJam), enabled);
  }
  return out;
}

@fragment
fn fs_composite(in: FullscreenOut) -> @location(0) vec4<f32> {
  // Smooth 5-tap prefilter (1.5 px) + wide central-difference gradient (3.5 px):
  // eliminates rgba16float quantization staircases at extreme zoom (10x-40x)
  // while keeping V-fold-free gradients on A = B equilibrium seams.
  const FILTER_PX: f32 = 1.5;
  const GRAD_PX: f32 = 3.5;
  let invW = 1.0 / max(1.0, cam.viewportW);
  let invH = 1.0 / max(1.0, cam.viewportH);

  let fDx = vec2<f32>(FILTER_PX * invW, 0.0);
  let fDy = vec2<f32>(0.0, FILTER_PX * invH);
  let gDx = vec2<f32>(GRAD_PX * invW, 0.0);
  let gDy = vec2<f32>(0.0, GRAD_PX * invH);

  var sigC = sampleSignals(in.uv);
  var sigFR = sampleSignals(in.uv + fDx);
  var sigFL = sampleSignals(in.uv - fDx);
  var sigFU = sampleSignals(in.uv + fDy);
  var sigFD = sampleSignals(in.uv - fDy);

  var sigGR = sampleSignals(in.uv + gDx);
  var sigGL = sampleSignals(in.uv - gDx);
  var sigGU = sampleSignals(in.uv + gDy);
  var sigGD = sampleSignals(in.uv - gDy);

  var sig: array<f32, 6>;
  var gradX: array<f32, 6>;
  var gradY: array<f32, 6>;
  let invSpan = 1.0 / (2.0 * GRAD_PX);
  for (var i: u32 = 0u; i < 6u; i = i + 1u) {
    sig[i] = (sigC[i] * 2.0 + sigFR[i] + sigFL[i] + sigFU[i] + sigFD[i]) * (1.0 / 6.0);
    gradX[i] = (sigGR[i] - sigGL[i]) * invSpan;
    gradY[i] = (sigGU[i] - sigGD[i]) * invSpan;
  }

  var top1Val: f32 = 0.0;
  var top1Idx: u32 = 0u;
  var top2Val: f32 = 0.0;
  var top2Idx: u32 = 0u;

  for (var i: u32 = 0u; i < 6u; i = i + 1u) {
    let v = sig[i];
    if (v > top1Val) {
      top2Val = top1Val;
      top2Idx = top1Idx;
      top1Val = v;
      top1Idx = i;
    } else if (v > top2Val) {
      top2Val = v;
      top2Idx = i;
    }
  }

  const THRESH: f32 = 0.015;

  let g1 = max(length(vec2<f32>(gradX[top1Idx], gradY[top1Idx])), 2e-5);
  let g2 = max(length(vec2<f32>(gradX[top2Idx], gradY[top2Idx])), 2e-5);
  let dist1Px = (top1Val - THRESH) / g1;
  let dist2Px = (top2Val - THRESH) / g2;

  let inside1 = smoothstep(-0.75, 0.75, dist1Px);
  let inside2 = smoothstep(-0.75, 0.75, dist2Px);

  // Fill strictly from top1 (zero RGB mixing) with concave radial glow around planets
  let top1Col = empireColor(f32(top1Idx + 1u));
  let baseTint = smoothstep(0.0, 0.08, top1Val) * 0.05;
  let radialGlow = pow(top1Val, 2.2) * 0.48;
  var fillA = inside1 * clamp(baseTint + radialGlow, 0.0, 0.65);

  // Equilibrium line (A = B): wide central-difference gradient with fixed (top1Idx, top2Idx)
  if (fieldCfg.eqMode == 0u && top2Val > 0.001) {
    let diffR = sigGR[top1Idx] - sigGR[top2Idx];
    let diffL = sigGL[top1Idx] - sigGL[top2Idx];
    let diffU = sigGU[top1Idx] - sigGU[top2Idx];
    let diffD = sigGD[top1Idx] - sigGD[top2Idx];
    let eqGradLen = length(vec2<f32>(diffR - diffL, diffU - diffD) * invSpan);
    if (eqGradLen >= 2e-5) {
      let eqDistPx = (top1Val - top2Val) / eqGradLen;
      let gapMask = smoothstep(0.55, 1.85, eqDistPx);
      fillA = fillA * mix(1.0, gapMask, inside2);
    }
  }

  var outRgb = top1Col;
  var outA = fillA;

  // Antialiased ~1 px range contours using the same smooth central-difference gradients
  if (fieldCfg.contourMode != 0u && fieldCfg.contourOpacity > 0.001) {
    if (fieldCfg.contourMode == 1u) {
      // Outer only: draw contour only where zone meets empty space
      let hasSig1 = smoothstep(0.003, 0.015, top1Val);
      let line1 = (1.0 - smoothstep(0.42, 1.32, abs(dist1Px))) * hasSig1;
      let outerMask = 1.0 - smoothstep(-0.5, 1.0, dist2Px);
      let cA = line1 * outerMask * fieldCfg.contourOpacity;
      if (cA > 0.001) {
        let combA = cA + outA * (1.0 - cA);
        outRgb = (top1Col * cA + outRgb * outA * (1.0 - cA)) / max(0.001, combA);
        outA = combA;
      }
    } else {
      // Subtle / Full everywhere: uniform isoline across entire perimeter for each enabled empire
      var bestA: f32 = 0.0;
      var bestCol = vec3<f32>(0.0);
      for (var i: u32 = 0u; i < 6u; i = i + 1u) {
        let v = sig[i];
        if (v > 0.002) {
          let g = max(length(vec2<f32>(gradX[i], gradY[i])), 2e-5);
          let linePx = abs(v - THRESH) / g;
          let hasSig = smoothstep(0.003, 0.015, v);
          let stroke = (1.0 - smoothstep(0.42, 1.32, linePx)) * hasSig;
          let cA = stroke * fieldCfg.contourOpacity;
          if (cA > bestA + 0.001 || (abs(cA - bestA) <= 0.001 && i == top1Idx && cA > 0.0)) {
            bestA = cA;
            bestCol = empireColor(f32(i + 1u));
          }
        }
      }
      if (bestA > 0.001) {
        let combA = bestA + outA * (1.0 - bestA);
        outRgb = (bestCol * bestA + outRgb * outA * (1.0 - bestA)) / max(0.001, combA);
        outA = combA;
      }
    }
  }

  if (outA < 0.003) {
    discard;
  }
  return vec4<f32>(outRgb, outA);
}
`;

const PLANET_CORE_WGSL =
  COMMON_WGSL +
  /* wgsl */ `
struct PlanetOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) localPx: vec2<f32>,
  @location(1) coreRad: f32,
  @location(2) color: vec3<f32>,
  @location(3) alphaMul: f32,
  @location(4) flags: f32, // bit0 = hasGate, bit2 = hasGarrison
};

@vertex
fn vs_main(
  @builtin(vertex_index) vid: u32,
  @location(0) worldPos: vec2<f32>,
  @location(1) meta0: vec4<f32>, // [owner, factories, hab, flags]
  @location(2) crustSum: f32,
) -> PlanetOut {
  let owner = meta0.x;
  let factories = meta0.y;
  let hab = meta0.z;
  let rawFlags = u32(round(meta0.w));
  let hasGate = owner > 0.5 && (rawFlags & 1u) != 0u;
  let hasGarrison = owner > 0.5 && (rawFlags & 4u) != 0u;

  var r: f32;
  var col: vec3<f32>;
  var aMul: f32 = 1.0;

  if (owner > 0.5) {
    r = min(13.0, 4.0 + sqrt(max(0.0, factories)) * 0.45) * cam.denseFactor;
    col = empireColor(owner);
  } else {
    let fastDot = cam.planetCount >= 5000.0 && cam.zoom < 4.0;
    if (fastDot) {
      r = 1.05;
      aMul = 0.58;
    } else {
      r = (3.0 + hab * 2.5) * cam.denseFactor;
      aMul = min(1.0, 0.3 + crustSum * cam.invCrustNorm * 0.7);
    }
    col = vec3<f32>(0.58, 0.643, 0.722);
  }

  let pad = select(select(1.4, 5.2, hasGarrison), 5.6, hasGate);
  let quadRad = r + pad;
  let corner = QUAD[vid];
  let center = worldToScreen(worldPos);
  let localPx = corner * quadRad;

  var out: PlanetOut;
  out.pos = screenToClip(center + localPx);
  out.localPx = localPx;
  out.coreRad = r;
  out.color = col;
  out.alphaMul = aMul;
  out.flags = select(0.0, 1.0, hasGate) + select(0.0, 4.0, hasGarrison);
  return out;
}

@fragment
fn fs_main(in: PlanetOut) -> @location(0) vec4<f32> {
  let dist = length(in.localPx);
  let coreAlpha = (1.0 - smoothstep(in.coreRad - 0.75, in.coreRad + 0.75, dist)) * in.alphaMul;
  let fl = u32(round(in.flags));

  var rgb = in.color;
  var alpha = coreAlpha;

  if ((fl & 1u) != 0u) {
    let ringDist = abs(dist - (in.coreRad + 3.5));
    let ringAlpha = 1.0 - smoothstep(0.55, 1.35, ringDist);
    if (ringAlpha > 0.01) {
      let gateCol = vec3<f32>(0.133, 0.827, 0.933);
      let outA = ringAlpha + alpha * (1.0 - ringAlpha);
      rgb = (gateCol * ringAlpha + rgb * alpha * (1.0 - ringAlpha)) / max(0.001, outA);
      alpha = outA;
    }
  }

  // Subtle orbital garrison pip at top-right rim of the star (indicates stationed fleets in orbit)
  if ((fl & 4u) != 0u) {
    let pipOffset = (in.coreRad + 2.4) * 0.7071;
    let pipCenter = vec2<f32>(pipOffset, -pipOffset);
    let pipD = length(in.localPx - pipCenter);
    let pipBorder = 1.0 - smoothstep(1.9, 2.5, pipD);
    let pipCore = 1.0 - smoothstep(1.0, 1.5, pipD);
    if (pipBorder > 0.01) {
      let pipCol = mix(vec3<f32>(0.05, 0.09, 0.16), vec3<f32>(0.95, 0.98, 1.0), pipCore);
      let outA = pipBorder + alpha * (1.0 - pipBorder);
      rgb = (pipCol * pipBorder + rgb * alpha * (1.0 - pipBorder)) / max(0.001, outA);
      alpha = outA;
    }
  }

  if (alpha < 0.01) {
    discard;
  }
  return vec4<f32>(rgb, alpha);
}
`;

const DEBRIS_WGSL =
  COMMON_WGSL +
  /* wgsl */ `
struct DebrisOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) localPx: vec2<f32>,
  @location(1) glowTier: f32,
};

@vertex
fn vs_main(
  @builtin(vertex_index) vid: u32,
  @location(0) debrisData: vec3<f32>,
) -> DebrisOut {
  // Smooth log-tier glow (0.0 for <50 kT, ~0.45 for 500 kT, ~0.8 for 3000 kT, 1.0 for >=10000 kT)
  let totalKt = max(debrisData.z, 1.0);
  let glowTier = clamp((log2(totalKt) - 4.5) / 9.0, 0.0, 1.0);
  let quadHalf = mix(9.0, 18.0, glowTier);
  let corner = QUAD[vid];
  let localPx = corner * quadHalf;
  let center = worldToScreen(debrisData.xy);
  var out: DebrisOut;
  out.pos = screenToClip(center + localPx);
  out.localPx = localPx;
  out.glowTier = glowTier;
  return out;
}

@fragment
fn fs_main(in: DebrisOut) -> @location(0) vec4<f32> {
  // Keep 4-point ✦ sparkle crisp at a readable base size; scale luminous glow halo by mass tier
  let p = abs(in.localPx);
  let d = length(p);
  let tier = in.glowTier;
  let starScale = mix(1.0, 1.25, tier);
  let sp = p / starScale;
  let starDist = pow(max(sp.x, 0.0001), 0.58) + pow(max(sp.y, 0.0001), 0.58);
  let star = 1.0 - smoothstep(2.45, 3.05, starDist);
  let core = 1.0 - smoothstep(0.6, mix(2.0, 3.2, tier), d);

  let haloDecay = mix(0.42, 0.16, tier);
  let haloStrength = mix(0.24, 0.78, tier);
  let halo = exp(-d * haloDecay) * haloStrength;
  let alpha = clamp(max(star * mix(0.84, 1.0, tier), halo), 0.0, 1.0);
  if (alpha < 0.02) {
    discard;
  }
  let dimViolet = vec3<f32>(0.68, 0.46, 0.94);
  let richViolet = vec3<f32>(0.82, 0.56, 1.0);
  let coreWhite = vec3<f32>(1.0, 0.96, 1.0);
  let baseCol = mix(dimViolet, richViolet, tier);
  let rgb = mix(baseCol, coreWhite, clamp(core * mix(0.65, 1.0, tier), 0.0, 1.0));
  return vec4<f32>(rgb, alpha);
}
`;

const FLEET_LINES_WGSL =
  COMMON_WGSL +
  /* wgsl */ `
struct LineOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(
  @builtin(vertex_index) vid: u32,
  @location(0) pos: vec2<f32>,
  @location(1) targetPos: vec2<f32>,
  @location(2) fleetInfo: vec3<f32>, // [emp, typeCode, flags]
) -> LineOut {
  let worldP = select(pos, targetPos, vid == 1u);
  let flags = u32(fleetInfo.z);
  let fType = u32(round(fleetInfo.y));

  // Skip trajectory lines for orbiting/docked fleets
  if ((flags & 4u) != 0u) {
    var emptyOut: LineOut;
    emptyOut.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
    emptyOut.color = vec4<f32>(0.0);
    return emptyOut;
  }

  var col: vec4<f32>;
  if ((flags & 1u) != 0u) {
    col = vec4<f32>(0.973, 0.443, 0.443, 0.45);
  } else if (fType == 4u) {
    col = vec4<f32>(0.753, 0.518, 0.988, 0.16);
  } else {
    col = vec4<f32>(1.0, 1.0, 1.0, 0.09);
  }

  var out: LineOut;
  out.pos = screenToClip(worldToScreen(worldP));
  out.color = col;
  return out;
}

@fragment
fn fs_main(in: LineOut) -> @location(0) vec4<f32> {
  return in.color;
}
`;

const FLEET_ICONS_WGSL =
  COMMON_WGSL +
  /* wgsl */ `
struct FleetOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) localPx: vec2<f32>,
  @location(1) color: vec3<f32>,
  @location(2) fType: f32,
};

@vertex
fn vs_main(
  @builtin(vertex_index) vid: u32,
  @location(0) pos: vec2<f32>,
  @location(2) fleetInfo: vec3<f32>, // [emp, typeCode, flags]
) -> FleetOut {
  let corner = QUAD[vid];
  let localPx = corner * 6.5;
  var out: FleetOut;
  out.pos = screenToClip(worldToScreen(pos) + localPx);
  out.localPx = localPx;
  out.color = empireColor(fleetInfo.x);
  out.fType = fleetInfo.y;
  return out;
}

@fragment
fn fs_main(in: FleetOut) -> @location(0) vec4<f32> {
  let p = in.localPx;
  let ft = u32(round(in.fType));

  if (ft == 2u) {
    // Battleship: diamond
    let d = abs(p.x) + abs(p.y);
    let a = 1.0 - smoothstep(4.8, 5.8, d);
    if (a < 0.02) { discard; }
    return vec4<f32>(in.color, a);
  } else if (ft == 3u) {
    // Bomber: circle with red rim
    let d = length(p);
    let fillA = 1.0 - smoothstep(3.8, 4.7, d);
    let rimA = 1.0 - smoothstep(0.45, 1.1, abs(d - 4.5));
    let outA = max(fillA, rimA);
    if (outA < 0.02) { discard; }
    let rimCol = vec3<f32>(0.973, 0.443, 0.443);
    let rgb = mix(in.color, rimCol, rimA);
    return vec4<f32>(rgb, outA);
  } else if (ft == 1u) {
    // Cruiser: upward triangle (0,-5), (-4.2,3.5), (4.2,3.5)
    let inY = p.y >= -5.0 && p.y <= 3.5;
    let halfW = ((p.y + 5.0) / 8.5) * 4.2;
    if (!inY || abs(p.x) > halfW + 0.4) { discard; }
    let edge = min(min(3.5 - p.y, p.y + 5.0), halfW - abs(p.x));
    let a = smoothstep(-0.4, 0.6, edge);
    if (a < 0.02) { discard; }
    return vec4<f32>(in.color, a);
  } else if (ft == 4u) {
    // Freighter: square
    let d = max(abs(p.x), abs(p.y));
    let a = 1.0 - smoothstep(2.4, 3.1, d);
    if (a < 0.02) { discard; }
    return vec4<f32>(in.color, a);
  } else {
    // Colonizer: small dot
    let d = length(p);
    let a = 1.0 - smoothstep(2.4, 3.3, d);
    if (a < 0.02) { discard; }
    return vec4<f32>(in.color, a);
  }
}
`;

const DYNAMIC_LINES_WGSL =
  COMMON_WGSL +
  /* wgsl */ `
struct DynLineOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(
  @location(0) worldPos: vec2<f32>,
  @location(1) color: vec4<f32>,
) -> DynLineOut {
  var out: DynLineOut;
  out.pos = screenToClip(worldToScreen(worldPos));
  out.color = color;
  return out;
}

@fragment
fn fs_main(in: DynLineOut) -> @location(0) vec4<f32> {
  return in.color;
}
`;

const ALPHA_BLEND: GPUBlendState = {
  color: {
    srcFactor: 'src-alpha',
    dstFactor: 'one-minus-src-alpha',
    operation: 'add',
  },
  alpha: {
    srcFactor: 'one',
    dstFactor: 'one-minus-src-alpha',
    operation: 'add',
  },
};

const MAX_BLEND: GPUBlendState = {
  color: {
    srcFactor: 'one',
    dstFactor: 'one',
    operation: 'max',
  },
  alpha: {
    srcFactor: 'one',
    dstFactor: 'one',
    operation: 'max',
  },
};

export class WebGpuGalaxyBackend {
  private readonly device: GPUDevice;
  private readonly context: GPUCanvasContext;
  private readonly format: GPUTextureFormat;

  private readonly cameraUniformBuf: GPUBuffer;
  private readonly paletteUniformBuf: GPUBuffer;
  private readonly rangesUniformBuf: GPUBuffer;
  private readonly fieldUniformBuf: GPUBuffer;
  private readonly cameraData = new Float32Array(12);
  private readonly paletteData = new Float32Array(17 * 4);
  private readonly rangesData = new Float32Array(17 * 4);
  private readonly fieldRawBuf = new ArrayBuffer(16);
  private readonly fieldU32 = new Uint32Array(this.fieldRawBuf);
  private readonly fieldF32 = new Float32Array(this.fieldRawBuf);
  private readonly sharedBindGroup: GPUBindGroup;
  private readonly fieldTexBindGroupLayout: GPUBindGroupLayout;
  private readonly fieldSampler: GPUSampler;

  private readonly bgStarsBuf: GPUBuffer;
  private readonly bgStarCount: number;

  private readonly planetsGpuBuf: DynamicGpuBuffer;
  private readonly fleetsGpuBuf: DynamicGpuBuffer;
  private readonly debrisGpuBuf: DynamicGpuBuffer;
  private readonly dynamicLinesGpuBuf: DynamicGpuBuffer;
  private dynamicLinesCpu = new Float32Array(1024 * 6);

  // Offscreen rgba16float signal & faction-aware ECM jammer field textures
  private fieldW = 0;
  private fieldH = 0;
  private signalTex0: GPUTexture | null = null;
  private signalTex1: GPUTexture | null = null;
  private jamTex0: GPUTexture | null = null;
  private fieldTexBindGroup: GPUBindGroup | null = null;

  private readonly bgStarsPipeline: GPURenderPipeline;
  private readonly signalFieldPipeline: GPURenderPipeline;
  private readonly ecmJammerPipeline: GPURenderPipeline;
  private readonly signalCompositePipeline: GPURenderPipeline;
  private readonly planetCorePipeline: GPURenderPipeline;
  private readonly debrisPipeline: GPURenderPipeline;
  private readonly fleetLinesPipeline: GPURenderPipeline;
  private readonly fleetIconsPipeline: GPURenderPipeline;
  private readonly dynamicLinesPipeline: GPURenderPipeline;

  private lastEmpireColorsKey = '';
  private lastRangesKey = '';
  private lastUploadedVersion = -1;
  private lastPlanetCount = 0;
  private lastFleetCount = 0;
  private lastDebrisCount = 0;

  public static async create(
    canvas: HTMLCanvasElement,
    bgStars: { x: number; y: number; r: number; a: number }[],
    onDeviceLost: () => void,
  ): Promise<WebGpuGalaxyBackend | null> {
    if (typeof navigator === 'undefined' || !navigator.gpu) {
      return null;
    }
    try {
      const adapter = await navigator.gpu.requestAdapter({
        powerPreference: 'high-performance',
      });
      if (!adapter) return null;

      const device = await adapter.requestDevice();
      const context = canvas.getContext('webgpu');
      if (!context) {
        device.destroy();
        return null;
      }

      const format = navigator.gpu.getPreferredCanvasFormat();
      context.configure({
        device,
        format,
        alphaMode: 'premultiplied',
      });

      device.lost.then(() => {
        onDeviceLost();
      });

      return new WebGpuGalaxyBackend(device, context, format, bgStars);
    } catch {
      return null;
    }
  }

  private constructor(
    device: GPUDevice,
    context: GPUCanvasContext,
    format: GPUTextureFormat,
    bgStars: { x: number; y: number; r: number; a: number }[],
  ) {
    this.device = device;
    this.context = context;
    this.format = format;

    this.cameraUniformBuf = device.createBuffer({
      size: 48,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    this.paletteUniformBuf = device.createBuffer({
      size: 17 * 16,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    this.rangesUniformBuf = device.createBuffer({
      size: 17 * 16,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    this.fieldUniformBuf = device.createBuffer({
      size: 16,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    // Index 0 = neutral gray
    this.paletteData[0] = 0.58;
    this.paletteData[1] = 0.643;
    this.paletteData[2] = 0.722;
    this.paletteData[3] = 1.0;

    const bindGroupLayout = device.createBindGroupLayout({
      entries: [
        {
          binding: 0,
          visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT,
          buffer: { type: 'uniform' },
        },
        {
          binding: 1,
          visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT,
          buffer: { type: 'uniform' },
        },
        {
          binding: 2,
          visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT,
          buffer: { type: 'uniform' },
        },
        {
          binding: 3,
          visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT,
          buffer: { type: 'uniform' },
        },
      ],
    });

    this.sharedBindGroup = device.createBindGroup({
      layout: bindGroupLayout,
      entries: [
        { binding: 0, resource: { buffer: this.cameraUniformBuf } },
        { binding: 1, resource: { buffer: this.paletteUniformBuf } },
        { binding: 2, resource: { buffer: this.rangesUniformBuf } },
        { binding: 3, resource: { buffer: this.fieldUniformBuf } },
      ],
    });

    this.fieldSampler = device.createSampler({
      minFilter: 'linear',
      magFilter: 'linear',
      addressModeU: 'clamp-to-edge',
      addressModeV: 'clamp-to-edge',
    });

    this.fieldTexBindGroupLayout = device.createBindGroupLayout({
      entries: [
        {
          binding: 0,
          visibility: GPUShaderStage.FRAGMENT,
          texture: { sampleType: 'float' },
        },
        {
          binding: 1,
          visibility: GPUShaderStage.FRAGMENT,
          texture: { sampleType: 'float' },
        },
        {
          binding: 2,
          visibility: GPUShaderStage.FRAGMENT,
          sampler: { type: 'filtering' },
        },
        {
          binding: 3,
          visibility: GPUShaderStage.FRAGMENT,
          texture: { sampleType: 'float' },
        },
      ],
    });

    const pipelineLayout = device.createPipelineLayout({
      bindGroupLayouts: [bindGroupLayout],
    });
    const compositePipelineLayout = device.createPipelineLayout({
      bindGroupLayouts: [bindGroupLayout, this.fieldTexBindGroupLayout],
    });

    // Static background stars buffer
    this.bgStarCount = bgStars.length;
    const bgArr = new Float32Array(this.bgStarCount * 4);
    for (let i = 0; i < this.bgStarCount; i++) {
      bgArr[i * 4] = bgStars[i].x;
      bgArr[i * 4 + 1] = bgStars[i].y;
      bgArr[i * 4 + 2] = bgStars[i].r;
      bgArr[i * 4 + 3] = bgStars[i].a;
    }
    this.bgStarsBuf = device.createBuffer({
      size: bgArr.byteLength,
      usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST,
    });
    device.queue.writeBuffer(this.bgStarsBuf, 0, bgArr);

    // Dynamic growable GPU buffers
    this.planetsGpuBuf = new DynamicGpuBuffer(device, GPUBufferUsage.VERTEX);
    this.fleetsGpuBuf = new DynamicGpuBuffer(device, GPUBufferUsage.VERTEX);
    this.debrisGpuBuf = new DynamicGpuBuffer(device, GPUBufferUsage.VERTEX);
    this.dynamicLinesGpuBuf = new DynamicGpuBuffer(device, GPUBufferUsage.VERTEX);

    // 1. Background stars pipeline
    const bgMod = device.createShaderModule({ code: BG_STARS_WGSL });
    this.bgStarsPipeline = device.createRenderPipeline({
      layout: pipelineLayout,
      vertex: {
        module: bgMod,
        entryPoint: 'vs_main',
        buffers: [
          {
            arrayStride: 16,
            stepMode: 'instance',
            attributes: [{ shaderLocation: 0, offset: 0, format: 'float32x4' }],
          },
        ],
      },
      fragment: {
        module: bgMod,
        entryPoint: 'fs_main',
        targets: [{ format, blend: ALPHA_BLEND }],
      },
      primitive: { topology: 'triangle-list', cullMode: 'none' },
    });

    // 2. Smooth 2-pass scanner signal + faction-aware ECM jammer field (MRT rgba16float)
    const signalMod = device.createShaderModule({ code: SIGNAL_FIELD_WGSL });
    const signalVertexBuffers: GPUVertexBufferLayout[] = [
      {
        arrayStride: 7 * 4,
        stepMode: 'instance',
        attributes: [
          { shaderLocation: 0, offset: 0, format: 'float32x2' },
          { shaderLocation: 1, offset: 8, format: 'float32x4' },
        ],
      },
    ];
    this.signalFieldPipeline = device.createRenderPipeline({
      layout: pipelineLayout,
      vertex: {
        module: signalMod,
        entryPoint: 'vs_signal',
        buffers: signalVertexBuffers,
      },
      fragment: {
        module: signalMod,
        entryPoint: 'fs_signal',
        targets: [
          { format: 'rgba16float', blend: MAX_BLEND },
          {
            format: 'rgba16float',
            blend: MAX_BLEND,
            writeMask: GPUColorWrite.RED | GPUColorWrite.GREEN,
          },
          { format: 'rgba16float', writeMask: 0 },
        ],
      },
      primitive: { topology: 'triangle-list', cullMode: 'none' },
    });

    this.ecmJammerPipeline = device.createRenderPipeline({
      layout: pipelineLayout,
      vertex: {
        module: signalMod,
        entryPoint: 'vs_jammer',
        buffers: signalVertexBuffers,
      },
      fragment: {
        module: signalMod,
        entryPoint: 'fs_jammer',
        targets: [
          { format: 'rgba16float', writeMask: 0 },
          {
            format: 'rgba16float',
            blend: MAX_BLEND,
            writeMask: GPUColorWrite.BLUE | GPUColorWrite.ALPHA,
          },
          { format: 'rgba16float', blend: MAX_BLEND },
        ],
      },
      primitive: { topology: 'triangle-list', cullMode: 'none' },
    });

    const compositeMod = device.createShaderModule({ code: SIGNAL_COMPOSITE_WGSL });
    this.signalCompositePipeline = device.createRenderPipeline({
      layout: compositePipelineLayout,
      vertex: {
        module: compositeMod,
        entryPoint: 'vs_fullscreen',
      },
      fragment: {
        module: compositeMod,
        entryPoint: 'fs_composite',
        targets: [{ format, blend: ALPHA_BLEND }],
      },
      primitive: { topology: 'triangle-list', cullMode: 'none' },
    });

    // 3. Planet core + Stargate ring pipeline
    const coreMod = device.createShaderModule({ code: PLANET_CORE_WGSL });
    this.planetCorePipeline = device.createRenderPipeline({
      layout: pipelineLayout,
      vertex: {
        module: coreMod,
        entryPoint: 'vs_main',
        buffers: [
          {
            arrayStride: 7 * 4,
            stepMode: 'instance',
            attributes: [
              { shaderLocation: 0, offset: 0, format: 'float32x2' },
              { shaderLocation: 1, offset: 8, format: 'float32x4' },
              { shaderLocation: 2, offset: 24, format: 'float32' },
            ],
          },
        ],
      },
      fragment: {
        module: coreMod,
        entryPoint: 'fs_main',
        targets: [{ format, blend: ALPHA_BLEND }],
      },
      primitive: { topology: 'triangle-list', cullMode: 'none' },
    });

    // 4. Debris fields pipeline
    const debrisMod = device.createShaderModule({ code: DEBRIS_WGSL });
    this.debrisPipeline = device.createRenderPipeline({
      layout: pipelineLayout,
      vertex: {
        module: debrisMod,
        entryPoint: 'vs_main',
        buffers: [
          {
            arrayStride: 3 * 4,
            stepMode: 'instance',
            attributes: [{ shaderLocation: 0, offset: 0, format: 'float32x3' }],
          },
        ],
      },
      fragment: {
        module: debrisMod,
        entryPoint: 'fs_main',
        targets: [{ format, blend: ALPHA_BLEND }],
      },
      primitive: { topology: 'triangle-list', cullMode: 'none' },
    });

    // 5. Fleet route lines pipeline (instanced directly from fleetsBuf!)
    const fleetLinesMod = device.createShaderModule({ code: FLEET_LINES_WGSL });
    this.fleetLinesPipeline = device.createRenderPipeline({
      layout: pipelineLayout,
      vertex: {
        module: fleetLinesMod,
        entryPoint: 'vs_main',
        buffers: [
          {
            arrayStride: 7 * 4,
            stepMode: 'instance',
            attributes: [
              { shaderLocation: 0, offset: 0, format: 'float32x2' },
              { shaderLocation: 1, offset: 8, format: 'float32x2' },
              { shaderLocation: 2, offset: 16, format: 'float32x3' },
            ],
          },
        ],
      },
      fragment: {
        module: fleetLinesMod,
        entryPoint: 'fs_main',
        targets: [{ format, blend: ALPHA_BLEND }],
      },
      primitive: { topology: 'line-list' },
    });

    // 6. Fleet icons pipeline (instanced directly from fleetsBuf!)
    const fleetIconsMod = device.createShaderModule({ code: FLEET_ICONS_WGSL });
    this.fleetIconsPipeline = device.createRenderPipeline({
      layout: pipelineLayout,
      vertex: {
        module: fleetIconsMod,
        entryPoint: 'vs_main',
        buffers: [
          {
            arrayStride: 7 * 4,
            stepMode: 'instance',
            attributes: [
              { shaderLocation: 0, offset: 0, format: 'float32x2' },
              { shaderLocation: 2, offset: 16, format: 'float32x3' },
            ],
          },
        ],
      },
      fragment: {
        module: fleetIconsMod,
        entryPoint: 'fs_main',
        targets: [{ format, blend: ALPHA_BLEND }],
      },
      primitive: { topology: 'triangle-list', cullMode: 'none' },
    });

    // 7. Dynamic lines pipeline (wormholes, jump FX, combat lasers)
    const dynLinesMod = device.createShaderModule({ code: DYNAMIC_LINES_WGSL });
    this.dynamicLinesPipeline = device.createRenderPipeline({
      layout: pipelineLayout,
      vertex: {
        module: dynLinesMod,
        entryPoint: 'vs_main',
        buffers: [
          {
            arrayStride: 6 * 4,
            stepMode: 'vertex',
            attributes: [
              { shaderLocation: 0, offset: 0, format: 'float32x2' },
              { shaderLocation: 1, offset: 8, format: 'float32x4' },
            ],
          },
        ],
      },
      fragment: {
        module: dynLinesMod,
        entryPoint: 'fs_main',
        targets: [{ format, blend: ALPHA_BLEND }],
      },
      primitive: { topology: 'line-list' },
    });
  }

  public reconfigure(): void {
    this.context.configure({
      device: this.device,
      format: this.format,
      alphaMode: 'premultiplied',
    });
  }

  private ensurePalette(empireColors: string[]): void {
    const key = empireColors.join(',');
    if (key === this.lastEmpireColorsKey) return;
    this.lastEmpireColorsKey = key;
    const maxEmp = Math.min(16, empireColors.length);
    for (let i = 0; i < maxEmp; i++) {
      parseHexRgba(empireColors[i], this.paletteData, (i + 1) * 4);
    }
    this.device.queue.writeBuffer(this.paletteUniformBuf, 0, this.paletteData);
  }

  private buildDynamicLines(
    wormholes: WormholeView[],
    jumpFx: JumpFxView[],
    combatFx: CombatFxView[],
  ): number {
    const neededVerts = (wormholes.length + jumpFx.length + combatFx.length) * 2;
    if (neededVerts === 0) return 0;
    const neededFloats = neededVerts * 6;
    if (neededFloats > this.dynamicLinesCpu.length) {
      this.dynamicLinesCpu = new Float32Array(Math.ceil(neededFloats * 1.5));
    }

    const buf = this.dynamicLinesCpu;
    let f = 0;
    const pushLine = (
      x1: number,
      y1: number,
      x2: number,
      y2: number,
      r: number,
      g: number,
      b: number,
      a: number,
    ): void => {
      buf[f++] = x1;
      buf[f++] = y1;
      buf[f++] = r;
      buf[f++] = g;
      buf[f++] = b;
      buf[f++] = a;
      buf[f++] = x2;
      buf[f++] = y2;
      buf[f++] = r;
      buf[f++] = g;
      buf[f++] = b;
      buf[f++] = a;
    };

    for (let i = 0; i < wormholes.length; i++) {
      const wh = wormholes[i];
      pushLine(wh.x1, wh.y1, wh.x2, wh.y2, 0.851, 0.275, 0.937, 0.28);
    }

    const tmp = new Float32Array(4);
    for (let i = 0; i < jumpFx.length; i++) {
      const jx = jumpFx[i];
      parseHexRgba(jx.col, tmp, 0);
      pushLine(jx.x1, jx.y1, jx.x2, jx.y2, tmp[0], tmp[1], tmp[2], 0.9);
    }

    for (let i = 0; i < combatFx.length; i++) {
      const cx = combatFx[i];
      pushLine(cx.x1, cx.y1, cx.x2, cx.y2, 1.0, 1.0, 1.0, 0.95);
    }

    this.dynamicLinesGpuBuf.upload(buf, f * 4);
    return f / 6;
  }

  private ensureRanges(empireRanges: [number, number, number, number][]): void {
    const key = empireRanges
      .map((r) => `${Math.round(r[0])}:${Math.round(r[1])}:${Math.round(r[2])}:${Math.round(r[3] * 100)}`)
      .join(',');
    if (key === this.lastRangesKey) return;
    this.lastRangesKey = key;
    const maxEmp = Math.min(16, empireRanges.length);
    for (let i = 0; i < maxEmp; i++) {
      const off = (i + 1) * 4;
      this.rangesData[off] = empireRanges[i][0];
      this.rangesData[off + 1] = empireRanges[i][1];
      this.rangesData[off + 2] = empireRanges[i][2];
      this.rangesData[off + 3] = empireRanges[i][3];
    }
    this.device.queue.writeBuffer(this.rangesUniformBuf, 0, this.rangesData);
  }

  private ensureFieldTextures(cssW: number, cssH: number): void {
    const maxDim = this.device.limits.maxTextureDimension2D;
    const fw = Math.max(1, Math.min(maxDim, Math.round(cssW)));
    const fh = Math.max(1, Math.min(maxDim, Math.round(cssH)));
    if (fw === this.fieldW && fh === this.fieldH && this.signalTex0 && this.signalTex1 && this.jamTex0) {
      return;
    }

    this.signalTex0?.destroy();
    this.signalTex1?.destroy();
    this.jamTex0?.destroy();

    this.fieldW = fw;
    this.fieldH = fh;
    this.signalTex0 = this.device.createTexture({
      size: [fw, fh],
      format: 'rgba16float',
      usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.TEXTURE_BINDING,
    });
    this.signalTex1 = this.device.createTexture({
      size: [fw, fh],
      format: 'rgba16float',
      usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.TEXTURE_BINDING,
    });
    this.jamTex0 = this.device.createTexture({
      size: [fw, fh],
      format: 'rgba16float',
      usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.TEXTURE_BINDING,
    });
    this.fieldTexBindGroup = this.device.createBindGroup({
      layout: this.fieldTexBindGroupLayout,
      entries: [
        { binding: 0, resource: this.signalTex0.createView() },
        { binding: 1, resource: this.signalTex1.createView() },
        { binding: 2, resource: this.fieldSampler },
        { binding: 3, resource: this.jamTex0.createView() },
      ],
    });
  }

  public render(
    cam: CameraUniformData,
    empireColors: string[],
    empireRanges: [number, number, number, number][],
    planetsBuf: Float32Array,
    fleetsBuf: Float32Array,
    debrisBuf: Float32Array,
    wormholes: WormholeView[],
    jumpFx: JumpFxView[],
    combatFx: CombatFxView[],
    dataVersion: number,
    fieldSettings: ScannerFieldSettings,
  ): void {
    // 1. Upload camera uniform (48 bytes), empire palette, real ranges & scanner field config
    this.cameraData[0] = cam.offX;
    this.cameraData[1] = cam.offY;
    this.cameraData[2] = cam.scale;
    this.cameraData[3] = cam.zoom;
    this.cameraData[4] = cam.viewportW;
    this.cameraData[5] = cam.viewportH;
    this.cameraData[6] = cam.denseFactor;
    this.cameraData[7] = cam.invCrustNorm;
    this.cameraData[8] = cam.planetCount;
    this.device.queue.writeBuffer(this.cameraUniformBuf, 0, this.cameraData);
    this.ensurePalette(empireColors);
    this.ensureRanges(empireRanges);

    const contourCode =
      fieldSettings.contourMode === 'hidden'
        ? 0
        : fieldSettings.contourMode === 'outer'
          ? 1
          : fieldSettings.contourMode === 'subtle'
            ? 2
            : 3;
    this.fieldU32[0] = fieldSettings.empireMask >>> 0;
    this.fieldU32[1] = fieldSettings.equilibriumMode === 'gap' ? 0 : 1;
    this.fieldU32[2] = contourCode;
    this.fieldF32[3] = Math.max(0, Math.min(0.8, fieldSettings.contourOpacity));
    this.device.queue.writeBuffer(this.fieldUniformBuf, 0, this.fieldRawBuf);

    // 2. Direct zero-copy uploads ONLY when simulation data version changed (0 MB upload during paused pan/zoom!)
    let planetCount = this.lastPlanetCount;
    let fleetCount = this.lastFleetCount;
    let debrisCount = this.lastDebrisCount;
    if (dataVersion !== this.lastUploadedVersion) {
      this.lastUploadedVersion = dataVersion;
      planetCount = Math.floor(planetsBuf.length / 7);
      fleetCount = Math.floor(fleetsBuf.length / 7);
      debrisCount = Math.floor(debrisBuf.length / 3);
      this.lastPlanetCount = planetCount;
      this.lastFleetCount = fleetCount;
      this.lastDebrisCount = debrisCount;

      if (planetCount > 0) this.planetsGpuBuf.upload(planetsBuf, planetCount * 7 * 4);
      if (fleetCount > 0) this.fleetsGpuBuf.upload(fleetsBuf, fleetCount * 7 * 4);
      if (debrisCount > 0) this.debrisGpuBuf.upload(debrisBuf, debrisCount * 3 * 4);
    }

    const pBuf = planetCount > 0 ? this.planetsGpuBuf.gpuBuffer : null;
    const fBuf = fleetCount > 0 ? this.fleetsGpuBuf.gpuBuffer : null;
    const dBuf = debrisCount > 0 ? this.debrisGpuBuf.gpuBuffer : null;
    const dynLineVerts = this.buildDynamicLines(wormholes, jumpFx, combatFx);

    // 3. Encode Pass 1 (offscreen signal field MRT) + main render pass (Pass 2 composite + entities)
    const currentTex = this.context.getCurrentTexture();
    this.ensureFieldTextures(cam.viewportW, cam.viewportH);

    const encoder = this.device.createCommandEncoder();

    // Pass 1: Accumulate scanner signal cones + active ECM jammer cones into MRT rgba16float targets
    if (
      pBuf &&
      planetCount > 0 &&
      this.signalTex0 &&
      this.signalTex1 &&
      this.jamTex0 &&
      fieldSettings.empireMask !== 0
    ) {
      const hasAnyJammer = empireRanges.some((r) => r[2] > 0 && r[3] > 0);
      const signalPass = encoder.beginRenderPass({
        colorAttachments: [
          {
            view: this.signalTex0.createView(),
            clearValue: { r: 0, g: 0, b: 0, a: 0 },
            loadOp: 'clear',
            storeOp: 'store',
          },
          {
            view: this.signalTex1.createView(),
            clearValue: { r: 0, g: 0, b: 0, a: 0 },
            loadOp: 'clear',
            storeOp: 'store',
          },
          {
            view: this.jamTex0.createView(),
            clearValue: { r: 0, g: 0, b: 0, a: 0 },
            loadOp: 'clear',
            storeOp: 'store',
          },
        ],
      });
      signalPass.setBindGroup(0, this.sharedBindGroup);
      signalPass.setVertexBuffer(0, pBuf);
      signalPass.setPipeline(this.signalFieldPipeline);
      signalPass.draw(6, planetCount);
      if (hasAnyJammer) {
        signalPass.setPipeline(this.ecmJammerPipeline);
        signalPass.draw(6, planetCount);
      }
      signalPass.end();
    }

    const view = currentTex.createView();
    const pass = encoder.beginRenderPass({
      colorAttachments: [
        {
          view,
          clearValue: { r: 0.02, g: 0.031, b: 0.059, a: 1.0 },
          loadOp: 'clear',
          storeOp: 'store',
        },
      ],
    });

    pass.setBindGroup(0, this.sharedBindGroup);

    // Layer 0: Background stars
    if (this.bgStarCount > 0) {
      pass.setPipeline(this.bgStarsPipeline);
      pass.setVertexBuffer(0, this.bgStarsBuf);
      pass.draw(6, this.bgStarCount);
    }

    // Layer 1 (Pass 2): Smooth signal field composite, equilibrium line & antialiased isolines
    if (pBuf && planetCount > 0 && this.fieldTexBindGroup && fieldSettings.empireMask !== 0) {
      pass.setPipeline(this.signalCompositePipeline);
      pass.setBindGroup(1, this.fieldTexBindGroup);
      pass.draw(3);
    }

    // Layer 2: Dynamic lines (wormhole links, jump FX, combat lasers)
    if (dynLineVerts > 0 && this.dynamicLinesGpuBuf.gpuBuffer) {
      pass.setPipeline(this.dynamicLinesPipeline);
      pass.setVertexBuffer(0, this.dynamicLinesGpuBuf.gpuBuffer);
      pass.draw(dynLineVerts);
    }

    // Layer 3: Debris fields
    if (dBuf && debrisCount > 0) {
      pass.setPipeline(this.debrisPipeline);
      pass.setVertexBuffer(0, dBuf);
      pass.draw(6, debrisCount);
    }

    // Layer 4: Planet cores & Stargate rings
    if (pBuf && planetCount > 0) {
      pass.setPipeline(this.planetCorePipeline);
      pass.setVertexBuffer(0, pBuf);
      pass.draw(6, planetCount);
    }

    // Layer 5 & 6: Fleet route lines & fleet icons
    if (fBuf && fleetCount > 0) {
      pass.setPipeline(this.fleetLinesPipeline);
      pass.setVertexBuffer(0, fBuf);
      pass.draw(2, fleetCount);

      pass.setPipeline(this.fleetIconsPipeline);
      pass.setVertexBuffer(0, fBuf);
      pass.draw(6, fleetCount);
    }

    pass.end();
    this.device.queue.submit([encoder.finish()]);
  }

  public dispose(): void {
    this.planetsGpuBuf.destroy();
    this.fleetsGpuBuf.destroy();
    this.debrisGpuBuf.destroy();
    this.dynamicLinesGpuBuf.destroy();
    this.bgStarsBuf.destroy();
    this.cameraUniformBuf.destroy();
    this.paletteUniformBuf.destroy();
    this.rangesUniformBuf.destroy();
    this.fieldUniformBuf.destroy();
    this.signalTex0?.destroy();
    this.signalTex1?.destroy();
    this.jamTex0?.destroy();
    this.device.destroy();
  }
}
