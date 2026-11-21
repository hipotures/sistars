import type { BodyType, MoonView, StarKind, StarView, SystemBodyView } from './types';

// Procedural GPU Planet & Star Renderer (WebGL2 ray-sphere + 3D Simplex fBm)
// Zero static PNG assets: generates planets, rings, stars, and 2048x1024 equirectangular
// PNG exports directly from deterministic seeds and live SI-STARS simulation state.

const FULLSCREEN_VS = `#version 300 es
precision highp float;
const vec2 POS[3] = vec2[3](
  vec2(-1.0, -1.0),
  vec2( 3.0, -1.0),
  vec2(-1.0,  3.0)
);
out vec2 vUv;
void main() {
  vec2 p = POS[gl_VertexID];
  gl_Position = vec4(p, 0.0, 1.0);
  vUv = p * 0.5 + 0.5;
}
`;

const NOISE_GLSL = `
vec3 mod289(vec3 x) { return x - floor(x * (1.0 / 289.0)) * 289.0; }
vec4 mod289(vec4 x) { return x - floor(x * (1.0 / 289.0)) * 289.0; }
vec4 permute(vec4 x) { return mod289(((x * 34.0) + 1.0) * x); }
vec4 taylorInvSqrt(vec4 r) { return 1.79284291400159 - 0.85373472095314 * r; }

float snoise(vec3 v) {
  const vec2 C = vec2(1.0 / 6.0, 1.0 / 3.0);
  const vec4 D = vec4(0.0, 0.5, 1.0, 2.0);
  vec3 i = floor(v + dot(v, C.yyy));
  vec3 x0 = v - i + dot(i, C.xxx);
  vec3 g = step(x0.yzx, x0.xyz);
  vec3 l = 1.0 - g;
  vec3 i1 = min(g.xyz, l.zxy);
  vec3 i2 = max(g.xyz, l.zxy);
  vec3 x1 = x0 - i1 + C.xxx;
  vec3 x2 = x0 - i2 + C.yyy;
  vec3 x3 = x0 - D.yyy;
  i = mod289(i);
  vec4 p = permute(permute(permute(
    i.z + vec4(0.0, i1.z, i2.z, 1.0))
    + i.y + vec4(0.0, i1.y, i2.y, 1.0))
    + i.x + vec4(0.0, i1.x, i2.x, 1.0));
  float n_ = 0.142857142857;
  vec3 ns = n_ * D.wyz - D.xzx;
  vec4 j = p - 49.0 * floor(p * ns.z * ns.z);
  vec4 x_ = floor(j * ns.z);
  vec4 y_ = floor(j - 7.0 * x_);
  vec4 x = x_ * ns.x + ns.yyyy;
  vec4 y = y_ * ns.x + ns.yyyy;
  vec4 h = 1.0 - abs(x) - abs(y);
  vec4 b0 = vec4(x.xy, y.xy);
  vec4 b1 = vec4(x.zw, y.zw);
  vec4 s0 = floor(b0) * 2.0 + 1.0;
  vec4 s1 = floor(b1) * 2.0 + 1.0;
  vec4 sh = -step(h, vec4(0.0));
  vec4 a0 = b0.xzyw + s0.xzyw * sh.xxyy;
  vec4 a1 = b1.xzyw + s1.xzyw * sh.zzww;
  vec3 p0 = vec3(a0.xy, h.x);
  vec3 p1 = vec3(a0.zw, h.y);
  vec3 p2 = vec3(a1.xy, h.z);
  vec3 p3 = vec3(a1.zw, h.w);
  vec4 norm = taylorInvSqrt(vec4(dot(p0, p0), dot(p1, p1), dot(p2, p2), dot(p3, p3)));
  p0 *= norm.x; p1 *= norm.y; p2 *= norm.z; p3 *= norm.w;
  vec4 m = max(0.6 - vec4(dot(x0, x0), dot(x1, x1), dot(x2, x2), dot(x3, x3)), 0.0);
  m = m * m;
  return 42.0 * dot(m * m, vec4(dot(p0, x0), dot(p1, x1), dot(p2, x2), dot(p3, x3)));
}

float fbm(vec3 p) {
  float v = 0.0;
  float a = 0.5;
  for (int i = 0; i < 5; i++) {
    v += a * snoise(p);
    p = p * 2.04 + vec3(13.7, -9.2, 5.4);
    a *= 0.5;
  }
  return v;
}

vec3 hash33(vec3 p) {
  p = fract(p * vec3(0.1031, 0.1030, 0.0973));
  p += dot(p, p.yxz + 33.33);
  return fract((p.xxy + p.yxx) * p.zyx);
}

// 3D Worley / Voronoi impact craters: returns (darkFloor, brightRim)
vec2 worleyCraterLayer(vec3 p) {
  vec3 i = floor(p);
  vec3 f = fract(p);
  float minD = 1.0;
  for (int z = -1; z <= 1; z++) {
    for (int y = -1; y <= 1; y++) {
      for (int x = -1; x <= 1; x++) {
        vec3 nb = vec3(float(x), float(y), float(z));
        vec3 rnd = hash33(i + nb);
        vec3 diff = nb + rnd - f;
        minD = min(minD, dot(diff, diff));
      }
    }
  }
  float d = sqrt(minD);
  float floorDark = smoothstep(0.34, 0.12, d);
  float rimBright = smoothstep(0.46, 0.35, d) * smoothstep(0.24, 0.35, d);
  return vec2(floorDark, rimBright);
}

vec2 worleyCraters(vec3 p) {
  vec2 c1 = worleyCraterLayer(p * 4.8);
  vec2 c2 = worleyCraterLayer(p * 10.5 + vec3(7.1, -3.4, 5.2));
  return vec2(
    clamp(c1.x * 0.72 + c2.x * 0.48, 0.0, 1.0),
    clamp(c1.y * 0.80 + c2.y * 0.55, 0.0, 1.0)
  );
}
`;

const SURFACE_EVAL_GLSL = `
struct SurfaceSample {
  vec3 albedo;
  float spec;
  float emissive;
  vec3 emissiveCol;
  float cloudMask;
};

SurfaceSample evalPlanetSurface(
  vec3 n,
  vec3 seedOff,
  int bodyType,
  float tempK,
  float waterPct,
  float icePct,
  float atmoDensity,
  vec3 atmoColor,
  float cloudPct,
  float popNorm,
  vec3 mineralTint,
  float cloudRot
) {
  SurfaceSample s;
  s.spec = 0.06;
  s.emissive = 0.0;
  s.emissiveCol = vec3(0.0);
  s.cloudMask = 0.0;

  if (bodyType == 6 || bodyType == 7) {
    // Gas Giant (6) or Ice Giant (7): domain-warped latitude bands & vortices
    float warp = fbm(n * 2.4 + seedOff) * 0.45;
    float lat = n.y * 7.5 + warp * 3.2;
    float bands = sin(lat * 2.2) * 0.5 + 0.5;
    float fine = fbm(vec3(n.x * 3.5, n.y * 12.0 + warp * 2.0, n.z * 3.5) + seedOff * 1.3) * 0.5 + 0.5;
    vec3 cA = bodyType == 6 ? mix(vec3(0.76, 0.58, 0.38), atmoColor, 0.55) : mix(vec3(0.24, 0.56, 0.86), atmoColor, 0.6);
    vec3 cB = bodyType == 6 ? vec3(0.94, 0.86, 0.72) : vec3(0.72, 0.90, 0.98);
    vec3 cC = bodyType == 6 ? vec3(0.55, 0.34, 0.22) : vec3(0.14, 0.34, 0.62);
    s.albedo = mix(mix(cC, cA, bands), cB, smoothstep(0.35, 0.82, fine) * 0.65);
    s.spec = 0.12;
    return s;
  }

  // Rocky / Terrestrial / Oceanic / Desert / Ice / Volcanic
  float h = fbm(n * 2.55 + seedOff) * 0.5 + 0.5;
  float detail = fbm(n * 7.2 - seedOff * 0.7) * 0.12;
  float elev = clamp(h + detail, 0.0, 1.0);

  // Subtle mineral crust tinting (Iron=blue-rust, Boranium=emerald, Germanium=amber)
  vec3 minTint = vec3(0.72, 0.48, 0.38) * mineralTint.r
               + vec3(0.34, 0.64, 0.48) * mineralTint.g
               + vec3(0.78, 0.66, 0.36) * mineralTint.b;
  if (length(mineralTint) < 0.01) {
    minTint = vec3(0.58, 0.54, 0.48);
  }

  // Base terrain palette by biome
  vec3 landLow = mix(vec3(0.24, 0.46, 0.24), minTint, 0.35);
  vec3 landMid = mix(vec3(0.52, 0.46, 0.34), minTint, 0.45);
  vec3 landHigh = vec3(0.66, 0.64, 0.62);

  if (bodyType == 2) { // Desert
    landLow = mix(vec3(0.74, 0.52, 0.30), minTint, 0.35);
    landMid = vec3(0.84, 0.65, 0.40);
    landHigh = vec3(0.58, 0.40, 0.26);
  } else if (bodyType == 3) { // Ice
    landLow = vec3(0.58, 0.74, 0.86);
    landMid = vec3(0.78, 0.88, 0.96);
    landHigh = vec3(0.92, 0.96, 1.0);
  } else if (bodyType == 4) { // Volcanic
    landLow = vec3(0.18, 0.14, 0.13);
    landMid = vec3(0.30, 0.22, 0.19);
    landHigh = vec3(0.42, 0.34, 0.30);
  } else if (bodyType == 5) { // Rock
    landLow = mix(vec3(0.34, 0.34, 0.37), minTint, 0.30);
    landMid = vec3(0.50, 0.49, 0.48);
    landHigh = vec3(0.68, 0.67, 0.65);
  } else if (bodyType == 8) { // Dwarf / Plutoid (high-contrast bright ice plains + dark tholins)
    landLow = vec3(0.32, 0.21, 0.19);
    landMid = vec3(0.66, 0.56, 0.48);
    landHigh = vec3(0.90, 0.92, 0.96);
  }

  vec3 col = mix(landLow, landMid, smoothstep(0.38, 0.66, elev));
  col = mix(col, landHigh, smoothstep(0.68, 0.90, elev));

  // Crisp 3D Worley impact craters (dark bowls + bright rims) on airless bodies
  if (atmoDensity < 0.08) {
    vec2 cr = worleyCraters(n + seedOff * 0.25);
    col = col * (1.0 - cr.x * 0.34) + vec3(0.16, 0.16, 0.17) * cr.y;
  }

  // Ocean level & specular
  float seaLevel = waterPct * 0.72;
  bool isWater = waterPct > 0.02 && elev < seaLevel;
  if (isWater) {
    float depth = clamp((seaLevel - elev) / max(0.05, seaLevel), 0.0, 1.0);
    vec3 shallow = vec3(0.10, 0.42, 0.64);
    vec3 deep = vec3(0.03, 0.13, 0.30);
    col = mix(shallow, deep, smoothstep(0.0, 0.75, depth));
    s.spec = 0.68;
  }

  // Polar ice caps
  float pole = abs(n.y) + (elev - 0.5) * 0.18;
  float iceThresh = 1.02 - icePct * 0.78;
  if (icePct > 0.02 && pole > iceThresh) {
    float iceMix = smoothstep(iceThresh, iceThresh + 0.08, pole);
    col = mix(col, vec3(0.88, 0.94, 1.0), iceMix);
    s.spec = mix(s.spec, 0.28, iceMix);
  }

  // Volcanic lava fissures for hot worlds (> 610 K)
  if (tempK > 610.0) {
    float cracks = abs(snoise(n * 8.5 + seedOff * 1.7));
    float lava = smoothstep(0.09, 0.01, cracks) * clamp((tempK - 600.0) / 180.0, 0.25, 1.0);
    s.emissive = max(s.emissive, lava * 1.4);
    s.emissiveCol = mix(s.emissiveCol, vec3(1.0, 0.36, 0.06), lava);
  }

  // Night-side city lights on inhabited land
  if (popNorm > 0.01 && !isWater && pole < iceThresh + 0.02) {
    float cluster = snoise(n * 9.0 + seedOff * 2.1);
    float fineCity = snoise(n * 34.0 - seedOff * 1.4);
    float city = smoothstep(0.32 - popNorm * 0.25, 0.62, cluster)
               * smoothstep(0.28, 0.72, fineCity)
               * popNorm;
    if (city > 0.01) {
      s.emissive = max(s.emissive, city * 1.65);
      s.emissiveCol = mix(vec3(1.0, 0.80, 0.38), vec3(0.45, 0.88, 1.0), smoothstep(0.45, 0.85, fineCity));
    }
  }

  // Independent rotating cloud layer
  if (cloudPct > 0.02) {
    float cs = cos(cloudRot);
    float sn = sin(cloudRot);
    vec3 cn = vec3(n.x * cs - n.z * sn, n.y, n.x * sn + n.z * cs);
    float cNoise = fbm(cn * 3.1 + seedOff * 2.3 + vec3(3.1, 1.7, -2.4)) * 0.5 + 0.5;
    s.cloudMask = smoothstep(1.0 - cloudPct * 0.92, 1.0 - cloudPct * 0.35, cNoise);
  }

  s.albedo = col;
  return s;
}
`;

const SPHERE_FS = `#version 300 es
precision highp float;
in vec2 vUv;
out vec4 outColor;

uniform vec2 uRes;
uniform float uTime;
uniform int uIsStar;
uniform int uStarKind; // 0=MainSeq, 1=BrownDwarf, 2=WhiteDwarf, 3=Pulsar, 4=BlackHole
uniform int uBodyType;
uniform float uSeed;
uniform float uTempK;
uniform float uWaterPct;
uniform float uIcePct;
uniform float uAtmoDensity;
uniform vec3 uAtmoColor;
uniform float uCloudPct;
uniform float uPopNorm;
uniform vec3 uMineralTint;
uniform vec3 uStarColor;
uniform int uHasStar2;
uniform vec3 uStar2Color;
uniform int uHasRings;
uniform float uRingInner;
uniform float uRingOuter;
uniform vec3 uRingColor;
uniform int uMoonCount;
uniform vec4 uMoonOrbit[4]; // (orbitRad, phase, speed, moonRadius)
uniform vec3 uMoonColor[4];

${NOISE_GLSL}
${SURFACE_EVAL_GLSL}

void main() {
  vec2 p = (vUv * 2.0 - 1.0) * vec2(uRes.x / max(1.0, uRes.y), 1.0);
  vec3 seedOff = vec3(
    fract(uSeed * 0.00137) * 41.0,
    fract(uSeed * 0.00319) * 37.0,
    fract(uSeed * 0.00731) * 29.0
  );

  // Subtle dark space backdrop with faint starlight vignette
  vec3 bg = vec3(0.022, 0.032, 0.058);

  if (uIsStar == 1) {
    float r = length(p);

    if (uStarKind == 4) {
      // Black Hole: pitch-black event horizon, blazing photon ring & gravitationally lensed accretion disk
      float rs = 0.34;
      vec3 col = bg;

      // Tilted horizontal accretion disk (swirling relativistic plasma)
      vec2 dp = vec2(p.x * 0.72, p.y * 3.8 + p.x * 0.32);
      float dr = length(dp);
      float dang = atan(dp.y, dp.x);
      float diskMask = smoothstep(0.28, 0.38, dr) * (1.0 - smoothstep(0.68, 1.05, dr));
      float swirl = fbm(vec3(cos(dang - uTime * 0.85) * dr * 5.0, sin(dang - uTime * 0.85) * dr * 5.0, uSeed * 0.01)) * 0.5 + 0.5;
      // Relativistic Doppler beaming (left side brighter)
      float doppler = 0.72 + 0.45 * smoothstep(0.5, -0.5, p.x);
      vec3 diskCol = mix(vec3(0.95, 0.36, 0.08), vec3(1.0, 0.88, 0.58), smoothstep(0.25, 0.85, swirl)) * doppler;

      // Gravitationally lensed upper & lower Einstein arches over the shadow
      float lensR = abs(r - (rs * 1.18 + 0.06 * abs(p.y)));
      float lensArch = exp(-lensR * 22.0) * smoothstep(0.08, 0.38, abs(p.y)) * (0.55 + 0.45 * swirl);
      col += diskCol * lensArch * 0.88;

      // Background disk half (behind shadow)
      if (p.y > -0.02) {
        col += diskCol * diskMask * (0.45 + 0.65 * swirl);
      }

      // Razor-sharp photon ring around event horizon
      float photonRing = exp(-abs(r - rs * 1.04) * 58.0) * 1.65;
      float outerHalo = exp(-max(0.0, r - rs) * 7.5) * 0.42;
      col += vec3(1.0, 0.86, 0.56) * photonRing + vec3(1.0, 0.54, 0.18) * outerHalo;

      // Pure black event horizon core
      float horizon = smoothstep(rs + 0.014, rs - 0.012, r);
      col = mix(col, vec3(0.0), horizon);

      // Foreground disk half crossing in front of the black hole shadow
      if (p.y <= 0.06) {
        float frontFade = smoothstep(0.06, -0.03, p.y);
        col += diskCol * diskMask * (0.65 + 0.75 * swirl) * frontFade;
      }

      outColor = vec4(clamp(col, 0.0, 1.0), 1.0);
      return;
    }

    if (uStarKind == 3) {
      // Pulsar / Neutron Star: compact blazing core + dual sweeping relativistic jet cones
      float rad = 0.22;
      vec3 col = bg;
      float spin = uTime * 3.8;
      float sweep = sin(spin) * 0.28;
      vec2 jetAxis = normalize(vec2(0.46 + sweep, 0.88));
      vec2 jetPerp = vec2(-jetAxis.y, jetAxis.x);
      float along = dot(p, jetAxis);
      float across = abs(dot(p, jetPerp));
      float distA = abs(along);
      float coneW = 0.035 + distA * 0.22;
      float beamNoise = 0.78 + 0.22 * snoise(vec3(along * 8.0 - sign(along) * uTime * 6.0, across * 16.0, uSeed * 0.01));
      float jet = exp(-(across * across) / (coneW * coneW)) * exp(-distA * 1.35) * smoothstep(0.10, 0.24, distA);
      float pulse = 0.75 + 0.25 * cos(spin);
      col += vec3(0.42, 0.84, 1.0) * jet * beamNoise * pulse * 1.35;

      // Subtle dipole magnetic field rings
      float magR = length(vec2(abs(dot(p, jetPerp)) - 0.34, along * 0.85));
      float magArc = exp(-abs(magR - 0.26) * 28.0) * 0.22 * smoothstep(0.16, 0.35, r);
      col += vec3(0.55, 0.78, 1.0) * magArc;

      // Intense corona + neutron core
      col += vec3(0.52, 0.86, 1.0) * exp(-max(0.0, r - rad) * 7.0) * 0.95;
      if (r <= rad) {
        vec2 sp = p / rad;
        float z = sqrt(max(0.0, 1.0 - dot(sp, sp)));
        vec3 core = mix(vec3(0.58, 0.88, 1.0), vec3(1.0, 1.0, 1.0), pow(z, 0.55));
        col = core * (0.7 + 0.5 * z);
      }
      outColor = vec4(clamp(col, 0.0, 1.0), 1.0);
      return;
    }

    if (uStarKind == 2) {
      // White Dwarf: compact dazzling blue-white sphere + sharp corona + subtle diffraction spikes
      float rad = 0.38;
      float corona = exp(-max(0.0, r - rad) * 7.8) * 1.05;
      float spikeX = exp(-abs(p.y) * 34.0) * exp(-abs(p.x) * 2.8) * 0.55;
      float spikeY = exp(-abs(p.x) * 34.0) * exp(-abs(p.y) * 2.8) * 0.55;
      vec3 col = bg + uStarColor * (corona + spikeX + spikeY);
      if (r <= rad) {
        vec2 sp = p / rad;
        float z = sqrt(max(0.0, 1.0 - dot(sp, sp)));
        vec3 n = vec3(sp.x, sp.y, z);
        float gran = fbm(n * 8.0 + seedOff + vec3(0.0, 0.0, uTime * 0.18)) * 0.5 + 0.5;
        vec3 surf = mix(vec3(0.76, 0.88, 1.0), vec3(1.0, 1.0, 1.0), smoothstep(0.15, 0.85, gran * 0.4 + z * 0.7));
        col = surf * (0.65 + 0.55 * pow(z, 0.35));
      }
      outColor = vec4(clamp(col, 0.0, 1.0), 1.0);
      return;
    }

    if (uStarKind == 1) {
      // Brown Dwarf (L/T/Y): stormy dark cloud bands with crimson-magenta thermal glow in fissures
      float rad = 0.64;
      float dimGlow = exp(-max(0.0, r - rad) * 6.5) * 0.45;
      vec3 col = bg + vec3(0.88, 0.22, 0.34) * dimGlow;
      if (r <= rad) {
        vec2 sp = p / rad;
        float z = sqrt(max(0.0, 1.0 - dot(sp, sp)));
        vec3 n = vec3(sp.x, sp.y, z);
        float ang = uTime * 0.18;
        vec3 rn = vec3(n.x * cos(ang) - n.z * sin(ang), n.y, n.x * sin(ang) + n.z * cos(ang));
        float warp = fbm(rn * 2.8 + seedOff) * 0.45;
        float bands = sin(rn.y * 14.0 + warp * 5.0) * 0.5 + 0.5;
        float cracks = fbm(vec3(rn.x * 4.5, rn.y * 11.0 + warp * 3.0, rn.z * 4.5) + seedOff) * 0.5 + 0.5;
        vec3 darkCloud = mix(vec3(0.16, 0.06, 0.10), vec3(0.36, 0.14, 0.20), bands);
        float heat = smoothstep(0.56, 0.88, cracks);
        vec3 magmaGlow = mix(vec3(0.88, 0.18, 0.32), vec3(1.0, 0.58, 0.28), smoothstep(0.72, 0.96, cracks));
        float limb = pow(max(0.0, z), 0.55);
        col = mix(darkCloud * (0.45 + 0.7 * limb), magmaGlow, heat * 0.85);
        col += vec3(0.72, 0.18, 0.32) * pow(1.0 - z, 3.0) * 0.35;
      }
      outColor = vec4(clamp(col, 0.0, 1.0), 1.0);
      return;
    }

    // Main Sequence Star (O/B/A/F/G/K/M)
    float rad = 0.64;
    float corona = exp(-max(0.0, r - rad) * 5.2) * 0.85;
    float rays = 0.88 + 0.12 * snoise(vec3(normalize(p + 1e-4) * 4.5, uTime * 0.35));
    vec3 col = bg + uStarColor * corona * rays;
    if (r <= rad) {
      vec2 sp = p / rad;
      float z = sqrt(max(0.0, 1.0 - dot(sp, sp)));
      vec3 n = vec3(sp.x, sp.y, z);
      float ang = uTime * 0.14;
      vec3 rn = vec3(n.x * cos(ang) - n.z * sin(ang), n.y, n.x * sin(ang) + n.z * cos(ang));
      float gran = fbm(rn * 5.2 + seedOff + vec3(0.0, 0.0, uTime * 0.12)) * 0.5 + 0.5;
      float limb = pow(max(0.0, z), 0.45);
      vec3 hotCore = mix(uStarColor, vec3(1.0, 1.0, 0.98), 0.55);
      vec3 photosphere = mix(uStarColor * 0.72, hotCore, smoothstep(0.25, 0.85, gran));
      col = photosphere * (0.48 + 0.65 * limb);
    }
    outColor = vec4(col, 1.0);
    return;
  }

  // Asteroid Belt (9) & Kuiper Belt (10): tilted dust band + orbiting procedural rock/ice chunks
  if (uBodyType == 9 || uBodyType == 10) {
    bool isKuiper = uBodyType == 10;
    vec3 col = bg;
    // Tilted disk projection
    vec2 dp = vec2(p.x * 0.88, p.y * 2.15 + p.x * 0.24);
    float r = length(dp);
    float ang = atan(dp.y, dp.x);
    float innerR = isKuiper ? 0.38 : 0.42;
    float outerR = isKuiper ? 0.94 : 0.90;
    float band = smoothstep(innerR, innerR + 0.12, r) * (1.0 - smoothstep(outerR - 0.14, outerR, r));
    if (band > 0.001) {
      float dust = fbm(vec3(cos(ang - uTime * 0.08) * r * 4.5, sin(ang - uTime * 0.08) * r * 4.5, uSeed * 0.01)) * 0.5 + 0.5;
      vec3 dustCol = isKuiper ? vec3(0.46, 0.72, 0.96) : vec3(0.72, 0.64, 0.54);
      col += dustCol * uStarColor * band * (0.12 + 0.22 * dust);

      // Multi-ring Keplerian asteroid / ice speckles
      for (int layer = 0; layer < 3; layer++) {
        float fl = float(layer);
        float speed = 0.24 / (0.6 + fl * 0.35);
        float aRot = ang - uTime * speed + fl * 2.1;
        vec2 polarCell = vec2(aRot * (24.0 + fl * 14.0), r * (34.0 + fl * 16.0));
        vec2 cid = floor(polarCell);
        vec2 cf = fract(polarCell) - 0.5;
        vec3 rnd = hash33(vec3(cid, uSeed * 0.01 + fl * 13.7));
        vec2 off = (rnd.xy - 0.5) * 0.55;
        float dist = length(cf - off);
        float rockRad = mix(0.06, 0.20, rnd.z);
        float rock = smoothstep(rockRad, rockRad * 0.35, dist) * step(0.58 - fl * 0.06, rnd.z);
        vec3 rockCol = isKuiper ? mix(vec3(0.70, 0.88, 1.0), vec3(0.95, 0.98, 1.0), rnd.x)
                                : mix(vec3(0.55, 0.50, 0.46), vec3(0.86, 0.78, 0.66), rnd.x);
        col = mix(col, rockCol * uStarColor * (0.65 + 0.5 * rnd.y), rock * band);
      }
    }
    // Faint central starlight point for spatial orientation
    float starGlow = exp(-length(p) * 9.0) * 0.38;
    col += uStarColor * starGlow;
    outColor = vec4(clamp(col, 0.0, 1.0), 1.0);
    return;
  }

  // Comet (11): irregular dark icy nucleus + coma + curved dust tail + straight blue ion tail
  if (uBodyType == 11) {
    vec3 col = bg;
    vec2 headPos = vec2(-0.32, 0.14);
    vec2 q = p - headPos;

    // Anti-solar tail coordinates (pointing down-right away from star)
    vec2 ionDir = normalize(vec2(0.88, -0.46));
    vec2 ionPerp = vec2(-ionDir.y, ionDir.x);
    float along = dot(q, ionDir);
    float across = dot(q, ionPerp);

    if (along > -0.15) {
      float tAlong = max(0.0, along);
      // 1. Straight electric-blue ion tail
      float ionWidth = 0.035 + tAlong * 0.11;
      float ionNoise = 0.72 + 0.28 * snoise(vec3(along * 6.5 - uTime * 1.4, across * 14.0, uSeed * 0.01));
      float ionMask = exp(-(across * across) / (ionWidth * ionWidth)) * exp(-tAlong * 1.25) * smoothstep(-0.04, 0.08, along);
      col += vec3(0.28, 0.72, 1.0) * ionMask * ionNoise * 0.95;

      // 2. Curved warm-white dust tail
      float dustAcross = across - tAlong * tAlong * 0.24 - tAlong * 0.08;
      float dustWidth = 0.06 + tAlong * 0.24;
      float dustNoise = 0.75 + 0.25 * fbm(vec3(along * 4.0 - uTime * 0.45, dustAcross * 8.0, uSeed * 0.01));
      float dustMask = exp(-(dustAcross * dustAcross) / (dustWidth * dustWidth)) * exp(-tAlong * 1.15) * smoothstep(-0.04, 0.06, along);
      col += vec3(0.95, 0.90, 0.78) * uStarColor * dustMask * dustNoise * 0.72;
    }

    // Bright cyan-white coma around nucleus
    float dHead = length(q);
    float coma = exp(-dHead * 5.8) * 0.95 + exp(-dHead * 14.0) * 0.65;
    col += mix(vec3(0.45, 0.85, 1.0), vec3(1.0, 0.98, 0.95), exp(-dHead * 9.0)) * coma;

    // Irregular dark icy/rocky nucleus (noise-deformed silhouette)
    float nucAngle = atan(q.y, q.x) + uTime * 0.4;
    float nucRad = 0.115 + 0.025 * snoise(vec3(cos(nucAngle) * 2.2, sin(nucAngle) * 2.2, uSeed * 0.01));
    if (dHead < nucRad) {
      float z = sqrt(max(0.0, 1.0 - (dHead / nucRad) * (dHead / nucRad)));
      vec3 nn = normalize(vec3(q / nucRad, z));
      float nTex = fbm(nn * 6.0 + seedOff) * 0.5 + 0.5;
      float ndl = smoothstep(-0.2, 0.85, dot(nn, normalize(vec3(-0.85, 0.42, 0.35))));
      vec3 nucCol = mix(vec3(0.10, 0.12, 0.16), vec3(0.55, 0.68, 0.78), nTex) * (0.18 + 0.95 * ndl);
      float edgeAA = smoothstep(nucRad, nucRad - 0.015, dHead);
      col = mix(col, nucCol, edgeAA);
    }

    outColor = vec4(clamp(col, 0.0, 1.0), 1.0);
    return;
  }

  // Camera & ray for planet + optional tilted rings + orbiting moons
  float planetRad = uHasRings == 1 ? 0.54 : (uMoonCount > 0 ? 0.62 : 0.74);
  vec3 ro = vec3(0.0, 0.0, 2.8);
  vec3 rd = normalize(vec3(p * 0.95, -2.2));
  vec3 lightDir = normalize(vec3(-0.76, 0.32, 0.56));

  // Ray-sphere intersection for planet
  float b = dot(ro, rd);
  float c = dot(ro, ro) - planetRad * planetRad;
  float disc = b * b - c;
  float tSphere = disc >= 0.0 ? (-b - sqrt(disc)) : 1e9;

  // Ray-sphere intersection for up to 4 orbiting moons (nearest hit tracked separately for behind/front)
  float tMoonBack = 1e9;
  vec3 moonBackCol = vec3(0.0);
  float moonBackAA = 0.0;
  float tMoonFront = 1e9;
  vec3 moonFrontCol = vec3(0.0);
  float moonFrontAA = 0.0;

  for (int m = 0; m < 4; m++) {
    if (m >= uMoonCount) break;
    vec4 mo = uMoonOrbit[m];
    float ang = mo.y + uTime * mo.z;
    float orbR = clamp(mo.x, 1.45, 2.55) * planetRad;
    float tilt = 0.24 + float(m) * 0.07;
    vec3 mCenter = vec3(
      cos(ang) * orbR,
      sin(ang) * orbR * sin(tilt),
      sin(ang) * orbR * cos(tilt)
    );
    float mRad = mo.w * planetRad;
    vec3 oc = ro - mCenter;
    float mb = dot(oc, rd);
    float mc = dot(oc, oc) - mRad * mRad;
    float mdisc = mb * mb - mc;
    if (mdisc >= 0.0) {
      float tm = -mb - sqrt(mdisc);
      if (tm > 0.0) {
        vec3 mHit = ro + rd * tm;
        vec3 mn = normalize(mHit - mCenter);
        float mNoise = fbm(mn * 5.2 + vec3(float(m) * 7.3, uSeed * 0.01, -3.1)) * 0.5 + 0.5;
        float mDiff = smoothstep(-0.1, 0.9, dot(mn, lightDir));
        // Planet eclipse shadow on moon
        float eb = dot(mHit, lightDir);
        float ec = dot(mHit, mHit) - planetRad * planetRad;
        float edisc = eb * eb - ec;
        float eclipse = (edisc > 0.0 && eb < 0.0) ? 0.14 : 1.0;
        vec3 mCol = uMoonColor[m] * (0.72 + 0.45 * mNoise) * uStarColor * (0.05 + mDiff * 0.98 * eclipse);
        float mAA = smoothstep(0.0, 0.18, sqrt(mdisc) / max(0.01, mRad));
        if (tm < tSphere) {
          if (tm < tMoonFront) {
            tMoonFront = tm;
            moonFrontCol = mCol;
            moonFrontAA = mAA;
          }
        } else {
          if (tm < tMoonBack) {
            tMoonBack = tm;
            moonBackCol = mCol;
            moonBackAA = mAA;
          }
        }
      }
    }
  }

  // Ray-ring plane intersection (tilted ring normal)
  vec3 ringN = normalize(vec3(0.18, 0.92, 0.34));
  float tRing = 1e9;
  float ringAlpha = 0.0;
  vec3 ringCol = vec3(0.0);
  if (uHasRings == 1) {
    float denom = dot(rd, ringN);
    if (abs(denom) > 1e-4) {
      float tr = -dot(ro, ringN) / denom;
      if (tr > 0.0) {
        vec3 hitR = ro + rd * tr;
        float rDist = length(hitR) / planetRad;
        if (rDist >= uRingInner && rDist <= uRingOuter) {
          tRing = tr;
          float bandPos = (rDist - uRingInner) / max(0.01, uRingOuter - uRingInner);
          float stripes = 0.6 + 0.4 * sin(bandPos * 38.0 + uSeed) * cos(bandPos * 17.0);
          float edgeFade = smoothstep(0.0, 0.08, bandPos) * (1.0 - smoothstep(0.90, 1.0, bandPos));
          // Planet shadow cast onto ring
          float lb = dot(hitR, lightDir);
          float lc = dot(hitR, hitR) - planetRad * planetRad;
          float ldisc = lb * lb - lc;
          float inShadow = (ldisc > 0.0 && lb < 0.0) ? 0.16 : 1.0;
          ringAlpha = clamp(stripes * edgeFade * 0.82, 0.0, 0.88);
          ringCol = uRingColor * uStarColor * (0.75 + 0.25 * stripes) * inShadow;
        }
      }
    }
  }

  vec3 col = bg;

  // Moons behind planet
  if (tMoonBack < 1e8 && tMoonBack > tRing) {
    col = mix(col, moonBackCol, moonBackAA);
  }

  // Background ring half (behind planet)
  if (tRing < 1e8 && tRing > tSphere) {
    col = mix(col, ringCol, ringAlpha);
  }

  // Moons between background ring and planet
  if (tMoonBack < 1e8 && tMoonBack <= tRing) {
    col = mix(col, moonBackCol, moonBackAA);
  }

  // Planet sphere shading
  if (disc >= 0.0) {
    vec3 hitP = ro + rd * tSphere;
    vec3 nWorld = normalize(hitP);

    // Rotate surface around Y axis
    float rot = uTime * 0.22;
    float cs = cos(rot);
    float sn = sin(rot);
    vec3 nObj = vec3(nWorld.x * cs - nWorld.z * sn, nWorld.y, nWorld.x * sn + nWorld.z * cs);

    SurfaceSample s = evalPlanetSurface(
      nObj,
      seedOff,
      uBodyType,
      uTempK,
      uWaterPct,
      uIcePct,
      uAtmoDensity,
      uAtmoColor,
      uCloudPct,
      uPopNorm,
      uMineralTint,
      uTime * 0.07
    );

    float ndl = dot(nWorld, lightDir);
    float diff = smoothstep(-0.12, 0.88, ndl);
    vec3 lightDir2 = normalize(vec3(0.76, -0.24, 0.44));
    float ndl2 = dot(nWorld, lightDir2);
    float diff2 = uHasStar2 == 1 ? smoothstep(-0.12, 0.88, ndl2) * 0.46 : 0.0;
    float totalIllum = clamp(diff + diff2, 0.0, 1.0);
    float nightMask = smoothstep(0.14, -0.18, ndl) * (1.0 - smoothstep(0.15, 0.65, diff2));

    // Ring shadow cast onto planet
    float ringShadow = 1.0;
    if (uHasRings == 1) {
      float denom = dot(lightDir, ringN);
      if (abs(denom) > 1e-4) {
        float ts = -dot(hitP, ringN) / denom;
        if (ts > 0.0) {
          float rdSh = length(hitP + lightDir * ts) / planetRad;
          if (rdSh >= uRingInner && rdSh <= uRingOuter) {
            ringShadow = 0.45;
          }
        }
      }
    }

    vec3 halfV = normalize(lightDir - rd);
    float spec = pow(max(0.0, dot(nWorld, halfV)), 28.0) * s.spec * max(0.0, ndl);
    vec3 halfV2 = normalize(lightDir2 - rd);
    float spec2 = uHasStar2 == 1 ? pow(max(0.0, dot(nWorld, halfV2)), 24.0) * s.spec * max(0.0, ndl2) * 0.45 : 0.0;

    vec3 lightTint = uStarColor * (0.045 + diff * 0.96 * ringShadow)
                   + (uHasStar2 == 1 ? uStar2Color * diff2 : vec3(0.0));
    vec3 lit = s.albedo * lightTint + uStarColor * spec * ringShadow + uStar2Color * spec2;

    // Cloud layer + subtle shadow
    if (s.cloudMask > 0.01) {
      vec3 cloudCol = mix(vec3(0.92, 0.95, 1.0), uAtmoColor, 0.18) * lightTint;
      lit = mix(lit, cloudCol, s.cloudMask * 0.82);
    }

    // Emissive (lava everywhere, city lights strictly on night side)
    if (uTempK > 610.0) {
      lit += s.emissiveCol * s.emissive;
    } else if (s.emissive > 0.001) {
      lit += s.emissiveCol * s.emissive * nightMask * (1.0 - s.cloudMask * 0.6);
    }

    // Fresnel atmospheric rim lighting (dual-tinted in binary systems)
    float fresnel = pow(1.0 - max(0.0, dot(nWorld, -rd)), 3.2);
    vec3 rimLight = uStarColor * (0.24 + 0.76 * max(0.0, ndl + 0.25))
                  + (uHasStar2 == 1 ? uStar2Color * (0.14 + 0.48 * max(0.0, ndl2 + 0.2)) : vec3(0.0));
    lit += uAtmoColor * rimLight * fresnel * uAtmoDensity * (0.45 + 0.55 * totalIllum);

    // Smooth anti-aliased sphere limb
    float limbAA = smoothstep(0.0, 0.045, sqrt(max(0.0, disc)) / planetRad);
    col = mix(col, lit, limbAA);
  } else {
    // Subtle outer atmospheric halo around limb
    float distEdge = length(p) - planetRad * 1.02;
    if (distEdge > -0.05 && distEdge < 0.22) {
      float halo = exp(-max(0.0, distEdge) * 14.0) * uAtmoDensity * 0.34;
      col += uAtmoColor * uStarColor * halo;
    }
  }

  // Foreground ring half (in front of planet)
  if (tRing < tSphere && tRing < 1e8) {
    col = mix(col, ringCol, ringAlpha);
  }

  // Moons passing in front of planet & rings
  if (tMoonFront < 1e8) {
    col = mix(col, moonFrontCol, moonFrontAA);
  }

  // Gamma tone mapping
  col = pow(clamp(col, 0.0, 1.0), vec3(0.92));
  outColor = vec4(col, 1.0);
}
`;

const EQUIRECT_FS = `#version 300 es
precision highp float;
in vec2 vUv;
out vec4 outColor;

uniform int uBodyType;
uniform float uSeed;
uniform float uTempK;
uniform float uWaterPct;
uniform float uIcePct;
uniform float uAtmoDensity;
uniform vec3 uAtmoColor;
uniform float uCloudPct;
uniform float uPopNorm;
uniform vec3 uMineralTint;
uniform vec3 uStarColor;

${NOISE_GLSL}
${SURFACE_EVAL_GLSL}

void main() {
  float theta = vUv.x * 6.28318530718;
  float phi = (vUv.y - 0.5) * 3.14159265359;
  vec3 n = vec3(cos(phi) * cos(theta), sin(phi), cos(phi) * sin(theta));
  vec3 seedOff = vec3(
    fract(uSeed * 0.00137) * 41.0,
    fract(uSeed * 0.00319) * 37.0,
    fract(uSeed * 0.00731) * 29.0
  );

  SurfaceSample s = evalPlanetSurface(
    n,
    seedOff,
    uBodyType,
    uTempK,
    uWaterPct,
    uIcePct,
    uAtmoDensity,
    uAtmoColor,
    uCloudPct,
    uPopNorm,
    uMineralTint,
    0.0
  );

  vec3 col = s.albedo;
  if (s.cloudMask > 0.01) {
    col = mix(col, mix(vec3(0.94, 0.96, 1.0), uAtmoColor, 0.15), s.cloudMask * 0.65);
  }
  if (s.emissive > 0.001) {
    col += s.emissiveCol * s.emissive * 0.55;
  }
  col = mix(col, col * uStarColor, 0.25);
  outColor = vec4(clamp(col, 0.0, 1.0), 1.0);
}
`;

function bodyTypeToInt(t: BodyType): number {
  switch (t) {
    case 'Terrestrial':
      return 0;
    case 'Oceanic':
      return 1;
    case 'Desert':
      return 2;
    case 'Ice':
      return 3;
    case 'Volcanic':
      return 4;
    case 'Rock':
      return 5;
    case 'GasGiant':
      return 6;
    case 'IceGiant':
      return 7;
    case 'Dwarf':
      return 8;
    case 'AsteroidBelt':
      return 9;
    case 'KuiperBelt':
      return 10;
    case 'Comet':
      return 11;
  }
}

function compileProgram(gl: WebGL2RenderingContext, vsSrc: string, fsSrc: string): WebGLProgram | null {
  const vs = gl.createShader(gl.VERTEX_SHADER);
  const fs = gl.createShader(gl.FRAGMENT_SHADER);
  if (!vs || !fs) return null;
  gl.shaderSource(vs, vsSrc);
  gl.compileShader(vs);
  if (!gl.getShaderParameter(vs, gl.COMPILE_STATUS)) {
    gl.deleteShader(vs);
    gl.deleteShader(fs);
    return null;
  }
  gl.shaderSource(fs, fsSrc);
  gl.compileShader(fs);
  if (!gl.getShaderParameter(fs, gl.COMPILE_STATUS)) {
    gl.deleteShader(vs);
    gl.deleteShader(fs);
    return null;
  }
  const prog = gl.createProgram();
  if (!prog) return null;
  gl.attachShader(prog, vs);
  gl.attachShader(prog, fs);
  gl.linkProgram(prog);
  gl.deleteShader(vs);
  gl.deleteShader(fs);
  if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
    gl.deleteProgram(prog);
    return null;
  }
  return prog;
}

function starKindToInt(k: StarKind): number {
  switch (k) {
    case 'MainSequence':
      return 0;
    case 'BrownDwarf':
      return 1;
    case 'WhiteDwarf':
      return 2;
    case 'Pulsar':
      return 3;
    case 'BlackHole':
      return 4;
  }
}

export type ProceduralTarget =
  | { kind: 'star'; star: StarView }
  | { kind: 'body'; body: SystemBodyView; star: StarView; companionStar?: StarView | null }
  | { kind: 'moon'; moon: MoonView; star: StarView; companionStar?: StarView | null };

function moonBaseRgb(m: MoonView): [number, number, number] {
  if (m.bodyType === 'Volcanic') return [0.85, 0.44, 0.24];
  if (m.bodyType === 'Ice') return [0.76, 0.88, 0.98];
  if (m.bodyType === 'Desert') return [0.86, 0.68, 0.40];
  return [0.64, 0.64, 0.66];
}

/**
 * Lightweight WebGL2 procedural sphere/star canvas renderer.
 * Automatically stops its animation loop when hidden so idle GPU usage stays at 0%.
 */
export class ProceduralBodyRenderer {
  private readonly canvas: HTMLCanvasElement;
  private readonly gl: WebGL2RenderingContext | null;
  private readonly prog: WebGLProgram | null = null;
  private readonly vao: WebGLVertexArrayObject | null = null;
  private readonly fixedSize: boolean;
  private target: ProceduralTarget | null = null;
  private rafId: number | null = null;
  private startMs = performance.now();
  private active = false;

  constructor(canvas: HTMLCanvasElement, fixedSize = false) {
    this.canvas = canvas;
    this.fixedSize = fixedSize;
    this.gl = canvas.getContext('webgl2', {
      alpha: false,
      antialias: true,
      premultipliedAlpha: false,
      preserveDrawingBuffer: fixedSize,
    });
    if (this.gl) {
      this.prog = compileProgram(this.gl, FULLSCREEN_VS, SPHERE_FS);
      this.vao = this.gl.createVertexArray();
    }
  }

  public setTarget(target: ProceduralTarget | null, animate = true): void {
    this.target = target;
    if (!target) {
      this.stop();
      return;
    }
    this.active = animate;
    if (!animate) {
      this.renderOnce(0);
      return;
    }
    if (this.rafId === null) {
      this.startMs = performance.now();
      this.renderOnce(0);
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

  private readonly tick = (now: number): void => {
    this.rafId = null;
    if (!this.active || !this.target) return;
    const elapsed = (now - this.startMs) * 0.001;
    this.renderOnce(elapsed);
    this.rafId = requestAnimationFrame(this.tick);
  };

  public renderOnce(timeSec: number): void {
    if (!this.target) return;
    let bufW: number;
    let bufH: number;
    if (this.fixedSize || this.canvas.clientWidth === 0 || this.canvas.clientHeight === 0) {
      bufW = Math.max(1, this.canvas.width || 128);
      bufH = Math.max(1, this.canvas.height || 128);
    } else {
      const dpr = Math.min(window.devicePixelRatio || 1, 2);
      bufW = Math.round(this.canvas.clientWidth * dpr);
      bufH = Math.round(this.canvas.clientHeight * dpr);
      if (this.canvas.width !== bufW || this.canvas.height !== bufH) {
        this.canvas.width = bufW;
        this.canvas.height = bufH;
      }
    }

    const gl = this.gl;
    if (!gl || !this.prog || !this.vao) return;

    gl.viewport(0, 0, bufW, bufH);
    gl.useProgram(this.prog);
    gl.bindVertexArray(this.vao);

    const loc = (name: string) => gl.getUniformLocation(this.prog!, name);
    gl.uniform2f(loc('uRes'), bufW, bufH);
    gl.uniform1f(loc('uTime'), timeSec);

    if (this.target.kind === 'star') {
      const st = this.target.star;
      gl.uniform1i(loc('uIsStar'), 1);
      gl.uniform1i(loc('uStarKind'), starKindToInt(st.starKind));
      gl.uniform1f(loc('uSeed'), st.seed % 100000);
      gl.uniform3f(loc('uStarColor'), st.colorRgb[0], st.colorRgb[1], st.colorRgb[2]);
      gl.uniform1i(loc('uHasStar2'), 0);
    } else if (this.target.kind === 'moon') {
      const m = this.target.moon;
      const st = this.target.star;
      const st2 = this.target.companionStar;
      gl.uniform1i(loc('uIsStar'), 0);
      gl.uniform1i(loc('uBodyType'), bodyTypeToInt(m.bodyType));
      gl.uniform1f(loc('uSeed'), m.seed % 100000);
      gl.uniform1f(loc('uTempK'), m.tempK);
      gl.uniform1f(loc('uWaterPct'), m.waterPct);
      gl.uniform1f(loc('uIcePct'), m.icePct);
      gl.uniform1f(loc('uAtmoDensity'), m.atmoDensity);
      gl.uniform3f(loc('uAtmoColor'), m.atmoColorRgb[0], m.atmoColorRgb[1], m.atmoColorRgb[2]);
      gl.uniform1f(loc('uCloudPct'), m.cloudPct);
      gl.uniform1f(loc('uPopNorm'), 0);
      gl.uniform3f(loc('uMineralTint'), 0, 0, 0);
      gl.uniform3f(loc('uStarColor'), st.colorRgb[0], st.colorRgb[1], st.colorRgb[2]);
      gl.uniform1i(loc('uHasStar2'), st2 ? 1 : 0);
      if (st2) {
        gl.uniform3f(loc('uStar2Color'), st2.colorRgb[0], st2.colorRgb[1], st2.colorRgb[2]);
      }
      gl.uniform1i(loc('uHasRings'), 0);
      gl.uniform1i(loc('uMoonCount'), 0);
    } else {
      const b = this.target.body;
      const st = this.target.star;
      const st2 = this.target.companionStar;
      const ls = b.liveState;
      gl.uniform1i(loc('uIsStar'), 0);
      gl.uniform1i(loc('uBodyType'), bodyTypeToInt(b.bodyType));
      gl.uniform1f(loc('uSeed'), b.seed % 100000);
      gl.uniform1f(loc('uTempK'), b.tempK);
      gl.uniform1f(loc('uWaterPct'), b.waterPct);
      gl.uniform1f(loc('uIcePct'), b.icePct);
      gl.uniform1f(loc('uAtmoDensity'), b.atmoDensity);
      gl.uniform3f(loc('uAtmoColor'), b.atmoColorRgb[0], b.atmoColorRgb[1], b.atmoColorRgb[2]);
      gl.uniform1f(loc('uCloudPct'), b.cloudPct);
      gl.uniform1f(loc('uPopNorm'), ls ? ls.popNorm : 0);
      gl.uniform3f(
        loc('uMineralTint'),
        ls ? ls.mineralTint[0] : 0,
        ls ? ls.mineralTint[1] : 0,
        ls ? ls.mineralTint[2] : 0,
      );
      gl.uniform3f(loc('uStarColor'), st.colorRgb[0], st.colorRgb[1], st.colorRgb[2]);
      gl.uniform1i(loc('uHasStar2'), st2 ? 1 : 0);
      if (st2) {
        gl.uniform3f(loc('uStar2Color'), st2.colorRgb[0], st2.colorRgb[1], st2.colorRgb[2]);
      }
      gl.uniform1i(loc('uHasRings'), b.hasRings ? 1 : 0);
      gl.uniform1f(loc('uRingInner'), b.ringInner);
      gl.uniform1f(loc('uRingOuter'), b.ringOuter);
      gl.uniform3f(loc('uRingColor'), b.ringColorRgb[0], b.ringColorRgb[1], b.ringColorRgb[2]);

      const moons = b.moons ?? [];
      const mCount = Math.min(4, moons.length);
      gl.uniform1i(loc('uMoonCount'), mCount);
      if (mCount > 0) {
        const orbArr = new Float32Array(16);
        const colArr = new Float32Array(12);
        for (let i = 0; i < mCount; i++) {
          const m = moons[i];
          const relRad = Math.max(0.08, Math.min(0.18, (m.radiusEarth / Math.max(0.5, b.radiusEarth)) * 0.45));
          orbArr[i * 4] = Math.min(2.45, 1.48 + i * 0.28);
          orbArr[i * 4 + 1] = m.orbitAngleRad;
          orbArr[i * 4 + 2] = m.orbitSpeed;
          orbArr[i * 4 + 3] = relRad;
          const rgb = moonBaseRgb(m);
          colArr[i * 3] = rgb[0];
          colArr[i * 3 + 1] = rgb[1];
          colArr[i * 3 + 2] = rgb[2];
        }
        gl.uniform4fv(loc('uMoonOrbit'), orbArr);
        gl.uniform3fv(loc('uMoonColor'), colArr);
      }
    }

    gl.drawArrays(gl.TRIANGLES, 0, 3);
    if (this.fixedSize) {
      gl.flush();
    }
  }
}

/**
 * Renders a static 1-frame procedural thumbnail onto a small canvas using a shared offscreen WebGL2 context
 * (avoids hitting browser max-WebGL-context limits when displaying 10+ planets in System View).
 */
let sharedThumbCanvas: HTMLCanvasElement | null = null;
let sharedThumbRenderer: ProceduralBodyRenderer | null = null;

export function renderProceduralThumbnailTo2d(
  destCanvas: HTMLCanvasElement,
  target: ProceduralTarget,
): void {
  if (!sharedThumbCanvas) {
    sharedThumbCanvas = document.createElement('canvas');
    sharedThumbCanvas.width = 128;
    sharedThumbCanvas.height = 128;
    sharedThumbRenderer = new ProceduralBodyRenderer(sharedThumbCanvas, true);
  }
  if (!sharedThumbRenderer) return;
  if (sharedThumbCanvas.width !== 128 || sharedThumbCanvas.height !== 128) {
    sharedThumbCanvas.width = 128;
    sharedThumbCanvas.height = 128;
  }
  sharedThumbRenderer.setTarget(target, false);
  sharedThumbRenderer.renderOnce(1.2);
  const ctx = destCanvas.getContext('2d');
  if (ctx) {
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, destCanvas.width, destCanvas.height);
    ctx.drawImage(sharedThumbCanvas, 0, 0, 128, 128, 0, 0, destCanvas.width, destCanvas.height);
  }
}

/**
 * Exports a 2048x1024 equirectangular PNG texture of any procedural planet using the exact same shader.
 */
export function exportPlanetTexturePng(
  body: SystemBodyView | MoonView,
  star: StarView,
): boolean {
  const canvas = document.createElement('canvas');
  canvas.width = 2048;
  canvas.height = 1024;
  const gl = canvas.getContext('webgl2', { preserveDrawingBuffer: true });
  if (!gl) return false;
  const prog = compileProgram(gl, FULLSCREEN_VS, EQUIRECT_FS);
  if (!prog) return false;
  const vao = gl.createVertexArray();
  gl.viewport(0, 0, canvas.width, canvas.height);
  gl.useProgram(prog);
  gl.bindVertexArray(vao);

  const loc = (name: string) => gl.getUniformLocation(prog, name);
  const ls = 'liveState' in body ? body.liveState : null;
  gl.uniform1i(loc('uBodyType'), bodyTypeToInt(body.bodyType));
  gl.uniform1f(loc('uSeed'), body.seed % 100000);
  gl.uniform1f(loc('uTempK'), body.tempK);
  gl.uniform1f(loc('uWaterPct'), body.waterPct);
  gl.uniform1f(loc('uIcePct'), body.icePct);
  gl.uniform1f(loc('uAtmoDensity'), body.atmoDensity);
  gl.uniform3f(loc('uAtmoColor'), body.atmoColorRgb[0], body.atmoColorRgb[1], body.atmoColorRgb[2]);
  gl.uniform1f(loc('uCloudPct'), body.cloudPct);
  gl.uniform1f(loc('uPopNorm'), ls ? ls.popNorm : 0);
  gl.uniform3f(
    loc('uMineralTint'),
    ls ? ls.mineralTint[0] : 0,
    ls ? ls.mineralTint[1] : 0,
    ls ? ls.mineralTint[2] : 0,
  );
  gl.uniform3f(loc('uStarColor'), star.colorRgb[0], star.colorRgb[1], star.colorRgb[2]);

  gl.drawArrays(gl.TRIANGLES, 0, 3);
  gl.finish();

  const url = canvas.toDataURL('image/png');
  const a = document.createElement('a');
  const safeName = body.name.toLowerCase().replace(/[^a-z0-9]+/g, '-');
  a.href = url;
  a.download = `sistars-${safeName}-2048x1024.png`;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);

  gl.deleteVertexArray(vao);
  gl.deleteProgram(prog);
  return true;
}
