#include "common.hlsli"
#include "layers.hlsli"
uint hash(uint x){x^=x>>16;x*=0x7feb352d;x^=x>>15;x*=0x846ca68b;x^=x>>16;return x;}
static uint rng_state,sequence_index,dimension_index,pixel_seed;
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
static uint proceed_steps;
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
uint candidate_material(uint instance,uint prim){Instance inst=instances[instance];return inst.material!=0xffffffffu?inst.material:static_vertices[prim*3].material;}
float4 sample_map(uint slot,float2 uv){return maps[NonUniformResourceIndex(slot-1)].SampleLevel(map_sampler,uv,0);}
// Ray cones pick the mip level (core track 5): the width of the cone at the current ray's origin
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
// maps, 16 any texture maps, 4096 beam splitter. Without TEXTURE_MAPS (build.rs's `trace` variant, for scenes with
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
bool fog_on(){return !lean && fog.x>0;}
bool restir_on(){return !lean && (restir.x&1)!=0;}
// The gates only (coat, anisotropy, sheen, film strength, extinction): every use of the other
// layer fields sits behind one of them, and sheen.w also turns gratings (kept).
void lean_strip(inout Material m){if(lean){m.layers.x=0;m.layers.w=0;m.sheen.rgb=0;m.surface_scale.z=0;m.medium.w=0;}}
bool has_grating(Material m){return texture_maps && m.optical.w<0;}
bool flagged(uint m,uint bit){return texture_maps && ((uint)materials[m].lobes.w&bit)!=0;}
// Masked texture maps: a candidate whose base-colour alpha is below the cutoff is a hole.
bool alpha_hole(uint instance,uint prim,float2 b,uint m){
    uint4 mp=materials[m].maps;
    uint idx=instances[instance].vertex_base+prim*3;
    float2 uv=vertices[idx].uv*(1-b.x-b.y)+vertices[idx+1].uv*b.x+vertices[idx+2].uv*b.y;
    return sample_map(mp.x,uv).a<asfloat(mp.w);
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
float sdf_mandelbulb(float3 p){
    // Quilez's trigonometry-free power 8 (the same bulb, about 3× cheaper).
    p*=1.15;float3 w=p;float m=dot(w,w),dz=1;
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
float sdf_menger(float3 p){
    float d=sdf_box(p,1),s=1;
    for(int m=0;m<5;m++){float3 a=p*s;a=a-2*floor(a*0.5)-1;s*=3;float3 r=abs(1-3*abs(a));
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
float sdf(uint raw,float3 p){
    uint kind=raw&255;
    if(kind==5)return sdf_metaballs(p,(raw>>8)*0.001);
    if(kind==2)return length(float2(length(p.xz)-0.72,p.y))-0.28;
    if(kind==3)return sdf_mandelbulb(p);
    if(kind==4)return sdf_menger(p);
    return length(p)-1;
}
float3 sdf_normal(uint raw,float3 p){
    uint kind=raw&255;
    if(kind==1)return p;
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
bool intersect(float3 o,float3 d,float limit,out Hit h,bool shadow,bool camera_ray){
    RayDesc ray;ray.Origin=o;ray.Direction=d;ray.TMin=0.0001;ray.TMax=limit;
    // Nonopaque geometry allows the light quad and optional glass to be ignored for shadow rays.
    uint mask=0xffu&~(camera_ray?MASK_HIDDEN:0u)&~(shadow && jitter.w<=0.5?MASK_CLEAR_GLASS:0u);
    RayQuery<RQ_BASE> q;q.TraceRayInline(scene,RAY_FLAG_NONE,mask,ray);
    while(q.Proceed()){proceed_steps++;
        if(!lean && q.CandidateType()==CANDIDATE_PROCEDURAL_PRIMITIVE){
            Instance pi=instances[q.CandidateInstanceID()];uint pm=pi.material;float t;
            if(camera_ray && (((uint)materials[pm].lobes.w&2u)!=0))continue;
            if(shadow && jitter.w<=0.5 && materials[pm].lobes.z>=0.5)continue;
            if(sdf_hit(pi.vertex_base,q.CandidateObjectRayOrigin(),q.CandidateObjectRayDirection(),q.RayTMin(),q.CommittedRayT(),t))q.CommitProceduralPrimitiveHit(t);
            continue;
        }
        uint m=candidate_material(q.CandidateInstanceID(),q.CandidatePrimitiveIndex());
        if(flagged(m,8u) && alpha_hole(q.CandidateInstanceID(),q.CandidatePrimitiveIndex(),q.CandidateTriangleBarycentrics(),m))continue;
        if(camera_ray && (((uint)materials[m].lobes.w&2u)!=0))continue;
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
    h.t=q.CommittedRayT();h.p=o+d*h.t;float2 uv=0;float4 tangent=0;bool mapped;float uv_scale=0;float3 T=0,B=0;
    if(inst.material==0xffffffffu){
        // Static geometry: geometry 1 is the opaque range; its primitive indices restart at 0.
        prim+=q.CommittedGeometryIndex()!=0?(uint)environment.w:0;uint idx=prim*3;
        Vertex a=static_vertices[idx],v=static_vertices[idx+1],c=static_vertices[idx+2];
        h.n=normalize(a.normal*(1-b.x-b.y)+v.normal*b.x+c.normal*b.y);h.material=a.material;
        mapped=flagged(h.material,16u);
        if(mapped){uv_scale=materials[h.material].base_scale.w;float3 t,bt;box_map(h.p,h.n,uv_scale,uv,t,bt);tangent=float4(t,tangent_sign(h.n,t,bt));T=t/uv_scale;B=bt/uv_scale;}
    }else{
        // An object: object-space normal to world by the inverse transpose (row vector × W2O).
        uint idx=inst.vertex_base+prim*3;Vertex a=vertices[idx],v=vertices[idx+1],c=vertices[idx+2];
        float3 n=a.normal*(1-b.x-b.y)+v.normal*b.x+c.normal*b.y;
        h.n=normalize(mul(n,(float3x3)q.CommittedWorldToObject3x4()));h.material=inst.material;
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
        }
    }
    h.front=dot(d,h.n)<0;if(!h.front)h.n=-h.n;h.ng=h.n;
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
    uint mask=glass_blocks || jitter.w>0.5?0xffu:0xffu&~MASK_CLEAR_GLASS;
    RayQuery<RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH|RQ_BASE> q;q.TraceRayInline(scene,RAY_FLAG_NONE,mask,ray);
    while(q.Proceed()){proceed_steps++;
        if(!lean && q.CandidateType()==CANDIDATE_PROCEDURAL_PRIMITIVE){
            Instance pi=instances[q.CandidateInstanceID()];float t;
            if((glass_blocks || jitter.w>0.5 || materials[pi.material].lobes.z<0.5) && sdf_hit(pi.vertex_base,q.CandidateObjectRayOrigin(),q.CandidateObjectRayDirection(),q.RayTMin(),q.CommittedRayT(),t))q.CommitProceduralPrimitiveHit(t);
            continue;
        }
        uint m=candidate_material(q.CandidateInstanceID(),q.CandidatePrimitiveIndex());
        if(flagged(m,8u) && alpha_hole(q.CandidateInstanceID(),q.CandidatePrimitiveIndex(),q.CandidateTriangleBarycentrics(),m))continue;
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
float env_pdf(float3 d){
    if(!hdri_on())return 0;float s,r;uint2 t=env_texel(d,s,r);
    return luminance(env_map[t].rgb)*environment_map.w*r/max(s,1e-6);
}
// First index whose CDF value exceeds u, among `count` entries of row `row`. The row's
// cutpoints (environment::build, rows h + 1 on; K a power of two, so u·K is exact) bracket the
// answer, and the search between them returns exactly the full binary search's index.
uint cdf_search(uint row,uint count,float u){
    uint2 size=env_size();uint K=1u<<firstbithigh(size.x),k=(uint)(u*K),g=size.y+1+row;
    uint lo=(uint)env_cdf[uint2(k,g)],hi=k+1<K?(uint)env_cdf[uint2(k+1,g)]:count-1;
    while(lo<hi){uint mid=(lo+hi)/2;if(u<env_cdf[uint2(mid,row)])hi=mid;else lo=mid+1;}return lo;}
// A direction from the HDRI's luminance · sinθ, uniform within the chosen texel.
float3 sample_env(out float pdf){
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
float3 environment_radiance(float3 d){
    if(hdri_on())return env_bilinear(d)*environment_map.x;
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
uint64_t medium_push(uint64_t s,uint e){[unroll]for(uint i=0;i<4;i++)if(medium_at(s,i)==0)return s|((uint64_t)e<<(16*i));return s;}
uint64_t medium_remove(uint64_t s,uint e){int j=medium_find(s,e);if(j<0)return s;uint64_t low=j>0?s&((1ull<<(16*j))-1):0;return low|((s>>(16*(j+1)))<<(16*j));}
Material medium_material(uint64_t s,int top){Material m=materials[(medium_at(s,top)&0x1fffu)-1];lean_strip(m);return m;}
float medium_ior(uint64_t s,int top){return top>=0?medium_material(s,top).optical.x:1;}
float3 medium_absorption(uint64_t s,int top){return top>=0?-log(max(medium_material(s,top).color_rough.rgb,0.001)):float3(0,0,0);}
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
float3 film_fresnel(float c1,float eta2,float3 eta3,float thickness){
    // Below ~30 nm the film fades out (its IOR towards the outside's), leaving the base's Fresnel.
    eta2=lerp(1,eta2,smoothstep(0,30,thickness));
    float s2=(1-c1*c1)/(eta2*eta2);if(s2>=1)return 1;float c2=sqrt(1-s2);
    float r12=schlick(pow((eta2-1)/(eta2+1),2),c1),t121=1-r12;
    float phi21=eta2<1?0:PI;
    float3 r23=float3(schlick(pow((eta3.x-eta2)/(eta3.x+eta2),2),c2),schlick(pow((eta3.y-eta2)/(eta3.y+eta2),2),c2),schlick(pow((eta3.z-eta2)/(eta3.z+eta2),2),c2));
    float3 phi=phi21+select(eta3<eta2,PI.xxx,0.0.xxx);
    float opd=2*eta2*thickness*c2;
    float3 r123=clamp(r12*r23,1e-5,0.9999),rs=t121*t121*r23/(1-r123),i=r12+rs,cm=rs-t121,sq=sqrt(r123);
    [unroll]for(int k=1;k<=2;k++){cm*=sq;i+=cm*2*film_sensitivity(k*opd,k*phi);}
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
    m.base_scale.rgb=film_fresnel(c,film_ior(m)/outside,eta3,film_thickness(m));
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
uint lpe_step(uint k,uint state,uint event){return lpe_table[(k*32+state)*8+event];}
void lpe_advance(uint event){
    if(!lpe_on)return;uint s=0;
    [unroll]for(uint k=0;k<4;k++){uint st=(lpe_state>>(8*k))&255;s|=(k<lpe.x && st!=255?lpe_step(k,st,event)&255:255)<<(8*k);}
    lpe_state=s;
}
// A contribution e reaching the camera through the path so far, then `event` (8: none), then
// the light event `light` (5 or 6).
void lpe_add(uint event,uint light,float3 e){
    if(!lpe_on || !all(isfinite(e)))return;
    [unroll]for(uint k=0;k<4;k++){
        uint st=(lpe_state>>(8*k))&255;if(k>=lpe.x || st==255)continue;
        if(event!=8){st=lpe_step(k,st,event)&255;if(st==255)continue;}
        if((lpe_step(k,st,light)>>8)!=0)lpe_sum[k]+=e*lpe_weight;
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
static float path_lambda;
static const float3 GRATING_MU=float3(600,545,455),GRATING_SIGMA=float3(40,35,25);
void grating_wavelength(uint key){
    uint a=hash(key),b=hash(a^0x68e31da4);uint c=a%3;float u1=max(((a>>8)&0xffff)*(1.0/65536.0),1e-6),u2=(b>>8)*(1.0/16777216.0);
    path_lambda=GRATING_MU[c]+GRATING_SIGMA[c]*sqrt(-2*log(u1+(1.0/131072.0)))*cos(2*PI*u2);
    path_lambda=clamp(path_lambda,300,900);
}
// The RGB weight of the hit's wavelength, recomputed where needed: kept from the hit, it (like the
// groove axis and the order shares, also recomputed) stayed live across the whole bounce, and
// those registers cost scenes without a grating 4–12%.
float3 grating_weight(){float3 x=(path_lambda-GRATING_MU)/GRATING_SIGMA,nc=exp(-0.5*x*x)/GRATING_SIGMA;return 3*nc/max(nc.r+nc.g+nc.b,1e-30);}
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
float spot(Material m,float c){return ((uint)m.lobes.w&32u)!=0?smoothstep(m.secondary.x,m.secondary.y,c):1;}
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
        float before=i>0?lens_data[3+2*(i-1)][c]:1;
        float3 w=refract(d,dot(nrm,d)>0?-nrm:nrm,lens_data[3+2*i][c]/before);if(dot(w,w)==0)return false;d=w;
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
        // Dispersion: each sample traces one channel (weight 3 there) through that channel's indices.
        bool spectral=lens_sample && real_lens.y>0;uint c=spectral?min((uint)(random()*3),2u):1;
        float3 lo,ld;
        if(!lens_ray(ndc,lens_sample,c,lo,ld)){
            // A vignetted sample is invalid; the guide falls back to the pinhole ray.
            d=camera_direction(pixel);if(lens_sample)valid=false;camera_weight=1;return;
        }
        o=eye.xyz+(right*lo.x+up*lo.y+forward*(lens_data[1].z-lo.z))*real_lens.z;
        d=normalize(right*ld.x+up*ld.y-forward*ld.z);
        if(spectral){float3 m=0;m[c]=3;camera_weight*=m;}
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
bool sample_light(float3 x,bool env_nee,bool tri_nee,float p_env,out float3 ld,out float3 le,out float pl,out float limit){
    bool nee=false;ld=float3(0,1,0);le=0;pl=0;limit=1e4;if(lpe_on)lpe_env_sample=false;
    if(env_nee && random()<p_env){
        float pe;ld=sample_env(pe);if(lpe_on)lpe_env_sample=true;
        if(pe>0){le=environment_radiance(ld);pl=p_env*pe;nee=true;}
    }else if(tri_nee){
    Emitter light=emitters[select_index(random())];
    if(light.position_area.w<0){
        // Sphere or sun: a direction in the cone it covers, then a shadow ray up to the
        // sphere's true surface (its inscribed mesh lies beyond) or to infinity.
        float om=0,pc=0;
        if(light.position_area.w==-1){
            float3 c=light.position_area.xyz-x;float d2=dot(c,c),r=light.edge1_cdf.x;om=sphere_om(d2,r);
            if(om>0){ld=cone_sample(c*rsqrt(d2),om,pc);float b=dot(c,ld);limit=b-sqrt(max(0,r*r-(d2-b*b)));}
        }else{om=light.edge1_cdf.x;ld=cone_sample(light.position_area.xyz,om,pc);}
        if(pc>0){le=light.emission_material.rgb;pl=(1-p_env)*light.edge2_probability.w*pc;limit=max(0.0002,limit-0.0006);nee=true;}
    }else{
    float su=sqrt(random()),v=random();float3 lp=light.position_area.xyz+light.edge1_cdf.xyz*(1-su)+light.edge2_probability.xyz*(su*v);ld=lp-x;float len=length(ld);ld/=max(len,1e-8);
    Material lm=material_at((uint)light.emission_material.w,lp);float3 ln=normalize(cross(light.edge1_cdf.xyz,light.edge2_probability.xyz));
    if(lm.optical.w<0.5 || dot(ln,-ld)>0){le=lm.color_rough.rgb*lm.optical.y*spot(lm,dot(ln,-ld));pl=(1-p_env)*emitter_pdf(x,lp,light);limit=nee_limit(len,dot(ln,ld));nee=true;}
    }}
    return nee;
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
    return lm.optical.w>=0.5 && c<=0?0:lm.color_rough.rgb*lm.optical.y*spot(lm,c);
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
    LightPoint l=light_point(light,q);float g;le=light_toward(light,l,h.p,ld,g,limit)*g*W;
    return W>0 && any(le>0);
}
// Volumes (track 12): homogeneous media, sampled by free flight on their extinction (one grey
// sigma, so the pdf cancels the transmittance), with the albedo applied at each scatter and a
// Henyey-Greenstein phase function. c is the cosine between the travel and the new direction.
float hg_pdf(float c,float g){float d=max(1+g*g-2*g*c,1e-6);return (1-g*g)/(4*PI*d*sqrt(d));}
float3 hg_sample(float3 d,float g,out float pdf){
    float u=random(),c;
    if(abs(g)<1e-3)c=1-2*u;else{float s=(1-g*g)/(1-g+2*g*u);c=(1+g*g-s*s)/(2*g);}
    c=clamp(c,-1,1);float sn=sqrt(max(0,1-c*c)),a=2*PI*random();pdf=hg_pdf(c,g);
    return basis(d,float3(sn*cos(a),sn*sin(a),c));
}
// Where the fog box (floor to fog.z, ±fog.w on x and z) covers a ray, clipped to [0, tmax].
bool fog_interval(float3 o,float3 d,float tmax,out float t0,out float t1){
    float3 inv=1/select(abs(d)>1e-9,d,1e-9),a=(float3(-fog.w,-1,-fog.w)-o)*inv,b=(float3(fog.w,fog.z,fog.w)-o)*inv;
    float3 lo=min(a,b),hi=max(a,b);t0=max(0,max(lo.x,max(lo.y,lo.z)));t1=min(tmax,min(hi.x,min(hi.y,hi.z)));
    return fog_on() && t1>t0;
}
// The scattering medium the path is in: the top of the dielectric stack, else the fog (only
// outside every object). [t0, t1] is where it fills the ray up to tmax.
bool medium_params(uint64_t media,float3 o,float3 d,float tmax,out float sigma,out float3 albedo,out float g,out float t0,out float t1){
    int top=medium_top(media);t0=0;t1=tmax;
    if(top>=0){Material m=medium_material(media,top);sigma=m.medium.w;albedo=m.medium.rgb;g=m.phase.x;return sigma>0;}
    sigma=fog.x;albedo=fog_color.rgb;g=fog.y;return fog_interval(o,d,tmax,t0,t1);
}
// The distance to the nearest surface (alpha-tested holes skipped), or limit: a medium's exit.
float surface_distance(float3 o,float3 d,float limit){
    RayDesc ray;ray.Origin=o;ray.Direction=d;ray.TMin=0.0001;ray.TMax=limit;
    RayQuery<RQ_BASE> q;q.TraceRayInline(scene,RAY_FLAG_NONE,255,ray);
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
        if(m.medium.w>0){start=surface_distance(o,d,limit);tr=exp(-(m.medium.w-log(max(m.color_rough.rgb,0.001)))*start);}
        else return 1;
    }
    float t0,t1;if(fog_interval(o,d,limit,t0,t1)){t0=max(t0,start);if(t1>t0)tr*=exp(-fog.x*(t1-t0));}
    return tr;
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
    float3 go,gd;bool gvalid;camera_ray(float2(p)+0.5+jitter.xy,false,go,gd,gvalid);
    if(intersect(go,gd,1e4,guide,false,true)){
        Material m=material_at(guide.material,guide.p);apply_maps(guide,m);prepare_film(m,abs(dot(-gd,guide.n)),1,0);normal_roughness[p]=float4(guide.n,m.color_rough.a);object_id[p]=guide.material+1;
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
    }else{guide.material=0xffffffffu;normal_roughness[p]=float4(0,0,0,1);object_id[p]=0;diffuse_albedo[p]=0;specular_albedo[p]=0;depth[p]=1e4;device_depth[p]=1;motion[p]=0;specular_distance[p]=65504.0;}
}
[numthreads(8,8,1)]void trace(uint3 tid:SV_DispatchThreadID){
    uint2 p=tid.xy;
    if(recording){if(any(p!=0))return;p=min(path_record.xy,dimensions.xy-1);}
    else{p+=region.xy;if(any(p>=region.zw))return;}
    if(any(p>=dimensions.xy))return;
    pixel_seed=hash(p.x+p.y*dimensions.x);rng_state=hash(pixel_seed+sampling.w*747796405);proceed_steps=0;float bounces_used=0;
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
    float pixel_angle=camera_model.x==1?0:2*environment.z/dimensions.y,pixel_width=camera_model.x==1?2*camera_model.w/dimensions.y:0;
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
        float3 origin,direction;bool lens_valid;camera_ray(float2(p)+0.5+subpixel,true,origin,direction,lens_valid);
        cone_width=pixel_width;cone_spread=pixel_angle;
        float3 throughput=lens_valid?camera_weight:0,radiance=0,direct=0;float4 first_diffuse=0;float3 first_specular=0;uint portal_crossings=0,false_hits=0;uint64_t media=0;bool first_spec=false,previous_delta=true,previous_restir=false;float previous_pdf=0;float3 previous_position=origin;float3 absorption=0;float density=0;
        // Recording only (dead code otherwise): why the path ended, and its last bounce.
        uint end_reason=lens_valid?3:5,last_bounce=0;
        if(lpe_on){lpe_state=0;lpe_weight=1;}
        record(0,0,origin,throughput,0,direction);
        for(uint bounce=0;bounce<sampling.z && lens_valid;bounce++){
            last_bounce=bounce;
            bounces_used+=1;
            uint mis=(scene_info.w>>4)&3;
            // Light selection: the HDRI and the emissive triangles share NEE half and half.
            bool env_nee=hdri_on() && mis!=2,tri_nee=scene_info.x>0 && mis!=2;
            float p_env=env_nee?(tri_nee?0.5:1):0;
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
            {float sigma,g,t0,t1;float3 albedo;
            if((density>0 || (fog_on() && media==0)) && medium_params(media,origin,direction,found?h.t:1e4,sigma,albedo,g,t0,t1)){
                float ts=t0-log(1-random())/sigma;
                if(ts<t1){
                    // A scatter before the surface: NEE by the phase function, then a new direction.
                    float3 arriving=throughput*exp(-absorption*ts);
                    throughput*=exp(-absorption*ts)*albedo;cone_width+=cone_spread*ts;
                    float3 x=origin+direction*ts;
                    if(bounce==0 && sample_albedo)first_diffuse=float4(albedo,0);
                    float3 ld,le;float pl,limit;
                    if(sample_light(x,env_nee,tri_nee,p_env,ld,le,pl,limit)){
                        float pp=hg_pdf(dot(direction,ld),g);
                        if(!occluded(x,ld,limit)){float3 e=throughput*medium_transmittance(media,x,ld,limit)*pp*le/max(mis==1?pl:pl+pp,1e-8);radiance+=e;if(bounce==0)direct+=e;record(11,bounce,x,e,limit,ld);lpe_add(4,lpe_env_sample?6:5,e);}
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
                float3 env=bounce==0 && hdri_on() && ((uint)environment_map.z&2u)==0?background.rgb:environment_radiance(direction);
                float weight=1;if(hdri_on() && bounce>0 && !previous_delta)weight=mis==1?0:mis==2?1:previous_pdf/(previous_pdf+p_env*env_pdf(direction));
                float3 e=throughput*env*weight;lpe_add(8,6,e);
                // Suns (they lead the emitter table): seen inside their disk, weighted against
                // light sampling as triangles are; a hidden one only from bounce 1 on.
                for(uint i=0;i<(uint)lights.x;i++){Emitter sun=emitters[i];
                    // Inside the disk by sin² (from a cross product), which keeps its precision.
                    float3 x=cross(direction,sun.position_area.xyz);float om=sun.edge1_cdf.x;
                    if(dot(direction,sun.position_area.xyz)<=0 || dot(x,x)>om*(2-om))continue;
                    if(bounce==0 && (((uint)materials[(uint)sun.emission_material.w].lobes.w&2u)!=0))continue;
                    float w=1;if(bounce>0 && !previous_delta)w=mis==1 || previous_restir?0:mis==2?1:previous_pdf/(previous_pdf+(1-p_env)*sun.edge2_probability.w/(2*PI*sun.edge1_cdf.x));
                    float3 se=throughput*sun.emission_material.rgb*w;e+=se;lpe_add(8,5,se);}
                radiance+=e;if(bounce<=1)direct+=e;record(10,bounce,origin,throughput,luminance(e),direction);end_reason=0;break;}
            throughput*=exp(-absorption*h.t);cone_width+=cone_spread*h.t;
            float3 arriving=throughput;
            // Portal: continue from the matching point of the paired object, without a bounce.
            // Light sampling cannot see through portals, so emission found after one has weight 1.
            Material m=material_at(h.material,h.p);
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
                throughput*=m.color_rough.rgb;
                if(random()<m.lobes.y){direction=reflect(direction,h.n);origin=h.p+direction*0.0005;record(3,bounce,h.p,throughput,0,direction);lpe_advance(2);}
                else{origin=h.p+direction*0.0005;record(8,bounce,h.p,throughput,0,direction);bounce--;}
                continue;
            }
            float3 emitted=m.color_rough.rgb*m.optical.y;
            if(m.optical.w>0.5 && !h.front)emitted=0;
            emitted*=spot(m,-dot(direction,h.n));
            // Emission keeps the flat colour: light sampling has no texture lookup.
            apply_maps(h,m);
            // A dielectric's film sits between the media on both sides (the transmission branch).
            if(m.surface_scale.z>0){
                float n1=1,n2=0;
                if(m.lobes.z>0){uint e=medium_entry(h.material,m.lobes.w);uint64_t after=h.front?medium_push(media,e):medium_remove(media,e);n1=h.front?medium_ior(media,medium_top(media)):m.optical.x;n2=h.front?m.optical.x:medium_ior(after,medium_top(after));}
                prepare_film(m,saturate(dot(-direction,h.n)),n1,n2);
            }
            if(has_grating(m))grating_wavelength(pixel_seed^(sequence_index*0x9e3779b9u)^(bounce*0x85ebca6bu));
            if(bounce==0 && sample_albedo){first_diffuse=float4(m.color_rough.rgb*(1-m.lobes.x)*(1-m.lobes.z),h.material==guide.material?1:0);first_specular=spec_fresnel(m,abs(dot(-direction,h.n)),f0_of(m));}
            // Emission found by BSDF sampling: MIS weight, or all / nothing in the single-strategy modes.
            float weight=1;if(h.emitter!=0 && bounce>0 && !previous_delta)weight=mis==1 || (previous_restir && restir_covers(previous_position,emitters[h.emitter-1]))?0:mis==2?1:previous_pdf/(previous_pdf+(1-p_env)*emitter_pdf(previous_position,h.p,emitters[h.emitter-1]));
            float3 e=throughput*emitted*weight;radiance+=e;if(bounce<=1)direct+=e;
            float emitted_l=luminance(e);lpe_add(8,5,e);
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
                if(!occluded(h.p+h.ng*0.0003,ld,limit)){float3 e=throughput*f*nl*le/max(reused?1:mis==1?pl:pl+pdf,1e-8);
                    if(fog_on() || density>0)e*=medium_transmittance(media,h.p+h.ng*0.0003,ld,limit);
                    radiance+=e;if(bounce==0)direct+=e;record(11,bounce,h.p,e,limit,ld);
                    if(lpe_on){float3 ed=e*lpe_diffuse/max(f,1e-30);uint light=lpe_env_sample?6:5;lpe_add(0,light,ed);lpe_add(lpe_glossy(m),light,e-ed);}}
                else record(12,bounce,h.p,0,limit,ld);
            }
            float3 next;bool delta=false;float bsdf_pdf=0;uint lobe=1;
            if(random()<m.lobes.z){
                // The media on both sides: entering, the current one and this; leaving, this and
                // the one the stack falls back to (air when empty, as before nesting).
                uint e=medium_entry(h.material,m.lobes.w);uint64_t after=h.front?medium_push(media,e):medium_remove(media,e);
                float n1=h.front?medium_ior(media,medium_top(media)):m.optical.x,n2=h.front?m.optical.x:medium_ior(after,medium_top(after));
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
        if(!all(isfinite(radiance))){radiance=0;direct=0;}
        radiance=max(radiance,0);if(options.x>0 && sampling.x>4){float l=luminance(radiance);float scale=min(1,options.x*max(luminance(mean),0.05)/max(l,1e-6));radiance*=scale;direct*=scale;}
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
    noisy[p]=float4(batch,1);accumulation[p]=float4(mean,max(0,m2));direct_light[p]=float4(dmean,1);specular_light[p]=float4(smean,1);
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
    cone_width=camera_model.x==1?2*camera_model.w/dimensions.y:0;cone_spread=camera_model.x==1?0:2*environment.z/dimensions.y;
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
