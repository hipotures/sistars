import * as THREE from 'three';
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js';
import type { FleetClass } from './types';

export type ShipViewMode = 'solid' | 'blueprint';

export interface ShipRenderState {
  fleetClass: FleetClass;
  empireColor: string;
  hpRatio: number; // 0..1
  cloakPct: number; // 0..1
  jammerPct: number; // 0..1
  cargoMineralType?: number; // 0=Iron, 1=Bora, 2=Germ (only used for freight)
}

export const SHIP_MODEL_PATHS: Record<FleetClass, string> = {
  battleship: '/models/ships/battleship.glb',
  cruiser: '/models/ships/cruiser.glb',
  bomber: '/models/ships/bomber.glb',
  freight: '/models/ships/freighter.glb',
  col: '/models/ships/colony.glb',
};

const MINERAL_COLORS: readonly string[] = ['#60a5fa', '#34d399', '#fbbf24'];

interface CachedShipAsset {
  geometry: THREE.BufferGeometry;
  texture: THREE.Texture | null;
  radius: number;
}

const gltfLoader = new GLTFLoader();
const assetCache = new Map<FleetClass, Promise<CachedShipAsset>>();

function loadShipAsset(fleetClass: FleetClass): Promise<CachedShipAsset> {
  const existing = assetCache.get(fleetClass);
  if (existing) return existing;

  const url = SHIP_MODEL_PATHS[fleetClass];
  const promise = new Promise<CachedShipAsset>((resolve, reject) => {
    gltfLoader.load(
      url,
      (gltf) => {
        let foundMesh: THREE.Mesh | null = null;
        gltf.scene.traverse((obj) => {
          if (!foundMesh && (obj as THREE.Mesh).isMesh) {
            foundMesh = obj as THREE.Mesh;
          }
        });
        if (!foundMesh) {
          reject(new Error(`No mesh found in ${url}`));
          return;
        }
        const mesh = foundMesh as THREE.Mesh;
        const geo = mesh.geometry.clone();
        geo.computeBoundingBox();
        const box = geo.boundingBox!;
        const center = new THREE.Vector3();
        const size = new THREE.Vector3();
        box.getCenter(center);
        box.getSize(size);

        const maxDim = Math.max(size.x, size.y, size.z, 0.001);
        const normScale = 2.35 / maxDim;
        geo.translate(-center.x, -center.y, -center.z);
        geo.scale(normScale, normScale, normScale);
        geo.computeVertexNormals();
        geo.computeBoundingSphere();

        let texture: THREE.Texture | null = null;
        const mat = mesh.material as THREE.MeshStandardMaterial | undefined;
        if (mat && mat.map) {
          texture = mat.map;
          texture.colorSpace = THREE.SRGBColorSpace;
          texture.needsUpdate = true;
        }

        resolve({
          geometry: geo,
          texture,
          radius: geo.boundingSphere?.radius ?? 1.2,
        });
      },
      undefined,
      (err) => {
        assetCache.delete(fleetClass);
        reject(err);
      },
    );
  });

  assetCache.set(fleetClass, promise);
  return promise;
}

const SOLID_VERT = /* glsl */ `
  varying vec2 vUv;
  varying vec3 vNormalW;
  varying vec3 vViewDir;
  varying vec3 vLocalPos;
  void main() {
    vUv = uv;
    vLocalPos = position;
    vec4 worldPos = modelMatrix * vec4(position, 1.0);
    vNormalW = normalize(mat3(modelMatrix) * normal);
    vViewDir = normalize(cameraPosition - worldPos.xyz);
    gl_Position = projectionMatrix * viewMatrix * worldPos;
  }
`;

const SOLID_FRAG = /* glsl */ `
  uniform sampler2D uMap;
  uniform float uHasMap;
  uniform vec3 uEmpireColor;
  uniform vec3 uAccentColor;
  uniform float uHasCargoAccent;
  uniform float uCloak;
  uniform float uHpRatio;
  uniform float uTime;

  varying vec2 vUv;
  varying vec3 vNormalW;
  varying vec3 vViewDir;
  varying vec3 vLocalPos;

  void main() {
    vec3 N = normalize(vNormalW);
    vec3 V = normalize(vViewDir);

    vec4 tex = uHasMap > 0.5 ? texture2D(uMap, vUv) : vec4(0.62, 0.66, 0.72, 0.35);
    // tex.a encodes the faction paint mask (0 = neutral metallic hull, 1 = painted empire accent)
    vec3 metalAlbedo = tex.rgb;
    vec3 paintedAlbedo = tex.rgb * uEmpireColor * 1.35;
    vec3 hullBase = mix(metalAlbedo, paintedAlbedo, tex.a);

    // Subtle cargo mineral trim on Freighters
    if (uHasCargoAccent > 0.5) {
      float stripe = smoothstep(0.35, 0.75, tex.a) * smoothstep(-0.2, 0.4, vLocalPos.y);
      hullBase = mix(hullBase, tex.rgb * uAccentColor * 1.35, stripe * 0.42);
    }

    // Damage darkening when HP < 100%
    float dmg = 1.0 - clamp(uHpRatio, 0.0, 1.0);
    hullBase *= 1.0 - dmg * 0.38;

    // 3-point studio hangar lighting: warm-white key, cool-blue fill, back rim in empire color
    vec3 L_key = normalize(vec3(0.68, 0.82, 0.62));
    vec3 L_fill = normalize(vec3(-0.78, 0.25, -0.45));
    vec3 L_back = normalize(vec3(0.15, -0.55, -0.82));

    float diffKey = max(dot(N, L_key), 0.0);
    float diffFill = max(dot(N, L_fill), 0.0);
    float diffBack = max(dot(N, L_back), 0.0);

    vec3 H_key = normalize(L_key + V);
    float spec = pow(max(dot(N, H_key), 0.0), 36.0) * (0.42 + 0.28 * (1.0 - tex.a));
    float fresnel = pow(1.0 - max(dot(N, V), 0.0), 3.0);
    vec3 rimCol = mix(uEmpireColor, uAccentColor, uHasCargoAccent * 0.35);

    vec3 color = hullBase * (0.30 + diffKey * 0.95 + diffFill * 0.30 + diffBack * 0.16)
               + vec3(0.92, 0.96, 1.0) * spec
               + rimCol * fresnel * (0.32 + uCloak * 0.85);

    // Low-HP warning ember pulse on damaged armor
    if (dmg > 0.15) {
      float flicker = 0.5 + 0.5 * sin(uTime * 16.0 + vLocalPos.z * 7.0);
      float spark = smoothstep(0.68, 0.98, flicker) * dmg * 0.38;
      color += vec3(1.0, 0.34, 0.08) * spark * (1.0 - tex.a);
    }

    // Cloak: keep crisp metallic hull readable with depthWrite=true, add sleek stealth Fresnel sheen
    float alpha = 1.0;
    if (uCloak > 0.01) {
      float sweep = 0.5 + 0.5 * sin(vLocalPos.z * 14.0 - uTime * 3.8);
      vec3 cloakEdge = mix(uEmpireColor, vec3(0.28, 0.92, 1.0), 0.55);
      color += cloakEdge * fresnel * uCloak * (0.55 + 0.35 * sweep);
      alpha = mix(1.0, clamp(0.62 + fresnel * 0.35, 0.62, 0.98), uCloak);
    }

    gl_FragColor = vec4(color, alpha);
  }
`;

const JAMMER_VERT = /* glsl */ `
  varying vec3 vNormalW;
  varying vec3 vViewDir;
  varying vec3 vLocalPos;
  void main() {
    vLocalPos = position;
    vec4 worldPos = modelMatrix * vec4(position, 1.0);
    vNormalW = normalize(mat3(modelMatrix) * normal);
    vViewDir = normalize(cameraPosition - worldPos.xyz);
    gl_Position = projectionMatrix * viewMatrix * worldPos;
  }
`;

const JAMMER_FRAG = /* glsl */ `
  uniform vec3 uColor;
  uniform float uStrength;
  uniform float uTime;
  varying vec3 vNormalW;
  varying vec3 vViewDir;
  varying vec3 vLocalPos;
  void main() {
    float ndv = abs(dot(normalize(vNormalW), normalize(vViewDir)));
    float rim = pow(1.0 - ndv, 3.4);
    float band = 0.6 + 0.4 * sin(vLocalPos.y * 16.0 - uTime * 3.5);
    float pulse = 0.72 + 0.28 * sin(uTime * 3.0);
    float alpha = rim * (0.14 + uStrength * 0.34) * band * pulse;
    gl_FragColor = vec4(uColor, alpha);
  }
`;

const BLUEPRINT_VERT = /* glsl */ `
  varying vec3 vNormalW;
  varying vec3 vViewDir;
  void main() {
    vec4 worldPos = modelMatrix * vec4(position, 1.0);
    vNormalW = normalize(mat3(modelMatrix) * normal);
    vViewDir = normalize(cameraPosition - worldPos.xyz);
    gl_Position = projectionMatrix * viewMatrix * worldPos;
  }
`;

const BLUEPRINT_FRAG = /* glsl */ `
  uniform vec3 uEmpireColor;
  uniform float uOpacity;
  varying vec3 vNormalW;
  varying vec3 vViewDir;
  void main() {
    float fres = pow(1.0 - abs(dot(normalize(vNormalW), normalize(vViewDir))), 1.8);
    vec3 col = mix(uEmpireColor * 0.55, uEmpireColor * 1.35 + vec3(0.25), fres);
    gl_FragColor = vec4(col, uOpacity * (0.35 + fres * 0.65));
  }
`;

export class ShipPreviewRenderer {
  private readonly canvas: HTMLCanvasElement;
  private readonly interactive: boolean;
  private renderer: THREE.WebGLRenderer | null = null;
  private readonly scene = new THREE.Scene();
  private readonly camera = new THREE.PerspectiveCamera(36, 1, 0.1, 100);
  private readonly shipRoot = new THREE.Group();

  private solidMesh: THREE.Mesh | null = null;
  private solidMat: THREE.ShaderMaterial;
  private wireMesh: THREE.Mesh | null = null;
  private wireMat: THREE.MeshBasicMaterial;
  private holoMesh: THREE.Mesh | null = null;
  private holoMat: THREE.ShaderMaterial;
  private blueprintGrid: THREE.GridHelper;
  private jammerField: THREE.Mesh;
  private jammerMat: THREE.ShaderMaterial;
  private engineGlow: THREE.Sprite;
  private engineGlowMat: THREE.SpriteMaterial;
  private stars: THREE.Points;

  private state: ShipRenderState = {
    fleetClass: 'cruiser',
    empireColor: '#38bdf8',
    hpRatio: 1,
    cloakPct: 0,
    jammerPct: 0,
  };
  private loadedClass: FleetClass | null = null;
  private loadSeq = 0;
  private mode: ShipViewMode = 'solid';

  private rafId: number | null = null;
  private startMs = performance.now();
  private yaw = 0.65;
  private pitch = 0.28;
  private camDist = 4.2;
  private dragging = false;
  private lastPtrX = 0;
  private lastPtrY = 0;
  private lastW = 0;
  private lastH = 0;

  constructor(canvas: HTMLCanvasElement, interactive: boolean) {
    this.canvas = canvas;
    this.interactive = interactive;
    this.camDist = interactive ? 3.85 : 3.7;

    try {
      this.renderer = new THREE.WebGLRenderer({
        canvas,
        antialias: true,
        alpha: true,
        powerPreference: 'low-power',
      });
      this.renderer.setClearColor(0x050814, 1);
      this.renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
    } catch {
      this.renderer = null;
    }

    // Subtle deep-space starfield
    const starCount = interactive ? 180 : 75;
    const starPos = new Float32Array(starCount * 3);
    let seed = interactive ? 48271 : 19381;
    const rnd = () => {
      seed = (seed * 1664525 + 1013904223) >>> 0;
      return seed / 4294967296;
    };
    for (let i = 0; i < starCount; i++) {
      const r = 12 + rnd() * 24;
      const th = rnd() * Math.PI * 2;
      const ph = Math.acos(2 * rnd() - 1);
      starPos[i * 3] = r * Math.sin(ph) * Math.cos(th);
      starPos[i * 3 + 1] = r * Math.cos(ph);
      starPos[i * 3 + 2] = r * Math.sin(ph) * Math.sin(th);
    }
    const starGeo = new THREE.BufferGeometry();
    starGeo.setAttribute('position', new THREE.BufferAttribute(starPos, 3));
    const starMat = new THREE.PointsMaterial({
      color: 0xcbd5e1,
      size: interactive ? 0.11 : 0.14,
      transparent: true,
      opacity: 0.65,
      sizeAttenuation: true,
    });
    this.stars = new THREE.Points(starGeo, starMat);
    this.scene.add(this.stars);

    // Materials
    this.solidMat = new THREE.ShaderMaterial({
      vertexShader: SOLID_VERT,
      fragmentShader: SOLID_FRAG,
      transparent: true,
      depthWrite: true,
      side: THREE.FrontSide,
      uniforms: {
        uMap: { value: null },
        uHasMap: { value: 0 },
        uEmpireColor: { value: new THREE.Color('#38bdf8') },
        uAccentColor: { value: new THREE.Color('#38bdf8') },
        uHasCargoAccent: { value: 0 },
        uCloak: { value: 0 },
        uHpRatio: { value: 1 },
        uTime: { value: 0 },
      },
    });

    this.wireMat = new THREE.MeshBasicMaterial({
      color: new THREE.Color('#38bdf8'),
      wireframe: true,
      transparent: true,
      opacity: 0.78,
    });

    this.holoMat = new THREE.ShaderMaterial({
      vertexShader: BLUEPRINT_VERT,
      fragmentShader: BLUEPRINT_FRAG,
      transparent: true,
      depthWrite: false,
      side: THREE.DoubleSide,
      uniforms: {
        uEmpireColor: { value: new THREE.Color('#38bdf8') },
        uOpacity: { value: 0.34 },
      },
    });

    // Technical Blueprint grid
    this.blueprintGrid = new THREE.GridHelper(6.2, 18, 0x38bdf8, 0x1e293b);
    this.blueprintGrid.position.y = -0.92;
    this.blueprintGrid.visible = false;
    this.scene.add(this.blueprintGrid);

    // EW Jammer smooth Fresnel energy field (no jagged wireframe cage)
    const jamGeo = new THREE.SphereGeometry(1.42, 32, 24);
    this.jammerMat = new THREE.ShaderMaterial({
      vertexShader: JAMMER_VERT,
      fragmentShader: JAMMER_FRAG,
      transparent: true,
      blending: THREE.AdditiveBlending,
      depthWrite: false,
      side: THREE.FrontSide,
      uniforms: {
        uColor: { value: new THREE.Color('#a855f7') },
        uStrength: { value: 0 },
        uTime: { value: 0 },
      },
    });
    this.jammerField = new THREE.Mesh(jamGeo, this.jammerMat);
    this.jammerField.visible = false;
    this.shipRoot.add(this.jammerField);

    // Radial thruster glow sprite
    const glowTex = this.createGlowTexture();
    this.engineGlowMat = new THREE.SpriteMaterial({
      map: glowTex,
      color: new THREE.Color('#38bdf8'),
      transparent: true,
      opacity: 0.72,
      blending: THREE.AdditiveBlending,
      depthWrite: false,
    });
    this.engineGlow = new THREE.Sprite(this.engineGlowMat);
    this.engineGlow.scale.set(0.95, 0.95, 1);
    this.engineGlow.position.set(0, -0.02, -1.08);
    this.shipRoot.add(this.engineGlow);

    this.scene.add(this.shipRoot);

    if (this.interactive) {
      this.bindInteraction();
    }
  }

  private createGlowTexture(): THREE.CanvasTexture {
    const c = document.createElement('canvas');
    c.width = 64;
    c.height = 64;
    const ctx = c.getContext('2d')!;
    const g = ctx.createRadialGradient(32, 32, 1, 32, 32, 32);
    g.addColorStop(0, 'rgba(255,255,255,1)');
    g.addColorStop(0.28, 'rgba(255,255,255,0.65)');
    g.addColorStop(0.65, 'rgba(255,255,255,0.16)');
    g.addColorStop(1, 'rgba(255,255,255,0)');
    ctx.fillStyle = g;
    ctx.fillRect(0, 0, 64, 64);
    const tex = new THREE.CanvasTexture(c);
    tex.needsUpdate = true;
    return tex;
  }

  private bindInteraction(): void {
    this.canvas.addEventListener('pointerdown', (e) => {
      this.dragging = true;
      this.lastPtrX = e.clientX;
      this.lastPtrY = e.clientY;
      this.canvas.setPointerCapture(e.pointerId);
    });
    this.canvas.addEventListener('pointermove', (e) => {
      if (!this.dragging) return;
      const dx = e.clientX - this.lastPtrX;
      const dy = e.clientY - this.lastPtrY;
      this.lastPtrX = e.clientX;
      this.lastPtrY = e.clientY;
      this.yaw += dx * 0.009;
      this.pitch = Math.max(-1.15, Math.min(1.15, this.pitch + dy * 0.008));
    });
    const endDrag = (e: PointerEvent) => {
      this.dragging = false;
      if (this.canvas.hasPointerCapture(e.pointerId)) {
        this.canvas.releasePointerCapture(e.pointerId);
      }
    };
    this.canvas.addEventListener('pointerup', endDrag);
    this.canvas.addEventListener('pointercancel', endDrag);
    this.canvas.addEventListener(
      'wheel',
      (e) => {
        e.preventDefault();
        const factor = e.deltaY > 0 ? 1.1 : 1 / 1.1;
        this.camDist = Math.max(1.35, Math.min(8.5, this.camDist * factor));
      },
      { passive: false },
    );
  }

  public setMode(mode: ShipViewMode): void {
    this.mode = mode;
    this.syncModeVisibility();
  }

  public getMode(): ShipViewMode {
    return this.mode;
  }

  public resetCamera(): void {
    this.yaw = 0.65;
    this.pitch = 0.28;
    this.camDist = this.interactive ? 3.85 : 3.7;
  }

  public setState(next: ShipRenderState): void {
    const classChanged = this.loadedClass !== next.fleetClass;
    this.state = { ...next };
    this.applyUniforms();

    if (classChanged) {
      const seq = ++this.loadSeq;
      void loadShipAsset(next.fleetClass)
        .then((asset) => {
          if (seq !== this.loadSeq) return;
          this.loadedClass = next.fleetClass;
          this.attachGeometry(asset);
        })
        .catch(() => {
          // Keep previous geometry if a load fails
        });
    }
  }

  private attachGeometry(asset: CachedShipAsset): void {
    if (this.solidMesh) this.shipRoot.remove(this.solidMesh);
    if (this.wireMesh) this.shipRoot.remove(this.wireMesh);
    if (this.holoMesh) this.shipRoot.remove(this.holoMesh);

    this.solidMat.uniforms.uMap.value = asset.texture;
    this.solidMat.uniforms.uHasMap.value = asset.texture ? 1.0 : 0.0;

    this.solidMesh = new THREE.Mesh(asset.geometry, this.solidMat);
    this.wireMesh = new THREE.Mesh(asset.geometry, this.wireMat);
    this.holoMesh = new THREE.Mesh(asset.geometry, this.holoMat);

    this.shipRoot.add(this.solidMesh);
    this.shipRoot.add(this.wireMesh);
    this.shipRoot.add(this.holoMesh);

    const jr = Math.max(1.25, asset.radius * 1.18);
    this.jammerField.scale.setScalar(jr / 1.45);
    this.engineGlow.position.set(0, -0.02, -asset.radius * 0.85);

    this.applyUniforms();
    this.syncModeVisibility();
  }

  private applyUniforms(): void {
    const empCol = new THREE.Color(this.state.empireColor || '#38bdf8');
    const isFreight = this.state.fleetClass === 'freight';
    const cargoCol = isFreight
      ? new THREE.Color(MINERAL_COLORS[this.state.cargoMineralType ?? 0] ?? '#60a5fa')
      : empCol;

    this.solidMat.uniforms.uEmpireColor.value.copy(empCol);
    this.solidMat.uniforms.uAccentColor.value.copy(cargoCol);
    this.solidMat.uniforms.uHasCargoAccent.value = isFreight ? 1.0 : 0.0;
    this.solidMat.uniforms.uCloak.value = Math.max(0, Math.min(1, this.state.cloakPct));
    this.solidMat.uniforms.uHpRatio.value = Math.max(0, Math.min(1, this.state.hpRatio));
    this.solidMat.depthWrite = true;

    this.wireMat.color.copy(empCol);
    this.holoMat.uniforms.uEmpireColor.value.copy(empCol);
    this.engineGlowMat.color.copy(isFreight ? cargoCol : empCol);

    const hasJam = this.state.jammerPct > 0.01;
    this.jammerField.visible = hasJam;
    if (hasJam) {
      this.jammerMat.uniforms.uColor.value.copy(empCol).lerp(new THREE.Color('#c084fc'), 0.55);
      this.jammerMat.uniforms.uStrength.value = Math.max(0, Math.min(1, this.state.jammerPct));
    }
  }

  private syncModeVisibility(): void {
    const isBlueprint = this.mode === 'blueprint';
    if (this.solidMesh) this.solidMesh.visible = !isBlueprint;
    if (this.wireMesh) this.wireMesh.visible = isBlueprint;
    if (this.holoMesh) this.holoMesh.visible = isBlueprint;
    this.blueprintGrid.visible = isBlueprint;
  }

  public start(): void {
    if (!this.renderer || this.rafId !== null) return;
    this.startMs = performance.now();
    const tick = (now: number) => {
      this.rafId = requestAnimationFrame(tick);
      this.renderFrame((now - this.startMs) * 0.001);
    };
    this.rafId = requestAnimationFrame(tick);
  }

  public stop(): void {
    if (this.rafId !== null) {
      cancelAnimationFrame(this.rafId);
      this.rafId = null;
    }
  }

  private syncSize(): boolean {
    if (!this.renderer) return false;
    const rect = this.canvas.getBoundingClientRect();
    const w = Math.max(1, Math.round(rect.width));
    const h = Math.max(1, Math.round(rect.height));
    if (w !== this.lastW || h !== this.lastH) {
      this.lastW = w;
      this.lastH = h;
      this.renderer.setSize(w, h, false);
      this.camera.aspect = w / h;
      this.camera.updateProjectionMatrix();
    }
    return w > 1 && h > 1;
  }

  private renderFrame(t: number): void {
    if (!this.renderer || !this.syncSize()) return;

    if (!this.dragging) {
      this.yaw += this.interactive ? 0.0045 : 0.011;
    }

    const cp = Math.cos(this.pitch);
    this.camera.position.set(
      Math.sin(this.yaw) * cp * this.camDist,
      Math.sin(this.pitch) * this.camDist,
      Math.cos(this.yaw) * cp * this.camDist,
    );
    this.camera.lookAt(0, 0, 0);

    // Subtle idle float
    this.shipRoot.position.y = Math.sin(t * 1.8) * 0.045;
    this.solidMat.uniforms.uTime.value = t;

    // Thruster glow with low-HP flicker
    const hp = Math.max(0, Math.min(1, this.state.hpRatio));
    const dmg = 1 - hp;
    const flicker =
      dmg > 0.12
        ? 1 - dmg * 0.65 * (0.5 + 0.5 * Math.sin(t * 23.0) * Math.cos(t * 11.3))
        : 0.92 + 0.08 * Math.sin(t * 6.5);
    const cloakDim = 1 - this.state.cloakPct * 0.55;
    this.engineGlowMat.opacity = (this.mode === 'blueprint' ? 0.25 : 0.72) * flicker * cloakDim;
    const glowScale = (0.85 + 0.14 * flicker) * (0.65 + 0.35 * hp);
    this.engineGlow.scale.set(glowScale, glowScale, 1);

    // EW Jammer smooth Fresnel energy field
    if (this.jammerField.visible) {
      this.jammerField.rotation.y = -t * 0.25;
      this.jammerMat.uniforms.uTime.value = t;
    }

    this.stars.rotation.y = t * 0.012;
    this.renderer.render(this.scene, this.camera);
  }
}
