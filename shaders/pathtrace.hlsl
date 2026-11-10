#include "common.hlsli"
#include "layers.hlsli"
uint hash(uint x){x^=x>>16;x*=0x7feb352d;x^=x>>15;x*=0x846ca68b;x^=x>>16;return x;}
static uint rng_state=0,sequence_index=0,dimension_index=0,pixel_seed=0;
uint permute(uint x,uint seed){x^=x*0x3d20adea;x+=seed;x*=(seed>>16)|1;x^=x*0x05526c56;x^=x*0x53a22864;return x;}
uint owen(uint x,uint seed){return reversebits(permute(reversebits(x),seed));}
// Second Sobol dimension, in reversed bit order. Its generator is the Pascal matrix mod 2:
// bit m is the parity of the index bits k that contain m as a submask, so a 5-step superset
// transform replaces the 32-step loop over direction numbers (bit-identical results).
uint superset(uint i){i^=(i>>1)&0x55555555;i^=(i>>2)&0x33333333;i^=(i>>4)&0x0f0f0f0f;i^=(i>>8)&0x00ff00ff;i^=(i>>16)&0x0000ffff;return i;}
static float pair_second;
// The ReSTIR passes always draw white noise (they are not tied to a sample index).
static bool white_noise;
// BDPT and photon mapping (integrators.hlsl) see glass as solid in shadow rays: their caustics
// come from real paths, not from the path tracer's transparent shadows.
static bool glass_blocks;
// Debug statistics for this pixel and frame (views 8 and 9).
static uint proceed_steps=0;
float random(){
    if(!white_noise && jitter.z>0.5 && modes.w==0){
        // Padded 2D Owen-scrambled Sobol. RR uses independent white noise per NVIDIA's guide.
        // Both values of a dimension pair share one shuffled index, so compute them together.
        // owen(reversebits(x)) and owen(reversebits(superset(x))) reduce to one reversal each.
        uint dim=dimension_index++;if((dim&1)!=0)return pair_second;
        uint seed=hash(pixel_seed+(dim/2)*0x9e3779b9),index=owen(sequence_index,seed);
        pair_second=(reversebits(permute(superset(index),hash(seed^0x02e5be93)))>>8)*(1.0/16777216.0);
        return (reversebits(permute(index,hash(seed^0x68bc21eb)))>>8)*(1.0/16777216.0);
    }
    rng_state=hash(rng_state+0x9e3779b9);return (rng_state>>8)*(1.0/16777216.0);
}
float3 basis(float3 n,float3 h){float3 u=normalize(cross(abs(n.z)<0.999?float3(0,0,1):float3(0,1,0),n));return u*h.x+cross(n,u)*h.y+n*h.z;}
float3 cosine_sample(float3 n){float r=sqrt(random()),a=2*PI*random();return basis(n,float3(r*cos(a),r*sin(a),sqrt(max(0,1-r*r))));}
// n: shading normal (normal-mapped), ng: interpolated normal for ray offsets, both facing the ray.
// Texture maps are sampled in intersect (fewer live registers than the bounce loop): textured is
// set when albedo and rough_metal hold the mapped base colour, roughness and metallic.
// tangent: the brushing direction of an anisotropic material or a grating's grooves (layers.w > 0
// or optical.w < 0, else zero), not
// orthogonalised: the object's x axis, or with flag 2048 (spun) circles around its y axis.
struct Hit{float t;float3 p;float3 n;float3 ng;float3 tangent;float3 albedo;float2 rough_metal;uint material;uint emitter;bool front;bool textured;};
// Candidates only come from non-opaque geometry: static range 0 (no offset) or a glass object.
uint candidate_material(uint instance,uint prim){Instance inst=instances[instance];return inst.material!=0xffffffffu?inst.material:static_vertices[(inst.vertex_base+prim)*3].material;}
float4 sample_map(uint slot,float2 uv){return maps[NonUniformResourceIndex(slot-1)].SampleLevel(map_sampler,uv,0);}
// Ray cones pick the mip level (core track 5): the width of the cone at the current ray's origin
// Camera cones are one *output* pixel wide (dimensions.w), not one render pixel: with an upscaler
// (DLSS, XeSS, FSR) at a lower render scale that is the vendors' texture LOD bias log2(render/output),
// so the upscaler gets the output's texture detail instead of mips one level blurrier per halving.
// and its spread per unit length. Camera rays start at one pixel's angle; each rough bounce
// widens the cone (a crude stand-in for the lobe's width), so indirect hits read small mips.
static float cone_width,cone_spread;
// The footprint is an ellipse in uv: its axis across the ray (the cone's width) and along it
// (stretched by the grazing angle). The anisotropic sampler takes both as gradients: the mip
// follows the short axis and up to lights.w taps run along the long one. Past that ratio the short
// axis grows, as hardware does; at 1 both axes are the long one (isotropic, the old behaviour).
struct Footprint{float2 minor;float2 major;};
float4 sample_map_lod(uint slot,float2 uv,Footprint f){
    float lm=length(f.minor),lM=length(f.major),n=max(lights.w,1);
    if(lM>n*lm)f.minor=(lm>1e-20?f.minor/lm:float2(-f.major.y,f.major.x)/max(lM,1e-20))*lM/n;
    return maps[NonUniformResourceIndex(slot-1)].SampleGrad(map_sampler,uv,f.minor,f.major);
}
// The ray cone at a hit (width w, direction d, normal n facing the ray) as uv axes, with T and B
// the world-space dP/du and dP/dv (zero T: an isotropic footprint of w · uv_scale).
Footprint footprint(float3 d,float3 n,float w,float3 T,float3 B,float uv_scale){
    float c=max(abs(dot(d,n)),0.05);Footprint f;
    float tt=dot(T,T),tb=dot(T,B),bb=dot(B,B),det=tt*bb-tb*tb;
    if(tt==0 || det<=1e-20*tt*bb){f.minor=float2(w*uv_scale,0);f.major=float2(0,w*uv_scale/c);return f;}
    float3 across=cross(n,d);float len=length(across);
    across=len>1e-6?across/len:normalize(T-n*dot(n,T));
    float3 along=cross(across,n);
    // A world vector X in the tangent plane moves uv by G⁻¹ (X·T, X·B), G the Gram matrix of T, B.
    float2x2 inv=float2x2(bb,-tb,-tb,tt)/det;
    f.minor=mul(inv,float2(dot(across,T),dot(across,B)))*w;
    f.major=mul(inv,float2(dot(along,T),dot(along,B)))*(w/c);
    return f;
}
// Material flag bits (lobes.w): 1 unlit, 2 hidden from the camera, 4 portal, 8 masked texture
// maps, 16 any texture maps, 4096 beam splitter, 8192 a light that casts no shadows. Without TEXTURE_MAPS (build.rs's `trace` variant, for scenes with
// no maps) the map code folds away: it cost untextured scenes about 7% even when never run.
// Diffraction gratings ride in the same variant (Gpu picks it when any material has one): their
// code cost scenes without one 2–15% in registers even when never run.
#ifdef TEXTURE_MAPS
static const bool texture_maps=true;
#else
static const bool texture_maps=false;
#endif
// LEAN (track 20a (b)): variants for scenes without ReSTIR, fog or volume media, SDF
// procedurals, or the layered-BSDF extras (coat, sheen, anisotropy, thin film). The stripped
// material fields and the constant flags fold those branches away, which frees registers in
// the whole kernel (Sponza's trace pass −35%, Classic's −26%). Gpu::integrate picks a LEAN
// variant only when the scene and settings use none of them (Gpu::lean_scene).
#ifdef LEAN
static const bool lean=true;
#define RQ_BASE RAY_FLAG_SKIP_PROCEDURAL_PRIMITIVES
#else
static const bool lean=false;
#define RQ_BASE RAY_FLAG_NONE
#endif
// Opacity micromaps (track 20a (a)): alpha-tested meshes carry 4-state OMMs (Gpu omm_blas), so
// the RT cores settle known-opaque and known-transparent texels and only unknown ones come back
// to the alpha test. Shader Model 6.9 asks for them per RayQuery; the older compute tracers get
// them through the pipeline flag (Gpu::compute_pipeline).
#ifdef SER
#define RQ_OMM ,RAYQUERY_FLAG_ALLOW_OPACITY_MICROMAPS
#else
#define RQ_OMM
#endif
// Spectral rendering (engine-notes "Spectral rendering"): the SPECTRAL variant carries three
// wavelengths per path (hero wavelength sampling, Wilkie et al. 2014, with three instead of four)
// in the float3 colour channels, so every per-channel operation of the tracer is per wavelength.
// RGB values become spectra where they enter (up(): Mallett and Yuksel's primaries, linear, so
// emission and HDRIs uplift too) and a sample's radiance goes back to linear sRGB at its end
// (to_rgb()). D65 sits in the output weights. A thin film is exact per wavelength. An event whose
// direction depends on the wavelength (dispersion, a grating's orders, a dispersive real lens)
// keeps the hero only (hero()): the other two are dropped and the hero weighs 3, as pbrt-v4's
// TerminateSecondary.
// Wavelengths come from a per-pixel rotated Weyl sequence over the sample index, not from the
// Sobol dimensions, so the RGB variants' sampling is untouched.
// Initialized: a static without one starts with whatever the thread's registers held, and the
// RGB variants never write these (an RGB BDPT hung in the self-test after a spectral battle
// frame, 2026-11-03).
static float3 path_lambdas=550; // nm
static bool hero_only=false;
#ifdef SPECTRAL
#include "spectral_tables.hlsli"
static const bool spectral_on=true;
static float3x3 uplift_rows; // row i: the basis spectra at wavelength i
float3 spectral_table(float l,bool out_table){
    float x=clamp((l-SPECTRAL_LO)/SPECTRAL_STEP,0,SPECTRAL_COUNT-1.001);uint i=(uint)x;float f=x-i;
    return out_table?lerp(SPECTRAL_OUT[i],SPECTRAL_OUT[i+1],f):lerp(SPECTRAL_BASIS[i],SPECTRAL_BASIS[i+1],f);
}
// Visible-wavelength importance (Radziszewski et al. 2009, as pbrt-v4): pdf ∝ sech²(a(λ − 538)),
// cut to the tables' range, inverted in closed form.
static const float WAVE_A=0.0072;
float wave_t(float l){return tanh(WAVE_A*(l-538));}
float wave_pdf(float l){float c=cosh(WAVE_A*(l-538));return WAVE_A/((wave_t(SPECTRAL_HI)-wave_t(SPECTRAL_LO))*c*c);}
float wave_sample(float u){float t=lerp(wave_t(SPECTRAL_LO),wave_t(SPECTRAL_HI),u);return 538+0.5*log((1+t)/(1-t))/WAVE_A;}
void begin_wavelengths(uint key){
    float u=(hash(hash(pixel_seed^0x2c1b3c6du)+key*0x9e3779b9u)>>8)*(1.0/16777216.0);
    [unroll]for(uint i=0;i<3;i++){path_lambdas[i]=wave_sample(frac(u+i/3.0));uplift_rows[i]=spectral_table(path_lambdas[i],false);}
    hero_only=false;
}
float3 up(float3 rgb){return mul(uplift_rows,rgb);}
// A sample's spectral radiance as linear sRGB: (1/3) Σ out(λi) v_i / p(λi), or the hero alone.
float3 to_rgb(float3 v){
    float3 c=0;[unroll]for(uint i=0;i<3;i++)c+=spectral_table(path_lambdas[i],true)*(v[i]/(3*wave_pdf(path_lambdas[i])));
    return c;
}
void hero(inout float3 throughput){if(!hero_only){throughput=float3(3*throughput.x,0,0);hero_only=true;}}
// A material's own spectrum (phase.z, `lighting::spectrum_fields`): > 0 a measured one (the
// Cornell box's white, green, red, light), < 0 a blackbody at −phase.z K, Planck relative to
// 560 nm over D65 (emission is relative to D65); emitters (blackbody, the light) times phase.w,
// so the spectrum's RGB is the colour.
float4 spectral_measured(float l){
    float x=clamp((l-SPECTRAL_LO)/SPECTRAL_STEP,0,SPECTRAL_COUNT-1.001);uint i=(uint)x;float f=x-i;
    return lerp(SPECTRAL_MEASURED[i],SPECTRAL_MEASURED[i+1],f);
}
float spectral_d65(float l){float x=clamp((l-SPECTRAL_LO)/SPECTRAL_STEP,0,SPECTRAL_COUNT-1.001);uint i=(uint)x;return lerp(SPECTRAL_D65[i],SPECTRAL_D65[i+1],x-i);}
float3 own_spectrum(Material m){
    float3 v;
    [unroll]for(uint i=0;i<3;i++){float l=path_lambdas[i];
        if(m.phase.z<0){float t=-m.phase.z,r=560/l;v[i]=r*r*r*r*r*(exp(1.438777e7/(560*t))-1)/(exp(1.438777e7/(l*t))-1)/spectral_d65(l)*m.phase.w;}
        else{float4 s=spectral_measured(l);uint c=(uint)m.phase.z;v[i]=c==1?s.x:c==2?s.y:c==3?s.z:s.w*m.phase.w;}}
    return v;
}
#else
static const bool spectral_on=false;
void begin_wavelengths(uint key){}
float3 up(float3 rgb){return rgb;}
float3 to_rgb(float3 v){return v;}
void hero(inout float3 throughput){}
float3 own_spectrum(Material m){return 0;}
#endif
// A material's colour (an albedo) as the tracer carries it: its measured reflectance (phase.z
// 1–3) where it has one, else the uplift (RGB variants: the colour). An emitter's own spectrum
// (the light, a blackbody) is emission relative to D65, far above 1 in places, never an albedo.
float3 up_material(Material m,float3 rgb){return spectral_on && m.phase.z>0 && m.phase.z<4?own_spectrum(m):up(rgb);}
// A material's emitted colour as the tracer carries it: its own spectrum where it has one.
float3 up_emission(Material m,float3 rgb){return spectral_on && m.phase.z!=0?own_spectrum(m):up(rgb);}
// Emitted RGB x of material id (its colour times scalars) as the tracer carries it.
float3 up_emitted(uint id,float3 x){
    if(spectral_on){Material m=materials[id];if(m.phase.z!=0)return own_spectrum(m)*(dot(x,1)/max(dot(m.color_rough.rgb,1),1e-12));}
    return up(x);
}
// A medium's extinction per channel: sigma times the tint packed 8 bits per channel in t
// (`lighting::pack_tint`; 0 = grey), uplifted in the spectral variants.
float3 tint_of(float t){uint p=(uint)t;return p==0?1:up(float3(p&255u,(p>>8)&255u,p>>16)/255.0);}
float3 extinction_of(Material m){return m.medium.w*(m.phase.z<0 || m.phase.z==4?1:tint_of(m.phase.w));}
float3 fog_extinction(){return fog.x*tint_of(fog_color.w);}
// Free flight over [t0, t1) in a medium of extinction sigma, from t0. Grey: the old sampling
// (the pdf cancels the transmittance, w = 1). Chromatic (Kutz et al. 2017 / pbrt-v3): a
// channel picked uniformly sets the distance and w is the one-sample MIS weight, T·σ / mean(T·σ)
// at a scatter (ts < t1) or T / mean(T) past t1; the caller multiplies its throughput by w.
float flight(float3 sigma,float t0,float t1,out float3 w){
    w=1;if(hero_only)sigma=sigma.xxx;
    if(all(sigma==sigma.x))return t0-log(1-random())/sigma.x;
    uint c=min((uint)(random()*3),2u);float s=c==0?sigma.x:c==1?sigma.y:sigma.z;
    float ts=t0-log(1-random())/max(s,1e-12);
    float3 f=ts<t1?exp(-sigma*(ts-t0))*sigma:exp(-sigma*(t1-t0));
    w=f/max((f.x+f.y+f.z)/3,1e-30);
    return ts;
}
// Dispersion (spectral only): phase.y holds the Abbe number V (0 = none). Cauchy's n(λ) = A +
// B/λ² through n_d = optical.x at 587.6 nm with n_F − n_C = (n_d − 1)/V; read at the hero
// wavelength once the path keeps only it.
bool dispersive(Material m){return spectral_on && m.phase.y>0;}
float ior_of(Material m){
    if(!dispersive(m) || !hero_only)return m.optical.x;
    float l=path_lambdas.x*1e-3,b=(m.optical.x-1)/(m.phase.y*(1/(0.4861*0.4861)-1/(0.6563*0.6563)));
    return m.optical.x+b*(1/(l*l)-1/(0.5876*0.5876));
}
bool fog_on(){return !lean && fog.x>0;}
bool restir_on(){return !lean && (restir.x&1)!=0;}
// The gates only (coat, anisotropy, sheen, film strength, extinction): every use of the other
// layer fields sits behind one of them, and sheen.w also turns gratings (kept).
void lean_strip(inout Material m){if(lean){m.layers.x=0;m.layers.w=0;m.sheen.rgb=0;m.surface_scale.z=0;m.medium.w=0;}}
bool has_grating(Material m){return texture_maps && m.optical.w<0;}
bool flagged(uint m,uint bit){return texture_maps && ((uint)materials[m].lobes.w&bit)!=0;}
// Masked texture maps: a candidate whose base-colour alpha is below the cutoff is a hole. A
// negative cutoff is glTF BLEND, −(the factor's alpha): stochastic transparency, a hole with
// probability 1 − alpha, from a hash of the pixel, sample, path state and the candidate (no
// random() call, which would shift the Sobol dimensions by the candidate count).
bool alpha_hole(uint instance,uint prim,float2 b,uint m){
    uint4 mp=materials[m].maps;
    uint idx=instances[instance].vertex_base+prim*3;
    float2 uv=vertices[idx].uv*(1-b.x-b.y)+vertices[idx+1].uv*b.x+vertices[idx+2].uv*b.y;
    float a=sample_map(mp.x,uv).a,c=asfloat(mp.w);
    if(c>=0)return a<c;
    uint k=hash(pixel_seed^hash(sequence_index*0x9e3779b9u+dimension_index)^rng_state^hash(instance*0x85ebca6bu+prim)^asuint(b.x));
    return a*-c<=(k>>8)*(1.0/16777216.0);
}
// Texture sets on geometry without UVs (core track 5): box mapping along the dominant axis of
// the normal n, from a position p in world units, with dP/du (t) and dP/dv (bt) in that space.
void box_map(float3 p,float3 n,float scale,out float2 uv,out float3 t,out float3 bt){
    float3 a=abs(n);float s;
    if(a.x>=a.y && a.x>=a.z){s=n.x<0?-1:1;uv=float2(-s*p.z,-p.y);t=float3(0,0,-s);bt=float3(0,-1,0);}
    else if(a.y>=a.z){s=n.y<0?-1:1;uv=float2(p.x,s*p.z);t=float3(1,0,0);bt=float3(0,0,s);}
    else{s=n.z<0?-1:1;uv=float2(s*p.x,-p.y);t=float3(s,0,0);bt=float3(0,-1,0);}
    uv*=scale;
}
// glTF v runs down the image, so the map's +Y (green) is -dP/dv: the bitangent sign is flipped.
float tangent_sign(float3 n,float3 t,float3 bt){return dot(cross(n,t),bt)<0?1:-1;}
// Texture maps (core track 5): base colour, metallic-roughness (G, B) and a tangent-space normal
// map (+Y up, as glTF), at uv with world tangent (bitangent sign in w; 0 = no tangent frame).
// Each map is scaled by value / average, so the sliders still steer it.
void sample_maps(inout Hit h,float2 uv,float4 tangent,Footprint footprint){
    Material m=materials[h.material];uint4 mp=m.maps;h.textured=true;
    h.albedo=mp.x!=0?saturate(sample_map_lod(mp.x,uv,footprint).rgb*m.base_scale.rgb):m.color_rough.rgb;
    h.rough_metal=float2(m.color_rough.a,m.lobes.x);
    if(mp.z!=0){
        float4 r=sample_map_lod(mp.z,uv,footprint);
        if(m.surface_scale.x>0)h.rough_metal.x=saturate(r.g*m.surface_scale.x);
        if(m.surface_scale.y>0)h.rough_metal.y=saturate(r.b*m.surface_scale.y);
    }
    if(mp.y!=0 && tangent.w!=0){
        float3 t=sample_map_lod(mp.y,uv,footprint).xyz*2-1;float3 n=h.n;
        float3 T=tangent.xyz-n*dot(n,tangent.xyz);
        if(dot(T,T)>1e-12){T=normalize(T);float3 B=cross(n,T)*tangent.w;h.n=normalize(T*t.x+B*t.y+n*max(t.z,1e-3));}
    }
}
// Procedural surfaces (track 13): kinds as scene::Sdf, inside the unit cube of object space.
// Sphere tracing in object space along the unnormalized object ray, so t stays the world t.
float sdf_box(float3 p,float3 b){float3 q=abs(p)-b;return length(max(q,0))+min(max(q.x,max(q.y,q.z)),0);}
float sdf_mandelbulb(float3 p,float n){
    p*=1.15;
    // Other powers (Object::detail): the spherical-coordinate formula, y up.
    if(n!=8){float3 w=p;float dr=1,r=length(w);
        for(int i=0;i<6 && r<2;i++){float th=acos(clamp(w.y/max(r,1e-12),-1,1))*n,ph=atan2(w.z,w.x)*n;
            dr=pow(r,n-1)*n*dr+1;w=pow(r,n)*float3(sin(th)*cos(ph),cos(th),sin(th)*sin(ph))+p;r=length(w);}
        return 0.5*log(max(r,1e-8))*r/dr/1.15;}
    // Quilez's trigonometry-free power 8 (the same bulb, about 3× cheaper).
    float3 w=p;float m=dot(w,w),dz=1;
    for(int i=0;i<6 && m<4;i++){
        dz=8*m*m*m*sqrt(m)*dz+1;
        float x=w.x,x2=x*x,x4=x2*x2,y=w.y,y2=y*y,y4=y2*y2,z=w.z,z2=z*z,z4=z2*z2;
        float k3=x2+z2,k2=rsqrt(k3*k3*k3*k3*k3*k3*k3),k1=x4+y4+z4-6*y2*z2-6*x2*y2+2*z2*x2,k4=x2-y2+z2;
        w=p+float3(64*x*y*z*(x2-z2)*k4*(x4-6*x2*z2+z4)*k1*k2,-16*y2*k3*k4*k4+k1*k1,-8*y*k4*(x4*x4-28*x4*x2*z2+70*x4*z4-28*x2*z2*z4+z4*z4)*k1*k2);
        m=dot(w,w);
    }
    return 0.25*log(max(m,1e-16))*sqrt(m)/dz/1.15;
}
// Quilez's Menger sponge: the cube minus three crosses per level.
float sdf_menger(float3 p,int levels){
    float d=sdf_box(p,1),s=1;
    for(int m=0;m<levels;m++){float3 a=p*s;a=a-2*floor(a*0.5)-1;s*=3;float3 r=abs(1-3*abs(a));
        d=max(d,(min(max(r.x,r.y),min(max(r.y,r.z),max(r.z,r.x)))-1)/s);}
    return d;
}
// Lava-lamp metaballs, made for an object 3× taller than wide (round blobs): in q = (x, 3y, z)
// a pool at the bottom, a cap at the top and four blobs on slow Lissajous paths, joined by a
// polynomial smooth minimum (k 0.5). ph: the object's phase. q stretches y by 3, so d/3 bounds.
float sdf_metaballs(float3 p,float ph){
    float3 q=float3(p.x,3*p.y,p.z);
    float d=min(length(q-float3(0,-3.3,0))-0.95,length(q-float3(0,3.4,0))-0.7);
    [unroll]for(int i=0;i<4;i++){
        float a=ph*(0.5+0.17*i)+i*1.7,r=0.42+0.1*(i&1);
        float3 c=float3(0.4*sin(a*1.3+i),2.3*sin(a),0.4*cos(a*0.9+2*i));
        float e=length(q-c)-r,h=saturate(0.5+0.5*(e-d)/0.5);d=lerp(e,d,h)-0.5*h*(1-h);
    }
    return d/3;
}
// Exact capped cylinder and cone (kinds 6, 7), the mesh shapes': radius 1, y in [−1, 1]; the
// cone's radius is (1 − y) / 2 (Quilez's capped cone).
float sdf_cylinder(float3 p){float2 d=abs(float2(length(p.xz),p.y))-1;return min(max(d.x,d.y),0)+length(max(d,0));}
float sdf_cone(float3 p){
    float2 q=float2(length(p.xz),p.y),k1=float2(0,1),k2=float2(-1,2);
    float2 ca=float2(q.x-min(q.x,q.y<0?1:0),abs(q.y)-1),cb=q-k1+k2*saturate(dot(k1-q,k2)/dot(k2,k2));
    float s=cb.x<0 && ca.y<0?-1:1;return s*sqrt(min(dot(ca,ca),dot(cb,cb)));
}
// The parameter in the high 24 bits (Object::detail, thousandths; 0 = default).
float sdf_detail(uint raw,float fallback){float v=(raw>>8)*0.001;return v>0?v:fallback;}
float sdf(uint raw,float3 p){
    uint kind=raw&255;
    if(kind==5)return sdf_metaballs(p,(raw>>8)*0.001);
    if(kind==2){float r=sdf_detail(raw,0.28);return length(float2(length(p.xz)-(1-r),p.y))-r;}
    if(kind==3)return sdf_mandelbulb(p,sdf_detail(raw,8));
    if(kind==4)return sdf_menger(p,clamp((int)sdf_detail(raw,5),1,6));
    if(kind==6)return sdf_cylinder(p);
    if(kind==7)return sdf_cone(p);
    return length(p)-1;
}
float3 sdf_normal(uint raw,float3 p){
    uint kind=raw&255;
    if(kind==1)return p;
    // Analytic: the cap or the side, whichever p lies closer to.
    if(kind==6){float rho=length(p.xz);return abs(abs(p.y)-1)<abs(rho-1)?float3(0,p.y,0):float3(p.x,0,p.z);}
    if(kind==7){float rho=length(p.xz);return abs(p.y+1)<abs(rho-0.5*(1-p.y))*0.894?float3(0,-1,0):float3(p.x/max(rho,1e-8),0.5,p.z/max(rho,1e-8));}
    const float e=1e-4;const float2 k=float2(1,-1);
    return k.xyy*sdf(raw,p+k.xyy*e)+k.yyx*sdf(raw,p+k.yyx*e)+k.yxy*sdf(raw,p+k.yxy*e)+k.xxx*sdf(raw,p+k.xxx*e);
}
// The nearest surface crossing in (tmin, tmax); from inside (refraction) it marches to the exit.
bool sdf_hit(uint raw,float3 o,float3 d,float tmin,float tmax,out float t){
    t=0;uint kind=raw&255;
    float3 inv=1/select(abs(d)>1e-12,d,1e-12),a=(-1-o)*inv,b=(1-o)*inv,lo=min(a,b),hi=max(a,b);
    float t0=max(tmin,max(lo.x,max(lo.y,lo.z))),t1=min(tmax,min(hi.x,min(hi.y,hi.z)));
    if(t0>t1)return false;
    if(kind==1){float A=dot(d,d),B=dot(o,d),D=B*B-A*(dot(o,o)-1);if(D<0)return false;float r=sqrt(D);t=(-B-r)/A;if(t<=tmin)t=(-B+r)/A;return t>tmin && t<tmax;}
    // Cylinder and cone: the side's quadric roots inside y in [−1, 1], and the caps' planes
    // inside their discs; the nearest beyond tmin.
    if(kind==6 || kind==7){
        bool cone=kind==7;float best=tmax;
        float k=1-o.y,A=dot(d.xz,d.xz)-(cone?0.25*d.y*d.y:0),B=2*dot(o.xz,d.xz)+(cone?0.5*k*d.y:0),C=dot(o.xz,o.xz)-(cone?0.25*k*k:1);
        float D=B*B-4*A*C;
        if(abs(A)>1e-12 && D>=0){float r=sqrt(D);[unroll]for(int i=0;i<2;i++){float x=(-B+(i==0?-r:r))/(2*A),y=o.y+d.y*x;if(x>tmin && x<best && abs(y)<=1)best=x;}}
        [unroll]for(int i=0;i<2;i++){float y=i==0?-1:1;if(cone && y>0)continue;if(abs(d.y)<1e-12)continue;
            float x=(y-o.y)/d.y;float2 q=o.xz+d.xz*x;if(x>tmin && x<best && dot(q,q)<=1)best=x;}
        t=best;return best<tmax;
    }
    float L=length(d),s=t0*L,e=t1*L;float3 u=d/L;
    // Round shapes start at their bounding sphere (radius 1) instead of the cube's corners.
    if(kind!=4 && kind!=5){float b=dot(o,u),D=b*b-dot(o,o)+1;if(D<0)return false;float r=sqrt(D);s=max(s,-b-r);e=min(e,-b+r);}
    float side=sdf(raw,o+u*s)<0?-1:1;
    for(int i=0;i<128 && s<e;i++){
        float h=side*sdf(raw,o+u*s);
        // 1e-4 world units (L = object units per world unit along a normalized world ray).
        if(h<1e-4*L){t=s/L;return t>tmin;}
        s+=kind==3?0.9*h:h;
    }
    return false;
}
// Instance masks (track 20a (a), Gpu instance_mask): an object hidden from camera rays, or of
// glass that shadow rays pass, is opaque with just this bit, and the rays below leave it out.
// The per-candidate tests stay for the static BLAS's non-opaque range, alpha-tested objects
// and objects under both rules, which keep mask 0xff.
#define MASK_HIDDEN 2u
#define MASK_CLEAR_GLASS 4u
#define MASK_NO_SHADOW 8u
bool intersect(float3 o,float3 d,float limit,out Hit h,bool shadow,bool camera_ray){
    RayDesc ray;ray.Origin=o;ray.Direction=d;ray.TMin=0.0001;ray.TMax=limit;
    // Nonopaque geometry allows the light quad and optional glass to be ignored for shadow rays.
    uint mask=0xffu&~(camera_ray?MASK_HIDDEN:0u)&~(shadow && jitter.w<=0.5?MASK_CLEAR_GLASS:0u)&~(shadow?MASK_NO_SHADOW:0u);
    RayQuery<RQ_BASE RQ_OMM> q;q.TraceRayInline(scene,RAY_FLAG_NONE,mask,ray);
    while(q.Proceed()){proceed_steps++;
        if(!lean && q.CandidateType()==CANDIDATE_PROCEDURAL_PRIMITIVE){
            Instance pi=instances[q.CandidateInstanceID()];uint pm=pi.material;float t;
            if(camera_ray && (((uint)materials[pm].lobes.w&2u)!=0))continue;
            if(shadow && jitter.w<=0.5 && materials[pm].lobes.z>=0.5)continue;
            if(shadow && (((uint)materials[pm].lobes.w&8192u)!=0))continue;
            if(sdf_hit(pi.vertex_base,q.CandidateObjectRayOrigin(),q.CandidateObjectRayDirection(),q.RayTMin(),q.CommittedRayT(),t))q.CommitProceduralPrimitiveHit(t);
            continue;
        }
        uint m=candidate_material(q.CandidateInstanceID(),q.CandidatePrimitiveIndex());
        if(flagged(m,8u) && alpha_hole(q.CandidateInstanceID(),q.CandidatePrimitiveIndex(),q.CandidateTriangleBarycentrics(),m))continue;
        if(camera_ray && (((uint)materials[m].lobes.w&2u)!=0))continue;
        if(shadow && (((uint)materials[m].lobes.w&8192u)!=0))continue;
        if(!shadow || jitter.w>0.5 || materials[m].lobes.z<0.5)q.CommitNonOpaqueTriangleHit();
    }
    h=(Hit)0;
    if(q.CommittedStatus()==COMMITTED_PROCEDURAL_PRIMITIVE_HIT){
        // Procedural (track 13): the SDF gradient in object space, to world by the inverse transpose.
        Instance pi=instances[q.CommittedInstanceID()];
        h.t=q.CommittedRayT();h.p=o+d*h.t;float3 op=q.CommittedObjectRayOrigin()+q.CommittedObjectRayDirection()*h.t;
        h.n=normalize(mul(sdf_normal(pi.vertex_base,op),(float3x3)q.CommittedWorldToObject3x4()));h.material=pi.material;
        h.front=dot(d,h.n)<0;if(!h.front)h.n=-h.n;h.ng=h.n;
        h.tangent=normalize(cross(h.n,abs(h.n.y)<0.99?float3(0,1,0):float3(1,0,0)));
        return true;
    }
    if(q.CommittedStatus()!=COMMITTED_TRIANGLE_HIT)return false;
    Instance inst=instances[q.CommittedInstanceID()];uint prim=q.CommittedPrimitiveIndex();float2 b=q.CommittedTriangleBarycentrics();
    // uv_scale: texture units per world unit at the hit; T, B: world dP/du and dP/dv (both for
    // the footprint; T zero when only uv_scale is known).
    h.t=q.CommittedRayT();h.p=o+d*h.t;float2 uv=0;float4 tangent=0;bool mapped;float uv_scale=0;float3 T=0,B=0,gn;
    if(inst.material==0xffffffffu){
        // Static geometry: one instance per opacity group (Gpu::rebuild_scene), each starting at
        // triangle vertex_base of static_vertices.
        prim+=inst.vertex_base;uint idx=prim*3;
        Vertex a=static_vertices[idx],v=static_vertices[idx+1],c=static_vertices[idx+2];
        h.n=normalize(a.normal*(1-b.x-b.y)+v.normal*b.x+c.normal*b.y);h.material=a.material;
        gn=cross(v.position-a.position,c.position-a.position);
        mapped=flagged(h.material,16u);
        if(mapped){uv_scale=abs(materials[h.material].base_scale.w);float3 t,bt;box_map(h.p,h.n,uv_scale,uv,t,bt);tangent=float4(t,tangent_sign(h.n,t,bt));T=t/uv_scale;B=bt/uv_scale;}
    }else{
        // An object: object-space normal to world by the inverse transpose (row vector × W2O).
        uint idx=inst.vertex_base+prim*3;Vertex a=vertices[idx],v=vertices[idx+1],c=vertices[idx+2];
        float3 n=a.normal*(1-b.x-b.y)+v.normal*b.x+c.normal*b.y;
        h.n=normalize(mul(n,(float3x3)q.CommittedWorldToObject3x4()));h.material=inst.material;
        gn=mul(cross(v.position-a.position,c.position-a.position),(float3x3)q.CommittedWorldToObject3x4());
        // UVs and tangents only for materials with maps (flag 16).
        mapped=flagged(h.material,16u);float scale=materials[h.material].base_scale.w;float3x4 m2w=q.CommittedObjectToWorld3x4();
        if(mapped && scale>0){
            // Texture set: box-mapped in the object's own axes, at world size (so it moves and
            // turns with the object but keeps its density when the object is scaled).
            float3 size=float3(length(m2w._m00_m10_m20),length(m2w._m01_m11_m21),length(m2w._m02_m12_m22));
            float3 op=q.CommittedObjectRayOrigin()+q.CommittedObjectRayDirection()*h.t,t,bt;
            box_map(op*size,n,scale,uv,t,bt);tangent=float4(mul(m2w,float4(t,0)),tangent_sign(n,t,bt));uv_scale=scale;
            T=normalize(mul(m2w,float4(t,0)))/scale;B=normalize(mul(m2w,float4(bt,0)))/scale;
        }else if(mapped){
            uv=a.uv*(1-b.x-b.y)+v.uv*b.x+c.uv*b.y;
            // Per-triangle tangent from the UV derivatives (glTF's MikkTSpace tangents are not read).
            float3 e1=v.position-a.position,e2=c.position-a.position;float2 d1=v.uv-a.uv,d2=c.uv-a.uv;float det=d1.x*d2.y-d1.y*d2.x;
            if(abs(det)>1e-12){float3 t=(e1*d2.y-e2*d1.y)/det,bt=(e2*d1.x-e1*d2.x)/det;tangent=float4(mul(m2w,float4(t,0)),tangent_sign(n,t,bt));T=tangent.xyz;B=mul(m2w,float4(bt,0));}
            float world_area=length(cross(mul(m2w,float4(e1,0)),mul(m2w,float4(e2,0))));uv_scale=sqrt(abs(det)/max(world_area,1e-20));
            // A texture set on the mesh's UVs (base_scale.w < 0): |w| repeats per UV unit.
            if(scale<0){float k=-scale;uv*=k;uv_scale*=k;T/=k;B/=k;}
        }
    }
    // The side is the triangle's (its face normal turned toward the shading normal), not the
    // interpolated normal's: at grazing hits on smooth meshes those can disagree, and a wrong
    // side pushed or popped the wrong medium in a nested dielectric. ng is the face normal.
    gn=dot(gn,gn)>1e-30?normalize(gn):h.n;if(dot(gn,h.n)<0)gn=-gn;
    // The shading normal still faces the ray, as the BSDFs and cosines expect.
    h.front=dot(d,gn)<0;if(!h.front)gn=-gn;if(dot(d,h.n)>0)h.n=-h.n;h.ng=gn;
    if(materials[h.material].layers.w>0 || has_grating(materials[h.material])){
        float3 ax=float3(1,0,0),ay=float3(0,1,0),o=0;
        if(inst.material!=0xffffffffu){float3x4 w=q.CommittedObjectToWorld3x4();ax=w._m00_m10_m20;ay=w._m01_m11_m21;o=w._m03_m13_m23;}
        h.tangent=((uint)materials[h.material].lobes.w&2048u)!=0?cross(ay,h.p-o):ax;
    }
    // Footprint: the cone's width at the hit, stretched along the ray by the grazing angle
    // (capped at 20×).
    if(mapped)sample_maps(h,uv,tangent,footprint(d,h.ng,cone_width+cone_spread*h.t,T,B,uv_scale));
    h.emitter=inst.emitter_base==0xffffffffu?0:emitter_lookup[inst.emitter_base+prim];return true;
}
// Shadow test: any blocker ends the search, and no hit data is reconstructed.
bool occluded(float3 o,float3 d,float limit){
    RayDesc ray;ray.Origin=o;ray.Direction=d;ray.TMin=0.0001;ray.TMax=limit;
    uint mask=(glass_blocks || jitter.w>0.5?0xffu:0xffu&~MASK_CLEAR_GLASS)&~MASK_NO_SHADOW;
    RayQuery<RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH|RQ_BASE RQ_OMM> q;q.TraceRayInline(scene,RAY_FLAG_NONE,mask,ray);
    while(q.Proceed()){proceed_steps++;
        if(!lean && q.CandidateType()==CANDIDATE_PROCEDURAL_PRIMITIVE){
            Instance pi=instances[q.CandidateInstanceID()];float t;
            if(((uint)materials[pi.material].lobes.w&8192u)!=0)continue;
            if((glass_blocks || jitter.w>0.5 || materials[pi.material].lobes.z<0.5) && sdf_hit(pi.vertex_base,q.CandidateObjectRayOrigin(),q.CandidateObjectRayDirection(),q.RayTMin(),q.CommittedRayT(),t))q.CommitProceduralPrimitiveHit(t);
            continue;
        }
        uint m=candidate_material(q.CandidateInstanceID(),q.CandidatePrimitiveIndex());
        if(flagged(m,8u) && alpha_hole(q.CandidateInstanceID(),q.CandidatePrimitiveIndex(),q.CandidateTriangleBarycentrics(),m))continue;
        if(((uint)materials[m].lobes.w&8192u)!=0)continue;
        if(glass_blocks || jitter.w>0.5 || materials[m].lobes.z<0.5)q.CommitNonOpaqueTriangleHit();}
    return q.CommittedStatus()!=COMMITTED_NOTHING;
}
Material material_at(uint id,float3 p){
    Material m=materials[id];uint pattern=(uint)m.optical.z;float t=0;
    if(pattern==1)t=fmod(abs(floor(p.x*m.secondary.w)+floor(p.z*m.secondary.w)),2);
    if(pattern==2)t=step(0,sin((p.x+p.z)*m.secondary.w*PI));
    // Marble: soft veins where a turbulent sine crosses zero, at most 75% secondary color.
    if(pattern==3){float s=m.secondary.w;float vein=sin((p.x+p.z)*s*0.6+3*sin(p.y*3+p.z*2)+1.5*sin(p.x*5-p.z*2)+0.6*sin(p.x*13+p.y*11)+0.3*sin(p.z*27-p.y*21));t=0.75*pow(1-abs(vein),6);}
    m.color_rough.rgb=lerp(m.color_rough.rgb,m.secondary.rgb,t);lean_strip(m);return m;
}
// The textured surface values intersect sampled (sample_maps).
void apply_maps(Hit h,inout Material m){if(h.textured){m.color_rough.rgb=h.albedo;m.color_rough.a=h.rough_metal.x;m.lobes.x=h.rough_metal.y;}}
// HDRI (environment.rs has the mapping): nearest texel, so the sampling pdf matches exactly.
bool hdri_on(){return ((uint)environment_map.z&1u)!=0;}
uint2 env_size(){uint w,h;env_map.GetDimensions(w,h);return uint2(w,h);}
uint2 env_texel(float3 d,out float sin_theta,out float sin_row){
    uint2 size=env_size();float theta=acos(clamp(d.y,-1,1));float phi=atan2(d.x,-d.z)-environment_map.y;
    float u=frac(phi/(2*PI)+0.5);uint2 t=min(uint2(float2(u,theta/PI)*size),size-1);
    sin_theta=sin(theta);sin_row=sin(PI*(t.y+0.5)/size.y);return t;
}
// The HDRI's split sun (Settings::hdri_sun, environment.rs split_sun): a disc of uniform
// radiance the map no longer holds, sampled as a share env_sun_color.w of environment samples.
bool env_sun_on(){return env_sun.w>0;}
bool in_env_sun(float3 d){float3 x=cross(d,env_sun.xyz);float om=env_sun.w;return dot(d,env_sun.xyz)>0 && dot(x,x)<=om*(2-om)*1.000001;}
float env_pdf(float3 d){
    if(!hdri_on())return 0;float s,r;uint2 t=env_texel(d,s,r);
    float p=luminance(env_map[t].rgb)*environment_map.w*r/max(s,1e-6);
    if(env_sun_on())p=(1-env_sun_color.w)*p+(in_env_sun(d)?env_sun_color.w/(2*PI*env_sun.w):0);
    return p;
}
// First index whose CDF value exceeds u, among `count` entries of row `row`. The row's
// cutpoints (environment::build, rows h + 1 on; K a power of two, so u·K is exact) bracket the
// answer, and the search between them returns exactly the full binary search's index.
uint cdf_search(uint row,uint count,float u){
    uint2 size=env_size();uint K=1u<<firstbithigh(size.x),k=(uint)(u*K),g=size.y+1+row;
    uint lo=(uint)env_cdf[uint2(k,g)],hi=k+1<K?(uint)env_cdf[uint2(k+1,g)]:count-1;
    while(lo<hi){uint mid=(lo+hi)/2;if(u<env_cdf[uint2(mid,row)])hi=mid;else lo=mid+1;}return lo;}
// A direction from the HDRI's luminance · sinθ, uniform within the chosen texel.
float3 sample_env_map(out float pdf);
// The map or, with a split sun, the sun's disc by its share; pdf the mixture's (env_pdf).
float3 sample_env(out float pdf){
    if(env_sun_on() && random()<env_sun_color.w){
        float om=env_sun.w,x=random()*om,s=sqrt(x*(2-x)),a=2*PI*random();
        float3 d=basis(env_sun.xyz,float3(s*cos(a),s*sin(a),1-x));pdf=env_pdf(d);return d;
    }
    float3 d=sample_env_map(pdf);if(env_sun_on())pdf=(1-env_sun_color.w)*pdf+(in_env_sun(d)?env_sun_color.w/(2*PI*env_sun.w):0);return d;
}
float3 sample_env_map(out float pdf){
    uint2 size=env_size();
    float uy=random(),ux=random();
    uint y=cdf_search(size.y,size.y,uy);float y0=y>0?env_cdf[uint2(y-1,size.y)]:0,y1=env_cdf[uint2(y,size.y)];
    uint x=cdf_search(y,size.x,ux);float x0=x>0?env_cdf[uint2(x-1,y)]:0,x1=env_cdf[uint2(x,y)];
    // Reuse the remainder of each choice as the position inside the texel.
    float fy=saturate((uy-y0)/max(y1-y0,1e-9)),fx=saturate((ux-x0)/max(x1-x0,1e-9));
    float theta=PI*(y+fy)/size.y,phi=2*PI*((x+fx)/size.x-0.5)+environment_map.y;
    float3 d=float3(sin(theta)*sin(phi),cos(theta),-sin(theta)*cos(phi));
    pdf=luminance(env_map[uint2(x,y)].rgb)*environment_map.w*sin(PI*(y+0.5)/size.y)/max(sin(theta),1e-6);
    return d;
}
// Radiance is filtered bilinearly (nearest texels looked blocky as a background); sampling keeps
// the per-texel pdf. Still unbiased: MIS weights only need the pdf the samples were drawn from.
float3 env_bilinear(float3 d){
    uint2 size=env_size();float theta=acos(clamp(d.y,-1,1));float phi=atan2(d.x,-d.z)-environment_map.y;
    float2 uv=float2(frac(phi/(2*PI)+0.5),theta/PI)*size-0.5;int2 i=(int2)floor(uv);float2 f=uv-i;
    int2 n=(int2)size;
    #define ENV(o) env_map[uint2(((i.x+o.x)%n.x+n.x)%n.x,clamp(i.y+o.y,0,n.y-1))].rgb
    float3 c=lerp(lerp(ENV(int2(0,0)),ENV(int2(1,0)),f.x),lerp(ENV(int2(0,1)),ENV(int2(1,1)),f.x),f.y);
    #undef ENV
    return c;
}
// Ground projection (Settings::hdri_ground): below the horizon a camera-seen background is the
// map from the capture point (0, floor + height, 0) toward where the ray meets the floor's plane.
bool ground_on(){return env_ground.x>0 && hdri_on();}
float3 ground_dir(float3 p){return normalize(p-float3(0,env_ground.y+env_ground.x,0));}
float3 background_dir(float3 o,float3 d){
    if(!ground_on() || d.y>=0 || o.y<=env_ground.y)return d;
    return ground_dir(o+d*((env_ground.y-o.y)/d.y));
}
float3 environment_radiance(float3 d){
    if(hdri_on())return env_bilinear(d)*environment_map.x+(env_sun_on() && in_env_sun(d)?env_sun_color.rgb:0);
    return background.rgb+environment.x*lerp(float3(0.8,0.87,1.0),float3(0.16,0.35,0.72),saturate(d.y*0.5+0.5));
}
float3 fresnel(float c,float3 f0){return f0+(1-f0)*pow(1-saturate(c),5);}
// Exact unpolarised Fresnel reflectance between media with eta = n1 / n2 (1 at total internal
// reflection). Schlick's form is poor between two dense media (water against glass) and still
// reflects at grazing angles when n1 = n2.
float fresnel_dielectric(float cosi,float eta){
    float s2=eta*eta*(1-cosi*cosi);if(s2>=1)return 1;float cost=sqrt(1-s2);
    float rs=(eta*cosi-cost)/(eta*cosi+cost),rp=(cosi-eta*cost)/(cosi+eta*cost);
    return 0.5*(rs*rs+rp*rp);
}
// Nested dielectrics (track 10, Schmidt and Budge 2002): each path keeps the transmissive
// materials it is inside, up to 4 deep, in one 64-bit word: 16 bits per entry from the bottom,
// (material id + 1) | priority << 13, 0 = empty. The current medium is the one with the highest
// priority (flag bits 8–10), the latest on ties; -1 is air. A dielectric surface that does not
// bound the current medium is a false hit: the path passes straight through and only the stack
// changes, so water may overlap a glass wall (glass ranked higher) without an air gap. The word
// is small on purpose: the stack is live across every RayQuery, and a uint4 cost glass scenes
// about 0.4 ms at 1440×900 in occupancy.
uint medium_entry(uint id,float flags){return ((id+1)&0x1fffu)|(((uint)flags>>8)&7u)<<13;}
uint medium_at(uint64_t s,uint i){return (uint)(s>>(16*i))&0xffffu;}
int medium_top(uint64_t s){int best=-1;uint bp=0;[unroll]for(uint i=0;i<4;i++){uint e=medium_at(s,i);if(e!=0 && (best<0 || (e>>13)>=bp)){best=i;bp=e>>13;}}return best;}
int medium_find(uint64_t s,uint e){int j=-1;[unroll]for(uint i=0;i<4;i++)if(medium_at(s,i)==e)j=i;return j;}
// A full stack drops the push; leaving that material later then counts as leaving into the rest.
// The stack a camera ray starts with: the object the camera is inside (underwater), or none.
uint64_t camera_media(){return (uint64_t)(uint)environment.w;}
uint64_t medium_push(uint64_t s,uint e){[unroll]for(uint i=0;i<4;i++)if(medium_at(s,i)==0)return s|((uint64_t)e<<(16*i));return s;}
uint64_t medium_remove(uint64_t s,uint e){int j=medium_find(s,e);if(j<0)return s;uint64_t low=j>0?s&((1ull<<(16*j))-1):0;return low|((s>>(16*(j+1)))<<(16*j));}
Material medium_material(uint64_t s,int top){Material m=materials[(medium_at(s,top)&0x1fffu)-1];lean_strip(m);return m;}
float medium_ior(uint64_t s,int top){return top>=0?ior_of(medium_material(s,top)):1;}
float3 medium_absorption(uint64_t s,int top){if(top<0)return 0;Material m=medium_material(s,top);return -log(max(up_material(m,m.color_rough.rgb),0.001));}
// Thin-film interference (track 11, glTF's KHR_materials_iridescence): Belcour and Barla
// (2017), the Airy sum of a film between the outside and the base, kept to its first two
// orders, with the spectral integral done in closed form against Gaussian fits of the CIE XYZ
// matching functions (as the Khronos sample viewer), so no spectral rendering. IORs are relative
// to the outside medium: eta2 the film's, eta3 the base's (per channel, from f0 for an opaque
// base). surface_scale.z = strength, w = thickness (nm) + 2048 × IOR (hundredths) (lighting.rs).
static const float3x3 XYZ_TO_REC709={3.2404542,-1.5371385,-0.4985314,-0.9692660,1.8760108,0.0415560,0.0556434,-0.2040259,1.0572252};
float3 film_sensitivity(float opd,float3 shift){
    float phase=2*PI*opd*1e-9;
    const float3 val=float3(5.4856e-13,4.4201e-13,5.2481e-13),pos=float3(1.6810e6,1.7953e6,2.2084e6),var=float3(4.3278e9,9.3046e9,6.6121e9);
    float3 xyz=val*sqrt(2*PI*var)*cos(pos*phase+shift)*exp(-phase*phase*var);
    xyz.x+=9.7470e-14*sqrt(2*PI*4.5282e9)*cos(2.2399e6*phase+shift.x)*exp(-4.5282e9*phase*phase);
    return mul(XYZ_TO_REC709,xyz/1.0685e-7);
}
float schlick(float f0,float c){return f0+(1-f0)*pow(1-saturate(c),5);}
float3 film_fresnel(float c1,float eta2,float3 eta3,float thickness,bool waves){
    // Below ~30 nm the film fades out (its IOR towards the outside's), leaving the base's Fresnel.
    eta2=lerp(1,eta2,smoothstep(0,30,thickness));
    float s2=(1-c1*c1)/(eta2*eta2);if(s2>=1)return 1;float c2=sqrt(1-s2);
    float r12=schlick(pow((eta2-1)/(eta2+1),2),c1),t121=1-r12;
    float phi21=eta2<1?0:PI;
    float3 r23=float3(schlick(pow((eta3.x-eta2)/(eta3.x+eta2),2),c2),schlick(pow((eta3.y-eta2)/(eta3.y+eta2),2),c2),schlick(pow((eta3.z-eta2)/(eta3.z+eta2),2),c2));
    float3 phi=phi21+select(eta3<eta2,PI.xxx,0.0.xxx);
    float opd=2*eta2*thickness*c2;
    float3 r123=clamp(r12*r23,1e-5,0.9999),rs=t121*t121*r23/(1-r123),i=r12+rs,cm=rs-t121,sq=sqrt(r123);
    // Spectral: the Airy terms at the path's own wavelengths, exactly (no CMF fit).
    [unroll]for(int k=1;k<=2;k++){cm*=sq;i+=cm*2*(waves?cos(k*(2*PI*opd/path_lambdas+phi)):film_sensitivity(k*opd,k*phi));}
    return saturate(i);
}
float film_thickness(Material m){return m.surface_scale.w-floor(m.surface_scale.w/2048)*2048;}
float film_ior(Material m){return floor(m.surface_scale.w/2048)/100;}
float ggx_d(float nh,float a){float d=nh*nh*(a*a-1)+1;return a*a/(PI*d*d);}
float g1(float nv,float a){return 2*nv/max(1e-6,nv+sqrt(a*a+(1-a*a)*nv*nv));}
float3 f0_of(Material m){float f=pow((m.optical.x-1)/(m.optical.x+1),2);return lerp(lerp(f.xxx,1.0.xxx,m.lobes.y),m.color_rough.rgb,m.lobes.x);}
float spec_probability(Material m){return clamp(0.25+0.5*m.lobes.x+0.25*m.lobes.y,0.1,0.95);}
// The film is evaluated once per hit at the view cosine (as glTF's model does), not per half
// vector, and kept in the hit's copy of the material (base_scale.rgb, no longer needed once the
// maps are applied): inlined at every Fresnel site it cost scenes without a film 3–9% (registers).
// outside / base: the IORs on both sides (base 0 = an opaque base, its IOR from f0 per channel).
void prepare_film(inout Material m,float c,float outside,float base){
    if(m.surface_scale.z<=0)return;
    float3 q=sqrt(clamp(f0_of(m),0,0.9999));float3 eta3=base>0?(base/outside).xxx:(1+q)/(1-q);
    m.base_scale.rgb=film_fresnel(c,film_ior(m)/outside,eta3,film_thickness(m),false);
}
// The spectral tracer's film, on a material already uplifted (f0 per wavelength).
void prepare_film_waves(inout Material m,float c,float outside,float base){
    if(m.surface_scale.z<=0)return;
    float3 q=sqrt(clamp(f0_of(m),0,0.9999));float3 eta3=base>0?(base/outside).xxx:(1+q)/(1-q);
    m.base_scale.rgb=film_fresnel(c,film_ior(m)/outside,eta3,film_thickness(m),true);
}
// The base specular's Fresnel at cosine c, with the film (if any, prepare_film) over it.
float3 spec_fresnel(Material m,float c,float3 f0){
    float3 f=fresnel(c,f0);return m.surface_scale.z>0?lerp(f,m.base_scale.rgb,m.surface_scale.z):f;
}
// Layers (track 11, glTF's KHR_materials_clearcoat / _sheen model): a sheen over the base and
// a clear coat over both. layers = (coat, coat roughness, sheen roughness, anisotropy), sheen.rgb
// = the sheen colour (times its strength). Each layer dims what lies below by the energy it takes
// from the view direction, as glTF's spec has it: the coat by its compensated albedo (glTF: its
// Fresnel), the sheen by its directional albedo (layers.hlsli). Dimming by both directions (reciprocal) lost 8–12% of a
// white base in the furnace; the view-only form keeps it, and NEE and BSDF sampling still
// evaluate the same function, so MIS stays consistent.
float max3(float3 c){return max(c.r,max(c.g,c.b));}
// Light path expressions (track 14, lpe.rs): the LPE variant (build.rs's `trace_lpe`) runs a DFA
// per slot along each path. Events: 0 D, 1 G, 2 S, 3 T, 4 V, 5 L, 6 B; 8 = none. A light sample
// at a surface is split exactly into its diffuse and glossy parts (eval_bsdf's lpe_diffuse); a
// continued path takes one label, drawn from a hash (so the beauty's samples stay the same) by
// the luminance shares at the sampled direction, and weights later contributions by share /
// probability per channel: each AOV is unbiased, and a partition of all paths sums to the
// beauty in expectation (not per sample).
#ifdef LPE
static const bool lpe_on=true;
#else
static const bool lpe_on=false;
#endif
static float3 lpe_diffuse; // eval_bsdf: the diffuse part of the value it returned
static bool lpe_env_sample; // sample_light: the HDRI or sky was picked (event B, else L)
static uint lpe_state; // per slot, 8 bits: the DFA state after this path's events so far
static float3 lpe_weight,lpe_sum[4];
// Labels (lpe.rs): each symbol is label class * 8 + event, the class of a material id in the
// table's tail. lpe_label: the current surface's class; lpe_light_label: the class of the light
// the next light event comes from (sample_light, or an emitter hit). V and B carry class 0.
static uint lpe_label=0,lpe_light_label=0;
uint lpe_class(uint material){return material<8192?lpe_table[4*32*64+material]:0;}
uint lpe_step(uint k,uint state,uint event,uint cls){return lpe_table[(k*32+state)*64+cls*8+event];}
void lpe_advance(uint event){
    if(!lpe_on)return;uint s=0;uint cls=event==4?0:lpe_label;
    [unroll]for(uint k=0;k<4;k++){uint st=(lpe_state>>(8*k))&255;s|=(k<lpe.x && st!=255?lpe_step(k,st,event,cls)&255:255)<<(8*k);}
    lpe_state=s;
}
// A contribution e reaching the camera through the path so far, then `event` (8: none), then
// the light event `light` (5 or 6).
void lpe_add(uint event,uint light,float3 e){
    if(!lpe_on || !all(isfinite(e)))return;
    uint cls=event==4?0:lpe_label,lcls=light==6?0:lpe_light_label;
    [unroll]for(uint k=0;k<4;k++){
        uint st=(lpe_state>>(8*k))&255;if(k>=lpe.x || st==255)continue;
        if(event!=8){st=lpe_step(k,st,event,cls)&255;if(st==255)continue;}
        if((lpe_step(k,st,light,lcls)>>8)!=0)lpe_sum[k]+=e*lpe_weight;
    }
}
// The glossy event of a material: S below roughness 0.1, else G.
uint lpe_glossy(Material m){return m.color_rough.a<0.1?2:1;}
// Charlie sheen (Estevez and Kulla 2017) with their fitted visibility; divided by its albedo
// where that passes 1 (grazing views at low roughness), so it never adds energy.
float sheen_lambda_l(float x,float a){float t=(1-a)*(1-a);return lerp(21.5473,25.3245,t)/(1+lerp(3.82987,3.32435,t)*pow(x,lerp(0.19823,0.16801,t)))+lerp(-1.97760,-1.27393,t)*x+lerp(-4.32054,-4.85967,t);}
float sheen_lambda(float c,float a){return c<0.5?exp(sheen_lambda_l(c,a)):exp(2*sheen_lambda_l(0.5,a)-sheen_lambda_l(1-c,a));}
float sheen_lobe(float r,float nh,float nv,float nl){
    float a=max(r,SHEEN_MIN_ROUGHNESS);a*=a;
    float d=(2+1/a)*pow(saturate(1-nh*nh),0.5/a)/(2*PI);
    float vis=saturate(1/((1+sheen_lambda(nv,a)+sheen_lambda(nl,a))*4*max(nv*nl,1e-6)));
    return d*vis/max(1,sheen_albedo(nv,r));
}
float coat_fresnel(float c){return 0.04+0.96*pow(1-saturate(c),5);}
// Share of BSDF samples the coat's own lobe takes (0 without a coat: no extra random number).
float coat_probability(Material m){return 0.3*m.layers.x;}
// A GGX half vector around n (roughness a = alpha), reflected: the sampled direction.
float3 ggx_sample(float3 n,float3 d,float a){float u=random(),phi=2*PI*random();float c=sqrt((1-u)/(1+(a*a-1)*u));float3 h=basis(n,float3(sqrt(max(0,1-c*c))*cos(phi),sqrt(max(0,1-c*c))*sin(phi),c));return reflect(d,h);}
// Anisotropy (track 11, brushed metal): layers.w = strength, sheen.w = the brushing direction's
// rotation about the normal (radians). The base GGX takes alpha along the grooves (T) and
// lerp(alpha, 1, strength²) across them (B), so highlights streak across the brushing; glTF's
// KHR_materials_anisotropy raises it along its direction instead (mesh.rs turns that by 90°).
// The coat stays isotropic.
void aniso_frame(float3 n,float3 t,float rotation,out float3 T,out float3 B){
    T=t-n*dot(n,t);T=dot(T,T)>1e-12?normalize(T):basis(n,float3(1,0,0));
    float s,c;sincos(rotation,s,c);T=T*c+cross(n,T)*s;B=cross(n,T);
}
float2 aniso_alpha(Material m){float a=max(0.001,m.color_rough.a*m.color_rough.a);return float2(a,lerp(a,1,m.layers.w*m.layers.w));}
// D and G1 in the local frame (x along T, y along B, z the normal); with ax = ay they reduce to
// ggx_d and g1.
float ggx_d_aniso(float3 h,float2 a){float3 s=float3(h.x/a.x,h.y/a.y,h.z);float k=dot(s,s);return 1/(PI*a.x*a.y*k*k);}
float g1_aniso(float3 w,float2 a){return 2*w.z/max(1e-6,w.z+sqrt(w.x*w.x*a.x*a.x+w.y*w.y*a.y*a.y+w.z*w.z));}
// Visible normals (Heitz 2018): the weight stays near F·G1(l), where sampling D·nh (as
// ggx_sample does) put fireflies on a strongly anisotropic metal's steep facets. Its pdf for the
// reflected direction is G1(v)·D / (4 nv) (eval_bsdf).
float3 ggx_sample_aniso(float3 n,float3 T,float3 B,float3 d,float2 a){
    float3 v=-d;float3 vh=normalize(float3(a.x*dot(v,T),a.y*dot(v,B),max(dot(v,n),0)));
    float len2=vh.x*vh.x+vh.y*vh.y;float3 t1=len2>0?float3(-vh.y,vh.x,0)*rsqrt(len2):float3(1,0,0),t2=cross(vh,t1);
    float r=sqrt(random()),phi=2*PI*random();float x=r*cos(phi),y=r*sin(phi),s=0.5*(1+vh.z);y=(1-s)*sqrt(max(0,1-x*x))+s*y;
    float3 m=x*t1+y*t2+sqrt(max(0,1-x*x-y*y))*vh;
    return reflect(d,normalize(T*(a.x*m.x)+B*(a.y*m.y)+n*max(m.z,1e-6)));
}
// Diffraction grating (track 11, a CD or DVD): optical.w < 0 holds the strength (thousandths ×
// 16384) and the groove spacing (nm; lighting::grating_code). The grooves run along the brush
// direction T (aniso_frame). Order o of wavelength λ leaves where the tangential part of l is the
// mirror direction's moved by o·λ/spacing across them (along B): each order is the base GGX lobe
// translated on the projected disk, f_o(l) = f(l0) with l0 = l moved back (f·cos·dω is f times
// the projected area, which the move keeps). So it
// keeps the lobe's energy, minus what lands past the horizon (evanescent), and sampling it is
// the GGX sample moved the same way: pdf_o(l) = pdf(l0)·cos l / cos l0. Orders ±1 take 0.35 of
// the grating's share each and ±2 0.15 each. The orders use the isotropic lobe (an anisotropic
// grating keeps its anisotropy in the mirror reflection only). Each grating hit draws its own wavelength
// (`grating_wavelength`, from a hash, not the sampler's dimensions), used by both next-event
// estimation and the BSDF sample there, so MIS stays exact for it. It is importance-sampled from
// three Gaussian channel curves of equal area (a flat spectrum is white), so the RGB weight is
// 3·N_c / ΣN: at most 3 per channel. One wavelength per path instead multiplied the same weight
// in at every grating bounce, which is biased (E[w²] > 1) and blew up between grating spheres.
static float path_lambda=550;
static const float3 GRATING_MU=float3(600,545,455),GRATING_SIGMA=float3(40,35,25);
void grating_wavelength(uint key){
    uint a=hash(key),b=hash(a^0x68e31da4);uint c=a%3;float u1=max(((a>>8)&0xffff)*(1.0/65536.0),1e-6),u2=(b>>8)*(1.0/16777216.0);
    path_lambda=GRATING_MU[c]+GRATING_SIGMA[c]*sqrt(-2*log(u1+(1.0/131072.0)))*cos(2*PI*u2);
    path_lambda=clamp(path_lambda,300,900);
}
// The RGB weight of the hit's wavelength, recomputed where needed: kept from the hit, it (like the
// groove axis and the order shares, also recomputed) stayed live across the whole bounce, and
// those registers cost scenes without a grating 4–12%.
float3 grating_weight(){if(spectral_on)return 1;float3 x=(path_lambda-GRATING_MU)/GRATING_SIGMA,nc=exp(-0.5*x*x)/GRATING_SIGMA;return 3*nc/max(nc.r+nc.g+nc.b,1e-30);}
float grating_share(Material m){return floor(-m.optical.w/16384)/1000;}
float grating_shift(Material m){float c=-m.optical.w;return path_lambda/(c-floor(c/16384)*16384);}
static const float GRATING_ORDER[4]={-1,1,-2,2};
static const float GRATING_WEIGHT[4]={0.35,0.35,0.15,0.15};
// The direction whose lobe a shift carries to l (false past the horizon). B lies in the tangent plane.
bool grating_move(float3 n,float3 B,float3 l,float shift,out float3 moved){
    float3 lt=l-n*dot(n,l)+shift*B;float s=dot(lt,lt);moved=lt+n*sqrt(max(0,1-s));return s<1;
}
// The share of each order for view v: GRATING_WEIGHT × the grating's strength for orders that
// leave above the horizon from the mirror direction, else 0; what they don't take stays in the
// mirror reflection (evanescent orders give their energy back), so a Blu-ray's 0.32 µm grooves
// reflect plainly except at grazing views. Depends on v and the path's wavelength only, so
// sampling and evaluation agree.
float4 order_shares(Material m,float3 n,float3 B,float3 v){
    float g=grating_share(m),k=grating_shift(m);float3 r=reflect(-v,n);float4 w;
    [unroll]for(int o=0;o<4;o++){float3 lt=r-n*dot(n,r)+GRATING_ORDER[o]*k*B;w[o]=dot(lt,lt)<1?g*GRATING_WEIGHT[o]:0;}
    return w;
}
float3 eval_bsdf(Material m,float3 n,float3 t,float3 v,float3 l,out float pdf){
    float nl=saturate(dot(n,l)),nv=saturate(dot(n,v));float3 h=normalize(v+l);float nh=saturate(dot(n,h)),vh=saturate(dot(v,h));float a=max(0.001,m.color_rough.a*m.color_rough.a);float d=ggx_d(nh,a);float3 f0=f0_of(m),f=spec_fresnel(m,vh,f0);float ps=spec_probability(m);
    float gv=g1(nv,a),gl=g1(nl,a),ess,spec_pdf;
    if(m.layers.w<=0){ess=ggx_albedo(nv,m.color_rough.a);spec_pdf=d*nh/max(4*vh,1e-6);}
    else{
        float3 T,B;aniso_frame(n,t,m.sheen.w,T,B);float2 aa=aniso_alpha(m);
        d=ggx_d_aniso(float3(dot(h,T),dot(h,B),nh),aa);gv=g1_aniso(float3(dot(v,T),dot(v,B),nv),aa);gl=g1_aniso(float3(dot(l,T),dot(l,B),nl),aa);
        // The albedo for the compensation depends on the view's azimuth too (layers.hlsli).
        float vt=abs(dot(v,T)),vb=abs(dot(v,B));ess=aniso_albedo(nv,vt+vb>1e-6?atan2(vb,vt)*(2/PI):0,m.color_rough.a,m.layers.w);
        spec_pdf=gv*d/max(4*nv,1e-6);  // visible normals (ggx_sample_aniso)
    }
    pdf=(1-ps)*nl/PI+ps*spec_pdf;
    float3 diff=(1-f)*(1-m.lobes.x)*m.color_rough.rgb/PI;
    // Energy compensation (Turquin 2019, Kulla and Conty's lost multiple scattering in one table):
    // the single-scattering lobe returns E(nv) of the light with F = 1, so a white metal gains
    // 1/E and a dielectric's specular f0·(1/E − 1); the diffuse keeps the rest.
    // With a thin film the lost energy follows the film's reflectance (at normal incidence, as
    // f0 does) instead: f0 left a white diffuse base under a film up to 6% dark in some channels.
    float3 fc=m.surface_scale.z>0?spec_fresnel(m,1,f0):f0;
    float3 r=diff+f*d*gv*gl/max(4*nv*nl,1e-6)*(1+fc*(1/ess-1)),dd=diff;
    if(has_grating(m) && nl>0){
        float3 T,B,orders=0;aniso_frame(n,t,m.sheen.w,T,B);float k=grating_shift(m),op=0;float4 w=order_shares(m,n,B,v);float g=w.x+w.y+w.z+w.w;
        [unroll]for(int o=0;o<4;o++){
            float3 l0;if(w[o]<=0 || !grating_move(n,B,l,-GRATING_ORDER[o]*k,l0))continue;
            float nl0=max(dot(n,l0),1e-6);float3 h0=normalize(v+l0);float nh0=saturate(dot(n,h0)),vh0=saturate(dot(v,h0)),d0=ggx_d(nh0,a);
            orders+=w[o]*spec_fresnel(m,vh0,f0)*d0*g1(nl0,a)/nl0;op+=w[o]*d0*nh0/max(4*vh0,1e-6)*nl/nl0;
        }
        r=diff+(1-g)*(r-diff)+orders*(g1(nv,a)/max(4*nv,1e-6))*(1+fc*(1/ess-1))*grating_weight();pdf=(1-ps)*nl/PI+ps*((1-g)*spec_pdf+op);
    }
    // The sheen is sampled with the diffuse lobe's cosine directions (it adds no pdf term).
    float s=max3(m.sheen.rgb);
    if(s>0){float sr=m.layers.z;float keep=1-s*min(1,sheen_albedo(nv,sr));r=r*keep+m.sheen.rgb*sheen_lobe(sr,nh,nv,nl);dd*=keep;}
    float cc=m.layers.x;
    if(cc>0){
        float ac=max(0.001,m.layers.y*m.layers.y),dc=ggx_d(nh,ac);
        // The coat is compensated in full (1/E), and the layers below are dimmed by exactly the
        // albedo that leaves the coat (coat_albedo, = coat_fresnel(nv) when smooth): F(nv) nears 1
        // at grazing, where a rough coat's facets mostly see far smaller F(vh).
        float keep=1-cc*coat_albedo(nv,m.layers.y);
        r=r*keep+cc*coat_fresnel(vh)*dc*g1(nv,ac)*g1(nl,ac)/max(4*nv*nl,1e-6)/ggx_albedo(nv,m.layers.y);dd*=keep;
        pdf=lerp(pdf,dc*nh/max(4*vh,1e-6),coat_probability(m));
    }
    pdf*=1-m.lobes.z;if(lpe_on)lpe_diffuse=(1-m.lobes.z)*dd;return (1-m.lobes.z)*r;
}
// Track 7: analytic emitters have a negative position_area.w: −1 a sphere (centre, radius in
// edge1_cdf.x), −2 a sun (unit direction to it, 1 − cos of its angular radius in edge1_cdf.x).
// Both are sampled uniformly over the cone they cover; om = 1 − cos of the cone's half angle.
// The sine comes from 1 − cos directly (sin² = x·(2 − x)): near 1 a float cosine has too few
// steps for a small sun (0.05° is om ≈ 1e-7).
float3 cone_sample(float3 axis,float om,out float pdf){float x=random()*om,s=sqrt(x*(2-x)),a=2*PI*random();pdf=1/(2*PI*om);return basis(axis,float3(s*cos(a),s*sin(a),1-x));}
// 1 − cos of the half angle a sphere covers (0 from inside), without cancellation when far.
float sphere_om(float d2,float r){float s2=r*r/d2;return d2>r*r?s2/(1+sqrt(1-s2)):0;}
// Spot lights (flag bit 5): smoothstep between the cosines of the outer and inner half angles.
// IES profile (flag bit 14, `ies::pack`): the relative intensity at 60 angles over 0–90° from
// the light's normal, three 8-bit values per float in layers, sheen, base_scale, surface_scale,
// medium.xyz and phase.x (a light reads none of them otherwise), linearly interpolated.
float ies_sample(Material m,uint i){
    uint k=i/3;float v=k<4?m.layers[k]:k<8?m.sheen[k-4]:k<12?m.base_scale[k-8]:k<16?m.surface_scale[k-12]:k<19?m.medium[k-16]:m.phase.x;
    return (((uint)v>>(8*(i%3)))&255u)/255.0;
}
float ies_profile(Material m,float c){
    if(c<=0)return 0;
    float x=acos(min(c,1))*(59/(0.5*PI));uint i=min((uint)x,58u);
    return lerp(ies_sample(m,i),ies_sample(m,i+1),x-i);
}
// A light's emission factor toward cosine c from its normal: the IES profile, a spot's cone, or 1.
float spot(Material m,float c){
    uint f=(uint)m.lobes.w;if((f&16384u)!=0)return ies_profile(m,c);
    return (f&32u)!=0?smoothstep(m.secondary.x,m.secondary.y,c):1;
}
float emitter_pdf(float3 from,float3 at,Emitter light){
    if(light.position_area.w==-1){float3 c=light.position_area.xyz-from;float om=sphere_om(dot(c,c),light.edge1_cdf.x);return om>0?light.edge2_probability.w/(2*PI*om):0;}
    float3 d=at-from;float dist2=dot(d,d);float3 n=normalize(cross(light.edge1_cdf.xyz,light.edge2_probability.xyz));float c=abs(dot(n,normalize(d)));return light.edge2_probability.w*dist2/max(1e-8,light.position_area.w*c);}
// ReSTIR's emitter choice: Walker's alias table (lighting::alias_records, after the n emitters,
// eight threshold / alias pairs per record). Same probabilities as select_index's CDF, in O(1).
uint alias_index(float u,float v){
    uint n=scene_info.x,i=min((uint)(u*n),n-1);Emitter e=emitters[n+(i>>3)];uint j=i&7;
    float4 r=j<2?e.position_area:j<4?e.edge1_cdf:j<6?e.edge2_probability:e.emission_material;
    uint2 pair=asuint((j&1)!=0?r.zw:r.xy);return v<asfloat(pair.x)?i:pair.y;
}
// Word w of the records after the emitters (sixteen u32 per Emitter record).
uint emitter_word(uint w){
    Emitter e=emitters[w>>4];uint j=(w>>2)&3;
    float4 r=j==0?e.position_area:j==1?e.edge1_cdf:j==2?e.edge2_probability:e.emission_material;
    return asuint(r[w&3]);
}
// The emitter CDF's binary search, bracketed by its cutpoints (lighting::guide_records, after
// the alias records) once the table is big: the same index as the full search, in one or two
// loads plus a few steps. Small tables search in full: with Classic's two emitters the two
// guide loads cost 4% of the trace pass.
uint select_index(float u){
    uint n=scene_info.x,lo=0,hi=n-1;
    if(n>=64){uint K=1u<<firstbithigh(n),k=(uint)(u*K),base=(n+(n+7)/8)*16;lo=emitter_word(base+k);hi=k+1<K?emitter_word(base+k+1):n-1;}
    while(lo<hi){uint mid=(lo+hi)/2;if(u<emitters[mid].edge1_cdf.w)hi=mid;else lo=mid+1;}return lo;}
float3 camera_direction(float2 pixel){float2 ndc=(pixel/float2(dimensions.xy))*2-1;ndc.y=-ndc.y;float4 p=mul(inverse_vp,float4(ndc,1,1));return normalize(p.xyz/p.w-eye.xyz);}
// A point in the unit aperture: a disk, or an n-gon (one triangle of the fan, then uniform in it).
float2 aperture_point(){
    if(lens.z<2.5){float r=sqrt(random()),a=2*PI*random();return r*float2(cos(a),sin(a));}
    float n=lens.z,k=floor(random()*n);float a0=lens.w+2*PI*k/n,a1=lens.w+2*PI*(k+1)/n;
    float u=random(),v=random();if(u+v>1){u=1-u;v=1-v;}
    return u*float2(cos(a0),sin(a0))+v*float2(cos(a1),sin(a1));
}
// Track 16 real lens systems (lens.rs `pack`): millimetres, the film at z = 0, the lens toward
// -z. camera_weight is the sample's weight from the camera (1 except through a lens system).
static float3 camera_weight=1;
#define LENS_BANDS_BASE 34
// A surface's index in channel c, or (c = 3, spectral) at the hero wavelength: lens.rs `indices`
// builds the three on one Cauchy curve (green at 550 nm, red 610, blue 465), so the curve is
// recovered exactly from red and blue.
float lens_ior(float4 n,uint c){
    if(c<3)return n[c];
    float l=path_lambdas.x*1e-3,b=(n.z-n.x)/(1/(0.465*0.465)-1/(0.61*0.61));
    return n.y+b*(1/(l*l)-1/(0.55*0.55));
}
// From the film to the object side through every surface, in channel c's indices.
bool trace_lens(inout float3 o,inout float3 d,uint c){
    for(int i=int(real_lens.x)-1;i>=0;i--){
        float4 s=lens_data[2+2*i];float t;float3 nrm=float3(0,0,1);
        if(s.x==0)t=(s.y-o.z)/d.z;
        else{
            float3 oc=o-float3(0,0,s.y+s.x);float b=dot(oc,d),disc=b*b-(dot(oc,oc)-s.x*s.x);if(disc<0)return false;
            float q=sqrt(disc);t=((d.z>0)!=(s.x<0))?-b-q:-b+q;nrm=normalize(oc+t*d);
        }
        if(!(t>0))return false;o+=t*d;
        if(s.x==0){if(!inside_aperture(o.xy,s.z))return false;continue;}
        if(dot(o.xy,o.xy)>s.z*s.z)return false;
        float before=i>0?lens_ior(lens_data[3+2*(i-1)],c):1;
        float3 w=refract(d,dot(nrm,d)>0?-nrm:nrm,lens_ior(lens_data[3+2*i],c)/before);if(dot(w,w)==0)return false;d=w;
    }
    return true;
}
// A film point for the pixel (inverted: the lens flips the image), a point in its band's exit-pupil
// bounds turned to the film point's angle (their centre for the guide ray), traced out of the front
// element. Weight cos⁴θ · bounds area / the centre's effective pupil: 1 at the centre of the frame.
bool lens_ray(float2 ndc,bool sample_pupil,uint c,out float3 o,out float3 d){
    float4 h=lens_data[0],g=lens_data[1];
    float3 f=float3(-ndc.x*h.x,-ndc.y*h.y,0);float r=length(f.xy);
    float4 e=lens_data[LENS_BANDS_BASE+min((uint)(r/g.w*h.z),(uint)h.z-1)];
    o=f;d=float3(0,0,-1);if(e.w<=0)return false;
    float2 q=sample_pupil?float2(lerp(e.x,e.y,random()),(random()*2-1)*e.z):float2((e.x+e.y)*0.5,0);
    float2 cs=r>1e-6?f.xy/r:float2(1,0);q=float2(q.x*cs.x-q.y*cs.y,q.x*cs.y+q.y*cs.x);
    d=normalize(float3(q,g.x)-f);float c2=d.z*d.z;
    if(!trace_lens(o,d,c))return false;
    camera_weight=c2*c2*e.w*h.w;return true;
}
// Camera rays for every model (roadmap 5c). `lens_sample` adds depth of field; a ray the
// cat-eye vignette blocks comes back with valid=false and contributes nothing (optical vignetting).
void camera_ray(float2 pixel,bool lens_sample,out float3 o,out float3 d,out bool valid){
    float3 right=world_view[0].xyz,up=world_view[1].xyz,forward=-world_view[2].xyz;
    float2 ndc=(pixel/float2(dimensions.xy))*2-1;ndc.y=-ndc.y;float aspect=float(dimensions.x)/dimensions.y;
    uint model=(uint)camera_model.x;o=eye.xyz;valid=true;camera_weight=1;
    if(model==0 && real_lens.x>0){
        // Dispersion: each sample traces one channel (weight 3 there) through that channel's indices;
        // the spectral tracer traces the hero wavelength instead (no random number taken).
        bool spectral=lens_sample && real_lens.y>0;uint c=spectral?(spectral_on?3:min((uint)(random()*3),2u)):1;
        float3 lo,ld;
        if(!lens_ray(ndc,lens_sample,c,lo,ld)){
            // A vignetted sample is invalid; the guide falls back to the pinhole ray.
            d=camera_direction(pixel);if(lens_sample)valid=false;camera_weight=1;return;
        }
        o=eye.xyz+(right*lo.x+up*lo.y+forward*(lens_data[1].z-lo.z))*real_lens.z;
        d=normalize(right*ld.x+up*ld.y-forward*ld.z);
        if(spectral && spectral_on){camera_weight*=float3(3,0,0);hero_only=true;}
        else if(spectral){float3 m=0;m[c]=3;camera_weight*=m;}
        return;
    }
    if(model==1){o=eye.xyz+(right*ndc.x*aspect+up*ndc.y)*camera_model.w;d=forward;return;}
    if(model==2){float phi=ndc.x*PI,theta=ndc.y*PI*0.5;d=normalize(right*sin(phi)*cos(theta)+up*sin(theta)+forward*cos(phi)*cos(theta));return;}
    if(model==3){
        // Stereographic projection looking straight down: the ground in the middle, the sky around it.
        float3 fh=normalize(float3(forward.x,0,forward.z)+1e-5),rh=normalize(cross(fh,float3(0,1,0)));
        float2 q=float2(ndc.x*aspect,ndc.y)*environment.z*2;float s=1+dot(q,q);
        d=normalize(rh*(2*q.x/s)+fh*(2*q.y/s)-float3(0,1,0)*((1-dot(q,q))/s));return;
    }
    d=camera_direction(pixel);
    if(!lens_sample || lens.x<=0)return;
    // Focal plane: perpendicular to the view axis, or tilted about the camera's right axis.
    float3 n=model==4?normalize(forward*cos(camera_model.z)+up*sin(camera_model.z)):forward;
    float3 focal=eye.xyz+d*(dot(forward*lens.y,n)/max(dot(d,n),1e-4));
    float2 l=aperture_point();
    // Cat-eye: the aperture seen off-axis is cut by the lens barrel, a circle shifted toward the edge.
    if(camera_model.y>0 && length(l-ndc*float2(aspect,1)*camera_model.y*0.7)>1){valid=false;return;}
    o=eye.xyz+(right*l.x+up*l.y)*lens.x;d=normalize(focal-o);
}
// Light sampling from x: each strategy picks a direction ld with its radiance le, its pdf pl
// (solid angle, selection included) and a shadow-ray limit. Shared by surface vertices and
// volume scattering (track 12); false when nothing was picked.
// Shadow-ray length to a point on an emitter len away, seen at cosine c to its normal: stop
// 0.6 mm off its surface (along the normal, not the ray), so a grazing sample does not hit the
// emitter's own face (that lost ~0.5% of night-skyline's light-sampled light).
float nee_limit(float len,float c){return max(0.0002,len-min(0.5*len,0.0006/max(abs(c),0.02)));}
// The colour an emitter's surface emits at p (times optical.y): the material's colour, or with
// an emissive map (flag 64, `Gpu::attach_maps`) the texel at p's UV times medium.xyz (1 / the
// map's mean) and secondary.yzw (the mean emitted colour, which the emitter table holds);
// secondary.x is the map's heap slot + 1. e1: the emitter index + 1 (0: none, the colour). The
// UVs are the emitter's UV record, after the SDF photon records (`Gpu::upload_lights`).
float3 emitted_color(Material m,uint e1,float3 p){
    if(!texture_maps || e1==0 || ((uint)m.lobes.w&64u)==0)return m.color_rough.rgb;
    Emitter x=emitters[e1-1];float3 d=p-x.position_area.xyz,u=x.edge1_cdf.xyz,v=x.edge2_probability.xyz;
    float uu=dot(u,u),uv=dot(u,v),vv=dot(v,v),du=dot(d,u),dv=dot(d,v),det=max(uu*vv-uv*uv,1e-30);
    float b1=(vv*du-uv*dv)/det,b2=(uu*dv-uv*du)/det;
    Emitter r=emitters[(uint)photon.w+2*(uint)photon.z+e1-1];
    float2 t=r.position_area.xy*(1-b1-b2)+r.position_area.zw*b1+r.edge1_cdf.xy*b2;
    return m.secondary.yzw*sample_map((uint)m.secondary.x,t).rgb*m.medium.xyz;
}
// Light sampling of emissive procedural objects (trace only: sdf_nee_on, set by trace_main;
// BDPT and the photon gather weigh their hits as before). The SDF photon records (`sdf_photon`)
// give each one's bounding sphere and power CDF: NEE picks one by power, a direction in the
// cone its bounding sphere covers (the whole sphere of directions from inside it), and keeps
// the sample when the first surface that way is that object. With triangles too, the two
// share the non-sky half evenly (`sdf_share`); scenes without them sample as before.
static bool sdf_nee_on=false;
float sdf_share(){return sdf_nee_on?(scene_info.x>0?0.5:1):0;}
float sdf_record_prob(uint k){uint b=(uint)photon.w;return emitters[b+2*k].edge1_cdf.y-(k>0?emitters[b+2*k-2].edge1_cdf.y:0);}
float sdf_dir_pdf(float3 x,uint k){Emitter a=emitters[(uint)photon.w+2*k];float3 c=a.position_area.xyz-x;float om=sphere_om(dot(c,c),a.position_area.w);return om>0?1/(2*PI*om):1/(4*PI);}
// The record of the procedural object with material id, or 0xffffffff.
uint sdf_record_of(uint material){
    for(uint k=0;k<(uint)photon.z;k++)if(instances[asuint(emitters[(uint)photon.w+2*k].edge1_cdf.x)].material==material)return k;
    return 0xffffffffu;
}
// The cosine from a triangle light's normal below which it emits nothing (a spot's outer cone,
// an IES profile's or a one-sided light's half space), or −1: equiangular sampling skips the
// part of a ray outside it.
float emission_cos(Material lm){
    uint f=(uint)lm.lobes.w;float k=(f&16384u)!=0 || lm.optical.w>0.5?0:-1;
    if((f&32u)!=0 && (f&16384u)==0)k=max(k,lm.secondary.x);return k;
}
// The part [a, b] of [t0, t1] on o + d t inside the cone of apex p, axis n and cosine k (convex
// for k ≥ 0, so one interval); false when empty. k < 0 keeps [t0, t1].
bool cone_interval(float3 o,float3 d,float3 p,float3 n,float k,float t0,float t1,out float a,out float b){
    a=t0;b=t1;if(k<0)return true;
    float3 w=o-p;float dn=dot(d,n),wn=dot(w,n),dw=dot(d,w),k2=k*k;
    float A=dn*dn-k2,B=2*(dn*wn-k2*dw),C=wn*wn-k2*dot(w,w);
    float r[5]={t0,t1,t1,t1,t1};
    if(abs(dn)>1e-8)r[2]=clamp(-wn/dn,t0,t1);
    if(abs(A)>1e-8){float disc=B*B-4*A*C;if(disc>=0){float q=sqrt(disc);r[3]=clamp((-B-q)/(2*A),t0,t1);r[4]=clamp((-B+q)/(2*A),t0,t1);}}
    else if(abs(B)>1e-8)r[3]=clamp(-C/B,t0,t1);
    [unroll]for(uint i=1;i<5;i++)[unroll]for(uint j=4;j>i;j--)if(r[j]<r[j-1]){float x=r[j];r[j]=r[j-1];r[j-1]=x;}
    float lo=t1,hi=t0;
    [unroll]for(uint i=0;i<4;i++){if(r[i+1]<=r[i])continue;float3 q=w+d*(0.5*(r[i]+r[i+1]));
        if(dot(q,n)>=k*length(q)){lo=min(lo,r[i]);hi=max(hi,r[i+1]);}}
    a=lo;b=hi;return hi>lo;
}
// The light sample_light chose, for equiangular MIS in media: a sphere's centre or the triangle
// point (nee_anchored false for the sky and suns).
static float3 nee_anchor=0,nee_axis=0;
static float nee_cos=-1;
static bool nee_anchored=false;
// Night sky N3 (stars.rs): catalogue stars as point lights in the J2000 frame, turned into the
// world by star_x/y/z each frame. Light sampling takes them, by power, as a share star_y.w of
// the sky's samples, only where star_nee is set (the path tracer; BDPT and photon mapping
// leave them out, so their sky sampling and MIS stay as they were). A star is a delta
// direction: its sample comes back with a negative pl (no MIS; nee_density), and BSDF-sampled
// rays never find one. Camera rays and pure-delta chains see stars through a cone of chord
// star_z.w (half a pixel) instead (star_seen); the two cover disjoint paths.
static bool star_nee=false;
bool stars_on(){return star_tau.w>0;}
float star_share(){return star_nee?star_y.w:0;}
// The share of sky samples the map (and its split sun) keeps: env_pdf's factor in MIS.
float env_keep(){return 1-star_share();}
float3 star_world(float3 c){return float3(dot(star_x.xyz,c),dot(star_y.xyz,c),dot(star_z.xyz,c));}
float3 star_celestial(float3 d){return star_x.xyz*d.x+star_y.xyz*d.y+star_z.xyz*d.z;}
// The air toward world height y: exp(-tau X), X Kasten and Young's air mass; 0 below the horizon.
float3 star_air(float y){if(y<=0)return 0;float z=degrees(acos(min(y,1)));return exp(-star_tau.rgb/(y+0.50572*pow(96.07995-z,-1.6364)));}
float3 star_dir(uint i){uint b=8+8*i;return asfloat(uint3(star_data[b],star_data[b+1],star_data[b+2]));}
float3 star_rgb(uint i){uint b=8+8*i;return asfloat(uint3(star_data[b+4],star_data[b+5],star_data[b+6]));}
uint star_pick(float u,out float prob){u*=star_limit.y;uint lo=0,hi=(uint)star_limit.x-1;while(lo<hi){uint mid=(lo+hi)/2;if(u<asfloat(star_data[8+8*mid+3]))hi=mid;else lo=mid+1;}prob=asfloat(star_data[8+8*lo+7])/star_limit.y;return lo;}
// The stars inside the cone around world direction d, spread over its solid angle (π chord²).
float3 star_seen(float3 d){
    float3 c=star_celestial(d);float chord=star_z.w;uint rows=star_data[1],cols=star_data[2],ob=star_data[3],eb=star_data[4];
    float dec=asin(clamp(c.z,-1,1)),ra=atan2(c.y,c.x);
    uint row=min((uint)max(0,(0.5-dec/PI)*rows),rows-1),col=min((uint)(frac(ra/(2*PI))*cols),cols-1),cell=row*cols+col;
    float3 sum=0;for(uint k=star_data[ob+cell];k<star_data[ob+cell+1];k++){uint i=star_data[eb+k];float3 v=c-star_dir(i);if(i<(uint)star_limit.x && dot(v,v)<chord*chord)sum+=star_rgb(i);}
    return sum*star_x.w*star_air(d.y)/(PI*chord*chord);
}
// The density a light sample's contribution divides by: its pdf alone for a star (pl < 0) or
// without MIS, else the balance heuristic's sum.
float nee_density(float pl,float pp,bool alone){return max(pl<0?-pl:alone?pl:pl+pp,1e-8);}
bool sample_light(float3 x,bool env_nee,bool tri_nee,float p_env,out float3 ld,out float3 le,out float pl,out float limit){
    bool nee=false;uint lmat=0xffffffffu;ld=float3(0,1,0);le=0;pl=0;limit=1e4;nee_anchored=false;if(lpe_on)lpe_env_sample=false;
    // The sky's pick, subdivided for the stars (the same random number: no new dimension).
    bool env_pick=false;float u=0;if(env_nee){u=random();env_pick=u<p_env;}
    if(env_pick){
        float us=u/p_env;if(lpe_on)lpe_env_sample=true;
        if(us<star_share()){
            float prob;uint i=star_pick(us/star_share(),prob);ld=star_world(star_dir(i));
            if(ld.y>0 && prob>0){le=star_rgb(i)*star_x.w*star_air(ld.y);pl=-p_env*star_share()*prob;nee=true;}
        }else{float pe;ld=sample_env(pe);
        if(pe>0){le=environment_radiance(ld);pl=p_env*env_keep()*pe;nee=true;}}
    }else if(sdf_nee_on && (scene_info.x==0 || random()<0.5)){
        uint b=(uint)photon.w,n=(uint)photon.z,j=0;float u=random();
        for(;j+1<n && u>=emitters[b+2*j].edge1_cdf.y;j++){}
        Emitter A=emitters[b+2*j];float q=sdf_record_prob(j);uint mat=instances[asuint(A.edge1_cdf.x)].material;
        float3 c=A.position_area.xyz-x;float d2=dot(c,c),om=sphere_om(d2,A.position_area.w),pc;
        if(om>0)ld=cone_sample(c*rsqrt(d2),om,pc);else{float z=1-2*random(),s=sqrt(max(0,1-z*z)),a=2*PI*random();ld=float3(s*cos(a),z,s*sin(a));pc=1/(4*PI);}
        Hit hh;
        if(q>0 && intersect(x,ld,1e4,hh,true,false) && hh.material==mat){
            Material lm=material_at(mat,hh.p);
            if(lm.optical.w<0.5 || hh.front){le=lm.color_rough.rgb*lm.optical.y*spot(lm,-dot(ld,hh.front?hh.ng:-hh.ng));pl=(1-p_env)*sdf_share()*q*pc;limit=nee_limit(hh.t,dot(hh.n,ld));lmat=mat;nee=true;}
        }
    }else if(tri_nee){
    uint li=select_index(random());Emitter light=emitters[li];lmat=(uint)light.emission_material.w;
    if(light.position_area.w<0){
        // Sphere or sun: a direction in the cone it covers, then a shadow ray up to the
        // sphere's true surface (its inscribed mesh lies beyond) or to infinity.
        float om=0,pc=0;
        if(light.position_area.w==-1){
            nee_anchor=light.position_area.xyz;nee_cos=-1;nee_anchored=true;
            float3 c=light.position_area.xyz-x;float d2=dot(c,c),r=light.edge1_cdf.x;om=sphere_om(d2,r);
            if(om>0){ld=cone_sample(c*rsqrt(d2),om,pc);float b=dot(c,ld);limit=b-sqrt(max(0,r*r-(d2-b*b)));}
        }else{om=light.edge1_cdf.x;ld=cone_sample(light.position_area.xyz,om,pc);}
        if(pc>0){le=light.emission_material.rgb;pl=(1-p_env)*(1-sdf_share())*light.edge2_probability.w*pc;limit=max(0.0002,limit-0.0006);nee=true;}
    }else{
    float su=sqrt(random()),v=random();float3 lp=light.position_area.xyz+light.edge1_cdf.xyz*(1-su)+light.edge2_probability.xyz*(su*v);ld=lp-x;float len=length(ld);ld/=max(len,1e-8);
    Material lm=material_at((uint)light.emission_material.w,lp);float3 ln=normalize(cross(light.edge1_cdf.xyz,light.edge2_probability.xyz));
    nee_anchor=lp;nee_axis=ln;nee_cos=emission_cos(lm);nee_anchored=true;
    if(lm.optical.w<0.5 || dot(ln,-ld)>0){le=emitted_color(lm,li+1,lp)*lm.optical.y*spot(lm,dot(ln,-ld));pl=(1-p_env)*(1-sdf_share())*emitter_pdf(x,lp,light);limit=nee_limit(len,dot(ln,ld));nee=true;}
    }}
    if(lpe_on)lpe_light_label=lmat!=0xffffffffu?lpe_class(lmat):0;
    le=lmat!=0xffffffffu?up_emitted(lmat,le):up(le);return nee;
}
// ReSTIR DI (track 15; Bitterli et al. 2020, with the generalized RIS weights of Lin et al.
// 2022). A light sample is an emitter index and two 16-bit numbers that name the same point from
// every surface (area measure; a sun's direction, solid angle), so samples move between pixels and
// frames with a unit Jacobian. Reservoirs (32 B) hold the sample, its contribution weight W, the
// confidence M and the surface it was made for. Each of the frame's first `layers` samples has its
// own chain (sample s uses layer s), so samples stay independent: the final reservoirs
// (restir_spatial; next frame's history) of every layer, then restir_initial's of every layer,
// then the frame's light tiles (restir_presample).
struct Reservoir{uint light;uint uv;float W;uint m_material;float3 position;uint normal;};
RWStructuredBuffer<Reservoir> reservoirs:register(u2,space2);
// A surface the target is evaluated on, with its cheap BSDF terms read once (target_brdf).
struct Domain{float3 p;float3 n;float3 v;uint material;float diffuse;float spec;float alpha;};
struct LightPoint{float3 p;float3 n;float3 le;float source;uint kind;float lum;}; // kind 0 triangle, 1 sphere, 2 sun (p = direction); le the table's flat emission, lum its luminance
uint restir_layers(){return (restir.x>>8)&0xffu;}
// Light tiles (Wyman and Panteleev 2021): each frame restir_presample draws RESTIR_TILES ×
// RESTIR_TILE light samples from the source distribution, and every 8×8 group takes its
// candidates from one tile, so the reads stay in cache instead of hitting a random emitter (and
// its alias record) per candidate. Still unbiased: RIS needs each candidate's marginal pdf, not
// independent candidates. On fairy-lights (150k emitter triangles, 1440×900, RR 1 spp) ReSTIR's
// overhead fell from 14.3 ms (alias draws per candidate) to 2.5 ms.
static const uint RESTIR_TILES=128,RESTIR_TILE=1024;
uint restir_slot(uint stage,uint layer){return (stage*restir_layers()+layer)*dimensions.x*dimensions.y;}
uint pack_uv(float2 u){uint2 q=min((uint2)(u*65536),65535u);return q.x|q.y<<16;}
float2 unpack_uv(uint q){return (float2(q&0xffffu,q>>16)+0.5)/65536;}
uint pack_normal(float3 n){uint2 q=(uint2)round((oct_encode(n)*0.5+0.5)*65535);return q.x|q.y<<16;}
float3 unpack_normal(uint q){return oct_decode(float2(q&0xffffu,q>>16)/65535*2-1);}
float reservoir_m(Reservoir r){return (float)(r.m_material>>24);}
bool reservoir_valid(Reservoir r){return (r.m_material&0xffffffu)!=0xffffffu && r.light<scene_info.x;}
// The target's BSDF: Lambert with a floor, so every direction above the surface keeps support,
// plus one isotropic GGX lobe.
Domain make_domain(float3 p,float3 n,float3 v,uint material){
    Domain d;d.p=p;d.n=n;d.v=v;d.material=material;Material m=materials[material];
    d.diffuse=max(luminance(m.color_rough.rgb)*(1-m.lobes.x)*(1-m.lobes.z),0.05)/PI;d.spec=spec_probability(m);d.alpha=max(0.01,m.color_rough.a*m.color_rough.a);
    return d;
}
Domain domain_of(Reservoir r){return make_domain(r.position,unpack_normal(r.normal),normalize(eye.xyz-r.position),r.m_material&0xffffffu);}
// The point (or sun direction) a sample names, and its source pdf: power selection times uniform
// area (triangles, the whole sphere) or the sun's cone.
LightPoint light_point(uint i,uint q){
    Emitter e=emitters[i];float2 u=unpack_uv(q);float prob=e.edge2_probability.w;LightPoint l;l.le=e.emission_material.rgb;l.lum=luminance(l.le);
    if(e.position_area.w==-1){float z=1-2*u.x,s=sqrt(max(0,1-z*z)),a=2*PI*u.y,r=e.edge1_cdf.x;l.n=float3(s*cos(a),s*sin(a),z);l.p=e.position_area.xyz+r*l.n;l.source=prob/(4*PI*r*r);l.kind=1;}
    else if(e.position_area.w<0){float om=e.edge1_cdf.x,x=u.x*om,s=sqrt(x*(2-x)),a=2*PI*u.y;l.p=basis(e.position_area.xyz,float3(s*cos(a),s*sin(a),1-x));l.n=0;l.source=prob/(2*PI*om);l.kind=2;}
    else{float su=sqrt(u.x);l.p=e.position_area.xyz+e.edge1_cdf.xyz*(1-su)+e.edge2_probability.xyz*(su*u.y);l.n=normalize(cross(e.edge1_cdf.xyz,e.edge2_probability.xyz));l.source=prob/e.position_area.w;l.kind=0;}
    return l;
}
// Radiance from the sample toward x and the geometry factor (area measure; 1 for a sun), as
// sample_light computes them; ld and limit for the shadow ray.
float3 light_toward(uint i,LightPoint l,float3 x,out float3 ld,out float g,out float limit){
    if(l.kind==2){ld=l.p;g=1;limit=1e4;return l.le;}
    ld=l.p-x;float d2=dot(ld,ld),len=sqrt(d2);ld/=max(len,1e-8);float c=dot(l.n,-ld);limit=nee_limit(len,c);
    if(l.kind==1){g=max(c,0)/max(d2,1e-8);return c>0?l.le:0;}
    Material lm=material_at((uint)emitters[i].emission_material.w,l.p);g=abs(c)/max(d2,1e-8);
    return lm.optical.w>=0.5 && c<=0?0:emitted_color(lm,i+1,l.p)*lm.optical.y*spot(lm,c);
}
// The target function: unshadowed reflected luminance from the table's flat emission (no
// pattern or spot cone: a target only has to be positive wherever the light can be) through
// the domain's cheap BSDF. Visibility is left to the final shadow ray, which keeps every merge
// unbiased without extra rays.
float restir_target(Domain d,LightPoint l){
    float3 ld;float g;
    if(l.kind==2){ld=l.p;g=1;}
    else{
        ld=l.p-d.p;float d2=max(dot(ld,ld),1e-16);ld*=rsqrt(d2);float c=dot(l.n,-ld);
        // A triangle seen from behind: on a closed emitter (a bulb, an emissive box) the object
        // itself hides it, which the unshadowed target cannot know (64% of the remaining error
        // in fairy-lights). A small weight rather than 0 keeps such points reachable (two-sided
        // panels, a camera inside a luminous room), so the result stays unbiased.
        g=l.kind==1?max(c,0)/d2:abs(c)/d2*(c>0?1:0.02);
    }
    float c=dot(d.n,ld);if(c<=0 || g<=0)return 0;
    float spec=d.spec*ggx_d(saturate(dot(d.n,normalize(ld+d.v))),d.alpha)/(4*max(dot(d.n,d.v),0.1)*max(c,0.1));
    return l.lum*g*c*(d.diffuse+spec);
}
// Streaming RIS over k candidates drawn from one light tile; W = 0 when none has any target.
// A tile entry is a Reservoir record: light | kind << 30, uv, W = source pdf, m_material = the
// emission's luminance bits, position = the point (a sun: its direction), normal.
void restir_candidates(Domain d,uint k,uint tile,out uint light,out uint q,out float W){
    float sum=0,chosen=0;light=0;q=0;uint base=restir_slot(2,0)+tile*RESTIR_TILE;
    for(uint j=0;j<k;j++){
        Reservoir r=reservoirs[base+min((uint)(random()*RESTIR_TILE),RESTIR_TILE-1)];
        LightPoint l;l.p=r.position;l.n=unpack_normal(r.normal);l.le=0;l.source=r.W;l.kind=r.light>>30;l.lum=asfloat(r.m_material);
        float t=restir_target(d,l),w=t/(max(l.source,1e-30)*k);sum+=w;
        if(w>0 && random()*sum<w){light=r.light&0x3fffffffu;q=r.uv;chosen=t;}
    }
    W=chosen>0?sum/chosen:0;
}
// Emission found by BSDF sampling from a vertex that used ReSTIR counts 0, except from inside a
// sphere light, where the area samples (all facing away) can never reach it.
bool restir_covers(float3 x,Emitter e){float3 c=x-e.position_area.xyz;return e.position_area.w!=-1 || dot(c,c)>e.edge1_cdf.x*e.edge1_cdf.x;}
// Generalized RIS over two domains, a canonical: balance weights M·p̂ over both surfaces sum to 1
// for every sample, so the result is unbiased for a whatever b is (another pixel, last frame).
// Every merge takes the surfaces it chose by geometry alone: a reservoir that holds no sample
// (W = 0) still takes part with its M. Skipping it would condition on its random outcome and
// over-weight the reservoirs that did find one (measured +1.1% with one candidate).
void restir_merge(Domain a,uint la,uint qa,float Wa,float Ma,Domain b,uint lb,uint qb,float Wb,float Mb,out uint light,out uint q,out float W){
    float aa=0,ab=0,ba=0,bb=0;
    if(Wa>0){LightPoint l=light_point(la,qa);aa=restir_target(a,l);ab=restir_target(b,l);}
    if(Wb>0){LightPoint l=light_point(lb,qb);ba=restir_target(a,l);bb=restir_target(b,l);}
    float wa=aa>0?Ma*aa/(Ma*aa+Mb*ab)*aa*Wa:0,wb=bb>0?Mb*bb/(Ma*ba+Mb*bb)*ba*Wb:0,sum=wa+wb;
    bool pick=random()*sum<wb;light=pick?lb:la;q=pick?qb:qa;float t=pick?ba:aa;W=t>0?sum/t:0;
}
// Bounce-0 light sample: the pixel's final reservoir merged with one fresh candidate at this hit,
// which keeps every light reachable where the hit is not the reservoir's surface (edges inside
// the pixel filter, depth of field, portals). le comes back times G and W.
bool restir_nee(uint2 p,uint layer,Hit h,float3 v,out float3 ld,out float3 le,out float limit){
    Domain a=make_domain(h.p,h.n,v,h.material);
    uint lc=alias_index(random(),random()),qc=pack_uv(float2(random(),random()));float Wc=1/max(light_point(lc,qc).source,1e-30);
    Reservoir r=reservoirs[restir_slot(0,layer)+p.y*dimensions.x+p.x];uint light=lc,q=qc;float W=Wc;
    if(reservoir_valid(r))restir_merge(a,lc,qc,Wc,1,domain_of(r),r.light,r.uv,r.W,reservoir_m(r),light,q,W);
    LightPoint l=light_point(light,q);float g;if(lpe_on)lpe_light_label=lpe_class((uint)emitters[light].emission_material.w);le=up_emitted((uint)emitters[light].emission_material.w,light_toward(light,l,h.p,ld,g,limit))*g*W;
    return W>0 && any(le>0);
}
// Volumes (track 12): homogeneous media, sampled by free flight on their extinction (`flight`:
// grey or per channel), with the albedo applied at each scatter and a Henyey-Greenstein phase
// function. c is the cosine between the travel and the new direction.
float hg_pdf(float c,float g){float d=max(1+g*g-2*g*c,1e-6);return (1-g*g)/(4*PI*d*sqrt(d));}
float3 hg_sample(float3 d,float g,out float pdf){
    float u=random(),c;
    if(abs(g)<1e-3)c=1-2*u;else{float s=(1-g*g)/(1-g+2*g*u);c=(1+g*g-s*s)/(2*g);}
    c=clamp(c,-1,1);float sn=sqrt(max(0,1-c*c)),a=2*PI*random();pdf=hg_pdf(c,g);
    return basis(d,float3(sn*cos(a),sn*sin(a),c));
}
static uint side_state=0;
float side_random(){side_state=hash(side_state+0x9e3779b9);return (side_state>>8)*(1.0/16777216.0);}
// Heterogeneous fog (Fog::noise): extinction fog_extinction() × k(p), k = max(0, 1 + n (2 fBm − 1))
// with fBm in [0, 1] (four octaves of value noise), so the majorant is σ (1 + n). Free flight is
// spectral tracking (Kutz et al. 2017; plain delta tracking when grey), shadow rays ratio
// tracking. The first tentative collision takes random() as `flight` does; the rest take the
// side stream (white noise in BDPT and the photon passes), so Sobol's dimensions stay aligned.
bool fog_hetero(){return fog_noise.x>0;}
float fog_random(){return white_noise?random():side_random();}
float lattice(int3 c){return hash((uint)c.x*73856093u^(uint)c.y*19349663u^(uint)c.z*83492791u)*(1.0/4294967296.0);}
float value_noise(float3 p){
    int3 i=(int3)floor(p);float3 f=p-floor(p);f=f*f*(3-2*f);
    float a=lerp(lattice(i),lattice(i+int3(1,0,0)),f.x),b=lerp(lattice(i+int3(0,1,0)),lattice(i+int3(1,1,0)),f.x);
    float c=lerp(lattice(i+int3(0,0,1)),lattice(i+int3(1,0,1)),f.x),e=lerp(lattice(i+int3(0,1,1)),lattice(i+int3(1,1,1)),f.x);
    return lerp(lerp(a,b,f.y),lerp(c,e,f.y),f.z);
}
float fog_density(float3 p){
    p/=fog_noise.y;float s=0,a=0.5,n=0;
    [unroll]for(uint o=0;o<4;o++){s+=a*value_noise(p);n+=a;p=p*2.03+float3(17.1,-3.7,9.3);a*=0.5;}
    return max(0,1+fog_noise.x*(2*s/n-1));
}
// A free flight over [t0, t1) of the heterogeneous fog: ts < t1 a real collision, sigma then
// the extinction there; w the spectral-tracking weight (1 when grey).
float fog_track(float3 o,float3 d,float t0,float t1,inout float3 sigma,out float3 w){
    w=1;if(hero_only)sigma=sigma.xxx;float3 s0=sigma;float mu=max(s0.x,max(s0.y,s0.z))*(1+fog_noise.x);float t=t0;
    if(mu<=0)return t1;
    for(uint i=0;i<1024;i++){
        t-=log(1-(i==0?random():fog_random()))/mu;if(t>=t1)return t;
        float3 st=s0*fog_density(o+d*t),sn=mu-st;float a=dot(w,st),b=dot(w,sn),pr=a/max(a+b,1e-30);
        if(fog_random()<pr){w*=st/(mu*pr);sigma=st;return t;}
        w*=sn/(mu*(1-pr));
    }
    return t1;
}
// The fog's transmittance over [t0, t1] (exact when uniform, ratio tracking otherwise).
float3 fog_tr(float3 o,float3 d,float t0,float t1){
    if(!fog_hetero())return exp(-fog_extinction()*(t1-t0));
    float3 s0=fog_extinction();if(hero_only)s0=s0.xxx;float mu=max(s0.x,max(s0.y,s0.z))*(1+fog_noise.x);float3 tr=1;float t=t0;
    if(mu<=0)return 1;
    for(uint i=0;i<1024;i++){t-=log(1-fog_random())/mu;if(t>=t1)break;tr*=1-s0*fog_density(o+d*t)/mu;if(max(tr.x,max(tr.y,tr.z))<1e-4)return 0;}
    return tr;
}
// Where the fog box (floor to fog.z, ±fog.w on x and z) covers a ray, clipped to [0, tmax].
bool fog_interval(float3 o,float3 d,float tmax,out float t0,out float t1){
    float3 inv=1/select(abs(d)>1e-9,d,1e-9),a=(float3(-fog.w,-1,-fog.w)-o)*inv,b=(float3(fog.w,fog.z,fog.w)-o)*inv;
    float3 lo=min(a,b),hi=max(a,b);t0=max(0,max(lo.x,max(lo.y,lo.z)));t1=min(tmax,min(hi.x,min(hi.y,hi.z)));
    return fog_on() && t1>t0;
}
// The scattering medium the path is in: the top of the dielectric stack, else the fog (only
// outside every object). [t0, t1] is where it fills the ray up to tmax.
bool medium_params(uint64_t media,float3 o,float3 d,float tmax,out float3 sigma,out float3 albedo,out float g,out float t0,out float t1){
    int top=medium_top(media);t0=0;t1=tmax;
    if(top>=0){Material m=medium_material(media,top);sigma=extinction_of(m);albedo=m.medium.rgb;g=m.phase.x;return m.medium.w>0;}
    sigma=fog_extinction();albedo=fog_color.rgb;g=fog.y;return fog_interval(o,d,tmax,t0,t1);
}
// The distance to the nearest surface (alpha-tested holes skipped), or limit: a medium's exit.
float surface_distance(float3 o,float3 d,float limit){
    RayDesc ray;ray.Origin=o;ray.Direction=d;ray.TMin=0.0001;ray.TMax=limit;
    RayQuery<RQ_BASE RQ_OMM> q;q.TraceRayInline(scene,RAY_FLAG_NONE,255,ray);
    while(q.Proceed()){proceed_steps++;
        if(!lean && q.CandidateType()==CANDIDATE_PROCEDURAL_PRIMITIVE){
            Instance pi=instances[q.CandidateInstanceID()];float t;
            if(sdf_hit(pi.vertex_base,q.CandidateObjectRayOrigin(),q.CandidateObjectRayDirection(),q.RayTMin(),q.CommittedRayT(),t))q.CommitProceduralPrimitiveHit(t);
            continue;
        }
        uint m=candidate_material(q.CandidateInstanceID(),q.CandidatePrimitiveIndex());
        if(flagged(m,8u) && alpha_hole(q.CandidateInstanceID(),q.CandidatePrimitiveIndex(),q.CandidateTriangleBarycentrics(),m))continue;
        q.CommitNonOpaqueTriangleHit();}
    return q.CommittedStatus()!=COMMITTED_NOTHING?q.CommittedRayT():limit;
}
// A shadow ray's transmittance: through a scattering object's medium up to the first surface
// (refraction there is ignored, as glass shadows ignore it; clear glass keeps its old
// behaviour, no absorption), then the fog past that point. Glass crossed on the way is not
// subtracted from the fog.
float3 medium_transmittance(uint64_t media,float3 o,float3 d,float limit){
    float3 tr=1;float start=0;int top=medium_top(media);
    if(top>=0){Material m=medium_material(media,top);
        if(m.medium.w>0){start=surface_distance(o,d,limit);tr=exp(-(extinction_of(m)-log(max(up_material(m,m.color_rough.rgb),0.001)))*start);}
        else return 1;
    }
    float t0,t1;if(fog_interval(o,d,limit,t0,t1)){t0=max(t0,start);if(t1>t0)tr*=fog_tr(o,d,t0,t1);}
    return tr;
}
// Equiangular sampling (Kulla and Fajardo 2012): a distance on [t0, t1] of the ray o + d t with
// density proportional to 1 / (h² + (t − Δ)²) about the anchor a (a light point), so the
// single scattering a light close to the ray lights is sampled where it is bright.
float equiangular(float3 o,float3 d,float3 a,float t0,float t1,float u,out float pdf){
    float c=dot(a-o,d),h=max(length(a-o-d*c),1e-4),a0=atan2(t0-c,h),a1=atan2(t1-c,h);
    float t=clamp(c+h*tan(lerp(a0,a1,u)),t0,t1),s=t-c;pdf=h/max((a1-a0)*(h*h+s*s),1e-30);return t;
}
float equiangular_pdf(float3 o,float3 d,float3 a,float t0,float t1,float t){
    float c=dot(a-o,d),h=max(length(a-o-d*c),1e-4),s=t-c;return h/max((atan2(t1-c,h)-atan2(t0-c,h))*(h*h+s*s),1e-30);
}
// The density of `flight` scattering at t (one-sample MIS over the channels when chromatic).
float flight_pdf(float3 sigma,float t0,float t){if(hero_only)sigma=sigma.xxx;float3 f=sigma*exp(-sigma*(t-t0));return (f.x+f.y+f.z)/3;}
// Equiangular single scattering on a medium segment [t0, t1) from o along d (trace): one light
// chosen as sample_light chooses it, a distance about it, then the light from there. MIS against
// the free-flight scatter's light sample (weights pd / (pd + pe) there and pe / (pd + pe) here,
// both inside the phase / light MIS), so it adds the part a far scatter misses. 0 for the sky
// and suns, which free flight keeps whole. A triangle light's distances stay inside the cone it
// emits into (`cone_interval`), in both weights.
float3 equiangular_light_side(uint64_t media,float3 o,float3 d,float t0,float t1,float3 sigma,float3 albedo,float3 absorption,float g,bool env_nee,float p_env,uint mis){
    if(env_nee && side_random()<p_env)return 0;
    if(sdf_nee_on && side_random()<0.5)return 0;
    uint li=select_index(side_random());Emitter light=emitters[li];if(lpe_on)lpe_light_label=lpe_class((uint)light.emission_material.w);if(light.position_area.w<0 && light.position_area.w!=-1)return 0;
    bool sphere=light.position_area.w==-1;float3 lp=light.position_area.xyz;
    float ea=t0,eb=t1;
    if(!sphere){float su=sqrt(side_random()),v=side_random();lp+=light.edge1_cdf.xyz*(1-su)+light.edge2_probability.xyz*(su*v);
        if(!cone_interval(o,d,lp,normalize(cross(light.edge1_cdf.xyz,light.edge2_probability.xyz)),emission_cos(materials[(uint)light.emission_material.w]),t0,t1,ea,eb))return 0;}
    float pe;float t=equiangular(o,d,lp,ea,eb,side_random(),pe);float3 x=o+d*t;
    float3 ld,le=0;float pl=0,limit=1e4;
    if(sphere){
        float3 c=lp-x;float d2=dot(c,c),r=light.edge1_cdf.x,om=sphere_om(d2,r),pc=0;if(om<=0)return 0;
        {float x0=side_random()*om,s0=sqrt(x0*(2-x0)),a0=2*PI*side_random();pc=1/(2*PI*om);ld=basis(c*rsqrt(d2),float3(s0*cos(a0),s0*sin(a0),1-x0));}float b=dot(c,ld);limit=max(0.0002,b-sqrt(max(0,r*r-(d2-b*b)))-0.0006);
        le=light.emission_material.rgb;pl=(1-p_env)*(1-sdf_share())*light.edge2_probability.w*pc;
    }else{
        ld=lp-x;float len=length(ld);ld/=max(len,1e-8);
        Material lm=material_at((uint)light.emission_material.w,lp);float3 ln=normalize(cross(light.edge1_cdf.xyz,light.edge2_probability.xyz));
        if(lm.optical.w>0.5 && dot(ln,-ld)<=0)return 0;
        le=emitted_color(lm,li+1,lp)*lm.optical.y*spot(lm,dot(ln,-ld));pl=(1-p_env)*(1-sdf_share())*emitter_pdf(x,lp,light);limit=nee_limit(len,dot(ln,ld));
    }
    if(pl<=0 || !any(le>0))return 0;
    float pp=hg_pdf(dot(d,ld),g),pd=flight_pdf(sigma,t0,t);float nw=mis==1?1:pl/(pl+pp);
    if(occluded(x,ld,limit))return 0;
    float3 tr=exp(-absorption*t-sigma*(t-t0));
    return tr*sigma*up(albedo)*pp*up_emitted((uint)light.emission_material.w,le)*medium_transmittance(media,x,ld,limit)*nw/(pl*(pe+pd));
}
// Its random numbers come from a side stream (white noise keyed by the pixel, sample and path
// state), so the path's own Sobol dimensions stay bit-identical.
float3 equiangular_light(uint64_t media,float3 o,float3 d,float t0,float t1,float3 sigma,float3 albedo,float3 absorption,float g,bool env_nee,float p_env,uint mis){
    side_state=hash(pixel_seed^hash(sequence_index*0x85ebca6bu+dimension_index*0x9e3779b9u)^rng_state);
    return equiangular_light_side(media,o,d,t0,t1,sigma,albedo,absorption,g,env_nee,p_env,mis);
}
// Giles' single-precision inverse error function.
float erfinv(float x){
    float w=-log(max((1-x)*(1+x),1e-30)),p;
    if(w<5){w-=2.5;p=2.81022636e-08;p=3.43273939e-07+p*w;p=-3.5233877e-06+p*w;p=-4.39150654e-06+p*w;p=0.00021858087+p*w;p=-0.00125372503+p*w;p=-0.00417768164+p*w;p=0.246640727+p*w;p=1.50140941+p*w;}
    else{w=sqrt(w)-3;p=-0.000200214257;p=0.000100950558+p*w;p=0.00134934322+p*w;p=-0.00367342844+p*w;p=0.00573950773+p*w;p=-0.0076224613+p*w;p=0.00943887047+p*w;p=1.00167406+p*w;p=2.83297682+p*w;}
    return p*x;
}
// Blackman-Harris window on [0,1]: the inverse of its CDF by bisection (the window is ~0 at the
// ends, so Newton steps would stall there).
float blackman_harris_inverse(float u){
    float lo=0,hi=1;
    for(int i=0;i<16;i++){
        float t=0.5*(lo+hi);
        float c=(0.35875*t-0.48829*sin(2*PI*t)/(2*PI)+0.14128*sin(4*PI*t)/(4*PI)-0.01168*sin(6*PI*t)/(6*PI))/0.35875;
        if(c<u)lo=t;else hi=t;
    }
    return 0.5*(lo+hi);
}
// Progressive path: a sub-pixel offset distributed as the pixel filter (filter importance
// sampling: every sample has weight 1, so the mean converges to the filtered image).
// lights.y: 0 box, 1 Gaussian (sigma = width / 6, cut at 3 sigma), 2 Blackman-Harris;
// lights.z: the filter's full width in pixels. Each axis on its own keeps the 2D stratification.
float2 filter_offset(float2 u){
    float width=lights.z;
    if(lights.y==1)return clamp(width/6*1.41421356*float2(erfinv(2*u.x-1),erfinv(2*u.y-1)),-0.5*width,0.5*width);
    if(lights.y==2)return (float2(blackman_harris_inverse(u.x),blackman_harris_inverse(u.y))-0.5)*width;
    return (u-0.5)*width;
}
// filter_offset's density along one axis at offset x from a pixel centre: BDPT's light-tracing
// splats weight the pixels around a film point by it (in both axes), so BDPT and photon mapping
// render the path tracer's filtered image. The Gaussian is normalized over its ±3 sigma support.
float filter_density(float x){
    float width=lights.z,t=x/width+0.5;if(t<0 || t>1)return 0;
    if(lights.y==1){float s=width/6;return exp(-0.5*x*x/(s*s))/(s*2.50662827*0.9973);}
    if(lights.y==2)return (0.35875-0.48829*cos(2*PI*t)+0.14128*cos(4*PI*t)-0.01168*cos(6*PI*t))/(0.35875*width);
    return 1/width;
}
// Path visualizer (track 14): the PATH_RECORD variant re-traces one pixel (path_record.xy) on one
// thread, path_record.z paths with this frame's sample indices (the first sampling.y are exactly
// the paths this frame accumulates), and writes no texture. Each event is a record (paths.rs
// parses them); record 0 holds the count. Kinds: 0 camera, 1 diffuse, 2 glossy, 3 dielectric
// reflection, 4 refraction, 5 volume scatter, 6 portal entry, 7 portal exit, 8 pass-through (a
// nested-dielectric false hit, or inside a portal), 9 the path stops at a surface, 10 escape,
// 11 light sample, 12 blocked light sample, 13 end (color = the path's radiance, value = why:
// 0 escaped, 1 unlit surface, 2 Russian roulette, 3 bounce limit, 4 sample below the surface,
// 5 blocked by the lens). Vertices carry the throughput arriving there and the luminance they
// add (emission); light samples their contribution and distance; direction is the way out.
static uint path_count,path_index;
#ifdef PATH_RECORD
static const bool recording=true;
struct PathVertex{float3 position;uint info;float3 color;float value;float3 direction;uint pad;};
RWStructuredBuffer<PathVertex> path_vertices:register(u0,space2);
void record(uint kind,uint bounce,float3 position,float3 color,float value,float3 direction){
    if(path_count+1>=path_record.w)return;
    PathVertex v;v.position=position;v.info=kind|(min(bounce,255u)<<8)|(path_index<<16);v.color=color;v.value=value;v.direction=direction;v.pad=0;
    path_vertices[++path_count]=v;
}
#else
static const bool recording=false;
void record(uint kind,uint bounce,float3 position,float3 color,float value,float3 direction){}
#endif
// The guide ray (pinhole through the lens centre, at the frame's jitter): RR's guides, the native
// denoiser's and the temporal upscalers' inputs. Shared by every integrator.
void write_guides(uint2 p,out Hit guide){
    // The object ids (selection outlines, picking, the denoiser's edge stop) keep the first frame's
    // hits while samples accumulate under a jitter: else edge pixels flip object every frame.
    bool id_fresh=(scene_info.w&128u)!=0;
    float3 go,gd;bool gvalid;camera_ray(float2(p)+0.5+jitter.xy,false,go,gd,gvalid);
    if(intersect(go,gd,1e4,guide,false,true)){
        Material m=material_at(guide.material,guide.p);apply_maps(guide,m);prepare_film(m,abs(dot(-gd,guide.n)),1,0);normal_roughness[p]=float4(guide.n,m.color_rough.a);if(id_fresh)object_id[p]=guide.material+1;
        diffuse_albedo[p]=float4(m.color_rough.rgb*(1-m.lobes.x)*(1-m.lobes.z),1);
        specular_albedo[p]=float4(spec_fresnel(m,abs(dot(-gd,guide.n)),f0_of(m)),1);
        depth[p]=camera_model.x==0||camera_model.x==4?-mul(world_view,float4(guide.p,1)).z:guide.t;
        float4 projected=mul(current_vp,float4(guide.p,1));device_depth[p]=projected.z/projected.w;
        float4 prev=mul(previous_vp,float4(guide.p,1)),curr=mul(current_vp,float4(guide.p,1));
        float2 delta=prev.xy/prev.w-curr.xy/curr.w;motion[p]=delta*float2(0.5,-0.5)*dimensions.xy;
        // A stable primary reflection ray gives RR consistent reflection-motion
        // guidance, instead of alternating random-lobe / zero / sky distances.
        // Only RR reads it; the native path skips the extra ray.
        Hit reflected;float3 reflected_direction=reflect(gd,guide.n);
        specular_distance[p]=modes.w!=0 && intersect(guide.p+guide.n*0.0003,reflected_direction,1e4,reflected,false,false)?reflected.t:65504.0;
    }else{guide.material=0xffffffffu;normal_roughness[p]=float4(0,0,0,1);if(id_fresh)object_id[p]=0;diffuse_albedo[p]=0;specular_albedo[p]=0;depth[p]=1e4;device_depth[p]=1;motion[p]=0;specular_distance[p]=65504.0;}
}
// Track 20: the same kernel as a ray generation shader for Shader Execution Reordering (SER,
// Shader Model 6.9), dispatched by DispatchRays over the same threads (Gpu::integrate).
void trace_main(uint3 tid);
#ifdef SER
[shader("raygeneration")]void trace_rg(){trace_main(DispatchRaysIndex());}
#else
[numthreads(8,8,1)]void trace(uint3 tid:SV_DispatchThreadID){trace_main(tid);}
#endif
void trace_main(uint3 tid){
    uint2 p=tid.xy;
    if(recording){if(any(p!=0))return;p=min(path_record.xy,dimensions.xy-1);}
    else{p+=region.xy;if(any(p>=region.zw))return;}
    if(any(p>=dimensions.xy))return;
    pixel_seed=hash(p.x+p.y*dimensions.x);rng_state=hash(pixel_seed+sampling.w*747796405);proceed_steps=0;side_state=hash(rng_state^0x68e31da4u);star_nee=true;
    sdf_nee_on=!lean && photon.z>0 && ((scene_info.w>>4)&3)!=2;float bounces_used=0;
    float3 mean=sampling.x>0?accumulation[p].rgb:0;float m2=sampling.x>0?accumulation[p].a:0;
    float3 dmean=sampling.x>0?direct_light[p].rgb:0,smean=sampling.x>0?specular_light[p].rgb:0;
    // Without RR the albedo guides are the mean first-hit albedo of the samples themselves, read
    // before the guide ray overwrites them: the native denoiser divides by and multiplies back the
    // albedo, so a pixel-centre albedo would snap an edge pixel's mixed colour to one surface.
    // diffuse_albedo.a is then the coverage: the share of samples whose first hit is the guide's
    // surface (below 1 on edges, where the denoiser's surface stops don't hold).
    bool sample_albedo=modes.w==0;
    float4 damean=sample_albedo && sampling.x>0?diffuse_albedo[p]:0;float3 samean=sample_albedo && sampling.x>0?specular_albedo[p].rgb:0;
    // Guides come from the pinhole ray through the lens center (RR and the denoiser).
    // Ray cones: one pixel's angle (orthographic: one pixel's width, no spread).
    float pixel_angle=camera_model.x==1?0:2*environment.z/dimensions.w,pixel_width=camera_model.x==1?2*camera_model.w/dimensions.w:0;
    cone_width=pixel_width;cone_spread=pixel_angle;
    Hit guide;
    if(recording){float3 go,gd;bool gvalid;camera_ray(float2(p)+0.5+jitter.xy,false,go,gd,gvalid);if(!intersect(go,gd,1e4,guide,false,true))guide.material=0xffffffffu;}
    else write_guides(p,guide);
    float3 batch=0;
    if(lpe_on){[unroll]for(uint k=0;k<4;k++)lpe_sum[k]=0;}
    for(uint sample_id=0;sample_id<(recording?path_record.z:sampling.y);sample_id++){
        if(recording)path_index=sample_id;
        // The base changes on every reset, so a moving camera never reuses the previous frame's points.
        sequence_index=scene_info.z+sampling.x+sample_id;dimension_index=0;
        // RR and the temporal upscalers need every sample of a frame at that frame's jitter.
        float2 subpixel=(scene_info.w&1)!=0?jitter.xy:filter_offset(float2(random(),random()));
        begin_wavelengths(sequence_index);
        float3 origin,direction;bool lens_valid;camera_ray(float2(p)+0.5+subpixel,true,origin,direction,lens_valid);
        cone_width=pixel_width;cone_spread=pixel_angle;
        float3 throughput=lens_valid?camera_weight:0,radiance=0,direct=0;float4 first_diffuse=0;float3 first_specular=0;uint portal_crossings=0,false_hits=0;uint64_t media=0;bool first_spec=false,previous_delta=true,previous_restir=false,camera_chain=true;float previous_pdf=0;float3 previous_position=origin;float3 absorption=0;float density=0;
        if(environment.w>0){media=camera_media();int top=medium_top(media);absorption=medium_absorption(media,top);density=medium_material(media,top).medium.w;}
        // Recording only (dead code otherwise): why the path ended, and its last bounce.
        uint end_reason=lens_valid?3:5,last_bounce=0;
        if(lpe_on){lpe_state=0;lpe_weight=1;}
        record(0,0,origin,throughput,0,direction);
        for(uint bounce=0;bounce<sampling.z && lens_valid;bounce++){
            last_bounce=bounce;
            bounces_used+=1;
            // Only delta bounces since the camera: the background it sees is ground-projected.
            camera_chain=camera_chain && previous_delta;
            uint mis=(scene_info.w>>4)&3;
            // Light selection: the HDRI and the emissive triangles share NEE half and half.
            bool env_nee=hdri_on() && mis!=2,tri_nee=scene_info.x>0 && mis!=2;
            float p_env=env_nee?(tri_nee || sdf_nee_on?0.5:1):0;
            // False hits on dielectrics (see medium_top): entering one ranked below the current
            // medium, or leaving one that is not the current medium. The ray goes on in the same
            // direction without a bounce, so the MIS state of the last real vertex stays. Kept in
            // this small loop rather than as a `continue` of the bounce loop: that cost glass
            // scenes about 0.5 ms more at 1440×900.
            Hit h;bool found;
            for(;;){
                found=intersect(origin,direction,1e4,h,false,bounce==0);
                if(!found || false_hits>=32 || materials[h.material].lobes.z<=0)break;
                int top=medium_top(media);uint e=medium_entry(h.material,materials[h.material].lobes.w);bool skip;
                if(h.front)skip=top>=0 && (e>>13)<(medium_at(media,top)>>13);
                else skip=medium_find(media,e)>=0 && medium_at(media,top)!=e;
                if(!skip)break;
                // The segment up to the false hit lies in the medium before it.
                throughput*=exp(-absorption*h.t);cone_width+=cone_spread*h.t;
                media=h.front?medium_push(media,e):medium_remove(media,e);{int top=medium_top(media);absorption=medium_absorption(media,top);density=top>=0?medium_material(media,top).medium.w:0;}
                false_hits++;origin=h.p-h.ng*0.0003;
                record(8,bounce,h.p,throughput,0,direction);
            }
#ifdef SER
            // Regroup the threads by what they hit before shading it (misses together), so a
            // warp runs one material's code; the image is unchanged, only the thread order.
// SER_BITS 0 (the default) leaves the call out: measured 2026-11-03, the ray generation shader
// alone is 12-13% faster than the compute kernel, and a reorder at every bounce 55-75% slower
// (the whole path state spills at each reorder point). SER_AT reorders at one bounce only.
#ifndef SER_BITS
#define SER_BITS 0
#endif
#ifndef SER_AT
#define SER_AT -1
#endif
#if SER_BITS>0
            if(SER_AT<0 || bounce==SER_AT)dx::MaybeReorderThread(found?min(h.material,(1u<<SER_BITS)-2):(1u<<SER_BITS)-1,SER_BITS);
#endif
#endif
            {float g,t0,t1;float3 sigma,albedo,fw;
            if((density>0 || (fog_on() && media==0)) && medium_params(media,origin,direction,found?h.t:1e4,sigma,albedo,g,t0,t1)){
                // Equiangular single scattering toward local lights in fog (opt in, flag bit 6:
                // Fog::equiangular; `equiangular_light`).
                bool equi=(scene_info.w&64u)!=0 && media==0 && tri_nee && mis!=2 && !fog_hetero();
                if(equi){float3 e=throughput*equiangular_light(media,origin,direction,t0,t1,sigma,albedo,absorption,g,env_nee,p_env,mis);
                    if(all(isfinite(e))){radiance+=e;if(bounce==0)direct+=e;lpe_add(4,5,e);}}
                float ts=media==0 && fog_hetero()?fog_track(origin,direction,t0,t1,sigma,fw):flight(sigma,t0,t1,fw);
                if(ts>=t1)throughput*=fw;
                else{
                    // A scatter before the surface: NEE by the phase function, then a new direction.
                    float3 arriving=throughput*exp(-absorption*ts)*fw;
                    throughput*=exp(-absorption*ts)*fw*up(albedo);cone_width+=cone_spread*ts;
                    float3 x=origin+direction*ts;
                    if(bounce==0 && sample_albedo)first_diffuse=float4(albedo,0);
                    float3 ld,le;float pl,limit;
                    if(sample_light(x,env_nee,tri_nee,p_env,ld,le,pl,limit)){
                        float pp=hg_pdf(dot(direction,ld),g);
                        // MIS against the equiangular sample of the same light.
                        float we=1;if(equi && nee_anchored){float pd=flight_pdf(sigma,t0,ts),ea,eb;float pe=cone_interval(origin,direction,nee_anchor,nee_axis,nee_cos,t0,t1,ea,eb) && ts>=ea && ts<=eb?equiangular_pdf(origin,direction,nee_anchor,ea,eb,ts):0;we=pd/max(pd+pe,1e-30);}
                        if(!occluded(x,ld,limit)){float3 e=throughput*medium_transmittance(media,x,ld,limit)*pp*le*we/nee_density(pl,pp,mis==1);radiance+=e;if(bounce==0)direct+=e;record(11,bounce,x,e,limit,ld);lpe_add(4,lpe_env_sample?6:5,e);}
                        else record(12,bounce,x,0,limit,ld);
                    }
                    float pp;direction=hg_sample(direction,g,pp);
                    record(5,bounce,x,arriving,0,direction);lpe_advance(4);
                    previous_pdf=pp;previous_delta=false;previous_restir=false;previous_position=x;origin=x;
                    if(bounce>=3){float survival=clamp(max(throughput.r,max(throughput.g,throughput.b)),0.05,0.95);if(random()>survival){end_reason=2;break;}throughput/=survival;}
                    continue;
                }
            }}
            if(!found){
                if(bounce==0)first_diffuse.a=guide.material==0xffffffffu?1:0;
                // The camera sees the background colour when the HDRI is hidden from it.
                bool hidden=bounce==0 && hdri_on() && ((uint)environment_map.z&2u)==0;
                float3 env=hidden?background.rgb:environment_radiance(camera_chain?background_dir(origin,direction):direction);
                // Stars: camera rays and pure-delta chains only (light sampling covers the rest).
                if(stars_on() && camera_chain && !hidden)env+=star_seen(direction);
                env=up(env);
                float weight=1;if(hdri_on() && bounce>0 && !previous_delta)weight=mis==1?0:mis==2?1:previous_pdf/(previous_pdf+p_env*env_keep()*env_pdf(direction));
                float3 e=throughput*env*weight;lpe_add(8,6,e);
                // Suns (they lead the emitter table): seen inside their disk, weighted against
                // light sampling as triangles are; a hidden one only from bounce 1 on.
                for(uint i=0;i<(uint)lights.x;i++){Emitter sun=emitters[i];
                    // Inside the disk by sin² (from a cross product), which keeps its precision.
                    float3 x=cross(direction,sun.position_area.xyz);float om=sun.edge1_cdf.x;
                    if(dot(direction,sun.position_area.xyz)<=0 || dot(x,x)>om*(2-om))continue;
                    if(bounce==0 && (((uint)materials[(uint)sun.emission_material.w].lobes.w&2u)!=0))continue;
                    float w=1;if(bounce>0 && !previous_delta)w=mis==1 || previous_restir?0:mis==2?1:previous_pdf/(previous_pdf+(1-p_env)*(1-sdf_share())*sun.edge2_probability.w/(2*PI*sun.edge1_cdf.x));
                    float3 se=throughput*up_emitted((uint)sun.emission_material.w,sun.emission_material.rgb)*w;e+=se;if(lpe_on)lpe_light_label=lpe_class((uint)sun.emission_material.w);lpe_add(8,5,se);}
                radiance+=e;if(bounce<=1)direct+=e;record(10,bounce,origin,throughput,luminance(e),direction);end_reason=0;break;}
            throughput*=exp(-absorption*h.t);cone_width+=cone_spread*h.t;
            float3 arriving=throughput;
            // Portal: continue from the matching point of the paired object, without a bounce.
            // Light sampling cannot see through portals, so emission found after one has weight 1.
            Material m=material_at(h.material,h.p);if(lpe_on)lpe_label=lpe_class(h.material);
            // Shadow catcher (Settings::shadow_catcher): seen from the camera, the floor is the
            // background behind it (ground-projected when that is on) times the share of four
            // light samples' power the scene leaves unblocked; any other ray sees the projected
            // ground, unshadowed. The path ends there either way.
            if(h.material==3 && env_ground.z>0){
                float3 seen=ground_on()?ground_dir(h.p):direction,bg;
                if(camera_chain){
                    bg=bounce==0 && hdri_on() && ((uint)environment_map.z&2u)==0?background.rgb:environment_radiance(seen);
                    bool ce=hdri_on(),ct=scene_info.x>0;float cp=ce?(ct || sdf_nee_on?0.5:1):0,all=0,lit=0;
                    for(uint k=0;k<4;k++){float3 ld,le;float pl,limit;
                        if(sample_light(h.p,ce,ct,cp,ld,le,pl,limit) && pl!=0){float c=dot(h.n,ld);
                            if(c>0){float w=luminance(le)*c/abs(pl);all+=w;if(!occluded(h.p+h.ng*0.0003,ld,limit))lit+=w;}}}
                    bg*=all>0?lit/all:1;
                }else bg=environment_radiance(seen);
                // The denoiser's guides: the background as the albedo, no specular.
                if(bounce==0 && sample_albedo){first_diffuse=float4(saturate(bg),h.material==guide.material?1:0);first_specular=0;}
                float3 e=throughput*up(bg);radiance+=e;if(bounce<=1)direct+=e;lpe_add(8,6,e);
                record(10,bounce,h.p,throughput,luminance(e),direction);end_reason=0;break;
            }
            if(((uint)m.lobes.w&4u)!=0 && portal_crossings<16){
                portal_crossings++;
                // The map lands on the far face of the pair; inside a portal object the ray just
                // passes through, so it leaves through the pair's near face (portals work both ways).
                if(!h.front){origin=h.p+direction*0.001;record(8,bounce,h.p,throughput,0,direction);bounce--;continue;}
                Material pm=materials[(uint)m.secondary.w];
                float4 P=float4(h.p,1),D=float4(direction,0);
                record(6,bounce,h.p,throughput,0,direction);
                direction=normalize(float3(dot(pm.color_rough,D),dot(pm.lobes,D),dot(pm.optical,D)));
                origin=float3(dot(pm.color_rough,P),dot(pm.lobes,P),dot(pm.optical,P))+direction*0.001;
                record(7,bounce,origin,throughput,0,direction);
                previous_delta=true;bounce--;continue;
            }
            // Beam splitter (track 17): a thin partial mirror, a delta event either way.
            if(((uint)m.lobes.w&4096u)!=0 && portal_crossings<64){
                // One choice per sheet: a back face (leaving a closed splitter) passes untinted,
                // or a thin box would split twice and reflect ~67% instead of 50%.
                portal_crossings++;previous_delta=true;
                if(!h.front){origin=h.p+direction*0.0005;record(8,bounce,h.p,throughput,0,direction);bounce--;continue;}
                throughput*=up_material(m,m.color_rough.rgb);
                if(random()<m.lobes.y){direction=reflect(direction,h.n);origin=h.p+direction*0.0005;record(3,bounce,h.p,throughput,0,direction);lpe_advance(2);}
                else{origin=h.p+direction*0.0005;record(8,bounce,h.p,throughput,0,direction);bounce--;}
                continue;
            }
            // Emission: the flat colour or the emissive map, as light sampling sees it.
            float3 emitted=up_emission(m,emitted_color(m,h.emitter,h.p))*m.optical.y;
            if(m.optical.w>0.5 && !h.front)emitted=0;
            // Signed against the emitter's own (unflipped) face normal, as light sampling takes it: a spot's
            // cone leaves the front face, so a two-sided spot seen from behind is outside it.
            emitted*=spot(m,-dot(direction,h.front?h.ng:-h.ng));
            apply_maps(h,m);
            // A dielectric's film sits between the media on both sides (the transmission branch).
            float film_n1=1,film_n2=0;
            if(m.surface_scale.z>0){
                if(m.lobes.z>0){uint e=medium_entry(h.material,m.lobes.w);uint64_t after=h.front?medium_push(media,e):medium_remove(media,e);film_n1=h.front?medium_ior(media,medium_top(media)):ior_of(m);film_n2=h.front?ior_of(m):medium_ior(after,medium_top(after));}
                prepare_film(m,saturate(dot(-direction,h.n)),film_n1,film_n2);
            }
            // Spectral: a grating's orders leave at the hero wavelength's angles only.
            if(has_grating(m)){if(spectral_on){hero(throughput);path_lambda=path_lambdas.x;}else grating_wavelength(pixel_seed^(sequence_index*0x9e3779b9u)^(bounce*0x85ebca6bu));}
            if(bounce==0 && sample_albedo){first_diffuse=float4(m.color_rough.rgb*(1-m.lobes.x)*(1-m.lobes.z),h.material==guide.material?1:0);first_specular=spec_fresnel(m,abs(dot(-direction,h.n)),f0_of(m));}
            // Spectral: the guides above keep RGB; the BSDF from here on sees spectra.
            if(spectral_on){m.color_rough.rgb=up_material(m,m.color_rough.rgb);m.sheen.rgb=up(m.sheen.rgb);prepare_film_waves(m,saturate(dot(-direction,h.n)),film_n1,film_n2);}
            // Emission found by BSDF sampling: MIS weight, or all / nothing in the single-strategy modes.
            float weight=1;if(h.emitter!=0 && bounce>0 && !previous_delta)weight=mis==1 || (previous_restir && restir_covers(previous_position,emitters[h.emitter-1]))?0:mis==2?1:previous_pdf/(previous_pdf+(1-p_env)*(previous_restir?1:1-sdf_share())*emitter_pdf(previous_position,h.p,emitters[h.emitter-1]));
            // An emissive procedural object, against its light sample (`sdf_nee_on`).
            else if(h.emitter==0 && sdf_nee_on && bounce>0 && !previous_delta && !previous_restir && m.optical.y>0){
                uint k=sdf_record_of(h.material);
                if(k!=0xffffffffu)weight=mis==1?0:previous_pdf/(previous_pdf+(1-p_env)*sdf_share()*sdf_record_prob(k)*sdf_dir_pdf(previous_position,k));
            }
            float3 e=throughput*emitted*weight;radiance+=e;if(bounce<=1)direct+=e;
            float emitted_l=luminance(e);if(lpe_on)lpe_light_label=lpe_label;lpe_add(8,5,e);
            if(((uint)m.lobes.w&1u)!=0){record(9,bounce,h.p,arriving,emitted_l,0);end_reason=1;break;}
            // Light sampling (sample_light), then one shared BSDF evaluation: eval_bsdf is inlined
            // once here instead of once per strategy (code size and registers cost even scenes
            // that never take the anisotropic branch).
            // ReSTIR DI (track 15): at the first hit the emitter table is sampled through the
            // pixel's reservoir of this sample's layer with weight 1, so BSDF hits on emitters from this vertex count 0;
            // the HDRI keeps its own strategy, chosen with the same probability, and its MIS.
            bool restir_vertex=bounce==0 && restir_on() && sample_id<restir_layers() && tri_nee,reused=false;
            float3 ld,le;float pl,limit;bool nee;
            if(restir_vertex && !(env_nee && random()<p_env)){reused=true;nee=restir_nee(p,sample_id,h,-direction,ld,le,limit);le/=1-p_env;pl=1;if(lpe_on)lpe_env_sample=false;}
            else if(restir_vertex){nee=sample_light(h.p,true,false,1,ld,le,pl,limit);pl*=p_env;}
            else nee=sample_light(h.p,env_nee,tri_nee,p_env,ld,le,pl,limit);
            float nl=dot(h.n,ld);
            if(nee && nl>0){
                float pdf;float3 f=eval_bsdf(m,h.n,h.tangent,-direction,ld,pdf);
                if(!occluded(h.p+h.ng*0.0003,ld,limit)){float3 e=throughput*f*nl*le/(reused?1:nee_density(pl,pdf,mis==1));
                    if(fog_on() || density>0)e*=medium_transmittance(media,h.p+h.ng*0.0003,ld,limit);
                    radiance+=e;if(bounce==0)direct+=e;record(11,bounce,h.p,e,limit,ld);
                    if(lpe_on){float3 ed=e*lpe_diffuse/max(f,1e-30);uint light=lpe_env_sample?6:5;lpe_add(0,light,ed);lpe_add(lpe_glossy(m),light,e-ed);}}
                else record(12,bounce,h.p,0,limit,ld);
            }
            float3 next;bool delta=false;float bsdf_pdf=0;uint lobe=1;
            if(random()<m.lobes.z){
                // The media on both sides: entering, the current one and this; leaving, this and
                // the one the stack falls back to (air when empty, as before nesting).
                // Spectral dispersion: from here the path follows the hero wavelength's index.
                if(dispersive(m))hero(throughput);
                uint e=medium_entry(h.material,m.lobes.w);uint64_t after=h.front?medium_push(media,e):medium_remove(media,e);
                float n1=h.front?medium_ior(media,medium_top(media)):ior_of(m),n2=h.front?ior_of(m):medium_ior(after,medium_top(after));
                float eta=n1/n2,ci=saturate(dot(-direction,h.n));float fr=fresnel_dielectric(ci,eta);
                // A thin film (a soap bubble: IOR 1 inside a 1.33 film) colours the reflection and,
                // by what it leaves, the transmission: choose by the mean, weight per channel.
                float3 frc=fr.xxx;
                if(m.surface_scale.z>0 && fr<1){frc=lerp(frc,m.base_scale.rgb,m.surface_scale.z);fr=clamp((frc.r+frc.g+frc.b)/3,1e-4,1-1e-4);}
                bool reflected=random()<fr;lobe=reflected?3:4;lpe_advance(reflected?2:3);
                next=reflected?reflect(direction,h.n):refract(direction,h.n,eta);
                if(m.surface_scale.z>0)throughput*=reflected?frc/fr:(1-frc)/(1-fr);
                if(!reflected){media=after;int top=medium_top(media);absorption=medium_absorption(media,top);density=top>=0?medium_material(media,top).medium.w:0;throughput*=eta*eta;}
                if(m.color_rough.a>0.001){float3 pert=cosine_sample(reflected?h.n:-h.n);next=normalize(lerp(next,pert,m.color_rough.a*m.color_rough.a));}
                delta=true;
            }else{
                float pc=coat_probability(m),ps=spec_probability(m);
                if(pc>0 && random()<pc){float a=max(0.001,m.layers.y*m.layers.y);next=ggx_sample(h.n,direction,a);if(bounce==0)first_spec=true;cone_spread+=0.5*a;lobe=2;}
                else if(random()<ps){float a=max(0.001,m.color_rough.a*m.color_rough.a);lobe=2;
                    // A grating's order (order_shares): the isotropic GGX sample moved
                    // across the grooves; eval_bsdf has its pdf.
                    int o=-1;float3 T,B;
                    if(has_grating(m)){aniso_frame(h.n,h.tangent,m.sheen.w,T,B);float4 w=order_shares(m,h.n,B,-direction);float u=random(),c=0;[unroll]for(int i=0;i<4;i++){if(o<0 && u<c+w[i])o=i;c+=w[i];}}
                    if(o>=0){next=ggx_sample(h.n,direction,a);if(dot(h.n,next)<=0 || !grating_move(h.n,B,next,GRATING_ORDER[o]*grating_shift(m),next)){record(9,bounce,h.p,arriving,emitted_l,0);end_reason=4;break;}}
                    else if(m.layers.w>0){float3 T,B;aniso_frame(h.n,h.tangent,m.sheen.w,T,B);next=ggx_sample_aniso(h.n,T,B,direction,aniso_alpha(m));}else next=ggx_sample(h.n,direction,a);
                    if(bounce==0)first_spec=true;
                    cone_spread+=0.5*a;
                }else{next=cosine_sample(h.n);cone_spread+=0.5;}
                if(dot(h.n,next)<=0){record(9,bounce,h.p,arriving,emitted_l,0);end_reason=4;break;}
                float3 bsdf=eval_bsdf(m,h.n,h.tangent,-direction,next,bsdf_pdf);throughput*=bsdf*abs(dot(h.n,next))/max(bsdf_pdf,1e-7);
                if(lpe_on){
                    // One label for the rest of the path, by the luminance shares at `next`.
                    float3 share=saturate(lpe_diffuse/max(bsdf,1e-30));float pd=luminance(bsdf)>0?saturate(luminance(lpe_diffuse)/luminance(bsdf)):0;
                    float u=(hash(pixel_seed^(sequence_index*0x9e3779b9u)^(bounce*0x85ebca6bu)^0x5bd1e995u)>>8)*(1.0/16777216.0);
                    if(u<pd){lpe_weight*=share/pd;lpe_advance(0);}else{lpe_weight*=(1-share)/(1-pd);lpe_advance(lpe_glossy(m));}
                }
            }
            if(bounce==0 && delta)first_spec=true;
            previous_pdf=bsdf_pdf;previous_delta=delta;previous_restir=restir_vertex;previous_position=h.p;
            direction=normalize(next);origin=h.p+h.ng*(dot(direction,h.ng)>0?0.0003:-0.0003);
            record(lobe,bounce,h.p,arriving,emitted_l,direction);
            if(bounce>=3){float survival=clamp(max(throughput.r,max(throughput.g,throughput.b)),0.05,0.95);if(random()>survival){end_reason=2;break;}throughput/=survival;}
        }
        radiance=to_rgb(radiance);direct=to_rgb(direct);
        if(!all(isfinite(radiance))){radiance=0;direct=0;}
        // Spectral samples keep their negative components: one wavelength lies outside the sRGB
        // gamut (500 nm has negative red), and clamping each sample added red and green (+35% red
        // on Classic with hero-only paths). The mean stays unbiased; only `noisy` is clamped.
        if(!spectral_on)radiance=max(radiance,0);if(options.x>0 && sampling.x>4){float l=luminance(radiance);float scale=min(1,options.x*max(luminance(mean),0.05)/max(l,1e-6));radiance*=scale;direct*=scale;}
        record(13,last_bounce,0,radiance,end_reason,0);
        float count=sampling.x+sample_id+1;float delta_l=luminance(radiance)-luminance(mean);mean+=(radiance-mean)/count;m2+=delta_l*(luminance(radiance)-luminance(mean));
        dmean+=(direct-dmean)/count;smean+=((first_spec?radiance:0)-smean)/count;batch+=radiance/sampling.y;
        damean+=(first_diffuse-damean)/count;samean+=(first_specular-samean)/count;
    }
    #ifdef PATH_RECORD
    PathVertex header=(PathVertex)0;header.info=path_count;path_vertices[0]=header;return;
    #endif
    if(lpe_on){uint base=(p.y*dimensions.x+p.x)*lpe.x;float n=sampling.x+sampling.y;
        [unroll]for(uint k=0;k<4;k++)if(k<lpe.x){float3 old=sampling.x>0?lpe_aov[base+k].rgb:0;lpe_aov[base+k]=float4((old*sampling.x+lpe_sum[k])/n,1);}}
    if(sample_albedo){diffuse_albedo[p]=damean;specular_albedo[p]=float4(samean,1);}
    trace_stats[p]=float2(bounces_used/max(1,sampling.y),proceed_steps);
    noisy[p]=float4(spectral_on?max(batch,0):batch,1);accumulation[p]=float4(mean,max(0,m2));direct_light[p]=float4(dmean,1);specular_light[p]=float4(smean,1);
}

// ReSTIR DI, pass 0: the frame's light tiles, one sample per thread.
[numthreads(64,1,1)]void restir_presample(uint3 tid:SV_DispatchThreadID){
    uint i=tid.x;if(i>=RESTIR_TILES*RESTIR_TILE || scene_info.x==0)return;
    white_noise=true;rng_state=hash(i^hash(sampling.w*0x85ebca6bu+0x165667b1u));
    uint light=alias_index(random(),random()),q=pack_uv(float2(random(),random()));LightPoint l=light_point(light,q);
    Reservoir r;r.light=light|l.kind<<30;r.uv=q;r.W=l.source;r.m_material=asuint(l.lum);r.position=l.p;r.normal=pack_normal(l.n);
    reservoirs[restir_slot(2,0)+i]=r;
}
// ReSTIR DI, pass 1 (before trace; z = layer): the guide ray's hit, k fresh candidates, then the
// temporal merge with last frame's final reservoir of the same layer at the reprojected pixel.
[numthreads(8,8,1)]void restir_initial(uint3 tid:SV_DispatchThreadID){
    uint2 p=tid.xy+region.xy;uint layer=tid.z;if(any(p>=min(dimensions.xy,region.zw)))return;
    white_noise=true;pixel_seed=hash(p.x+p.y*dimensions.x);rng_state=hash(pixel_seed^hash(sampling.w*747796405+layer*0x9e3779b9u+0x2545f491u));
    cone_width=camera_model.x==1?2*camera_model.w/dimensions.w:0;cone_spread=camera_model.x==1?0:2*environment.z/dimensions.w;
    Reservoir r;r.light=0;r.uv=0;r.W=0;r.m_material=0xffffffu;r.position=0;r.normal=0;
    float3 o,d;bool valid;camera_ray(float2(p)+0.5+jitter.xy,false,o,d,valid);Hit h;
    if(scene_info.x>0 && intersect(o,d,1e4,h,false,true) && ((uint)materials[h.material].lobes.w&1u)==0){
        Domain a=make_domain(h.p,h.n,-d,h.material);
        uint tile=hash((p.x>>3)|(p.y>>3)<<16^hash(sampling.w*0x9e3779b9u+layer))%RESTIR_TILES;
        uint light,q;float W,M=1;restir_candidates(a,restir.y,tile,light,q,W);
        if(!lean && (restir.x&2)!=0 && restir.w>0){
            float4 prev=mul(previous_vp,float4(h.p,1));float2 uv=prev.xy/prev.w*float2(0.5,-0.5)+0.5;int2 hp=int2(floor(uv*dimensions.xy));
            if(prev.w>0 && all(hp>=int2(region.xy)) && all(hp<int2(min(dimensions.xy,region.zw)))){
                Reservoir hr=reservoirs[restir_slot(0,layer)+hp.y*dimensions.x+hp.x];
                // Only histories on a similar surface: dissimilar ones would mostly be wasted.
                if(reservoir_valid(hr)){Domain b=domain_of(hr);
                    if(dot(b.n,a.n)>0.8 && length(b.p-a.p)<0.05*length(a.p-eye.xyz)){
                        float Mh=min(reservoir_m(hr),(float)restir.w);
                        restir_merge(a,light,q,W,1,b,hr.light,hr.uv,hr.W,Mh,light,q,W);M+=Mh;}}
            }
        }
        r.light=light;r.uv=q;r.W=W;r.m_material=h.material|(uint)min(M,255)<<24;r.position=h.p;r.normal=pack_normal(h.n);
    }
    reservoirs[restir_slot(1,layer)+p.y*dimensions.x+p.x]=r;
}
// ReSTIR DI, pass 2: merge up to 8 random neighbours on similar surfaces with pairwise MIS (each
// neighbour against this pixel: 3k + 1 target evaluations instead of (k + 1)^2). With
// a_i(y) = M_i p_i(y) / (M_i p_i(y) + M_c/k p_c(y)) (p: the targets), neighbour i's sample has
// weight a_i / k and the pixel's own sample the sum of (1 - a_i) / k; for any sample they sum
// to 1, so the result is unbiased. It is the final reservoir that trace reads and next frame's
// history.
[numthreads(8,8,1)]void restir_spatial(uint3 tid:SV_DispatchThreadID){
    uint2 p=tid.xy+region.xy;uint layer=tid.z;if(any(p>=min(dimensions.xy,region.zw)))return;
    uint self=p.y*dimensions.x+p.x;Reservoir c=reservoirs[restir_slot(1,layer)+self];
    if(!reservoir_valid(c) || restir.z==0){reservoirs[restir_slot(0,layer)+self]=c;return;}
    white_noise=true;pixel_seed=hash(p.x+p.y*dimensions.x);rng_state=hash(pixel_seed^hash(sampling.w*2891336453u+layer*0x85ebca6bu+0x68bc21ebu));
    Domain a=domain_of(c);float radius=max(4.0,30.0*dimensions.y/1080),reach=0.05*length(a.p-eye.xyz);
    uint index[8];uint k=0;
    for(uint t=0;t<min(restir.z,8u);t++){
        float rr=radius*sqrt(random()),ang=2*PI*random();int2 q=int2(p)+int2(round(rr*float2(cos(ang),sin(ang))));
        if(any(q<int2(region.xy)) || any(q>=int2(min(dimensions.xy,region.zw))) || all(q==int2(p)))continue;
        uint j=restir_slot(1,layer)+q.y*dimensions.x+q.x;Reservoir r=reservoirs[j];
        if(!reservoir_valid(r))continue;
        Domain b=domain_of(r);if(dot(b.n,a.n)<0.9 || abs(dot(a.n,b.p-a.p))>reach)continue;
        index[k++]=j;
    }
    float Mc=reservoir_m(c),M=Mc,sum=0,chosen=0,own=0,cc=0;uint light=c.light,q=c.uv;
    LightPoint lc=light_point(c.light,c.uv);if(c.W>0)cc=restir_target(a,lc);
    for(uint i=0;i<k;i++){
        Reservoir r=reservoirs[index[i]];Domain b=domain_of(r);float Mi=reservoir_m(r);M+=Mi;
        // The pixel's own sample against this neighbour.
        if(cc>0){float ti=restir_target(b,lc);own+=1-Mi*ti/(Mi*ti+Mc/k*cc);}
        LightPoint l=light_point(r.light,r.uv);float ti=restir_target(b,l),tc=restir_target(a,l);
        float w=ti>0 && tc>0?Mi*ti/(Mi*ti+Mc/k*tc)/k*tc*r.W:0;sum+=w;
        if(w>0 && random()*sum<w){light=r.light;q=r.uv;chosen=tc;}
    }
    float w=cc>0?(k>0?own/k:1)*cc*c.W:0;sum+=w;
    if(w>0 && random()*sum<w){light=c.light;q=c.uv;chosen=cc;}
    c.light=light;c.uv=q;c.W=chosen>0?sum/chosen:0;c.m_material=(c.m_material&0xffffffu)|(uint)min(M,255)<<24;
    reservoirs[restir_slot(0,layer)+self]=c;
}
