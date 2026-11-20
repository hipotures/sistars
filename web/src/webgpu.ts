import type { CombatFxView, JumpFxView, WormholeView } from './types';

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
  ranges: array<vec4<f32>, 17>, // .x = scannerRange (ly), .y = flightRange (ly)
};

@group(0) @binding(0) var<uniform> cam: CameraUniforms;
@group(0) @binding(1) var<uniform> palette: EmpirePalette;
@group(0) @binding(2) var<uniform> empRanges: EmpireRanges;

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

const TERRITORY_FIELD_WGSL =
  COMMON_WGSL +
  /* wgsl */ `
struct FieldVertOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) uv: vec2<f32>,
  @location(1) owner: f32,
};

struct FieldFragOut {
  @location(0) color: vec4<f32>,
  @builtin(frag_depth) depth: f32,
};

fn buildFieldQuad(vid: u32, worldPos: vec2<f32>, owner: f32, rangeLy: f32) -> FieldVertOut {
  var out: FieldVertOut;
  let rad = rangeLy * cam.scale;
  if (owner < 0.5 || rad < 1.5) {
    out.pos = vec4<f32>(2.0, 2.0, 1.0, 1.0);
    out.uv = vec2<f32>(0.0);
    out.owner = 0.0;
    return out;
  }
  let center = worldToScreen(worldPos);
  let corner = QUAD[vid];
  out.pos = screenToClip(center + corner * rad);
  out.uv = corner;
  out.owner = owner;
  return out;
}

@vertex
fn vs_scanner(
  @builtin(vertex_index) vid: u32,
  @location(0) worldPos: vec2<f32>,
  @location(1) meta0: vec4<f32>,
) -> FieldVertOut {
  let idx = u32(clamp(round(meta0.x), 0.0, 16.0));
  return buildFieldQuad(vid, worldPos, meta0.x, empRanges.ranges[idx].x);
}

@vertex
fn vs_flight(
  @builtin(vertex_index) vid: u32,
  @location(0) worldPos: vec2<f32>,
  @location(1) meta0: vec4<f32>,
) -> FieldVertOut {
  let idx = u32(clamp(round(meta0.x), 0.0, 16.0));
  return buildFieldQuad(vid, worldPos, meta0.x, empRanges.ranges[idx].y);
}

@fragment
fn fs_field(in: FieldVertOut) -> FieldFragOut {
  let d = length(in.uv);
  if (d >= 1.0) {
    discard;
  }
  var out: FieldFragOut;
  out.color = vec4<f32>(in.owner / 255.0, 1.0 - d, 0.0, 1.0);
  out.depth = d;
  return out;
}
`;

const TERRITORY_BORDER_WGSL =
  COMMON_WGSL +
  /* wgsl */ `
@group(1) @binding(0) var scannerTex: texture_2d<f32>;
@group(1) @binding(1) var flightTex: texture_2d<f32>;

@vertex
fn vs_fullscreen(@builtin(vertex_index) vid: u32) -> @builtin(position) vec4<f32> {
  var pos = array<vec2<f32>, 3>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>( 3.0, -1.0),
    vec2<f32>(-1.0,  3.0),
  );
  return vec4<f32>(pos[vid], 0.0, 1.0);
}

fn loadOwner(tex: texture_2d<f32>, c: vec2<i32>, maxC: vec2<i32>) -> u32 {
  let clamped = clamp(c, vec2<i32>(0, 0), maxC);
  return u32(round(textureLoad(tex, clamped, 0).r * 255.0));
}

fn isEdge(tex: texture_2d<f32>, c: vec2<i32>, maxC: vec2<i32>, owner: u32) -> bool {
  return loadOwner(tex, c + vec2<i32>(1, 0), maxC) != owner
      || loadOwner(tex, c + vec2<i32>(-1, 0), maxC) != owner
      || loadOwner(tex, c + vec2<i32>(0, 1), maxC) != owner
      || loadOwner(tex, c + vec2<i32>(0, -1), maxC) != owner;
}

@fragment
fn fs_border(@builtin(position) fragPos: vec4<f32>) -> @location(0) vec4<f32> {
  let dims = vec2<i32>(textureDimensions(scannerTex));
  let maxC = dims - vec2<i32>(1, 1);
  let c = clamp(vec2<i32>(fragPos.xy * 0.5), vec2<i32>(0, 0), maxC);

  var outRgb = vec3<f32>(0.0);
  var outA = 0.0;

  // 1. Real Scanner Zone: subtle translucent interior + crisp solid border
  let sOwner = loadOwner(scannerTex, c, maxC);
  if (sOwner > 0u) {
    let col = empireColor(f32(sOwner));
    if (isEdge(scannerTex, c, maxC, sOwner)) {
      outRgb = col;
      outA = 0.52;
    } else {
      outRgb = col;
      outA = 0.055;
    }
  }

  // 2. Real Flight / Fuel Range: clean dashed contour (overlaid cleanly)
  let fOwner = loadOwner(flightTex, c, maxC);
  if (fOwner > 0u && isEdge(flightTex, c, maxC, fOwner)) {
    let dash = ((u32(fragPos.x) + u32(fragPos.y)) / 5u) & 1u;
    if (dash == 1u) {
      let fCol = empireColor(f32(fOwner));
      let fA = 0.42;
      let combA = fA + outA * (1.0 - fA);
      outRgb = (fCol * fA + outRgb * outA * (1.0 - fA)) / max(0.001, combA);
      outA = combA;
    }
  }

  if (outA < 0.005) {
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
  @location(4) hasGate: f32,
};

@vertex
fn vs_main(
  @builtin(vertex_index) vid: u32,
  @location(0) worldPos: vec2<f32>,
  @location(1) meta0: vec4<f32>, // [owner, factories, hab, hasGate]
  @location(2) crustSum: f32,
) -> PlanetOut {
  let owner = meta0.x;
  let factories = meta0.y;
  let hab = meta0.z;
  let hasGate = select(0.0, 1.0, owner > 0.5 && meta0.w > 0.5);

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

  let quadRad = select(r + 1.4, r + 5.6, hasGate > 0.5);
  let corner = QUAD[vid];
  let center = worldToScreen(worldPos);
  let localPx = corner * quadRad;

  var out: PlanetOut;
  out.pos = screenToClip(center + localPx);
  out.localPx = localPx;
  out.coreRad = r;
  out.color = col;
  out.alphaMul = aMul;
  out.hasGate = hasGate;
  return out;
}

@fragment
fn fs_main(in: PlanetOut) -> @location(0) vec4<f32> {
  let dist = length(in.localPx);
  let coreAlpha = (1.0 - smoothstep(in.coreRad - 0.75, in.coreRad + 0.75, dist)) * in.alphaMul;

  var rgb = in.color;
  var alpha = coreAlpha;

  if (in.hasGate > 0.5) {
    let ringDist = abs(dist - (in.coreRad + 3.5));
    let ringAlpha = 1.0 - smoothstep(0.55, 1.35, ringDist);
    if (ringAlpha > 0.01) {
      let gateCol = vec3<f32>(0.133, 0.827, 0.933);
      let outA = ringAlpha + alpha * (1.0 - ringAlpha);
      rgb = (gateCol * ringAlpha + rgb * alpha * (1.0 - ringAlpha)) / max(0.001, outA);
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
};

@vertex
fn vs_main(
  @builtin(vertex_index) vid: u32,
  @location(0) worldPos: vec2<f32>,
) -> DebrisOut {
  let corner = QUAD[vid];
  let localPx = corner * 6.5;
  let center = worldToScreen(worldPos);
  var out: DebrisOut;
  out.pos = screenToClip(center + localPx);
  out.localPx = localPx;
  return out;
}

@fragment
fn fs_main(in: DebrisOut) -> @location(0) vec4<f32> {
  let d = length(in.localPx);
  let ring = 1.0 - smoothstep(0.5, 1.25, abs(d - 5.0));
  if (ring < 0.02) {
    discard;
  }
  let ang = atan2(in.localPx.y, in.localPx.x);
  let dash = step(0.0, sin(ang * 8.0));
  if (dash < 0.5) {
    discard;
  }
  return vec4<f32>(0.753, 0.518, 0.988, ring * 0.78);
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

export class WebGpuGalaxyBackend {
  private readonly device: GPUDevice;
  private readonly context: GPUCanvasContext;
  private readonly format: GPUTextureFormat;

  private readonly cameraUniformBuf: GPUBuffer;
  private readonly paletteUniformBuf: GPUBuffer;
  private readonly rangesUniformBuf: GPUBuffer;
  private readonly cameraData = new Float32Array(12);
  private readonly paletteData = new Float32Array(17 * 4);
  private readonly rangesData = new Float32Array(17 * 4);
  private readonly sharedBindGroup: GPUBindGroup;
  private readonly fieldTexBindGroupLayout: GPUBindGroupLayout;

  private readonly bgStarsBuf: GPUBuffer;
  private readonly bgStarCount: number;

  private readonly planetsGpuBuf: DynamicGpuBuffer;
  private readonly fleetsGpuBuf: DynamicGpuBuffer;
  private readonly debrisGpuBuf: DynamicGpuBuffer;
  private readonly dynamicLinesGpuBuf: DynamicGpuBuffer;
  private dynamicLinesCpu = new Float32Array(1024 * 6);

  // Half-res offscreen textures for Voronoi-merged scanner & flight range contours
  private fieldW = 0;
  private fieldH = 0;
  private scannerFieldTex: GPUTexture | null = null;
  private flightFieldTex: GPUTexture | null = null;
  private fieldDepthTex: GPUTexture | null = null;
  private fieldTexBindGroup: GPUBindGroup | null = null;

  private readonly bgStarsPipeline: GPURenderPipeline;
  private readonly scannerFieldPipeline: GPURenderPipeline;
  private readonly flightFieldPipeline: GPURenderPipeline;
  private readonly territoryBorderPipeline: GPURenderPipeline;
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
      ],
    });

    this.sharedBindGroup = device.createBindGroup({
      layout: bindGroupLayout,
      entries: [
        { binding: 0, resource: { buffer: this.cameraUniformBuf } },
        { binding: 1, resource: { buffer: this.paletteUniformBuf } },
        { binding: 2, resource: { buffer: this.rangesUniformBuf } },
      ],
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
      ],
    });

    const pipelineLayout = device.createPipelineLayout({
      bindGroupLayouts: [bindGroupLayout],
    });
    const borderPipelineLayout = device.createPipelineLayout({
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

    // 2. Voronoi-merged territory field pipelines (real scanner range + real flight range)
    const fieldMod = device.createShaderModule({ code: TERRITORY_FIELD_WGSL });
    const fieldVertexBuffers: GPUVertexBufferLayout[] = [
      {
        arrayStride: 7 * 4,
        stepMode: 'instance',
        attributes: [
          { shaderLocation: 0, offset: 0, format: 'float32x2' },
          { shaderLocation: 1, offset: 8, format: 'float32x4' },
        ],
      },
    ];
    const fieldDepthStencil: GPUDepthStencilState = {
      format: 'depth16unorm',
      depthWriteEnabled: true,
      depthCompare: 'less',
    };

    this.scannerFieldPipeline = device.createRenderPipeline({
      layout: pipelineLayout,
      vertex: {
        module: fieldMod,
        entryPoint: 'vs_scanner',
        buffers: fieldVertexBuffers,
      },
      fragment: {
        module: fieldMod,
        entryPoint: 'fs_field',
        targets: [{ format: 'rgba8unorm' }],
      },
      depthStencil: fieldDepthStencil,
      primitive: { topology: 'triangle-list', cullMode: 'none' },
    });

    this.flightFieldPipeline = device.createRenderPipeline({
      layout: pipelineLayout,
      vertex: {
        module: fieldMod,
        entryPoint: 'vs_flight',
        buffers: fieldVertexBuffers,
      },
      fragment: {
        module: fieldMod,
        entryPoint: 'fs_field',
        targets: [{ format: 'rgba8unorm' }],
      },
      depthStencil: fieldDepthStencil,
      primitive: { topology: 'triangle-list', cullMode: 'none' },
    });

    const borderMod = device.createShaderModule({ code: TERRITORY_BORDER_WGSL });
    this.territoryBorderPipeline = device.createRenderPipeline({
      layout: borderPipelineLayout,
      vertex: {
        module: borderMod,
        entryPoint: 'vs_fullscreen',
      },
      fragment: {
        module: borderMod,
        entryPoint: 'fs_border',
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
            arrayStride: 2 * 4,
            stepMode: 'instance',
            attributes: [{ shaderLocation: 0, offset: 0, format: 'float32x2' }],
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

  private ensureRanges(empireRanges: [number, number][]): void {
    const key = empireRanges.map((r) => `${Math.round(r[0])}:${Math.round(r[1])}`).join(',');
    if (key === this.lastRangesKey) return;
    this.lastRangesKey = key;
    const maxEmp = Math.min(16, empireRanges.length);
    for (let i = 0; i < maxEmp; i++) {
      const off = (i + 1) * 4;
      this.rangesData[off] = empireRanges[i][0];
      this.rangesData[off + 1] = empireRanges[i][1];
      this.rangesData[off + 2] = 0;
      this.rangesData[off + 3] = 0;
    }
    this.device.queue.writeBuffer(this.rangesUniformBuf, 0, this.rangesData);
  }

  private ensureFieldTextures(targetW: number, targetH: number): void {
    const fw = Math.max(1, Math.floor(targetW / 2));
    const fh = Math.max(1, Math.floor(targetH / 2));
    if (fw === this.fieldW && fh === this.fieldH && this.scannerFieldTex) return;

    this.scannerFieldTex?.destroy();
    this.flightFieldTex?.destroy();
    this.fieldDepthTex?.destroy();

    this.fieldW = fw;
    this.fieldH = fh;
    this.scannerFieldTex = this.device.createTexture({
      size: [fw, fh],
      format: 'rgba8unorm',
      usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.TEXTURE_BINDING,
    });
    this.flightFieldTex = this.device.createTexture({
      size: [fw, fh],
      format: 'rgba8unorm',
      usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.TEXTURE_BINDING,
    });
    this.fieldDepthTex = this.device.createTexture({
      size: [fw, fh],
      format: 'depth16unorm',
      usage: GPUTextureUsage.RENDER_ATTACHMENT,
    });
    this.fieldTexBindGroup = this.device.createBindGroup({
      layout: this.fieldTexBindGroupLayout,
      entries: [
        { binding: 0, resource: this.scannerFieldTex.createView() },
        { binding: 1, resource: this.flightFieldTex.createView() },
      ],
    });
  }

  public render(
    cam: CameraUniformData,
    empireColors: string[],
    empireRanges: [number, number][],
    planetsBuf: Float32Array,
    fleetsBuf: Float32Array,
    debrisBuf: Float32Array,
    wormholes: WormholeView[],
    jumpFx: JumpFxView[],
    combatFx: CombatFxView[],
    dataVersion: number,
  ): void {
    // 1. Upload camera uniform (48 bytes), empire palette & real scanner/flight ranges
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

    // 2. Direct zero-copy uploads ONLY when simulation data version changed (0 MB upload during paused pan/zoom!)
    let planetCount = this.lastPlanetCount;
    let fleetCount = this.lastFleetCount;
    let debrisCount = this.lastDebrisCount;
    if (dataVersion !== this.lastUploadedVersion) {
      this.lastUploadedVersion = dataVersion;
      planetCount = Math.floor(planetsBuf.length / 7);
      fleetCount = Math.floor(fleetsBuf.length / 7);
      debrisCount = Math.floor(debrisBuf.length / 2);
      this.lastPlanetCount = planetCount;
      this.lastFleetCount = fleetCount;
      this.lastDebrisCount = debrisCount;

      if (planetCount > 0) this.planetsGpuBuf.upload(planetsBuf, planetCount * 7 * 4);
      if (fleetCount > 0) this.fleetsGpuBuf.upload(fleetsBuf, fleetCount * 7 * 4);
      if (debrisCount > 0) this.debrisGpuBuf.upload(debrisBuf, debrisCount * 2 * 4);
    }

    const pBuf = planetCount > 0 ? this.planetsGpuBuf.gpuBuffer : null;
    const fBuf = fleetCount > 0 ? this.fleetsGpuBuf.gpuBuffer : null;
    const dBuf = debrisCount > 0 ? this.debrisGpuBuf.gpuBuffer : null;
    const dynLineVerts = this.buildDynamicLines(wormholes, jumpFx, combatFx);

    // 3. Encode offscreen Voronoi territory passes + main render pass
    const currentTex = this.context.getCurrentTexture();
    this.ensureFieldTextures(currentTex.width, currentTex.height);

    const encoder = this.device.createCommandEncoder();

    // Offscreen Pass A & B: real scanner zone & flight range fields (closest-planet Voronoi via depth16)
    if (pBuf && planetCount > 0 && this.scannerFieldTex && this.flightFieldTex && this.fieldDepthTex) {
      const depthView = this.fieldDepthTex.createView();

      const scanPass = encoder.beginRenderPass({
        colorAttachments: [
          {
            view: this.scannerFieldTex.createView(),
            clearValue: { r: 0, g: 0, b: 0, a: 0 },
            loadOp: 'clear',
            storeOp: 'store',
          },
        ],
        depthStencilAttachment: {
          view: depthView,
          depthClearValue: 1.0,
          depthLoadOp: 'clear',
          depthStoreOp: 'discard',
        },
      });
      scanPass.setBindGroup(0, this.sharedBindGroup);
      scanPass.setPipeline(this.scannerFieldPipeline);
      scanPass.setVertexBuffer(0, pBuf);
      scanPass.draw(6, planetCount);
      scanPass.end();

      const flightPass = encoder.beginRenderPass({
        colorAttachments: [
          {
            view: this.flightFieldTex.createView(),
            clearValue: { r: 0, g: 0, b: 0, a: 0 },
            loadOp: 'clear',
            storeOp: 'store',
          },
        ],
        depthStencilAttachment: {
          view: depthView,
          depthClearValue: 1.0,
          depthLoadOp: 'clear',
          depthStoreOp: 'discard',
        },
      });
      flightPass.setBindGroup(0, this.sharedBindGroup);
      flightPass.setPipeline(this.flightFieldPipeline);
      flightPass.setVertexBuffer(0, pBuf);
      flightPass.draw(6, planetCount);
      flightPass.end();
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

    // Layer 1: Merged Scanner Zone (solid) + Flight Range (dashed) territory contours
    if (pBuf && planetCount > 0 && this.fieldTexBindGroup) {
      pass.setPipeline(this.territoryBorderPipeline);
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
    this.scannerFieldTex?.destroy();
    this.flightFieldTex?.destroy();
    this.fieldDepthTex?.destroy();
    this.device.destroy();
  }
}
