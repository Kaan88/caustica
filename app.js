'use strict';
/* ============================================================================
 *  Advanced Interactive Path-Traced Cornell Box Simulator
 *  WebGL2 + GLSL ES 3.00 · GPU path tracing · progressive accumulation
 * ========================================================================== */

/* ---------------------------------------------------------------- helpers -- */
const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
const lerp = (a, b, t) => a + (b - a) * t;

function hexToLinear(hex) {
  const n = parseInt(hex.slice(1), 16);
  const s = [(n >> 16) & 255, (n >> 8) & 255, n & 255].map(c => c / 255);
  const toLin = c => (c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4));
  return [toLin(s[0]), toLin(s[1]), toLin(s[2])];
}
/* Tanner Helland blackbody approximation -> linear RGB */
function kelvinToLinear(kelvin) {
  const t = clamp(kelvin, 1000, 12000) / 100;
  let r, g, b;
  r = t <= 66 ? 255 : 329.698727446 * Math.pow(t - 60, -0.1332047592);
  g = t <= 66 ? 99.4708025861 * Math.log(t) - 161.8392315709
              : 288.1221695283 * Math.pow(t - 60, -0.0755148492);
  b = t <= 66 ? (t <= 19 ? 0 : 138.5177312231 * Math.log(t - 10) - 305.0447923966) : 255;
  return hexToLinear('#' + [r, g, b].map(c => Math.round(clamp(c, 0, 255)).toString(16).padStart(2, '0')).join(''));
}
function mulRgb(a, m) { return [a[0] * m, a[1] * m, a[2] * m]; }

/* N x N tileable blue-noise masks by void-and-cluster (Ulichney 1993): pixel
   ranks scaled to (0, 1). Two independent masks are interleaved (R, G). */
function makeBlueNoise(N) {
  const n = N * N, R = 6, sigma = 1.9;
  const taps = [];                                   // truncated toroidal Gaussian
  for (let dy = -R; dy <= R; dy++) for (let dx = -R; dx <= R; dx++) {
    taps.push(dx, dy, Math.exp(-(dx * dx + dy * dy) / (2 * sigma * sigma)));
  }
  const mask = seed => {
    let st = seed >>> 0;
    const rand = () => {                             // mulberry32
      st = (st + 0x6D2B79F5) >>> 0;
      let t = Math.imul(st ^ (st >>> 15), st | 1);
      t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
    const E = new Float32Array(n), on = new Uint8Array(n), rank = new Float32Array(n);
    const splat = (p, sign) => {
      const px = p % N, py = (p / N) | 0;
      for (let k = 0; k < taps.length; k += 3) {
        E[((py + taps[k + 1] + N) % N) * N + (px + taps[k] + N) % N] += sign * taps[k + 2];
      }
    };
    /* tightest cluster (max energy among set pixels) or largest void (min among unset) */
    const extreme = (set, wantMax) => {
      let best = -1, bv = wantMax ? -Infinity : Infinity;
      for (let i = 0; i < n; i++) {
        if (on[i] !== set) continue;
        if (wantMax ? E[i] > bv : E[i] < bv) { bv = E[i]; best = i; }
      }
      return best;
    };
    /* initial pattern: 10% random points, relaxed until no point moves */
    const m = Math.round(n / 10);
    for (let placed = 0; placed < m;) {
      const p = (rand() * n) | 0;
      if (!on[p]) { on[p] = 1; splat(p, 1); placed++; }
    }
    for (let it = 0; it < n; it++) {
      const c = extreme(1, true);
      on[c] = 0; splat(c, -1);
      const v = extreme(0, false);
      on[v] = 1; splat(v, 1);
      if (v === c) break;
    }
    const onInit = on.slice(), eInit = E.slice();
    for (let r = m - 1; r >= 0; r--) {               // rank the initial points
      const c = extreme(1, true);
      on[c] = 0; splat(c, -1); rank[c] = r;
    }
    on.set(onInit); E.set(eInit);
    for (let r = m; r < n; r++) {                    // then fill the largest voids
      const v = extreme(0, false);
      on[v] = 1; splat(v, 1); rank[v] = r;
    }
    return rank.map(r => (r + 0.5) / n);
  };
  const a = mask(0x1234567), b = mask(0x89abcdef);
  const out = new Float32Array(n * 2);
  for (let i = 0; i < n; i++) { out[2 * i] = a[i]; out[2 * i + 1] = b[i]; }
  return out;
}
function degToRad(d) { return d * Math.PI / 180; }

/* --------------------------------------------------------------- settings -- */
const DEFAULTS = {
  /* scene */
  preset: 'classic',
  objAShape: 'box', objBShape: 'box',
  objASize: 1.0, objAYStretch: 1.7, objARotY: 18, objAX: -0.42, objAY: -0.405, objAZ: 0.05,
  objBSize: 0.62, objBYStretch: 0.62, objBRotY: -14, objBX: 0.38, objBY: -0.866, objBZ: -0.38,
  wallLeftColor: '#c0403a', wallRightColor: '#3f8f45', wallBaseColor: '#dedede',
  objAColor: '#e8e4dc', objBColor: '#d8d2c8',
  bgColor: '#0a0c11',
  /* lighting */
  lightOn: true,
  lightIntensity: 12, lightSize: 0.75, lightAspect: 1.0,
  lightTemp: 5200, lightTint: '#ffffff',
  lightX: 0.0, lightY: 0.965, lightZ: 0.15,
  directIntensity: 1.0, indirectIntensity: 1.0, glassShadow: true,
  /* materials (per target) */
  matTarget: 'A',
  mats: {
    A: { rough: 0.55, metal: 0.0, refl: 0.05, ior: 1.5, trans: 0.0, emis: 0.0 },
    B: { rough: 0.5, metal: 0.0, refl: 0.05, ior: 1.5, trans: 0.0, emis: 0.0 },
    walls: { rough: 0.85, metal: 0.0, refl: 0.0, ior: 1.5, trans: 0.0, emis: 0.0 },
    floor: { rough: 0.85, metal: 0.0, refl: 0.0, ior: 1.5, trans: 0.0, emis: 0.0 }
  },
  /* rendering */
  maxBounce: 6, spp: 2, resolution: 60,
  exposure: 1.0, gamma: 2.2, tonemap: 'aces',
  denoise: false, denoiseStrength: 0.5,
  maxSamples: 1024, firefly: 0, sampler: 'sobol',
  viewMode: 'beauty', compareMode: 'off', exportScale: 2,
  /* camera */
  camYaw: 0, camPitch: 4, camDistance: 3.35, camFov: 36, camHeight: 0.12,
  camTargetX: 0.0, camTargetZ: -0.15,
  autoRotate: false, autoRotateSpeed: 6, temporalReuse: true
};

let S = structuredClone(DEFAULTS);

const VIEW_MODES = [
  ['beauty', 'Beauty'], ['normal', 'Normals'], ['albedo', 'Albedo'], ['depth', 'Depth'],
  ['direct', 'Direct light'], ['indirect', 'Indirect light'], ['specular', 'Specular reflections'],
  ['convergence', 'Convergence noise']
];
const TONEMAPS = [
  ['none', 'None (linear)'], ['reinhard', 'Reinhard'], ['reinhardWhite', 'Reinhard (white)'],
  ['aces', 'ACES filmic'], ['filmic', 'Filmic (Uncharted 2)'], ['contrast', 'Filmic high-contrast']
];
const TONEMAP_INDEX = Object.fromEntries(TONEMAPS.map(([k], i) => [k, i]));
const MODE_INDEX = Object.fromEntries(VIEW_MODES.map(([k], i) => [k, i]));

const PRESETS = {
  classic: {
    objAShape: 'box', objBShape: 'box', objASize: 1.0, objAYStretch: 1.7, objARotY: 18,
    objAX: -0.42, objAY: -0.62, objAZ: 0.05, objBSize: 0.62, objBYStretch: 0.62, objBRotY: -14,
    objBX: 0.38, objBY: -0.81, objBZ: -0.38,
    wallLeftColor: '#c0403a', wallRightColor: '#3f8f45', wallBaseColor: '#dedede',
    objAColor: '#e8e4dc', objBColor: '#d8d2c8', bgColor: '#0a0c11',
    lightOn: true, lightIntensity: 12, lightSize: 0.75, lightTemp: 5200, lightTint: '#ffffff',
    mats: { A: { rough: 0.55, metal: 0, refl: 0.05, ior: 1.5, trans: 0, emis: 0 },
            B: { rough: 0.5, metal: 0, refl: 0.05, ior: 1.5, trans: 0, emis: 0 },
            walls: { rough: 0.85, metal: 0, refl: 0, ior: 1.5, trans: 0, emis: 0 },
            floor: { rough: 0.85, metal: 0, refl: 0, ior: 1.5, trans: 0, emis: 0 } },
    exposure: 1.0, tonemap: 'aces', maxBounce: 6
  },
  reflective: {
    objAShape: 'sphere', objBShape: 'sphere', objASize: 0.95, objBSize: 0.55,
    objAX: -0.38, objAY: -0.42, objAZ: 0.18, objBX: 0.45, objBY: -0.65, objBZ: -0.35,
    objAColor: '#f2f2f5', objBColor: '#e6c9a8',
    mats: { A: { rough: 0.02, metal: 1.0, refl: 1.0, ior: 1.5, trans: 0, emis: 0 },
            B: { rough: 0.18, metal: 0.85, refl: 0.9, ior: 1.5, trans: 0, emis: 0 },
            walls: { rough: 0.85, metal: 0, refl: 0, ior: 1.5, trans: 0, emis: 0 },
            floor: { rough: 0.45, metal: 0, refl: 0.35, ior: 1.5, trans: 0, emis: 0 } },
    wallBaseColor: '#d6d6d6', exposure: 1.0, maxBounce: 8
  },
  mirrorglass: {
    objAShape: 'sphere', objBShape: 'box', objASize: 0.9, objBSize: 0.75, objBYStretch: 1.0,
    objAX: -0.35, objAY: -0.45, objAZ: 0.2, objBX: 0.42, objBY: -0.6, objBZ: -0.3,
    objAColor: '#ffffff', objBColor: '#eaf3ff',
    mats: { A: { rough: 0.02, metal: 1.0, refl: 1.0, ior: 1.5, trans: 0, emis: 0 },
            B: { rough: 0.05, metal: 0, refl: 0.1, ior: 1.52, trans: 1.0, emis: 0 },
            walls: { rough: 0.85, metal: 0, refl: 0, ior: 1.5, trans: 0, emis: 0 },
            floor: { rough: 0.6, metal: 0, refl: 0.12, ior: 1.5, trans: 0, emis: 0 } },
    exposure: 1.1, maxBounce: 10
  },
  spherebox: {
    objAShape: 'sphere', objBShape: 'box', objASize: 0.95, objBSize: 0.7, objBYStretch: 1.65, objBRotY: -20,
    objAX: -0.4, objAZ: 0.18, objBX: 0.4, objBZ: -0.32,
    objAColor: '#e9e5dd', objBColor: '#d8d2c8'
  },
  mirror: {
    /* turned so the mirrors face the coloured walls; facing the camera they would
       only reflect the dark open front of the box */
    objARotY: 40, objBRotY: -25,
    objAColor: '#f2f2f4', objBColor: '#e6e6ea', maxBounce: 10,
    mats: { A: { rough: 0.02, metal: 1.0, refl: 1.0, ior: 1.5, trans: 0, emis: 0 },
            B: { rough: 0.02, metal: 1.0, refl: 1.0, ior: 1.5, trans: 0, emis: 0 },
            walls: { rough: 0.85, metal: 0, refl: 0, ior: 1.5, trans: 0, emis: 0 },
            floor: { rough: 0.85, metal: 0, refl: 0, ior: 1.5, trans: 0, emis: 0 } }
  },
  glass: {
    objAShape: 'sphere', objBShape: 'box', objASize: 0.95, objBSize: 0.75, objBYStretch: 1.0, objBRotY: 22,
    objAX: -0.38, objAZ: 0.15, objBX: 0.42, objBZ: -0.3,
    objAColor: '#f4f8ff', objBColor: '#d9efe6', maxBounce: 12,
    mats: { A: { rough: 0.0, metal: 0, refl: 0, ior: 1.5, trans: 1.0, emis: 0 },
            B: { rough: 0.0, metal: 0, refl: 0, ior: 1.5, trans: 1.0, emis: 0 },
            walls: { rough: 0.85, metal: 0, refl: 0, ior: 1.5, trans: 0, emis: 0 },
            floor: { rough: 0.85, metal: 0, refl: 0, ior: 1.5, trans: 0, emis: 0 } }
  },
  warm: {
    lightIntensity: 18, lightSize: 0.9, lightTemp: 3200, lightTint: '#ffffff',
    wallLeftColor: '#b34036', wallRightColor: '#4c7f4a', wallBaseColor: '#e3dccf',
    objAColor: '#f0e6d6', objBColor: '#cfd6e0', exposure: 1.15, tonemap: 'filmic',
    mats: { A: { rough: 0.62, metal: 0, refl: 0.05, ior: 1.5, trans: 0, emis: 0 },
            B: { rough: 0.4, metal: 0.15, refl: 0.1, ior: 1.5, trans: 0, emis: 0 },
            walls: { rough: 0.9, metal: 0, refl: 0, ior: 1.5, trans: 0, emis: 0 },
            floor: { rough: 0.85, metal: 0, refl: 0, ior: 1.5, trans: 0, emis: 0 } }
  },
  dramatic: {
    lightIntensity: 26, lightSize: 0.35, lightTemp: 4300, lightTint: '#ffffff',
    lightY: 0.98, lightZ: -0.1, bgColor: '#05060a',
    wallLeftColor: '#8f2f28', wallRightColor: '#2f6b37', wallBaseColor: '#c9c9c9',
    objAColor: '#f5f5f5', objBColor: '#b9c2cc', exposure: 1.25, tonemap: 'contrast',
    maxBounce: 8, denoise: true,
    mats: { A: { rough: 0.35, metal: 0, refl: 0.18, ior: 1.5, trans: 0, emis: 0 },
            B: { rough: 0.15, metal: 1.0, refl: 1.0, ior: 1.5, trans: 0, emis: 0 },
            walls: { rough: 0.9, metal: 0, refl: 0, ior: 1.5, trans: 0, emis: 0 },
            floor: { rough: 0.7, metal: 0, refl: 0.08, ior: 1.5, trans: 0, emis: 0 } }
  },
  emissive: {
    objAShape: 'sphere', objBShape: 'sphere', objASize: 0.55, objBSize: 0.85,
    objAX: -0.45, objAY: -0.45, objAZ: 0.1, objBX: 0.42, objBY: -0.35, objBZ: -0.25,
    objAColor: '#ffcf7a', objBColor: '#7fd8ff',
    lightIntensity: 3, lightSize: 0.5, lightTemp: 6500,
    mats: { A: { rough: 0.4, metal: 0, refl: 0.1, ior: 1.5, trans: 0, emis: 6.0 },
            B: { rough: 0.2, metal: 0, refl: 0.25, ior: 1.6, trans: 0.55, emis: 0 },
            walls: { rough: 0.85, metal: 0, refl: 0, ior: 1.5, trans: 0, emis: 0 },
            floor: { rough: 0.85, metal: 0, refl: 0, ior: 1.5, trans: 0, emis: 0 } },
    exposure: 1.0, maxBounce: 7
  }
};

/* ------------------------------------------------------------------- UI ---- */
const SCHEMA = [
  {
    id: 'scene', title: 'Scene', open: true, items: [
      { key: 'preset', label: 'Scene preset', type: 'select', options: [
        ['classic', 'Classic Cornell Box'], ['reflective', 'Two reflective spheres'],
        ['spherebox', 'Sphere and box'], ['mirrorglass', 'Mirror + glass'],
        ['mirror', 'Mirror'], ['glass', 'Glass'], ['warm', 'Warm lighting'],
        ['dramatic', 'Dramatic lighting'], ['emissive', 'Emissive spheres'],
        ['custom', 'Custom (edited by hand)']
      ], scope: 'scene', action: 'applyPreset' },
      { key: 'objAShape', label: 'Object A shape', type: 'select', scope: 'scene',
        options: [['box', 'Box'], ['sphere', 'Sphere'], ['none', 'None']] },
      { key: 'objBShape', label: 'Object B shape', type: 'select', scope: 'scene',
        options: [['box', 'Box'], ['sphere', 'Sphere'], ['none', 'None']] },
      { key: 'objAX', label: 'A position X', type: 'range', min: -1, max: 1, step: 0.01, scope: 'scene' },
      { key: 'objAY', label: 'A position Y', type: 'range', min: -1, max: 1, step: 0.01, scope: 'scene' },
      { key: 'objAZ', label: 'A position Z', type: 'range', min: -1, max: 1, step: 0.01, scope: 'scene' },
      { key: 'objASize', label: 'A size', type: 'range', min: 0.1, max: 1.6, step: 0.01, scope: 'scene' },
      { key: 'objAYStretch', label: 'A height scale', type: 'range', min: 0.2, max: 2.6, step: 0.01, scope: 'scene',
        only: 'box' },
      { key: 'objARotY', label: 'A rotation Y', type: 'range', min: -180, max: 180, step: 1, unit: '°', scope: 'scene',
        only: 'box' },
      { key: 'objBX', label: 'B position X', type: 'range', min: -1, max: 1, step: 0.01, scope: 'scene' },
      { key: 'objBY', label: 'B position Y', type: 'range', min: -1, max: 1, step: 0.01, scope: 'scene' },
      { key: 'objBZ', label: 'B position Z', type: 'range', min: -1, max: 1, step: 0.01, scope: 'scene' },
      { key: 'objBSize', label: 'B size', type: 'range', min: 0.1, max: 1.6, step: 0.01, scope: 'scene' },
      { key: 'objBYStretch', label: 'B height scale', type: 'range', min: 0.2, max: 2.6, step: 0.01, scope: 'scene',
        only: 'box' },
      { key: 'objBRotY', label: 'B rotation Y', type: 'range', min: -180, max: 180, step: 1, unit: '°', scope: 'scene',
        only: 'box' },
      { key: 'wallLeftColor', label: 'Left wall (red side)', type: 'color', scope: 'scene' },
      { key: 'wallRightColor', label: 'Right wall (green side)', type: 'color', scope: 'scene' },
      { key: 'wallBaseColor', label: 'Floor / ceiling / back', type: 'color', scope: 'scene' },
      { key: 'objAColor', label: 'Object A base color', type: 'color', scope: 'scene' },
      { key: 'objBColor', label: 'Object B base color', type: 'color', scope: 'scene' },
      { key: 'bgColor', label: 'Environment color', type: 'color', scope: 'scene' }
    ]
  },
  {
    id: 'light', title: 'Lighting', open: true, items: [
      { key: 'lightOn', label: 'Area light enabled', type: 'bool', scope: 'scene' },
      { key: 'lightIntensity', label: 'Light intensity', type: 'range', min: 0, max: 50, step: 0.1, scope: 'scene' },
      { key: 'lightSize', label: 'Light size', type: 'range', min: 0.1, max: 2.0, step: 0.01, scope: 'scene' },
      { key: 'lightAspect', label: 'Light aspect', type: 'range', min: 0.4, max: 2.5, step: 0.01, scope: 'scene' },
      { key: 'lightTemp', label: 'Color temperature', type: 'range', min: 2000, max: 10000, step: 50,
        unit: ' K', fmt: v => Math.round(v), scope: 'scene' },
      { key: 'lightTint', label: 'Light color', type: 'color', scope: 'scene' },
      { key: 'lightX', label: 'Light position X', type: 'range', min: -0.85, max: 0.85, step: 0.01, scope: 'scene' },
      { key: 'lightY', label: 'Light position Y', type: 'range', min: -0.9, max: 0.995, step: 0.005, scope: 'scene' },
      { key: 'lightZ', label: 'Light position Z', type: 'range', min: -0.85, max: 0.85, step: 0.01, scope: 'scene' },
      { key: 'directIntensity', label: 'Direct light intensity', type: 'range', min: 0, max: 3, step: 0.01,
        scope: 'view' },
      { key: 'indirectIntensity', label: 'Indirect light intensity', type: 'range', min: 0, max: 3, step: 0.01,
        scope: 'view' },
      { key: 'glassShadow', label: 'Glass casts shadows', type: 'bool', scope: 'scene' }
    ]
  },
  {
    id: 'material', title: 'Materials', open: true, items: [
      { key: 'matTarget', label: 'Editing', type: 'select', scope: 'none', action: 'matTarget',
        options: [['A', 'Object A'], ['B', 'Object B'], ['walls', 'Side walls'], ['floor', 'Floor / ceiling / back']] },
      { key: 'rough', label: 'Roughness', type: 'range', min: 0, max: 1, step: 0.01, scope: 'scene', mat: 'rough' },
      { key: 'metal', label: 'Metallic', type: 'range', min: 0, max: 1, step: 0.01, scope: 'scene', mat: 'metal',
        noWalls: true },
      { key: 'refl', label: 'Reflectivity', type: 'range', min: 0, max: 1, step: 0.01, scope: 'scene', mat: 'refl' },
      { key: 'ior', label: 'IOR', type: 'range', min: 1.0, max: 2.5, step: 0.01, scope: 'scene', mat: 'ior',
        noWalls: true },
      { key: 'trans', label: 'Transmission', type: 'range', min: 0, max: 1, step: 0.01, scope: 'scene', mat: 'trans',
        noWalls: true },
      { key: 'emis', label: 'Emission strength', type: 'range', min: 0, max: 20, step: 0.1, scope: 'scene',
        mat: 'emis', noWalls: true },
      { type: 'info', text: 'Roughness and IOR shape the reflective and refractive part of a surface. With Metallic, Reflectivity and Transmission all at 0 the surface is purely diffuse, so they have no visible effect.' }
    ]
  },
  {
    id: 'render', title: 'Rendering', open: true, items: [
      { key: 'maxBounce', label: 'Max bounces', type: 'range', min: 1, max: 16, step: 1, scope: 'trace',
        fmt: v => String(Math.round(v)) },
      { key: 'spp', label: 'Samples per frame', type: 'range', min: 1, max: 16, step: 1, scope: 'view',
        fmt: v => String(Math.round(v)) },
      { key: 'maxSamples', label: 'Max accumulated samples', type: 'range', min: 16, max: 8192, step: 16,
        scope: 'view', fmt: v => String(Math.round(v)) },
      { key: 'resolution', label: 'Render resolution', type: 'range', min: 25, max: 100, step: 5, unit: '%',
        scope: 'res' },
      { key: 'exposure', label: 'Exposure', type: 'range', min: 0.05, max: 4, step: 0.01, scope: 'view' },
      { key: 'gamma', label: 'Gamma', type: 'range', min: 1.6, max: 2.8, step: 0.01, scope: 'view' },
      { key: 'tonemap', label: 'Tone mapping', type: 'select', scope: 'view', options: TONEMAPS },
      { key: 'viewMode', label: 'View mode', type: 'select', scope: 'view', options: VIEW_MODES },
      { key: 'denoise', label: 'Denoise (bilateral)', type: 'bool', scope: 'view' },
      { key: 'denoiseStrength', label: 'Denoise strength', type: 'range', min: 0, max: 1, step: 0.01, scope: 'view' },
      { key: 'firefly', label: 'Firefly clamp', type: 'range', min: 0, max: 12, step: 0.1, scope: 'trace',
        fmt: v => (v <= 0 ? 'off' : v.toFixed(1) + '×') },
      { key: 'sampler', label: 'Sampling pattern', type: 'select', scope: 'trace', options: [
        ['sobol', 'Sobol + blue noise'], ['random', 'Random (white noise)']
      ] }
    ]
  },
  {
    id: 'camera', title: 'Camera', open: false, items: [
      { key: 'camYaw', label: 'Orbit angle', type: 'range', min: -180, max: 180, step: 0.5, unit: '°', scope: 'camera' },
      { key: 'camPitch', label: 'Elevation', type: 'range', min: -35, max: 82, step: 0.5, unit: '°', scope: 'camera' },
      { key: 'camDistance', label: 'Distance', type: 'range', min: 1.2, max: 9, step: 0.01, scope: 'camera' },
      { key: 'camFov', label: 'Field of view', type: 'range', min: 15, max: 90, step: 0.5, unit: '°', scope: 'camera' },
      { key: 'camHeight', label: 'Height', type: 'range', min: -0.9, max: 1.6, step: 0.01, scope: 'camera' },
      { key: 'camTargetX', label: 'Target X', type: 'range', min: -1, max: 1, step: 0.01, scope: 'camera' },
      { key: 'camTargetZ', label: 'Target Z', type: 'range', min: -1, max: 1, step: 0.01, scope: 'camera' },
      { key: 'autoRotate', label: 'Auto-rotate', type: 'bool', scope: 'view' },
      { key: 'temporalReuse', label: 'Reuse samples while moving', type: 'bool', scope: 'view' },
      { key: 'autoRotateSpeed', label: 'Auto-rotate speed', type: 'range', min: 1, max: 40, step: 0.5,
        unit: '°/s', scope: 'view' },
      { type: 'button', label: 'Reset camera', action: 'resetCamera' }
    ]
  },
  {
    id: 'compare', title: 'A/B comparison', open: false, items: [
      { type: 'button', label: 'Capture current as reference', action: 'captureRef' },
      { key: 'compareMode', label: 'Comparison', type: 'select', scope: 'view', options: [
        ['off', 'Off'], ['split', 'Split (reference left)'], ['reference', 'Reference only'],
        ['diff', 'Difference heatmap']
      ] },
      { type: 'info', text: 'Captures the Beauty view as shown, denoise and light sliders included. Exposure, tone mapping and gamma stay live on both sides, so you can capture a look, change settings and compare instantly. The comparison shows in the Beauty view.' }
    ]
  },
  {
    id: 'actions', title: 'Controls & export', open: true, items: [
      { type: 'button', label: 'Pause / Resume', action: 'togglePause', key: 'btnPause' },
      { type: 'button', label: 'Reset accumulation', action: 'resetAccum' },
      { type: 'button', label: 'Screenshot (PNG)', action: 'screenshot' },
      { key: 'exportScale', label: 'Export scale', type: 'select', scope: 'none', options: [
        ['1', '1× render resolution'], ['2', '2× render resolution'], ['3', '3×'], ['4', '4× (max 4096 px)']
      ] },
      { type: 'button', label: 'Export high-res PNG', action: 'exportPNG' },
      { type: 'button', label: 'Randomize scene', action: 'randomize' },
      { type: 'button', label: 'Reset everything', action: 'resetAll' },
      { type: 'button', label: 'Save settings (JSON)', action: 'saveJSON' },
      { type: 'button', label: 'Load settings (JSON)', action: 'loadJSON' },
      { type: 'info', text: 'Drag = orbit · Wheel / pinch = zoom · Right-drag, Shift-drag or two-finger drag = pan · Space = pause · R = reset samples · A = auto-rotate · C = capture reference · D = denoise · 1-8 = view modes.' }
    ]
  }
];

/* ------------------------------------------------------------ webgl state -- */
let gl = null, glOK = false, hdrInternal = null, hdrType = null, nTargets = 0;
let progTrace = null, progDenoise = null, progDisplay = null, progPrep = null, progHistory = null;
let vao = null, rafId = 0;
let blueNoiseTex = null, blueNoiseData = null;
/* camera-motion history (display-only, see FS_HISTORY) */
let histTex = [null, null], histFbo = [null, null], hc = 0;
let histCam = null, moveFrom = null, moveOldSamples = 0;
const HISTORY_CAP = 24;      // most samples' worth of weight the history may carry
const HISTORY_FADE = 256;    // fresh samples over which it fades out
/* Sobol index of the accumulation's first sample: 0 after a reset, a fresh
   multiple of 8192 after each camera move so frames never reuse points */
let seqBase = 0, moveCount = 0;
let timerExt = null, gpuQueries = [], gpuMsEMA = 0, lastDisplaySig = '';
let sets = [null, null], fbos = [null, null];
let dnTex = [null, null], dnFbo = [null, null];
let refTex = null, refFbo = null;
let expTex = null, expFbo = null;
let cur = 0, samples = 0, paused = false, needsReset = true, sceneDirty = true;
let hasRef = false, statusText = 'Tracing';
let internalW = 0, internalH = 0;
let frameMsEMA = 0, fpsEMA = 0;
let gpuName = 'unknown GPU';
let dragging = false, dragMode = 'orbit';
let pointers = new Map();
let pinchDist = 0;
let lastTime = performance.now();

const canvas = document.getElementById('viewport');
const hud = document.getElementById('hud');
const overlay = document.getElementById('overlay');

const escapeHtml = s => String(s).replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);

/* msg is trusted HTML; anything from a driver log or an exception goes through escapeHtml */
function fatal(msg) {
  overlay.style.display = 'grid';
  overlay.innerHTML = '<div class="fatal">' + msg + '</div>';
  glOK = false;
}

/* ------------------------------------------------------------- gl helpers -- */
function compile(name, fsSrc) {
  const vs = gl.createShader(gl.VERTEX_SHADER);
  gl.shaderSource(vs, VS_QUAD);
  gl.compileShader(vs);
  if (!gl.getShaderParameter(vs, gl.COMPILE_STATUS)) {
    throw new Error('vertex shader (' + name + '): ' + gl.getShaderInfoLog(vs));
  }
  const fs = gl.createShader(gl.FRAGMENT_SHADER);
  gl.shaderSource(fs, fsSrc);
  gl.compileShader(fs);
  if (!gl.getShaderParameter(fs, gl.COMPILE_STATUS)) {
    throw new Error('fragment shader (' + name + '): ' + gl.getShaderInfoLog(fs));
  }
  const p = gl.createProgram();
  gl.attachShader(p, vs); gl.attachShader(p, fs);
  gl.linkProgram(p);
  if (!gl.getProgramParameter(p, gl.LINK_STATUS)) {
    throw new Error('program (' + name + '): ' + gl.getProgramInfoLog(p));
  }
  gl.deleteShader(vs); gl.deleteShader(fs);
  const u = {};
  const n = gl.getProgramParameter(p, gl.ACTIVE_UNIFORMS);
  for (let i = 0; i < n; i++) {
    const info = gl.getActiveUniform(p, i);
    const base = info.name.replace(/\[0\]$/, '');
    u[base] = gl.getUniformLocation(p, info.name);
  }
  return { p, u, name };
}

function makeTex(w, h, internal, type, filter) {
  const t = gl.createTexture();
  gl.bindTexture(gl.TEXTURE_2D, t);
  gl.texImage2D(gl.TEXTURE_2D, 0, internal, w, h, 0, gl.RGBA, type, null);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, filter);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, filter);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  return t;
}
function bindTex(prog, uniformName, tex, unit) {
  const loc = prog.u[uniformName];
  if (loc === undefined || loc === null) return;
  gl.activeTexture(gl.TEXTURE0 + unit);
  gl.bindTexture(gl.TEXTURE_2D, tex);
  gl.uniform1i(loc, unit);
}
function allocateBuffers(w, h) {
  internalW = Math.max(16, w); internalH = Math.max(16, h);
  for (let i = 0; i < 2; i++) {
    if (sets[i]) sets[i].forEach(t => gl.deleteTexture(t));
    if (fbos[i]) gl.deleteFramebuffer(fbos[i]);
    const texs = [];
    for (let k = 0; k < 5; k++) texs.push(makeTex(internalW, internalH, hdrInternal, hdrType, gl.NEAREST));
    const fb = gl.createFramebuffer();
    gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
    /* with 4 draw buffers the 5th (specular) texture stays unattached and reads as 0 */
    const attach = [];
    for (let k = 0; k < nTargets; k++) {
      attach.push(gl.COLOR_ATTACHMENT0 + k);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0 + k, gl.TEXTURE_2D, texs[k], 0);
    }
    gl.drawBuffers(attach);
    const st = gl.checkFramebufferStatus(gl.FRAMEBUFFER);
    if (st !== gl.FRAMEBUFFER_COMPLETE) {
      fatal('Framebuffer is incomplete (' + st + '). Float render targets may not be supported here.');
      return false;
    }
    sets[i] = texs; fbos[i] = fb;
  }
  for (let i = 0; i < 2; i++) {
    if (dnTex[i]) gl.deleteTexture(dnTex[i]);
    if (dnFbo[i]) gl.deleteFramebuffer(dnFbo[i]);
    dnTex[i] = makeTex(internalW, internalH, hdrInternal, hdrType, gl.NEAREST);
    dnFbo[i] = gl.createFramebuffer();
    gl.bindFramebuffer(gl.FRAMEBUFFER, dnFbo[i]);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, dnTex[i], 0);
    gl.drawBuffers([gl.COLOR_ATTACHMENT0]);
  }
  for (let i = 0; i < 2; i++) {
    if (histTex[i]) gl.deleteTexture(histTex[i]);
    if (histFbo[i]) gl.deleteFramebuffer(histFbo[i]);
    histTex[i] = makeTex(internalW, internalH, hdrInternal, hdrType, gl.NEAREST);
    histFbo[i] = gl.createFramebuffer();
    gl.bindFramebuffer(gl.FRAMEBUFFER, histFbo[i]);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, histTex[i], 0);
    gl.drawBuffers([gl.COLOR_ATTACHMENT0]);
  }
  if (refTex) gl.deleteTexture(refTex);
  if (refFbo) gl.deleteFramebuffer(refFbo);
  refTex = makeTex(internalW, internalH, hdrInternal, hdrType, gl.NEAREST);
  refFbo = gl.createFramebuffer();
  gl.bindFramebuffer(gl.FRAMEBUFFER, refFbo);
  gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, refTex, 0);
  gl.drawBuffers([gl.COLOR_ATTACHMENT0]);
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  hasRef = false;
  return true;
}

function resetAccumulation() {
  samples = 0; cur = 0;
  gl.bindFramebuffer(gl.FRAMEBUFFER, fbos[0]);
  gl.drawBuffers([gl.COLOR_ATTACHMENT0, gl.COLOR_ATTACHMENT1, gl.COLOR_ATTACHMENT2,
                  gl.COLOR_ATTACHMENT3, gl.COLOR_ATTACHMENT4].slice(0, nTargets));
  gl.viewport(0, 0, internalW, internalH);
  gl.clearColor(0, 0, 0, 0);
  gl.clear(gl.COLOR_BUFFER_BIT);
  for (const fb of histFbo) {                      // no history across a scene change
    if (!fb) continue;
    gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
    gl.clear(gl.COLOR_BUFFER_BIT);
  }
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  needsReset = false;
  seqBase = 0; moveFrom = null;
}

/* Camera moved with temporal reuse on: restart the accumulation without
   clearing it (the trace ignores old contents when samples is 0), so the
   history pass can still reproject the previous image afterwards. */
function beginMove() {
  moveFrom = histCam;
  moveOldSamples = samples;
  samples = 0;
  moveCount++;
  seqBase = (Math.imul(moveCount, 0x9E3779B1) & ~8191) >>> 0;
}
const snapshotCamera = () => ({ o: CAM.o.slice(), r: CAM.r.slice(), u: CAM.u.slice(), f: CAM.f.slice() });
const sameCamera = (a, b) => ['o', 'r', 'u', 'f'].every(k => a[k][0] === b[k][0] && a[k][1] === b[k][1] && a[k][2] === b[k][2]);

/* ------------------------------------------------------------ init webgl --- */
function initGL() {
  if (!window.WebGL2RenderingContext) {
    fatal('WebGL2 is not available in this browser. Use a recent Chrome, Edge, Firefox or Safari.');
    return false;
  }
  try {
    gl = canvas.getContext('webgl2', {
      antialias: false, alpha: false, depth: false, stencil: false,
      preserveDrawingBuffer: true, powerPreference: 'high-performance'
    });
  } catch (e) { gl = null; }
  if (!gl) { fatal('Could not create a WebGL2 context.'); return false; }
  return setupGL();
}

/* Extensions, programs and fixed resources. Runs again after a lost context is
   restored, since every GL object from before the loss is dead. */
function setupGL() {
  const floatRT = gl.getExtension('EXT_color_buffer_float');
  const halfRT = gl.getExtension('EXT_color_buffer_half_float');
  if (floatRT) { hdrInternal = gl.RGBA32F; hdrType = gl.FLOAT; }
  else if (halfRT) { hdrInternal = gl.RGBA16F; hdrType = gl.HALF_FLOAT; }
  else {
    fatal('This GPU/driver cannot render to floating-point textures (EXT_color_buffer_float / half_float missing). Path tracing needs float render targets.');
    return false;
  }
  nTargets = Math.min(5, gl.getParameter(gl.MAX_DRAW_BUFFERS) || 1);
  if (nTargets < 4) { fatal('This GPU exposes fewer than 4 draw buffers, which WebGL2 requires.'); return false; }

  const dbg = gl.getExtension('WEBGL_debug_renderer_info');
  gpuName = dbg ? (gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL) || gpuName) : gl.getParameter(gl.RENDERER);

  /* GPU time for the HUD, where the driver exposes it */
  timerExt = gl.getExtension('EXT_disjoint_timer_query_webgl2');
  gpuQueries = []; gpuMsEMA = 0;

  try {
    /* WebGL2 only guarantees 4 draw buffers; without a 5th the specular AOV is dropped */
    const traceSrc = FS_TRACE.replace('#version 300 es\n',
      '#version 300 es\n#define SPEC_TARGET ' + (nTargets >= 5 ? 1 : 0) + '\n');
    progTrace = compile('trace', traceSrc);
    progDenoise = compile('denoise', FS_DENOISE);
    progPrep = compile('svgf-prep', FS_SVGF_PREP);
    progHistory = compile('history', FS_HISTORY);
    progDisplay = compile('display', FS_DISPLAY);
  } catch (e) { fatal('Shader compilation failed:<pre>' + escapeHtml(e.message) + '</pre>'); return false; }

  vao = gl.createVertexArray();
  gl.bindVertexArray(vao);

  /* blue-noise masks for the Sobol sampler (computed once, ~0.1 s) */
  blueNoiseData = blueNoiseData || makeBlueNoise(64);
  blueNoiseTex = gl.createTexture();
  gl.bindTexture(gl.TEXTURE_2D, blueNoiseTex);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RG32F, 64, 64, 0, gl.RG, gl.FLOAT, blueNoiseData);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);

  /* size-dependent buffers are (re)made by resizeIfNeeded() */
  sets = [null, null]; fbos = [null, null];
  dnTex = [null, null]; dnFbo = [null, null];
  histTex = [null, null]; histFbo = [null, null]; hc = 0; histCam = null; moveFrom = null;
  refTex = null; refFbo = null; hasRef = false;

  expTex = makeTex(4, 4, gl.RGBA8, gl.UNSIGNED_BYTE, gl.NEAREST);
  expFbo = gl.createFramebuffer();
  gl.bindFramebuffer(gl.FRAMEBUFFER, expFbo);
  gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, expTex, 0);
  gl.drawBuffers([gl.COLOR_ATTACHMENT0]);
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);

  gl.disable(gl.DEPTH_TEST);
  gl.disable(gl.BLEND);
  glOK = true;
  return true;
}

/* --------------------------------------------------------------- scene ---- */
const MAX_OBJ = 12;
const SCENE = {
  count: 0,
  type: new Int32Array(MAX_OBJ),
  c: new Float32Array(MAX_OBJ * 3),
  h: new Float32Array(MAX_OBJ * 3),
  rad: new Float32Array(MAX_OBJ),
  rot: new Float32Array(MAX_OBJ * 9),
  alb: new Float32Array(MAX_OBJ * 3),
  emis: new Float32Array(MAX_OBJ * 3),
  rough: new Float32Array(MAX_OBJ),
  metal: new Float32Array(MAX_OBJ),
  refl: new Float32Array(MAX_OBJ),
  ior: new Float32Array(MAX_OBJ),
  trans: new Float32Array(MAX_OBJ)
};

function rotY(deg) {
  const a = degToRad(deg), s = Math.sin(a), c = Math.cos(a);
  return [c, 0, -s, 0, 1, 0, s, 0, c];
}
function addObj(i, spec) {
  const alb = spec.albedo || [1, 1, 1];
  const emis = Array.isArray(spec.emis) ? spec.emis : mulRgb(alb, Number(spec.emis) || 0);
  SCENE.type[i] = spec.sphere ? 1 : 0;
  SCENE.c.set(spec.pos, i * 3);
  SCENE.h.set(spec.half || [0.35, 0.35, 0.35], i * 3);
  SCENE.rad[i] = spec.rad || 0.35;
  SCENE.rot.set(spec.rot || [1, 0, 0, 0, 1, 0, 0, 0, 1], i * 9);
  SCENE.alb.set(alb, i * 3);
  SCENE.emis.set(emis, i * 3);
  SCENE.rough[i] = spec.rough ?? 0.5;
  SCENE.metal[i] = spec.metal ?? 0;
  SCENE.refl[i] = spec.refl ?? 0;
  SCENE.ior[i] = spec.ior ?? 1.5;
  SCENE.trans[i] = spec.trans ?? 0;
}

function buildScene() {
  const wallMat = S.mats.walls, floorMat = S.mats.floor;
  const left = hexToLinear(S.wallLeftColor), right = hexToLinear(S.wallRightColor), base = hexToLinear(S.wallBaseColor);
  let i = 0;
  addObj(i++, { sphere: false, pos: [-1.1, 0, 0], half: [0.1, 1.1, 1.1], albedo: left, ...wallMat });
  addObj(i++, { sphere: false, pos: [1.1, 0, 0], half: [0.1, 1.1, 1.1], albedo: right, ...wallMat });
  addObj(i++, { sphere: false, pos: [0, -1.1, 0], half: [1.1, 0.1, 1.1], albedo: base, ...floorMat });
  addObj(i++, { sphere: false, pos: [0, 1.1, 0], half: [1.1, 0.1, 1.1], albedo: base, ...floorMat });
  addObj(i++, { sphere: false, pos: [0, 0, -1.1], half: [1.1, 1.1, 0.1], albedo: base, ...floorMat });
  SCENE.count = i;

  const mA = S.mats.A, mB = S.mats.B;
  if (S.objAShape !== 'none') {
    const col = hexToLinear(S.objAColor);
    const emis = mulRgb(col, S.mats.A.emis);
    if (S.objAShape === 'sphere') {
      const r = clamp(0.35 * S.objASize, 0.03, 1.2);
      addObj(SCENE.count++, { sphere: true, pos: [S.objAX, S.objAY, S.objAZ], rad: r,
                              albedo: col, emis, ...mA });
    } else {
      const hx = clamp(0.35 * S.objASize, 0.02, 1.2);
      const hy = clamp(0.35 * S.objASize * S.objAYStretch, 0.02, 1.6);
      addObj(SCENE.count++, { sphere: false, pos: [S.objAX, S.objAY, S.objAZ], half: [hx, hy, hx],
                             rot: rotY(S.objARotY), albedo: col, emis, ...mA });
    }
  }
  if (S.objBShape !== 'none') {
    const col = hexToLinear(S.objBColor);
    const emis = mulRgb(col, S.mats.B.emis);
    if (S.objBShape === 'sphere') {
      const r = clamp(0.35 * S.objBSize, 0.03, 1.2);
      addObj(SCENE.count++, { sphere: true, pos: [S.objBX, S.objBY, S.objBZ], rad: r,
                              albedo: col, emis, ...mB });
    } else {
      const hx = clamp(0.35 * S.objBSize, 0.02, 1.2);
      const hy = clamp(0.35 * S.objBSize * S.objBYStretch, 0.02, 1.6);
      addObj(SCENE.count++, { sphere: false, pos: [S.objBX, S.objBY, S.objBZ], half: [hx, hy, hx],
                              rot: rotY(S.objBRotY), albedo: col, emis, ...mB });
    }
  }
  sceneDirty = false;
}

/* -------------------------------------------------------------- camera ---- */
const CAM = { o: [0, 0, 0], r: [0, 0, 0], u: [0, 0, 0], f: [0, 0, 0], tan: 1 };

/* Y of an object resting on the floor (interior floor surface is y = -1) */
function restY(shape, size, stretch) {
  if (shape === 'sphere') return -1 + clamp(0.35 * size, 0.03, 1.2);
  return -1 + clamp(0.35 * size * stretch, 0.02, 1.6);
}
/* Drop both objects onto the floor so the sliders and the render agree. */
function settleObjects() {
  if (S.objAShape !== 'none') S.objAY = +restY(S.objAShape, S.objASize, S.objAYStretch).toFixed(3);
  if (S.objBShape !== 'none') S.objBY = +restY(S.objBShape, S.objBSize, S.objBYStretch).toFixed(3);
}
/* Camera distance whose frustum just fits through the open front face. */
function fitDistance(aspect) {
  const tan = Math.tan(degToRad(S.camFov) * 0.5);
  return clamp(1.02 / (tan * Math.max(aspect, 0.5)) + 1 - S.camTargetZ, 1.2, 9);
}
function buildCamera(aspect) {
  const yaw = degToRad(S.camYaw), pitch = degToRad(S.camPitch);
  const tx = S.camTargetX, ty = S.camHeight, tz = S.camTargetZ;
  const cp = Math.cos(pitch), sp = Math.sin(pitch);
  const o = [tx + S.camDistance * Math.sin(yaw) * cp, ty + S.camDistance * sp, tz + S.camDistance * Math.cos(yaw) * cp];
  let f = [tx - o[0], ty - o[1], tz - o[2]];
  const fl = Math.hypot(f[0], f[1], f[2]) || 1; f = f.map(v => v / fl);
  let r = [f[1] * 0 - f[2] * 1, f[2] * 0 - f[0] * 0, f[0] * 1 - f[1] * 0];   // cross(f, worldUp=(0,1,0))
  const rl = Math.hypot(r[0], r[1], r[2]) || 1; r = r.map(v => v / rl);
  const u = [r[1] * f[2] - r[2] * f[1], r[2] * f[0] - r[0] * f[2], r[0] * f[1] - r[1] * f[0]];
  const tan = Math.tan(degToRad(S.camFov) * 0.5);
  CAM.o = o; CAM.f = f;
  CAM.r = r.map(v => v * tan * aspect);
  CAM.u = u.map(v => v * tan);
  CAM.tan = 1.0;
}

/* ------------------------------------------------------------ light calc -- */
const LIGHT = { c: [0, 0, 0], half: [0.3, 0.3], n: [0, -1, 0], emit: [0, 0, 0], area: 0.36 };
function buildLight() {
  const halfX = clamp(0.5 * S.lightSize * S.lightAspect, 0.02, 1.0);
  const halfZ = clamp(0.5 * S.lightSize / Math.max(S.lightAspect, 0.001), 0.02, 1.0);
  LIGHT.c = [S.lightX, S.lightY, S.lightZ];
  LIGHT.half = [halfX, halfZ];
  /* blackbody colour at unit luminance, so intensity means the same at every
     temperature; the colour picker then filters it like a gel */
  const tint = hexToLinear(S.lightTint);
  const kb = kelvinToLinear(S.lightTemp);
  const kLum = 0.2126 * kb[0] + 0.7152 * kb[1] + 0.0722 * kb[2];
  const emit = kb.map((c, i) => c / kLum * tint[i] * S.lightIntensity);
  LIGHT.emit = S.lightOn ? emit : [0, 0, 0];
  LIGHT.area = Math.max(4 * halfX * halfZ, 1e-4);
}

/* ----------------------------------------------------------- render pass -- */
function uploadScene() {
  if (sceneDirty) buildScene();
  const p = progTrace.u;
  gl.uniform1i(p.uObjCount, SCENE.count);
  gl.uniform1iv(p.uObjType, SCENE.type);
  gl.uniform3fv(p.uObjC, SCENE.c);
  gl.uniform3fv(p.uObjH, SCENE.h);
  gl.uniform1fv(p.uObjRad, SCENE.rad);
  gl.uniformMatrix3fv(p.uObjRot, false, SCENE.rot);
  gl.uniform3fv(p.uObjAlbedo, SCENE.alb);
  gl.uniform3fv(p.uObjEmis, SCENE.emis);
  gl.uniform1fv(p.uObjRough, SCENE.rough);
  gl.uniform1fv(p.uObjMetal, SCENE.metal);
  gl.uniform1fv(p.uObjRefl, SCENE.refl);
  gl.uniform1fv(p.uObjIor, SCENE.ior);
  gl.uniform1fv(p.uObjTrans, SCENE.trans);
  gl.uniform1i(p.uLightOn, S.lightOn ? 1 : 0);
  gl.uniform1i(p.uGlassShadow, S.glassShadow ? 1 : 0);
  gl.uniform3fv(p.uLightC, LIGHT.c);
  gl.uniform2fv(p.uLightHalf, LIGHT.half);
  gl.uniform3fv(p.uLightN, LIGHT.n);
  gl.uniform3fv(p.uLightEmit, LIGHT.emit);
  gl.uniform1f(p.uLightArea, LIGHT.area);
  gl.uniform3fv(p.uBg, hexToLinear(S.bgColor));
}

function tracePass() {
  const next = 1 - cur;
  gl.bindFramebuffer(gl.FRAMEBUFFER, fbos[next]);
  gl.viewport(0, 0, internalW, internalH);
  gl.useProgram(progTrace.p);
  gl.bindVertexArray(vao);

  gl.uniform2f(progTrace.u.uRes, internalW, internalH);
  /* seqBase keeps the white-noise seed fresh after camera moves as well */
  gl.uniform1ui(progTrace.u.uSeed, (Math.imul((samples + seqBase) >>> 0, 2654435761 | 0) + 1013904223) >>> 0);
  gl.uniform1ui(progTrace.u.uSeqBase, seqBase);
  gl.uniform1i(progTrace.u.uSampler, S.sampler === 'random' ? 0 : 1);
  gl.uniform1i(progTrace.u.uSpp, Math.round(S.spp));
  gl.uniform1f(progTrace.u.uPrevSamples, samples);
  gl.uniform1i(progTrace.u.uMaxBounce, Math.round(S.maxBounce));
  gl.uniform1f(progTrace.u.uFirefly, S.firefly);
  gl.uniform3fv(progTrace.u.uCamO, CAM.o);
  gl.uniform3fv(progTrace.u.uCamR, CAM.r);
  gl.uniform3fv(progTrace.u.uCamU, CAM.u);
  gl.uniform3fv(progTrace.u.uCamF, CAM.f);
  gl.uniform1f(progTrace.u.uCamTan, CAM.tan);
  uploadScene();
  bindTex(progTrace, 'uBlueNoise', blueNoiseTex, 0);
  bindTex(progTrace, 'uPrevBeauty', sets[cur][0], 1);
  bindTex(progTrace, 'uPrevAlbedo', sets[cur][1], 2);
  bindTex(progTrace, 'uPrevNormal', sets[cur][2], 3);
  bindTex(progTrace, 'uPrevDirect', sets[cur][3], 4);
  bindTex(progTrace, 'uPrevSpec', sets[cur][4], 5);

  gl.drawArrays(gl.TRIANGLES, 0, 3);
  cur = next;
  samples += Math.round(S.spp);
}

/* SVGF-style denoise: prep into dnTex[0], then five a-trous passes ping-ponging
   so the result ends in dnTex[1] (which the display reads). */
function denoisePass() {
  gl.bindVertexArray(vao);
  gl.viewport(0, 0, internalW, internalH);

  gl.bindFramebuffer(gl.FRAMEBUFFER, dnFbo[0]);
  gl.useProgram(progPrep.p);
  gl.uniform2f(progPrep.u.uRes, internalW, internalH);
  gl.uniform1f(progPrep.u.uSamples, samples);
  gl.uniform1f(progPrep.u.uFade, HISTORY_FADE);
  bindTex(progPrep, 'uBeauty', sets[cur][0], 0);
  bindTex(progPrep, 'uAlbedo', sets[cur][1], 1);
  bindTex(progPrep, 'uNormal', sets[cur][2], 2);
  bindTex(progPrep, 'uHist', histTex[hc], 3);
  gl.drawArrays(gl.TRIANGLES, 0, 3);

  gl.useProgram(progDenoise.p);
  const u = progDenoise.u;
  gl.uniform2f(u.uRes, internalW, internalH);
  /* the per-pixel tolerance schedule lives in FS_SVGF_PREP; the strength slider
     scales it (0.5 = as measured, 0 = off) */
  gl.uniform1f(u.uSigmaL, 2 * clamp(S.denoiseStrength, 0, 1));
  gl.uniform3fv(u.uCamO, CAM.o); gl.uniform3fv(u.uCamR, CAM.r);
  gl.uniform3fv(u.uCamU, CAM.u); gl.uniform3fv(u.uCamF, CAM.f);
  bindTex(progDenoise, 'uAlbedo', sets[cur][1], 1);
  bindTex(progDenoise, 'uNormal', sets[cur][2], 2);
  for (let i = 0; i < 5; i++) {
    gl.bindFramebuffer(gl.FRAMEBUFFER, dnFbo[(i + 1) & 1]);
    gl.uniform1i(u.uStep, 1 << i);
    bindTex(progDenoise, 'uIn', dnTex[i & 1], 0);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  }
}

/* After a camera move: reproject the old view's image into histTex[1 - hc].
   Runs right after the first trace in the new view, whose depth and normals
   locate each pixel's surface; the old buffers are still in sets[1 - cur]. */
function historyPass(from, oldSamples) {
  const old = 1 - cur, dst = 1 - hc;
  gl.bindFramebuffer(gl.FRAMEBUFFER, histFbo[dst]);
  gl.viewport(0, 0, internalW, internalH);
  gl.useProgram(progHistory.p);
  gl.bindVertexArray(vao);
  const u = progHistory.u;
  gl.uniform2f(u.uRes, internalW, internalH);
  gl.uniform3fv(u.uCamO, CAM.o); gl.uniform3fv(u.uCamR, CAM.r);
  gl.uniform3fv(u.uCamU, CAM.u); gl.uniform3fv(u.uCamF, CAM.f);
  gl.uniform3fv(u.uPrevO, from.o); gl.uniform3fv(u.uPrevR, from.r);
  gl.uniform3fv(u.uPrevU, from.u); gl.uniform3fv(u.uPrevF, from.f);
  gl.uniform1f(u.uOldSamples, oldSamples);
  gl.uniform1f(u.uCap, HISTORY_CAP);
  gl.uniform1f(u.uFade, HISTORY_FADE);
  gl.uniform1i(u.uHasSpec, nTargets >= 5 ? 1 : 0);
  bindTex(progHistory, 'uNewBeauty', sets[cur][0], 0);
  bindTex(progHistory, 'uNewAlb', sets[cur][1], 1);
  bindTex(progHistory, 'uNewNrm', sets[cur][2], 2);
  bindTex(progHistory, 'uNewSpec', sets[cur][4], 3);
  bindTex(progHistory, 'uOldBeauty', sets[old][0], 4);
  bindTex(progHistory, 'uOldAlb', sets[old][1], 5);
  bindTex(progHistory, 'uOldNrm', sets[old][2], 6);
  bindTex(progHistory, 'uOldHist', histTex[hc], 7);
  bindTex(progHistory, 'uNewDirect', sets[cur][3], 8);
  bindTex(progHistory, 'uOldDirect', sets[old][3], 9);
  gl.drawArrays(gl.TRIANGLES, 0, 3);
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  hc = dst;
}

/* linear = write the linear beauty composite instead of the display image
   (used to freeze the A/B reference) */
function displayPass(targetFbo, outW, outH, toScreen, linear = false) {
  gl.bindFramebuffer(gl.FRAMEBUFFER, toScreen ? null : targetFbo);
  gl.viewport(0, 0, outW, outH);
  gl.useProgram(progDisplay.p);
  gl.bindVertexArray(vao);
  gl.uniform2f(progDisplay.u.uRes, internalW, internalH);
  gl.uniform2f(progDisplay.u.uOutRes, outW, outH);
  gl.uniform1f(progDisplay.u.uSamples, samples);
  gl.uniform1i(progDisplay.u.uLinearOut, linear ? 1 : 0);
  gl.uniform1i(progDisplay.u.uMode, MODE_INDEX[S.viewMode] ?? 0);
  gl.uniform1i(progDisplay.u.uCompare,
    !hasRef ? 0 : (S.compareMode === 'split' ? 1 : S.compareMode === 'reference' ? 2 : S.compareMode === 'diff' ? 3 : 0));
  gl.uniform1i(progDisplay.u.uTonemap, TONEMAP_INDEX[S.tonemap] ?? 0);
  gl.uniform1i(progDisplay.u.uHasSpec, nTargets >= 5 ? 1 : 0);
  gl.uniform1f(progDisplay.u.uExposure, S.exposure);
  gl.uniform1f(progDisplay.u.uGamma, S.gamma);
  gl.uniform1f(progDisplay.u.uFar, Math.max(S.camDistance + 3.5, 2.0));
  gl.uniform1f(progDisplay.u.uDirectScale, S.directIntensity);
  gl.uniform1f(progDisplay.u.uIndirectScale, S.indirectIntensity);
  gl.uniform1i(progDisplay.u.uDenoiseOn, S.denoise ? 1 : 0);
  gl.uniform1f(progDisplay.u.uFade, HISTORY_FADE);
  bindTex(progDisplay, 'uBeauty', sets[cur][0], 0);
  bindTex(progDisplay, 'uAlbedo', sets[cur][1], 1);
  bindTex(progDisplay, 'uNormal', sets[cur][2], 2);
  bindTex(progDisplay, 'uDirect', sets[cur][3], 3);
  bindTex(progDisplay, 'uSpec', sets[cur][4], 4);
  /* never sample refTex while rendering into it (feedback loop) */
  bindTex(progDisplay, 'uRef', hasRef && !linear ? refTex : sets[cur][0], 5);
  bindTex(progDisplay, 'uDenoise', dnTex[1], 6);
  bindTex(progDisplay, 'uHist', histTex[hc], 7);
  gl.drawArrays(gl.TRIANGLES, 0, 3);
}

/* ------------------------------------------------------------ main loop --- */
function resizeIfNeeded() {
  const rect = canvas.getBoundingClientRect();
  const cssW = Math.max(32, Math.floor(rect.width || 640));
  const cssH = Math.max(32, Math.floor(rect.height || 400));
  const dpr = Math.min(window.devicePixelRatio || 1, 1.25);
  const w = Math.round(clamp(cssW * dpr * S.resolution / 100, 32, 3840));
  const h = Math.round(clamp(cssH * dpr * S.resolution / 100, 32, 3840));
  /* only a real size change (or missing buffers) costs the accumulation */
  if (w !== canvas.width || h !== canvas.height || !sets[0]) {
    canvas.width = w; canvas.height = h;
    if (!allocateBuffers(w, h)) return false;
    resetAccumulation();
  }
  return true;
}

/* everything the display pass depends on besides the accumulation buffers */
function displaySignature() {
  return [samples, cur, hasRef, canvas.width, canvas.height, S.exposure, S.gamma, S.tonemap, S.viewMode,
          S.denoise, S.denoiseStrength, S.directIntensity, S.indirectIntensity, S.compareMode, S.camDistance].join('|');
}

function pollGpuTimer() {
  if (!timerExt) return;
  const disjoint = gl.getParameter(timerExt.GPU_DISJOINT_EXT);
  while (gpuQueries.length && gl.getQueryParameter(gpuQueries[0], gl.QUERY_RESULT_AVAILABLE)) {
    const q = gpuQueries.shift();
    const ms = gl.getQueryParameter(q, gl.QUERY_RESULT) / 1e6;
    gl.deleteQuery(q);
    if (!disjoint) gpuMsEMA = gpuMsEMA ? lerp(gpuMsEMA, ms, 0.12) : ms;
  }
}

function frame(now) {
  if (!glOK) return;
  try {
    renderFrame(now);
  } catch (e) {
    console.error(e);
    fatal('Rendering stopped after an error:<pre>' + escapeHtml((e && e.stack) || e) + '</pre>Reload the page to continue.');
    return;
  }
  if (glOK) rafId = requestAnimationFrame(frame);
}

function renderFrame(now) {
  const t0 = performance.now();
  const dt = Math.min(0.1, (now - lastTime) / 1000);
  lastTime = now;

  if (S.autoRotate && !paused) {
    S.camYaw += S.autoRotateSpeed * dt;
    if (S.camYaw > 180) S.camYaw -= 360;
    syncSlider('camYaw');
  }

  if (!resizeIfNeeded()) return;
  buildLight();
  buildCamera(canvas.width / canvas.height);

  /* every camera change (sliders, drag, wheel, auto-rotate) is caught here:
     with temporal reuse the old image becomes history, otherwise start over */
  if (needsReset) resetAccumulation();
  else if (histCam && !sameCamera(histCam, CAM)) {
    if (S.temporalReuse) beginMove(); else resetAccumulation();
  }

  const converged = samples >= S.maxSamples;
  /* while paused, a reset still gets one batch so the viewport previews the change
     instead of going black */
  const trace = (!paused || samples === 0) && !converged;
  /* paused or converged with nothing changed: the canvas already shows this frame */
  const sig = displaySignature();
  const work = trace || sig !== lastDisplaySig;

  if (work) {
    let q = null;
    if (timerExt && gpuQueries.length < 4) {
      q = gl.createQuery();
      gl.beginQuery(timerExt.TIME_ELAPSED_EXT, q);
    }
    if (trace) {
      tracePass();
      if (moveFrom) { historyPass(moveFrom, moveOldSamples); moveFrom = null; }
      histCam = snapshotCamera();
    }
    if (S.denoise) denoisePass();
    displayPass(null, canvas.width, canvas.height, true);
    lastDisplaySig = displaySignature();
    if (q) { gl.endQuery(timerExt.TIME_ELAPSED_EXT); gpuQueries.push(q); }
    const ms = performance.now() - t0;
    frameMsEMA = frameMsEMA ? lerp(frameMsEMA, ms, 0.12) : ms;
  }
  pollGpuTimer();

  fpsEMA = fpsEMA ? lerp(fpsEMA, 1 / Math.max(dt, 1e-4), 0.1) : 1 / Math.max(dt, 1e-4);
  statusText = paused ? 'Paused' : converged ? 'Converged (max samples)' : S.autoRotate ? 'Orbiting' : 'Tracing';
  updateHUD(work);
}

function updateHUD(busy) {
  hud.children[0].lastChild.textContent = ' ' + samples;
  hud.children[1].lastChild.textContent = ' ' + canvas.width + '×' + canvas.height;
  hud.children[2].lastChild.textContent = ' ' + Math.round(S.spp);
  /* GPU time from the timer query when available, otherwise CPU submit time */
  const ms = timerExt ? gpuMsEMA : frameMsEMA;
  hud.children[3].lastChild.textContent = ' ' + (busy ? ms.toFixed(1) + ' ms' : 'idle');
  hud.children[4].lastChild.textContent = ' ' + fpsEMA.toFixed(0) + ' fps';
  hud.children[5].lastChild.textContent = ' ' + statusText;
}

/* ------------------------------------------------------------------ UI ---- */
const uiRefs = {};
function fmt(item, v) {
  if (item.fmt) return item.fmt(v) + (item.unit || '');
  const step = item.step || 1;
  const dec = step >= 1 ? 0 : step >= 0.1 ? 1 : step >= 0.01 ? 2 : 3;
  return v.toFixed(dec) + (item.unit || '');
}
/* puts a value into a range row's slider, readout and screen-reader text */
function showRange(r, v) {
  r.input.value = String(v);
  r.val.textContent = fmt(r.item, v);
  r.input.setAttribute('aria-valuetext', r.val.textContent);
}
function setMatValue(key, v) { S.mats[S.matTarget][key] = v; }
function getMatValue(key) { return S.mats[S.matTarget][key]; }

function buildUI() {
  const host = document.getElementById('panels');
  host.innerHTML = '';
  for (const group of SCHEMA) {
    const sec = document.createElement('section');
    sec.className = 'group' + (group.open ? ' open' : '');
    const head = document.createElement('button');
    head.className = 'group-head';
    head.type = 'button';
    head.innerHTML = '<span class="chev"></span><span>' + group.title + '</span>';
    head.setAttribute('aria-expanded', String(!!group.open));
    head.addEventListener('click', () => {
      head.setAttribute('aria-expanded', String(sec.classList.toggle('open')));
    });
    sec.appendChild(head);
    const body = document.createElement('div');
    body.className = 'group-body';
    sec.appendChild(body);
    host.appendChild(sec);

    for (const item of group.items) {
      if (item.type === 'info') {
        const p = document.createElement('p'); p.className = 'info'; p.textContent = item.text;
        body.appendChild(p); continue;
      }
      if (item.type === 'button') {
        const b = document.createElement('button');
        b.className = 'btn'; b.type = 'button'; b.textContent = item.label;
        b.addEventListener('click', () => doAction(item.action));
        if (item.key) uiRefs[item.key] = b;
        body.appendChild(b); continue;
      }
      const row = document.createElement('div');
      row.className = 'row' + (item.type === 'range' ? ' row-range' : '');
      const id = 'ctl-' + item.key;
      const lab = document.createElement('label');
      lab.htmlFor = id;                    // clicking the text focuses / toggles the control
      lab.textContent = item.label;
      row.appendChild(lab);
      /* the readout sits beside the label, not inside it, so it isn't part of the name */
      const val = document.createElement('span');
      val.className = 'val';
      val.setAttribute('aria-hidden', 'true');
      if (item.type === 'range') row.appendChild(val);

      let input;
      if (item.type === 'range') {
        input = document.createElement('input');
        input.type = 'range'; input.min = item.min; input.max = item.max; input.step = item.step;
        input.addEventListener('input', () => {
          const v = parseFloat(input.value);
          if (item.mat) setMatValue(item.mat, v); else S[item.key] = v;
          val.textContent = fmt(item, v);
          input.setAttribute('aria-valuetext', val.textContent);
          applyScope(item);
        });
      } else if (item.type === 'color') {
        input = document.createElement('input');
        input.type = 'color';
        input.addEventListener('input', () => { S[item.key] = input.value; applyScope(item); });
      } else if (item.type === 'select') {
        input = document.createElement('select');
        for (const [v, t] of item.options) {
          const o = document.createElement('option'); o.value = v; o.textContent = t; input.appendChild(o);
        }
        input.addEventListener('change', () => {
          S[item.key] = input.value;
          if (item.action) doAction(item.action, input.value);
          else applyScope(item);
          refreshMaterialState();
        });
      } else if (item.type === 'bool') {
        input = document.createElement('input');
        input.type = 'checkbox';
        input.addEventListener('change', () => { S[item.key] = input.checked; applyScope(item); });
      }
      input.id = id;
      row.appendChild(input);
      body.appendChild(row);
      uiRefs[item.key] = { item, input, val };
    }
  }
  syncUI();
  refreshMaterialState();
}

function applyScope(item) {
  if (item.scope === 'scene' || item.scope === 'trace') {
    sceneDirty = true; needsReset = true;
    /* changing shape or size re-drops the object onto the floor */
    if (['objAShape', 'objASize', 'objAYStretch'].includes(item.key)) {
      S.objAY = +restY(S.objAShape, S.objASize, S.objAYStretch).toFixed(3);
      syncSlider('objAY');
    } else if (['objBShape', 'objBSize', 'objBYStretch'].includes(item.key)) {
      S.objBY = +restY(S.objBShape, S.objBSize, S.objBYStretch).toFixed(3);
      syncSlider('objBY');
    }
    if (item.scope === 'scene' && S.preset !== 'custom') {
      S.preset = 'custom';
      const r = uiRefs.preset;
      if (r && r.input) r.input.value = 'custom';
    }
  }
  /* scope 'camera' needs nothing here: renderFrame() notices the camera moved */
  /* scope 'res' needs nothing here: resizeIfNeeded() sees the new size next frame */
}

function syncSlider(key) {
  const r = uiRefs[key];
  if (!r || r.item.type !== 'range') return;
  showRange(r, S[key]);
}

function syncUI() {
  for (const key in uiRefs) {
    const r = uiRefs[key];
    if (!r || !r.item) continue;
    const { item, input } = r;
    if (item.mat) showRange(r, getMatValue(item.mat));
    else if (item.type === 'range') showRange(r, S[key]);
    else if (item.type === 'color') input.value = S[key];
    else if (item.type === 'select') input.value = S[key];
    else if (item.type === 'bool') input.checked = !!S[key];
  }
}

function refreshMaterialState() {
  const walls = S.matTarget === 'walls' || S.matTarget === 'floor';
  for (const key in uiRefs) {
    const r = uiRefs[key];
    if (!r || !r.item || !r.input) continue;
    const shapeKey = key.startsWith('objA') ? 'objAShape' : key.startsWith('objB') ? 'objBShape' : null;
    const shape = shapeKey ? S[shapeKey] : null;
    /* rows that belong to a hidden or differently-shaped object are dead controls */
    const hidden = shapeKey && shapeKey !== key && shape === 'none';
    const boxOnly = r.item.only === 'box' && shape !== 'box';
    const dis = (walls && r.item.noWalls) || hidden || boxOnly;
    r.input.disabled = !!dis;
    const row = r.input.closest('.row');
    if (row) row.classList.toggle('disabled', !!dis);
    if (r.item.mat) showRange(r, getMatValue(r.item.mat));   // the newly selected target's values
  }
}

function doAction(action, value) {
  switch (action) {
    case 'applyPreset': applyPreset(value); break;
    case 'matTarget': refreshMaterialState(); break;
    case 'resetCamera':
      S.camYaw = DEFAULTS.camYaw; S.camPitch = DEFAULTS.camPitch;
      S.camFov = DEFAULTS.camFov; S.camHeight = DEFAULTS.camHeight;
      S.camTargetX = DEFAULTS.camTargetX; S.camTargetZ = DEFAULTS.camTargetZ;
      S.camDistance = fitDistance(canvas.width / Math.max(canvas.height, 1));
      S.autoRotate = false; syncUI(); break;
    case 'togglePause': paused = !paused; updatePauseButton(); break;
    case 'resetAccum': needsReset = true; break;
    case 'captureRef': captureReference(); break;
    case 'screenshot': screenshot(); break;
    case 'exportPNG': exportPNG(); break;
    case 'randomize': randomize(); break;
    case 'resetAll': S = structuredClone(DEFAULTS); paused = false; needsReset = true; sceneDirty = true;
      settleObjects(); updatePauseButton(); syncUI(); refreshMaterialState(); break;
    case 'saveJSON': saveJSON(); break;
    case 'loadJSON': loadJSON(); break;
  }
}
function updatePauseButton() {
  const b = uiRefs.btnPause;
  if (b) { b.textContent = paused ? 'Resume' : 'Pause'; b.classList.toggle('active', paused); }
}

/* Settings a preset leaves alone (camera, output and session state). Everything
   else goes back to DEFAULTS first, so a preset never inherits the previous one. */
const PRESET_KEEPS = new Set([
  'preset', 'matTarget', 'resolution', 'spp', 'maxSamples', 'viewMode', 'compareMode', 'exportScale',
  'denoiseStrength', 'camYaw', 'camPitch', 'camDistance', 'camFov', 'camHeight', 'camTargetX', 'camTargetZ',
  'autoRotate', 'autoRotateSpeed', 'temporalReuse', 'sampler'
]);

function applyPreset(name) {
  const p = PRESETS[name];
  if (!p) return;
  S.preset = name;
  for (const k in DEFAULTS) if (!PRESET_KEEPS.has(k)) S[k] = structuredClone(DEFAULTS[k]);
  for (const k in p) {
    if (k === 'mats') S.mats = structuredClone(p.mats);
    else S[k] = p[k];
  }
  sceneDirty = true; needsReset = true;
  settleObjects();
  /* re-frame so the frustum fits through the open front face of the box */
  S.camDistance = fitDistance(canvas.width / Math.max(canvas.height, 1));
  syncUI(); refreshMaterialState();
}

function randomize() {
  const rnd = (a, b) => a + Math.random() * (b - a);
  const hueHex = (h, s, l) => {
    const c = (1 - Math.abs(2 * l - 1)) * s, x = c * (1 - Math.abs((h / 60) % 2 - 1)), m = l - c / 2;
    let r, g, b;
    if (h < 60) [r, g, b] = [c, x, 0]; else if (h < 120) [r, g, b] = [x, c, 0];
    else if (h < 180) [r, g, b] = [0, c, x]; else if (h < 240) [r, g, b] = [0, x, c];
    else if (h < 300) [r, g, b] = [x, 0, c]; else [r, g, b] = [c, 0, x];
    const toS = v => Math.round(clamp(v + m, 0, 1) * 255).toString(16).padStart(2, '0');
    return '#' + toS(r) + toS(g) + toS(b);
  };
  S.wallLeftColor = hueHex(Math.floor(rnd(0, 360)), rnd(0.35, 0.8), rnd(0.3, 0.55));
  S.wallRightColor = hueHex(Math.floor(rnd(0, 360)), rnd(0.35, 0.8), rnd(0.3, 0.55));
  S.wallBaseColor = hueHex(Math.floor(rnd(0, 360)), rnd(0.02, 0.18), rnd(0.6, 0.85));
  S.objAColor = hueHex(Math.floor(rnd(0, 360)), rnd(0.05, 0.5), rnd(0.55, 0.85));
  S.objBColor = hueHex(Math.floor(rnd(0, 360)), rnd(0.05, 0.5), rnd(0.55, 0.85));
  S.objAShape = ['box', 'sphere'][Math.floor(Math.random() * 2)];
  S.objBShape = ['box', 'sphere'][Math.floor(Math.random() * 2)];
  S.objAX = rnd(-0.75, -0.2); S.objAZ = rnd(-0.6, 0.6);
  S.objBX = rnd(0.15, 0.75); S.objBZ = rnd(-0.7, 0.5);
  S.objASize = rnd(0.5, 1.2); S.objBSize = rnd(0.4, 1.0);
  S.objAYStretch = rnd(0.7, 2.0); S.objBYStretch = rnd(0.5, 1.6);
  S.objARotY = rnd(-60, 60); S.objBRotY = rnd(-60, 60);
  settleObjects();
  S.lightIntensity = rnd(6, 26); S.lightTemp = Math.round(rnd(2400, 8200) / 50) * 50;
  S.lightSize = rnd(0.3, 1.3); S.lightX = rnd(-0.4, 0.4); S.lightZ = rnd(-0.5, 0.5);
  for (const t of ['A', 'B']) {
    const m = S.mats[t];
    m.rough = rnd(0, 1); m.metal = Math.random() < 0.35 ? rnd(0.7, 1) : rnd(0, 0.25);
    m.refl = Math.random() < 0.5 ? rnd(0, 0.4) : rnd(0.7, 1);
    m.trans = Math.random() < 0.2 ? rnd(0.6, 1) : 0;
    m.emis = Math.random() < 0.15 ? rnd(1, 6) : 0;
  }
  S.preset = 'custom';
  sceneDirty = true; needsReset = true;
  syncUI(); refreshMaterialState();
}

/* ------------------------------------------------------------- captures -- */
/* Freezes the beauty image exactly as displayed (denoise and light sliders
   included), in linear HDR so exposure / tone mapping stay live on both sides. */
function captureReference() {
  if (!glOK || samples === 0) return;
  if (S.denoise) denoisePass();
  displayPass(refFbo, internalW, internalH, false, true);
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  hasRef = true;
}

function downloadURL(url, name) {
  const a = document.createElement('a');
  a.href = url; a.download = name;
  document.body.appendChild(a); a.click(); a.remove();
  /* the download has its own reference once started; free the blob afterwards */
  if (url.startsWith('blob:')) setTimeout(() => URL.revokeObjectURL(url), 10000);
}

function screenshot() {
  if (!glOK) return;
  displayPass(null, canvas.width, canvas.height, true);
  downloadURL(canvas.toDataURL('image/png'), 'cornell-box.png');
}

function exportPNG() {
  if (!glOK) return;
  const scale = parseInt(S.exportScale, 10) || 2;
  let w = Math.round(canvas.width * scale), h = Math.round(canvas.height * scale);
  const cap = 4096;
  if (Math.max(w, h) > cap) { const k = cap / Math.max(w, h); w = Math.round(w * k); h = Math.round(h * k); }
  if (expTex) gl.deleteTexture(expTex);
  if (expFbo) gl.deleteFramebuffer(expFbo);
  expTex = makeTex(w, h, gl.RGBA8, gl.UNSIGNED_BYTE, gl.NEAREST);
  expFbo = gl.createFramebuffer();
  gl.bindFramebuffer(gl.FRAMEBUFFER, expFbo);
  gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, expTex, 0);
  gl.drawBuffers([gl.COLOR_ATTACHMENT0]);
  if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) {
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    alert('This GPU cannot render a ' + w + '×' + h + ' export. Try a smaller export scale.');
    return;
  }

  displayPass(expFbo, w, h, false);
  const px = new Uint8Array(w * h * 4);
  gl.readBuffer(gl.COLOR_ATTACHMENT0);
  gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, px);
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);

  const out = document.createElement('canvas');
  out.width = w; out.height = h;
  const ctx2 = out.getContext('2d');
  const img = ctx2.createImageData(w, h);
  const row = w * 4;
  for (let y = 0; y < h; y++) {           // GL rows run bottom-up
    img.data.set(px.subarray((h - 1 - y) * row, (h - y) * row), y * row);
  }
  ctx2.putImageData(img, 0, 0);
  out.toBlob(b => {
    if (!b) { alert('The browser could not encode the export PNG.'); return; }
    downloadURL(URL.createObjectURL(b), 'cornell-box-' + w + 'x' + h + '.png');
  }, 'image/png');
}

function saveJSON() {
  const blob = new Blob([JSON.stringify({ version: 1, ...S }, null, 2)], { type: 'application/json' });
  downloadURL(URL.createObjectURL(blob), 'cornell-settings.json');
}

/* SCHEMA rows by settings key, and the material rows by material field */
const ITEM_BY_KEY = {}, MAT_ITEMS = {};
for (const g of SCHEMA) for (const it of g.items) {
  if (it.mat) MAT_ITEMS[it.mat] = it;
  else if (it.key) ITEM_BY_KEY[it.key] = it;
}

/* A loaded value if it is valid for its setting (type, range, option or colour
   format), else undefined. def is the DEFAULTS value, which fixes the type. */
function cleanSetting(item, def, v) {
  if (item && item.type === 'select') return item.options.some(([o]) => o === String(v)) ? v : undefined;
  if (typeof v !== typeof def) return undefined;
  if (typeof v === 'number') {
    if (!Number.isFinite(v)) return undefined;
    return item && item.type === 'range' ? clamp(v, item.min, item.max) : v;
  }
  if (item && item.type === 'color' && !/^#[0-9a-f]{6}$/i.test(v)) return undefined;
  return v;
}

/* Applies a parsed settings file; unknown keys and invalid values are skipped. */
function applySettings(obj) {
  if (!obj || typeof obj !== 'object' || Array.isArray(obj)) throw new Error('the file is not a settings object');
  const next = structuredClone(S);
  let used = 0;
  for (const k of Object.keys(obj)) {
    if (k === 'mats' || !Object.hasOwn(DEFAULTS, k)) continue;
    const v = cleanSetting(ITEM_BY_KEY[k], DEFAULTS[k], obj[k]);
    if (v !== undefined) { next[k] = v; used++; }
  }
  const mats = obj.mats;
  if (mats && typeof mats === 'object') {
    for (const t of Object.keys(DEFAULTS.mats)) {
      const m = mats[t];
      if (!m || typeof m !== 'object') continue;
      for (const f of Object.keys(DEFAULTS.mats[t])) {
        const v = cleanSetting(MAT_ITEMS[f], DEFAULTS.mats[t][f], m[f]);
        if (v !== undefined) { next.mats[t][f] = v; used++; }
      }
    }
  }
  if (!used) throw new Error('it contains no recognised settings');
  S = next;
  sceneDirty = true; needsReset = true;
  syncUI(); refreshMaterialState(); updatePauseButton();
}
function loadJSON() {
  const inp = document.createElement('input');
  inp.type = 'file'; inp.accept = '.json,application/json';
  inp.addEventListener('change', () => {
    const f = inp.files && inp.files[0];
    if (!f) return;
    const rd = new FileReader();
    rd.onload = () => {
      try {
        applySettings(JSON.parse(rd.result));
      } catch (e) { alert('Could not load that JSON file: ' + e.message + '.'); }
    };
    rd.readAsText(f);
  });
  inp.click();
}

/* ------------------------------------------------------------ interaction - */
const wrapDeg = d => ((d + 180) % 360 + 360) % 360 - 180;
const midpoint = p => [(p[0].x + p[1].x) / 2, (p[0].y + p[1].y) / 2];

/* Moves the orbit target along the camera's screen axes (right in XZ, world up). */
function panBy(dx, dy) {
  const k = S.camDistance * 0.0016, yaw = degToRad(S.camYaw);
  S.camTargetX = clamp(S.camTargetX - Math.cos(yaw) * dx * k, -1, 1);
  S.camTargetZ = clamp(S.camTargetZ + Math.sin(yaw) * dx * k, -1, 1);
  S.camHeight = clamp(S.camHeight + dy * k, -0.9, 1.6);
  syncSlider('camTargetX'); syncSlider('camTargetZ'); syncSlider('camHeight');
}

function setupInteraction() {
  let pinchMid = null;
  canvas.addEventListener('pointerdown', e => {
    canvas.setPointerCapture(e.pointerId);
    pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
    dragging = true;
    dragMode = (e.button === 2 || e.shiftKey) ? 'pan' : 'orbit';
    if (pointers.size === 2) {
      const p = [...pointers.values()];
      pinchDist = Math.hypot(p[0].x - p[1].x, p[0].y - p[1].y);
      pinchMid = midpoint(p);
    }
  });
  canvas.addEventListener('pointermove', e => {
    if (!pointers.has(e.pointerId)) return;
    const prev = pointers.get(e.pointerId);
    const dx = e.clientX - prev.x, dy = e.clientY - prev.y;
    pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (pointers.size === 2) {               // pinch zooms, two-finger drag pans
      const p = [...pointers.values()];
      const d = Math.hypot(p[0].x - p[1].x, p[0].y - p[1].y);
      const mid = midpoint(p);
      if (pinchDist > 0) S.camDistance = clamp(S.camDistance * pinchDist / d, 1.2, 9);
      if (pinchMid) panBy(mid[0] - pinchMid[0], mid[1] - pinchMid[1]);
      pinchDist = d; pinchMid = mid;
      syncSlider('camDistance');
      return;
    }
    if (!dragging) return;
    if (dragMode === 'orbit') {
      S.camYaw = wrapDeg(S.camYaw - dx * 0.35);
      S.camPitch = clamp(S.camPitch + dy * 0.3, -35, 82);
      syncSlider('camYaw'); syncSlider('camPitch');
    } else {
      panBy(dx, dy);
    }
  });
  const up = e => {
    pointers.delete(e.pointerId);
    if (pointers.size === 0) dragging = false;
    pinchDist = 0; pinchMid = null;
  };
  canvas.addEventListener('pointerup', up);
  canvas.addEventListener('pointercancel', up);
  canvas.addEventListener('lostpointercapture', up);
  canvas.addEventListener('contextmenu', e => e.preventDefault());
  canvas.addEventListener('wheel', e => {
    e.preventDefault();
    S.camDistance = clamp(S.camDistance * Math.exp(e.deltaY * 0.0012), 1.2, 9);
    syncSlider('camDistance');
  }, { passive: false });

  window.addEventListener('keydown', e => {
    if (e.ctrlKey || e.metaKey || e.altKey) return;        // leave browser shortcuts alone
    const t = e.target instanceof Element ? e.target : null;
    /* typing in a field or a select keeps every key; sliders still allow shortcuts */
    if (t && t.closest('select, textarea, [contenteditable], input:not([type="range"]):not([type="checkbox"])')) return;
    /* Space / Enter on a focused button or toggle presses it instead */
    if (t && t.closest('button, input[type="checkbox"]') && (e.key === ' ' || e.key === 'Enter')) return;
    const n = parseInt(e.key, 10);
    if (n >= 1 && n <= 8) { S.viewMode = VIEW_MODES[n - 1][0]; syncUI(); return; }
    switch (e.key.toLowerCase()) {
      case ' ': e.preventDefault(); paused = !paused; updatePauseButton(); break;
      case 'r': needsReset = true; break;
      case 'a': S.autoRotate = !S.autoRotate; syncUI(); break;
      case 'c': captureReference(); break;
      case 'd': S.denoise = !S.denoise; syncUI(); break;
    }
  });

  canvas.addEventListener('webglcontextlost', e => {
    e.preventDefault();                                    // lets the browser restore it
    fatal('The GPU context was lost (driver reset, or too much work per frame). Trying to restore it… ' +
          'If nothing happens, lower Samples per frame, Max bounces or Render resolution and reload the page.');
  });
  canvas.addEventListener('webglcontextrestored', () => {
    if (!setupGL()) return;
    overlay.style.display = 'none';
    sceneDirty = true; needsReset = true; lastDisplaySig = '';
    cancelAnimationFrame(rafId);
    rafId = requestAnimationFrame(frame);
  });
}

/* ------------------------------------------------------------------ boot -- */
function boot() {
  settleObjects();
  buildUI();
  if (!initGL()) return;
  const g = document.getElementById('gpu');
  if (g) g.textContent = gpuName;
  hud.children[3].firstChild.textContent = timerExt ? 'GPU time' : 'CPU time';
  setupInteraction();
  updatePauseButton();
  if (!resizeIfNeeded()) return;
  S.camDistance = fitDistance(canvas.width / canvas.height);
  syncSlider('camDistance');
  buildLight(); buildScene();
  rafId = requestAnimationFrame(frame);
}
boot();
