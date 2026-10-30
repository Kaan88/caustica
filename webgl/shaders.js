/* ============================================================================
 *  Cornell Box Path Tracer - GLSL ES 3.00 shader sources
 * ----------------------------------------------------------------------------
 *  FS_TRACE     : Monte Carlo path tracer + progressive accumulation (MRT)
 *  FS_SVGF_PREP : denoiser input: illumination + variance of the displayed estimate
 *  FS_DENOISE   : one SVGF a-trous pass (plane / normal / variance-guided luminance)
 *  FS_HISTORY   : reprojects the previous image after a camera move
 *  FS_DISPLAY   : AOV selection, exposure, tone mapping, gamma, compare view,
 *                and the linear beauty composite frozen as the A/B reference
 * ========================================================================== */

/* Fullscreen triangle, no vertex buffer needed (gl_VertexID). */
const VS_QUAD = `#version 300 es
out vec2 vUv;
void main() {
  int id = gl_VertexID;
  vec2 p = vec2(id == 1 ? 3.0 : -1.0, id == 2 ? 3.0 : -1.0);
  vUv = (p + 1.0) * 0.5;
  gl_Position = vec4(p, 0.0, 1.0);
}`;

/* -------------------------------------------------------------------------- */
const FS_TRACE = `#version 300 es
precision highp float;
precision highp int;
precision highp sampler2D;

/* app.js prepends 0 here on GPUs with only 4 draw buffers (no specular AOV) */
#ifndef SPEC_TARGET
#define SPEC_TARGET 1
#endif

#define MAX_OBJ 12
#define PI  3.141592653589793
#define TAU 6.283185307179586
#define EPS 1e-3
#define LUM vec3(0.2126, 0.7152, 0.0722)

uniform vec2  uRes;          // internal render resolution
uniform uint  uSeed;         // per-frame seed (white-noise sampler)
uniform int   uSampler;      // 0 = white noise, 1 = Owen-scrambled Sobol + blue-noise shift
uniform uint  uSeqBase;      // Sobol index of the first sample in this accumulation
uniform int   uSpp;          // samples per pixel per frame
uniform float uPrevSamples;  // samples already accumulated at this pixel
uniform int   uMaxBounce;
uniform float uFirefly;      // 0 = off, else clamp factor vs. running mean

uniform int   uObjCount;
uniform int   uObjType[MAX_OBJ];     // 0 = box, 1 = sphere
uniform vec3  uObjC[MAX_OBJ];
uniform vec3  uObjH[MAX_OBJ];        // box half extents
uniform float uObjRad[MAX_OBJ];      // sphere radius
uniform mat3  uObjRot[MAX_OBJ];
uniform vec3  uObjAlbedo[MAX_OBJ];
uniform vec3  uObjEmis[MAX_OBJ];
uniform float uObjRough[MAX_OBJ];
uniform float uObjMetal[MAX_OBJ];
uniform float uObjRefl[MAX_OBJ];     // dielectric / diffuse reflectivity
uniform float uObjIor[MAX_OBJ];
uniform float uObjTrans[MAX_OBJ];

uniform int   uLightOn;
uniform int   uGlassShadow;          // 1 = transmissive objects block direct light
uniform vec3  uLightC;
uniform vec2  uLightHalf;            // half width (x) / half depth (z)
uniform vec3  uLightN;               // emitting normal
uniform vec3  uLightEmit;            // radiance (linear, includes intensity)
uniform float uLightArea;

uniform vec3  uBg;                   // environment radiance
uniform vec3  uCamO, uCamR, uCamU, uCamF;
uniform float uCamTan;

uniform sampler2D uBlueNoise;        // 64x64 RG32F, two independent blue-noise masks
uniform sampler2D uPrevBeauty;
uniform sampler2D uPrevAlbedo;
uniform sampler2D uPrevNormal;
uniform sampler2D uPrevDirect;
uniform sampler2D uPrevSpec;

layout(location = 0) out vec4 oBeauty;  // rgb = running mean,  a = variance sum (M2)
layout(location = 1) out vec4 oAlbedo;  // rgb = albedo mean,   a = depth
layout(location = 2) out vec4 oNormal;  // rgb = world normal,  a = hit mask
layout(location = 3) out vec4 oDirect;  // rgb = light that bounced at most once (see trace)
#if SPEC_TARGET
layout(location = 4) out vec4 oSpec;    // rgb = light seen through a specular first hit
#endif

/* ------------------------------- RNG -------------------------------------- */
uint rngState;
uint pcgHash(uint v) {
  uint s = v * 747796405u + 2891336453u;
  uint w = ((s >> ((s >> 28u) + 4u)) ^ s) * 2777612352u;
  return (w >> 22u) ^ w;
}
float rnd() {
  rngState = pcgHash(rngState);
  return float(rngState) * 2.32830643657e-10;
}
vec2 rnd2() { return vec2(rnd(), rnd()); }

/* ------------------------ low-discrepancy sampler ---------------------------
   One Owen-scrambled Sobol sequence shared by every pixel (Burley 2020, "Practical
   Hash-based Owen Scrambling"), padded per 2D slot by shuffling the sample index,
   then shifted per pixel by a blue-noise mask (Georgiev & Fajardo 2016). Each
   pixel's samples stay stratified, and the error left at low sample counts is
   blue noise on screen instead of white noise. */
ivec2 gPix;      // this fragment's pixel
uint  gIndex;    // Sobol index of the current sample

uint reverseBits(uint x) {
  x = ((x >> 1u) & 0x55555555u) | ((x & 0x55555555u) << 1u);
  x = ((x >> 2u) & 0x33333333u) | ((x & 0x33333333u) << 2u);
  x = ((x >> 4u) & 0x0F0F0F0Fu) | ((x & 0x0F0F0F0Fu) << 4u);
  x = ((x >> 8u) & 0x00FF00FFu) | ((x & 0x00FF00FFu) << 8u);
  return (x >> 16u) | (x << 16u);
}
/* Laine-Karras style permutation (Vegdahl's improved constants) */
uint lkPermute(uint x, uint seed) {
  x ^= x * 0x3d20adeau;
  x += seed;
  x *= (seed >> 16u) | 1u;
  x ^= x * 0x05526c56u;
  x ^= x * 0x53a22864u;
  return x;
}
uint owen(uint x, uint seed) { return reverseBits(lkPermute(reverseBits(x), seed)); }
/* second Sobol dimension (the first is reverseBits(i)) */
uint sobol1(uint i) {
  uint r = 0u, v = 0x80000000u;
  for (int b = 0; b < 32; b++) {
    if (i == 0u) break;
    if ((i & 1u) != 0u) r ^= v;
    i >>= 1u;
    v ^= v >> 1u;
  }
  return r;
}
/* 2D sample for a fixed slot of the path (camera, light, BSDF... see trace) */
vec2 sample2(int slot) {
  if (uSampler == 0) return rnd2();
  uint s = pcgHash(uint(slot) * 0x9e3779b9u + 0x7f4a7c15u);
  uint i = owen(gIndex, s);
  uint x = owen(reverseBits(i), pcgHash(s ^ 0x68bc21ebu));
  uint y = owen(sobol1(i), pcgHash(s ^ 0x02e5be93u));
  vec2 u = vec2(float(x >> 8u), float(y >> 8u)) * (1.0 / 16777216.0);   // 24 bits, exact, < 1
  ivec2 off = ivec2(int(s & 63u), int((s >> 6u) & 63u));               // decorrelates the slots
  return fract(u + texelFetch(uBlueNoise, (gPix + off) & 63, 0).rg);
}

/* unit tangent perpendicular to v (never zero: the branch avoids the degenerate axis) */
vec3 ortho(vec3 v) {
  return normalize(abs(v.x) > abs(v.z) ? vec3(-v.y, v.x, 0.0) : vec3(0.0, -v.z, v.y));
}
vec3 randDir(vec2 u2) {
  float u = 2.0 * u2.x - 1.0;
  float phi = TAU * u2.y;
  float s = sqrt(max(0.0, 1.0 - u * u));
  return vec3(s * cos(phi), s * sin(phi), u);
}
vec3 cosineHemi(vec3 n, vec2 u) {
  float r = sqrt(u.x);
  float phi = TAU * u.y;
  vec3 t = ortho(n);
  return normalize(t * r * cos(phi) + cross(n, t) * r * sin(phi) + n * sqrt(max(0.0, 1.0 - u.x)));
}

/* --------------------------- geometry ------------------------------------- */
struct Hit { float t; vec3 p; vec3 n; int idx; };
struct Surf { vec3 alb; vec3 emis; float rough; float metal; float refl; float ior; float trans; };

Surf surfOf(int i) {
  Surf s;
  s.alb   = uObjAlbedo[i];
  s.emis  = uObjEmis[i];
  s.rough = uObjRough[i];
  s.metal = uObjMetal[i];
  s.refl  = uObjRefl[i];
  s.ior   = uObjIor[i];
  s.trans = uObjTrans[i];
  return s;
}

float sphHit(vec3 c, float r, vec3 ro, vec3 rd, out vec3 n) {
  vec3 oc = ro - c;
  float b = dot(oc, rd);
  float c2 = dot(oc, oc) - r * r;
  float disc = b * b - c2;
  if (disc < 0.0) return -1.0;
  float sq = sqrt(disc);
  float t = -b - sq;
  if (t < EPS) { t = -b + sq; if (t < EPS) return -1.0; }
  n = (ro + rd * t - c) / r;
  return t;
}

float boxHit(vec3 c, vec3 h, mat3 R, vec3 ro, vec3 rd, out vec3 n) {
  mat3 Rt = transpose(R);
  vec3 l = Rt * (ro - c);
  vec3 d = Rt * rd;

  float tmin = -1e30, tmax = 1e30;
  int aMin = -1, aMax = -1;
  float sMin = 0.0, sMax = 0.0;
  float a, b;

  if (abs(d.x) < 1e-12) { if (l.x < -h.x || l.x > h.x) return -1.0; }
  else {
    a = (-h.x - l.x) / d.x; b = (h.x - l.x) / d.x;
    if (a < b) { if (a > tmin) { tmin = a; aMin = 0; sMin = -1.0; } if (b < tmax) { tmax = b; aMax = 0; sMax = 1.0; } }
    else       { if (b > tmin) { tmin = b; aMin = 0; sMin = 1.0; }  if (a < tmax) { tmax = a; aMax = 0; sMax = -1.0; } }
    if (tmin > tmax) return -1.0;
  }
  if (abs(d.y) < 1e-12) { if (l.y < -h.y || l.y > h.y) return -1.0; }
  else {
    a = (-h.y - l.y) / d.y; b = (h.y - l.y) / d.y;
    if (a < b) { if (a > tmin) { tmin = a; aMin = 1; sMin = -1.0; } if (b < tmax) { tmax = b; aMax = 1; sMax = 1.0; } }
    else       { if (b > tmin) { tmin = b; aMin = 1; sMin = 1.0; }  if (a < tmax) { tmax = a; aMax = 1; sMax = -1.0; } }
    if (tmin > tmax) return -1.0;
  }
  if (abs(d.z) < 1e-12) { if (l.z < -h.z || l.z > h.z) return -1.0; }
  else {
    a = (-h.z - l.z) / d.z; b = (h.z - l.z) / d.z;
    if (a < b) { if (a > tmin) { tmin = a; aMin = 2; sMin = -1.0; } if (b < tmax) { tmax = b; aMax = 2; sMax = 1.0; } }
    else       { if (b > tmin) { tmin = b; aMin = 2; sMin = 1.0; }  if (a < tmax) { tmax = a; aMax = 2; sMax = -1.0; } }
    if (tmin > tmax) return -1.0;
  }

  if (tmax < EPS) return -1.0;
  float t = (tmin > EPS) ? tmin : tmax;
  int axis = (tmin > EPS) ? aMin : aMax;
  float sgn = (tmin > EPS) ? sMin : sMax;
  if (axis < 0) n = -normalize(rd);
  else {
    vec3 ln = vec3(axis == 0 ? sgn : 0.0, axis == 1 ? sgn : 0.0, axis == 2 ? sgn : 0.0);
    n = R * ln;
  }
  return t;
}

float lightHit(vec3 ro, vec3 rd, out vec3 p) {
  if (uLightOn == 0) return -1.0;
  if (abs(rd.y) < 1e-12) return -1.0;
  float t = (uLightC.y - ro.y) / rd.y;
  if (t < EPS) return -1.0;
  vec3 q = ro + rd * t;
  if (abs(q.x - uLightC.x) > uLightHalf.x || abs(q.z - uLightC.z) > uLightHalf.y) return -1.0;
  if (dot(uLightN, rd) >= 0.0) return -1.0;   // emitting side only
  p = q;
  return t;
}

bool sceneHit(vec3 ro, vec3 rd, out Hit h) {
  float best = 1e30;
  bool hit = false;
  for (int i = 0; i < uObjCount; i++) {
    float t; vec3 n;
    if (uObjType[i] == 1) t = sphHit(uObjC[i], uObjRad[i], ro, rd, n);
    else                  t = boxHit(uObjC[i], uObjH[i], uObjRot[i], ro, rd, n);
    if (t > EPS && t < best) {
      best = t; h.t = t; h.p = ro + rd * t; h.n = n; h.idx = i; hit = true;
    }
  }
  vec3 lp;
  float t = lightHit(ro, rd, lp);
  if (t > EPS && t < best) { best = t; h.t = t; h.p = lp; h.n = uLightN; h.idx = -1; hit = true; }
  return hit;
}

bool occluded(vec3 ro, vec3 rd, float maxT) {
  for (int i = 0; i < uObjCount; i++) {
    if (uGlassShadow == 0 && uObjTrans[i] > 0.5) continue;   /* glass lets direct light through */
    float t; vec3 n;
    if (uObjType[i] == 1) t = sphHit(uObjC[i], uObjRad[i], ro, rd, n);
    else                  t = boxHit(uObjC[i], uObjH[i], uObjRot[i], ro, rd, n);
    if (t > EPS && t < maxT) return true;
  }
  return false;
}

/* ------------------------- light sampling / MIS --------------------------- */
vec3 lightSample(vec2 u2, out vec3 ln) {
  vec2 u = u2 * 2.0 - 1.0;
  ln = uLightN;
  return uLightC + vec3(u.x * uLightHalf.x, 0.0, u.y * uLightHalf.y);
}

/* solid-angle pdf with which lightSample() would pick the direction dir from p;
   zero for directions that miss the light rectangle */
float lightPdf(vec3 p, vec3 dir) {
  if (uLightOn == 0 || uLightArea <= 0.0) return 0.0;
  if (abs(dir.y) < 1e-12) return 0.0;
  float t = (uLightC.y - p.y) / dir.y;
  if (t <= EPS) return 0.0;
  vec3 q = p + dir * t;
  if (abs(q.x - uLightC.x) > uLightHalf.x || abs(q.z - uLightC.z) > uLightHalf.y) return 0.0;
  float cosQ = dot(uLightN, -dir);
  if (cosQ <= 1e-5) return 0.0;
  return t * t / (uLightArea * cosQ);   // dir is unit length, so distance^2 = t^2
}

/* ------------------------------ BRDF -------------------------------------- */
float schlickG1(float noX, float a) {
  float t = noX * noX;
  return 1.0 / (noX + sqrt(t + a * a * (1.0 - t)));
}
vec3 ggxOverPdf(vec3 n, vec3 wo, vec3 wi, float a, vec3 F0) {
  vec3 h = normalize(wo + wi);
  float NoV = max(dot(n, wo), 1e-4);
  float NoL = max(dot(n, wi), 0.0);
  float NoH = max(dot(n, h), 1e-5);
  float VoH = max(dot(wo, h), 1e-5);
  if (NoL <= 1e-5) return vec3(0.0);
  float a2 = a * a;
  float dh = NoH * NoH * (a2 - 1.0) + 1.0;
  float D = a2 / (PI * dh * dh);
  vec3 F = F0 + (1.0 - F0) * pow(1.0 - VoH, 5.0);
  // schlickG1 is the visibility form G1 / (2 NoX), so V = G / (4 NoV NoL) and f = D F V
  float V = schlickG1(NoV, a) * schlickG1(NoL, a);
  // f * NoL / pdf(wi), with pdf(wi) = D*NoH / (4*VoH)
  return F * V * 4.0 * NoL * VoH / NoH;
}
bool sampleGGX(vec3 n, vec3 wo, float a, vec3 F0, vec2 u, out vec3 wi, out vec3 est) {
  float phi = TAU * u.x;
  float cosH = sqrt((1.0 - u.y) / (1.0 + (a * a - 1.0) * u.y));
  float sinH = sqrt(max(0.0, 1.0 - cosH * cosH));
  vec3 t1 = ortho(n), t2 = cross(n, t1);
  vec3 h = normalize(t1 * sinH * cos(phi) + t2 * sinH * sin(phi) + n * cosH);
  if (dot(h, n) <= 0.0) return false;
  wi = reflect(-wo, h);
  /* reflected below the horizon: that energy is lost (single-scattering GGX);
     substituting another direction would add energy that isn't there */
  if (dot(wi, n) <= 1e-5) return false;
  est = ggxOverPdf(n, wo, wi, a, F0);
  return dot(est, est) > 0.0;
}
float fresnelDielectric(float cosI, float eta) {
  float f = (1.0 - eta) / (1.0 + eta);
  f = f * f;
  float c = cosI;
  if (eta > 1.0) {                       // leaving the denser medium
    float sin2T = eta * eta * (1.0 - cosI * cosI);
    if (sin2T >= 1.0) return 1.0;        // total internal reflection
    c = sqrt(1.0 - sin2T);               // Schlick wants the angle on the thinner side
  }
  return clamp(f + (1.0 - f) * pow(1.0 - c, 5.0), 0.0, 1.0);
}

vec3 envRadiance(vec3 rd) {
  float t = clamp(rd.y * 0.5 + 0.5, 0.0, 1.0);
  return uBg * mix(0.72, 1.0, t);
}

/* ----------------------------- path trace --------------------------------- */
/* Adds one radiance contribution to the beauty sum and to the AOVs it belongs to. */
void deposit(vec3 c, bool direct, bool spec, inout vec3 L, inout vec3 dirAcc, inout vec3 specAcc) {
  L += c;
  if (direct) dirAcc += c;
  if (spec) specAcc += c;
}

/* AOV split:
     direct   = emitters seen by the camera, plus light reaching the first visible
                surface straight from an emitter (bounced at most once);
                indirect is everything else, so beauty = direct + indirect exactly
     specular = everything seen through a specular or refractive first hit      */
/* idOut: what the camera ray hit first, 0 = nothing, objects 1..MAX_OBJ, light
   MAX_OBJ + 1. Its running mean tells reprojection whether a pixel still shows
   the same thing (the light sits just below the ceiling with the same normal,
   so depth and normal alone can't tell them apart). */
vec3 trace(vec3 ro, vec3 rd, out vec3 albOut, out vec3 nrmOut, out float depthOut,
           out float hitOut, out float idOut, out vec3 dirOut, out vec3 specOut) {
  vec3 L = vec3(0.0);
  vec3 dirAcc = vec3(0.0);
  vec3 specAcc = vec3(0.0);
  vec3 thr = vec3(1.0);
  bool firstSpec = false;   // the camera's first hit picked a specular / refractive lobe
  /* balance-heuristic weight for light emission reached by the next BSDF-sampled
     ray: < 1 only after a diffuse bounce, where NEE already sampled the light */
  float misW = 1.0;
  albOut = vec3(0.0); nrmOut = vec3(0.0); depthOut = 0.0; hitOut = 0.0; idOut = 0.0;

  for (int b = 0; b < uMaxBounce; b++) {
    bool direct = b <= 1;                  // emission found now has bounced at most once
    Hit h;
    if (!sceneHit(ro, rd, h)) {
      deposit(thr * envRadiance(rd), direct, firstSpec, L, dirAcc, specAcc);
      break;
    }

    if (h.idx < 0) {                       // the area light
      /* give the light real guide values so reprojection and the denoiser see a surface */
      if (hitOut == 0.0) { albOut = vec3(1.0); nrmOut = uLightN; depthOut = h.t; hitOut = 1.0; idOut = float(MAX_OBJ + 1); }
      deposit(thr * uLightEmit * misW, direct, firstSpec, L, dirAcc, specAcc);
      break;
    }

    Surf s = surfOf(h.idx);
    if (dot(s.emis, s.emis) > 1e-8) {      // emissive geometry
      deposit(thr * s.emis, direct, firstSpec, L, dirAcc, specAcc);
    }
    if (hitOut == 0.0) { albOut = s.alb; nrmOut = h.n; depthOut = h.t; hitOut = 1.0; idOut = float(h.idx + 1); }

    vec3 n = h.n;
    vec3 wo = -rd;
    /* which side of the surface the ray arrives from (needed for IOR) */
    float entering = dot(n, wo) > 0.0 ? 1.0 : -1.0;
    if (dot(n, wo) < 0.0) n = -n;          // shade the side the ray arrives from
    vec3 po = h.p + n * EPS;
    /* Beer-Lambert: the ray just crossed h.t units of a transmissive object, whose
       base colour is its transmittance over one unit of distance */
    if (entering < 0.0 && s.trans > 0.0) thr *= pow(max(s.alb, vec3(1e-3)), vec3(h.t));

    float rough = clamp(s.rough, 0.0, 1.0);
    float a = max(rough * rough, 1e-4);
    float metal = clamp(s.metal, 0.0, 1.0);
    float refl  = clamp(s.refl, 0.0, 1.0);
    float trans = clamp(s.trans, 0.0, 1.0);
    float ior   = max(s.ior, 1.0001);

    float wT = trans;
    float wS = clamp(mix(refl, 1.0, metal) * (1.0 - trans), 0.0, 1.0);
    float wD = clamp((1.0 - metal) * (1.0 - trans) * (1.0 - refl), 0.0, 1.0);
    float wSum = wT + wS + wD;
    if (wSum < 1e-3) break;                // fully absorbing surface
    wT /= wSum; wS /= wSum; wD /= wSum;

    /* fixed sample slots per bounce, so each decision draws from its own
       stratified 2D sequence: +0 lobe pick / roulette, +1 light, +2 BSDF, +3 frost */
    int slot = 1 + b * 4;
    vec2 u0 = sample2(slot);
    float pick = u0.x;

    if (pick < wT) {
      /* ---- dielectric lobe (no NEE: transport cancels the Fresnel term) -- */
      float cosI = abs(dot(wo, n));
      float eta = entering > 0.0 ? 1.0 / ior : ior;      // n_incident / n_transmitted
      float F = fresnelDielectric(cosI, eta);
      vec3 wi;
      if (sample2(slot + 2).x < F) {
        wi = reflect(-wo, n);
      } else {
        float k = 1.0 - eta * eta * (1.0 - cosI * cosI);
        wi = (k <= 0.0) ? reflect(-wo, n) : refract(-wo, n, eta);
      }
      if (rough > 0.005) {                 // frosted glass approximation
        vec3 w0 = wi;
        float side = dot(w0, n) >= 0.0 ? 1.0 : -1.0;
        wi = normalize(w0 + rough * rough * 1.5 * randDir(sample2(slot + 3)));
        if (dot(wi, n) * side < 0.0) wi = w0;   // perturbation crossed the surface
      }
      wi = normalize(wi);
      if (b == 0) firstSpec = true;
      /* glass that NEE treats as transparent keeps the pending weight */
      if (uGlassShadow == 1 || trans <= 0.5) misW = 1.0;
      ro = h.p + wi * EPS;
      rd = wi;
    } else if (pick < wT + wS) {
      /* ---- microfacet specular lobe -------------------------------------- */
      /* dielectric F0 from the IOR (0.04 at 1.5), raised toward 1 by reflectivity */
      float f0 = (ior - 1.0) / (ior + 1.0);
      f0 *= f0;
      vec3 F0 = mix(vec3(f0 + (1.0 - f0) * refl), s.alb, metal);
      vec3 wi; vec3 est;
      if (!sampleGGX(n, wo, a, F0, sample2(slot + 2), wi, est)) break;
      thr *= est;
      if (b == 0) firstSpec = true;
      misW = 1.0;
      ro = po; rd = normalize(wi);
    } else {
      /* ---- diffuse lobe: next-event estimation + cosine sampling --------- */
      if (uLightOn == 1 && wD > 1e-4) {
        vec3 ln;
        vec3 lp = lightSample(sample2(slot + 1), ln);
        vec3 dv = lp - po;
        float d2 = dot(dv, dv);
        float dist = sqrt(d2);
        vec3 ld = dv / dist;
        float cosP = dot(n, ld);
        float cosQ = dot(ln, -ld);
        if (cosP > 0.0 && cosQ > 0.0 && !occluded(po, ld, dist - 2.0 * EPS)) {
          float pdfL = d2 / (uLightArea * cosQ);
          float pdfB = wD * cosP / PI;
          float wL = pdfL / (pdfL + pdfB);
          /* no wD factor: picking this lobe with probability wD already cancels
             its wD share of the mixture BSDF */
          vec3 c = thr * wL * (s.alb / PI) * uLightEmit * cosP * (uLightArea * cosQ / d2);
          deposit(c, b == 0, firstSpec, L, dirAcc, specAcc);
        }
      }
      vec3 wi = cosineHemi(n, sample2(slot + 2));
      float cosP = max(dot(n, wi), 1e-5);
      float pdfB = wD * cosP / PI;
      float pdfL = lightPdf(po, wi);
      /* the weight only applies if this ray actually reaches the light, so it is
         carried to the next hit instead of scaling the rest of the path */
      misW = pdfB / (pdfB + pdfL);
      thr *= s.alb;
      ro = po; rd = wi;
    }

    /* Russian roulette (from the 4th bounce on) */
    if (b >= 3) {
      float p = clamp(max(max(thr.x, thr.y), thr.z), 0.04, 1.0);
      if (u0.y > p) break;
      thr /= p;
    }
    if (dot(thr, thr) < 1e-7) break;
  }
  dirOut = dirAcc; specOut = specAcc;
  return L;
}

void main() {
  vec2 pix = gl_FragCoord.xy;
  vec2 uv = pix / uRes;

  vec4 prev = texture(uPrevBeauty, uv);
  float k = uPrevSamples;
  float n = float(uSpp);
  float meanOld = dot(prev.rgb, LUM);
  /* k = 0 starts over without clearing: after a camera move the old buffer
     must stay intact for the history pass, so its contents are ignored here */
  float m2Old = k > 0.0 ? prev.a : 0.0;

  vec3 acc = vec3(0.0), dacc = vec3(0.0), sacc = vec3(0.0);
  vec3 albAcc = vec3(0.0), nrmAcc = vec3(0.0);
  float depAcc = 0.0, hitAcc = 0.0, idAcc = 0.0;
  float sum = 0.0, sumsq = 0.0;

  uint pixKey = uint(pix.x) * 100003u ^ uint(pix.y) * 62041u;
  gPix = ivec2(pix);

  for (int i = 0; i < uSpp; i++) {
    rngState = pcgHash(pixKey + uint(i) * 17u + uSeed);
    gIndex = uSeqBase + uint(k) + uint(i);
    vec2 j = sample2(0) - 0.5;           // fresh sub-pixel offset every sample and frame
    vec2 cc = (pix + j) / uRes * 2.0 - 1.0;
    vec3 rd = normalize(uCamF + uCamR * cc.x * uCamTan + uCamU * cc.y * uCamTan);

    vec3 alb, nrm, dirC, specC; float dep, hitm, oid;
    vec3 L = trace(uCamO, rd, alb, nrm, dep, hitm, oid, dirC, specC);

    /* firefly clamp against the running mean of this pixel */
    if (uFirefly > 0.0 && k > 0.0) {
      float maxL = max(meanOld * uFirefly, 1e-3);
      float l = dot(L, LUM);
      if (l > maxL) { float sc = maxL / l; L *= sc; dirC *= sc; specC *= sc; }
    }

    acc += L; dacc += dirC; sacc += specC;
    albAcc += alb; nrmAcc += nrm; depAcc += dep; hitAcc += hitm; idAcc += oid;
    float lum = dot(L, LUM);
    sum += lum; sumsq += lum * lum;
  }

  vec3 mean = acc / n;
  float meanBatch = sum / n;
  float varBatch = max(0.0, sumsq / n - meanBatch * meanBatch);

  float w = n / (k + n);
  vec3 outBeauty = mix(prev.rgb, mean, w);
  float meanNew = dot(outBeauty, LUM);
  float m2 = m2Old + varBatch * n + (meanBatch - meanOld) * (meanBatch - meanOld) * (n * k / (k + n));

  vec4 pA = texture(uPrevAlbedo, uv);
  vec4 pN = texture(uPrevNormal, uv);
  vec4 pD = texture(uPrevDirect, uv);

  oBeauty = vec4(outBeauty, m2);
  oAlbedo = vec4(mix(pA.rgb, albAcc / n, w), mix(pA.a, depAcc / n, w));
  oNormal = vec4(mix(pN.rgb, nrmAcc / n, w), mix(pN.a, hitAcc / n, w));
  oDirect = vec4(mix(pD.rgb, dacc / n, w), mix(pD.a, idAcc / n, w));   // a = first-hit id (mean)
#if SPEC_TARGET
  oSpec   = vec4(mix(texture(uPrevSpec, uv).rgb, sacc / n, w), 1.0);
#endif
}`;

/* -------------------------------------------------------------------------- */
/* Denoising is an SVGF-style filter (Schied et al. 2017, "Spatiotemporal
   Variance-Guided Filtering"): FS_SVGF_PREP turns the displayed estimate into
   illumination (colour / albedo) plus the variance of its luminance, then five
   FS_DENOISE a-trous passes (spacing 1, 2, 4, 8, 16) blur it along surfaces.
   The luminance edge stop scales with the estimated noise, so a noisy image is
   smoothed hard and a converged one is left almost untouched. */

/* albedo used to split colour into albedo x illumination (shared by both sides) */
const GLSL_DEMOD = `
vec3 demodAlbedo(vec3 alb, float hit) { return hit > 0.5 ? max(alb, vec3(0.02)) : vec3(1.0); }`;

const FS_SVGF_PREP = `#version 300 es
precision highp float;
precision highp sampler2D;
#define LUM vec3(0.2126, 0.7152, 0.0722)

uniform vec2 uRes;
uniform sampler2D uBeauty;   // rgb = running mean, a = M2 of luminance
uniform sampler2D uAlbedo;   // rgb = albedo
uniform sampler2D uNormal;   // rgb = normal, a = hit mask
uniform sampler2D uHist;     // reprojected history: rgb, a = weight in samples
uniform float uSamples, uFade;
out vec4 fragColor;          // rgb = illumination, a = variance of its luminance
` + GLSL_DEMOD + `

void main() {
  ivec2 p = ivec2(gl_FragCoord.xy);
  ivec2 res = ivec2(uRes);
  vec4 b = texelFetch(uBeauty, p, 0);
  vec4 h = texelFetch(uHist, p, 0);
  vec4 n = texelFetch(uNormal, p, 0);
  float s = uSamples;
  /* the same history blend the display uses */
  float hEff = h.a * clamp(1.0 - s / uFade, 0.0, 1.0);
  float nEff = max(hEff + s, 1.0);
  vec3 c = (hEff * h.rgb + s * b.rgb) / nEff;
  vec3 alb = demodAlbedo(texelFetch(uAlbedo, p, 0).rgb, n.a);

  float var;
  if (s >= 4.0) {
    var = b.a / (s - 1.0) / nEff;          // per-sample variance over effective samples
  } else {
    /* too few samples to trust: spread of the means over this surface in 5x5 */
    float m1 = 0.0, m2 = 0.0, cnt = 0.0;
    for (int y = -2; y <= 2; y++) for (int x = -2; x <= 2; x++) {
      ivec2 q = clamp(p + ivec2(x, y), ivec2(0), res - 1);
      vec4 nq = texelFetch(uNormal, q, 0);
      if ((nq.a > 0.5) != (n.a > 0.5) || dot(nq.rgb, n.rgb) < 0.9 * length(nq.rgb) * length(n.rgb)) continue;
      float l = dot(texelFetch(uBeauty, q, 0).rgb, LUM);
      m1 += l; m2 += l * l; cnt += 1.0;
    }
    m1 /= cnt; m2 /= cnt;
    var = max(m2 - m1 * m1, 0.0) * max(s, 1.0) / nEff;
  }
  float la = max(dot(alb, LUM), 1e-3);
  /* luminance tolerance in standard deviations, folded into the variance so it
     can follow this pixel's effective sample count (history included): loose
     while noisy, tight as it converges. Measured best: ~5.5 at 4 spp, ~1.6 from
     64 spp on. The strength slider scales the result in the a-trous passes. */
  float sched = clamp(5.5 - 1.3 * log2(nEff / 4.0), 1.6, 5.5);
  fragColor = vec4(c / alb, var / (la * la) * sched * sched);
}`;

/* one a-trous iteration */
const FS_DENOISE = `#version 300 es
precision highp float;
precision highp sampler2D;
#define LUM vec3(0.2126, 0.7152, 0.0722)

uniform vec2 uRes;
uniform sampler2D uIn;       // rgb = illumination, a = its variance
uniform sampler2D uAlbedo;   // a = first-hit distance
uniform sampler2D uNormal;   // rgb = normal, a = hit mask
uniform int   uStep;         // tap spacing: 1, 2, 4, 8, 16
uniform float uSigmaL;       // scales the per-pixel tolerance the prep pass folded into a
uniform vec3  uCamO, uCamR, uCamU, uCamF;
out vec4 fragColor;

vec3 rayDir(ivec2 q) {
  vec2 cc = (vec2(q) + 0.5) / uRes * 2.0 - 1.0;
  return normalize(uCamF + uCamR * cc.x + uCamU * cc.y);
}

void main() {
  ivec2 p = ivec2(gl_FragCoord.xy);
  ivec2 res = ivec2(uRes);
  vec4 cp = texelFetch(uIn, p, 0);
  vec4 np = texelFetch(uNormal, p, 0);
  bool hitP = np.a > 0.5;
  vec3 nP = np.rgb / max(length(np.rgb), 1e-6);
  float dP = texelFetch(uAlbedo, p, 0).a;
  vec3 Pp = uCamO + rayDir(p) * dP;
  float pixelSize = 2.0 * dP * length(uCamU) / uRes.y;   // world size of a pixel here
  float lP = dot(cp.rgb, LUM);

  /* a 3x3 Gaussian of the variance steadies the luminance stop */
  float gv = 0.0;
  for (int y = -1; y <= 1; y++) for (int x = -1; x <= 1; x++) {
    float g = (x == 0 ? 0.5 : 0.25) * (y == 0 ? 0.5 : 0.25);
    gv += g * texelFetch(uIn, clamp(p + ivec2(x, y), ivec2(0), res - 1), 0).a;
  }
  float lumDen = uSigmaL * sqrt(max(gv, 0.0)) + 1e-6;

  const float kern[3] = float[3](0.375, 0.25, 0.0625);   // B3 spline
  vec3 sum = vec3(0.0);
  float wsum = 0.0, vsum = 0.0;
  for (int y = -2; y <= 2; y++) {
    for (int x = -2; x <= 2; x++) {
      ivec2 q = p + ivec2(x, y) * uStep;
      if (q.x < 0 || q.y < 0 || q.x >= res.x || q.y >= res.y) continue;
      vec4 cq = texelFetch(uIn, q, 0);
      float w = kern[abs(x)] * kern[abs(y)];
      if (x != 0 || y != 0) {
        vec4 nq = texelFetch(uNormal, q, 0);
        if ((nq.a > 0.5) != hitP) continue;
        if (hitP) {
          /* same orientation, and on the same plane (distance off p's tangent
             plane, relative to how far apart the pixels are) */
          vec3 nQ = nq.rgb / max(length(nq.rgb), 1e-6);
          w *= pow(max(dot(nP, nQ), 0.0), 64.0);
          vec3 Pq = uCamO + rayDir(q) * texelFetch(uAlbedo, q, 0).a;
          float plane = abs(dot(nP, Pq - Pp));
          w *= exp(-plane / (2.0 * pixelSize * length(vec2(x, y)) * float(uStep) + 1e-4));
        }
        w *= exp(-abs(dot(cq.rgb, LUM) - lP) / lumDen);
      }
      sum += w * cq.rgb;
      wsum += w;
      vsum += w * w * cq.a;
    }
  }
  fragColor = vec4(sum / wsum, vsum / (wsum * wsum));
}`;

/* -------------------------------------------------------------------------- */
/* Camera-motion history. After a move, the old view's image (its accumulation
   blended with its own history) is reprojected into the new view through each
   pixel's surface point. Taps whose depth, normal or coverage disagree are
   dropped, so disoccluded pixels start clean. The result only feeds the display
   (and the denoiser); the accumulation itself restarts, so it stays unbiased. */
const FS_HISTORY = `#version 300 es
precision highp float;
precision highp int;
precision highp sampler2D;
#define LUM vec3(0.2126, 0.7152, 0.0722)

uniform vec2 uRes;
uniform vec3 uCamO, uCamR, uCamU, uCamF;      // new camera
uniform vec3 uPrevO, uPrevR, uPrevU, uPrevF;  // camera of the old buffers
uniform sampler2D uNewBeauty, uNewAlb, uNewNrm, uNewSpec, uNewDirect;
uniform sampler2D uOldBeauty, uOldAlb, uOldNrm, uOldHist, uOldDirect;   // direct.a = first-hit id
uniform float uOldSamples, uCap, uFade;
uniform int   uHasSpec;
out vec4 fragColor;          // rgb = history colour, a = its weight in samples

void main() {
  ivec2 ip = ivec2(gl_FragCoord.xy);
  ivec2 res = ivec2(uRes);
  vec2 cc = gl_FragCoord.xy / uRes * 2.0 - 1.0;
  vec3 rd = normalize(uCamF + uCamR * cc.x + uCamU * cc.y);
  vec4 na = texelFetch(uNewAlb, ip, 0);
  vec4 nn = texelFetch(uNewNrm, ip, 0);
  bool hitNow = nn.a > 0.5;
  vec3 P = uCamO + rd * (hitNow ? na.a : 1e4);   // background: reproject by direction
  /* a pixel straddling two objects has a fractional mean id: no history there */
  float idNow = texelFetch(uNewDirect, ip, 0).a;
  if (abs(idNow - floor(idNow + 0.5)) > 0.01) { fragColor = vec4(0.0); return; }

  vec3 v = P - uPrevO;
  float z = dot(v, uPrevF);
  if (z <= 1e-4) { fragColor = vec4(0.0); return; }
  vec2 cp = vec2(dot(v, uPrevR) / dot(uPrevR, uPrevR), dot(v, uPrevU) / dot(uPrevU, uPrevU)) / z;
  vec2 pp = (cp * 0.5 + 0.5) * uRes - 0.5;       // old pixel coords, texel centres on integers
  vec2 fl = floor(pp), f = pp - fl;
  ivec2 i0 = ivec2(fl);
  float dist = length(v);
  float fade = clamp(1.0 - uOldSamples / uFade, 0.0, 1.0);

  vec3 acc = vec3(0.0);
  float wsum = 0.0, hsum = 0.0;
  for (int t = 0; t < 4; t++) {
    ivec2 o = ivec2(t & 1, t >> 1);
    ivec2 q = i0 + o;
    if (q.x < 0 || q.y < 0 || q.x >= res.x || q.y >= res.y) continue;
    vec4 oa = texelFetch(uOldAlb, q, 0);
    vec4 on = texelFetch(uOldNrm, q, 0);
    if ((on.a > 0.5) != hitNow) continue;
    if (abs(texelFetch(uOldDirect, q, 0).a - idNow) > 0.01) continue;     // same object only
    if (hitNow) {
      if (abs(oa.a - dist) > 0.03 * dist + 0.01) continue;
      if (dot(on.rgb, nn.rgb) < 0.8 * length(on.rgb) * length(nn.rgb)) continue;
    }
    vec4 oh = texelFetch(uOldHist, q, 0);
    float h = oh.a * fade;
    float wt = h + uOldSamples;
    if (wt <= 0.0) continue;
    float b = (o.x == 1 ? f.x : 1.0 - f.x) * (o.y == 1 ? f.y : 1.0 - f.y);
    acc += b * (h * oh.rgb + uOldSamples * texelFetch(uOldBeauty, q, 0).rgb) / wt;
    wsum += b;
    hsum += b * wt;
  }
  if (wsum < 0.25) { fragColor = vec4(0.0); return; }

  /* view-dependent light (mirrors, glass) would smear, so keep less of it */
  float cap = uCap;
  if (uHasSpec == 1) {
    float sp = 0.0, tot = 0.0;
    for (int y = -1; y <= 1; y++) for (int x = -1; x <= 1; x++) {
      ivec2 q = clamp(ip + ivec2(x, y), ivec2(0), res - 1);
      sp += dot(texelFetch(uNewSpec, q, 0).rgb, LUM);
      tot += dot(texelFetch(uNewBeauty, q, 0).rgb, LUM);
    }
    cap = mix(uCap, 2.0, clamp(sp / max(tot, 1e-4), 0.0, 1.0));
  }
  fragColor = vec4(acc / wsum, min(hsum / wsum, cap) * min(wsum, 1.0));
}`;

/* -------------------------------------------------------------------------- */
const FS_DISPLAY = `#version 300 es
precision highp float;
precision highp int;
precision highp sampler2D;

#define MODE_BEAUTY 0
#define MODE_NORMAL 1
#define MODE_ALBEDO 2
#define MODE_DEPTH  3
#define MODE_DIRECT 4
#define MODE_IND    5
#define MODE_SPEC   6
#define MODE_CONV   7
#define LUM vec3(0.2126, 0.7152, 0.0722)

uniform vec2 uRes;        // accumulation resolution
uniform vec2 uOutRes;     // output resolution
uniform sampler2D uBeauty, uAlbedo, uNormal, uDirect, uSpec, uRef, uDenoise, uHist;
uniform float uSamples;
uniform float uFade;      // fresh samples over which camera-motion history fades out
uniform int   uMode;
uniform int   uCompare;   // 0 off, 1 split, 2 reference, 3 difference (beauty view only)
uniform int   uLinearOut; // 1 = write the linear beauty composite (reference capture)
uniform int   uTonemap;   // 0 none, 1 reinhard, 2 reinhard white, 3 aces, 4 filmic, 5 high contrast
uniform int   uHasSpec;
uniform float uExposure, uGamma, uFar, uDirectScale, uIndirectScale;
uniform int   uDenoiseOn;

in vec2 vUv;
out vec4 fragColor;
` + GLSL_DEMOD + `

/* manual bilinear tap: keeps accumulation textures on NEAREST filtering, so
   no OES_texture_float_linear support is required. */
vec4 bilinear(sampler2D t, vec2 uv) {
  vec2 px = uv * uRes - 0.5;
  vec2 i0 = floor(px);
  vec2 f = px - i0;
  vec2 c0 = clamp((i0 + 0.5) / uRes, vec2(0.0), 1.0 - 1.0 / uRes);
  vec2 c1 = clamp((i0 + 1.5) / uRes, vec2(0.0), 1.0 - 1.0 / uRes);
  vec4 a = texture(t, vec2(c0.x, c0.y));
  vec4 b = texture(t, vec2(c1.x, c0.y));
  vec4 c = texture(t, vec2(c0.x, c1.y));
  vec4 d = texture(t, vec2(c1.x, c1.y));
  return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

vec3 reinhard(vec3 c)  { return c / (1.0 + c); }
vec3 reinhardWhite(vec3 c) { float w = 5.0; return c * (1.0 + c / (w * w)) / (1.0 + c); }
vec3 aces(vec3 x) {
  const float a = 2.51, b = 0.03, cc = 2.43, dd = 0.59, e = 0.14;
  return clamp((x * (a * x + b)) / (x * (cc * x + dd) + e), 0.0, 1.0);
}
vec3 hable(vec3 x) {
  x += 0.024;
  const float W = 11.2, F = 4.0;
  vec3 n = x * (2.51 * x + 0.03);
  vec3 d = x * (2.43 * x + 0.59) + 0.14;
  vec3 wn = vec3(W) * (2.51 * vec3(W) + 0.03);
  vec3 wd = vec3(W) * (2.43 * vec3(W) + 0.59) + 0.14;
  return clamp((n / d) / (wn / wd), 0.0, 1.0);
}
vec3 tone(vec3 c) {
  if (uTonemap == 1) return reinhard(c);
  if (uTonemap == 2) return reinhardWhite(c);
  if (uTonemap == 3) return aces(c);
  if (uTonemap == 4) return hable(c);
  if (uTonemap == 5) return pow(aces(c), vec3(1.15));
  return c;
}
vec3 heat(float t) {
  t = clamp(t, 0.0, 1.0);
  vec3 c = mix(vec3(0.02, 0.09, 0.35), vec3(0.0, 0.78, 0.72), smoothstep(0.0, 0.35, t));
  c = mix(c, vec3(0.92, 0.80, 0.10), smoothstep(0.35, 0.70, t));
  c = mix(c, vec3(0.95, 0.16, 0.10), smoothstep(0.70, 1.0, t));
  return c;
}
vec3 post(vec3 c, bool lit) {
  if (lit) c *= uExposure;
  if (lit) c = tone(c);
  c = clamp(c, 0.0, 1.0);
  return pow(max(c, 0.0), vec3(1.0 / max(uGamma, 0.1)));
}

void main() {
  vec2 uv = vUv;
  vec4 bx = bilinear(uBeauty, uv);
  vec4 ax = bilinear(uAlbedo, uv);
  vec4 nx = bilinear(uNormal, uv);

  vec3 raw = bx.rgb;
  vec3 direct = bilinear(uDirect, uv).rgb;
  vec3 spec = uHasSpec == 1 ? bilinear(uSpec, uv).rgb : vec3(0.0);
  /* every sample's direct part is a subset of it, so this only clips float error */
  vec3 indirect = max(vec3(0.0), raw - direct);

  /* camera-motion history fills in while fresh samples are few; its weight
     fades to zero over uFade samples, so the converged image is history-free */
  vec4 hs = bilinear(uHist, uv);
  float hEff = hs.a * clamp(1.0 - uSamples / uFade, 0.0, 1.0);
  vec3 beauty = (hEff * hs.rgb + uSamples * raw) / max(hEff + uSamples, 1e-6);
  if (uDenoiseOn == 1) {
    /* the filter works on illumination (colour / albedo); put the albedo back */
    beauty = bilinear(uDenoise, uv).rgb * demodAlbedo(ax.rgb, nx.a);
  }
  /* the light sliders rescale the two halves of the image; at 1 / 1 this is
     exactly the (denoised) beauty */
  vec3 shaded = max(vec3(0.0), beauty + (uDirectScale - 1.0) * direct + (uIndirectScale - 1.0) * indirect);
  if (uLinearOut == 1) { fragColor = vec4(shaded, 1.0); return; }

  vec3 live;
  if (uMode == MODE_BEAUTY)      live = shaded;
  else if (uMode == MODE_NORMAL) live = nx.rgb * 0.5 + 0.5;
  else if (uMode == MODE_ALBEDO) live = ax.rgb;
  else if (uMode == MODE_DEPTH) {
    float d = ax.a; float mask = nx.a;
    float v = clamp(1.0 - d / max(uFar, 0.01), 0.0, 1.0);
    live = mask > 0.5 ? mix(vec3(0.05, 0.12, 0.30), vec3(1.0), v * v) : vec3(0.0);
  }
  else if (uMode == MODE_DIRECT) live = direct * uDirectScale;
  else if (uMode == MODE_IND)    live = indirect * uIndirectScale;
  else if (uMode == MODE_SPEC)   live = spec;
  else {
    /* relative standard error of the running mean (shrinks as 1/sqrt(k)), on a
       log scale: blue <= 0.1 %, red >= 30 % */
    float k = max(uSamples, 1.0);
    float sem = sqrt(max(bx.a, 0.0) / (max(k - 1.0, 1.0) * k));
    float rel = sem / max(dot(raw, LUM), 1e-4);
    live = heat((log(max(rel, 1e-6)) / log(10.0) + 3.0) / 2.5);
  }

  bool lit = !(uMode == MODE_NORMAL || uMode == MODE_ALBEDO || uMode == MODE_DEPTH || uMode == MODE_CONV);
  vec3 col = post(live, lit);

  /* the reference holds a linear beauty composite, so it is only comparable there */
  if (uCompare != 0 && uMode == MODE_BEAUTY) {
    vec3 refLin = bilinear(uRef, uv).rgb;
    vec3 refCol = post(refLin, true);
    if (uCompare == 2) col = refCol;
    else if (uCompare == 1) {
      col = uv.x < 0.5 ? refCol : col;
      float lw = 1.5 / max(uOutRes.x, 1.0);
      if (abs(uv.x - 0.5) < lw) col = vec3(1.0);
    } else {
      float d = dot(abs(shaded - refLin), LUM);
      col = post(vec3(d) * 3.0, true);
    }
  }

  fragColor = vec4(col, 1.0);
}`;
