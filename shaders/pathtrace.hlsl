#include "common.hlsli"
// Nested dielectrics (track 10): interior list depth — the ambient medium plus 7 volumes.
#define MED_DEPTH 8u
uint hash(uint x){x^=x>>16;x*=0x7feb352d;x^=x>>15;x*=0x846ca68b;x^=x>>16;return x;}
static uint rng_state,sequence_index,dimension_index,pixel_seed;
uint permute(uint x,uint seed){x^=x*0x3d20adea;x+=seed;x*=(seed>>16)|1;x^=x*0x05526c56;x^=x*0x53a22864;return x;}
uint owen(uint x,uint seed){return reversebits(permute(reversebits(x),seed));}
// Second Sobol dimension, in reversed bit order. Its generator is the Pascal matrix mod 2:
// bit m is the parity of the index bits k that contain m as a submask, so a 5-step superset
// transform replaces the 32-step loop over direction numbers (bit-identical results).
uint superset(uint i){i^=(i>>1)&0x55555555;i^=(i>>2)&0x33333333;i^=(i>>4)&0x0f0f0f0f;i^=(i>>8)&0x00ff00ff;i^=(i>>16)&0x0000ffff;return i;}
static float pair_second;
// Debug statistics for this pixel and frame (views 8 and 9).
static uint proceed_steps;
float random(){
    if(jitter.z>0.5 && modes.w==0){
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
struct Hit{float t;float3 p;float3 n;float3 ng;float3 albedo;float2 rough_metal;uint material;uint emitter;bool front;bool textured;};
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
// maps, 16 any texture maps. Without TEXTURE_MAPS (build.rs's `trace` variant, for scenes with
// no maps) the map code folds away: it cost untextured scenes about 7% even when never run.
#ifdef TEXTURE_MAPS
static const bool texture_maps=true;
#else
static const bool texture_maps=false;
#endif
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
#ifdef PROCEDURALS
// ---- Procedural SDF chains (roadmap track 13), a mirror of src/sdf.rs -----------------
float proc_smin(float a,float b,float k){if(k<=1e-4)return min(a,b);float h=saturate(0.5+0.5*(b-a)/k);return b*(1.0-h)+a*h-k*h*(1.0-h);}
float proc_smax(float a,float b,float k){if(k<=1e-4)return max(a,b);float h=saturate(0.5+0.5*(a-b)/k);return a*h+b*(1.0-h)+k*h*(1.0-h);}
float proc_join(float a,float b,float op,float k){if(op==0.0)return proc_smin(a,b,k);if(op==1.0)return proc_smax(a,-b,k);return proc_smax(a,b,k);}
float proc_box3(float3 e){return length(max(e,0.0))+min(max(max(e.x,e.y),e.z),0.0);}
float proc_rect2(float a,float b){return length(max(float2(a,b),0.0))+min(max(a,b),0.0);}
float proc_wrap2(float v){return v-floor(v*0.5)*2.0;}
float proc_capped2(float ex,float ey){return length(max(float2(ex,ey),0.0))+min(max(ex,ey),0.0);}
float proc_menger(float3 p,float level){
    float bd=proc_box3(abs(p)-1.0);float rem=1e9,sc=1.0,li=0.0;
    while(li+0.5<level){
        float sx=(abs(proc_wrap2(p.x*sc+1.0)-1.0)-1.0/3.0)/sc;
        float sy=(abs(proc_wrap2(p.y*sc+1.0)-1.0)-1.0/3.0)/sc;
        float sz=(abs(proc_wrap2(p.z*sc+1.0)-1.0)-1.0/3.0)/sc;
        float t1=proc_rect2(sx,sy),t2=proc_rect2(sx,sz),t3=proc_rect2(sy,sz);
        float best=min(min(t1,t2),t3);if(best<rem)rem=best;
        sc*=3.0;li+=1.0;
    }
    return max(bd,-rem);
}
float proc_bulb(float3 q,float power){
    float3 c=q*(1.0/0.74);float r0=length(c);
    if(r0>2.0)return max((r0-2.0)*0.74,0.00005); // the bailout sphere guides the march but is
    float3 z=c;float r=0.0;float dch=1.0;
    for(int i=0;i<11;i++){
        r=length(z);
        if(r>2.0)break;
        dch*=power*(r>1.0?pow(r,power-1.0):1.0);
        if(z.y<0.0)z.y=-z.y;
        float inv=r>1e-9?1.0/r:0.0;
        float th=acos(clamp(z.y*inv,-1.0,1.0));
        float ph=atan2(z.x,z.z);
        float rn=pow(r,power),th2=th*power,ph2=ph*power;
        float s=rn*sin(th2);
        z=float3(s*sin(ph2),rn*cos(th2),s*cos(ph2))+c;
    }
    if(r>2.0)return min((r-2.0)/dch,0.5)*0.74;
    return -r*0.74;
}
uint proc_hash(uint x){x^=x>>16;x*=0x7feb352du;x^=x>>15;x*=0x846ca68bu;return x^x>>16;}
float proc_unit(uint x){return float(x>>8)*(1.0/16777216.0);}
float proc_blob(float3 q,float seed){
    uint s=asuint(seed);float acc=1e9;
    for(int i=0;i<6;i++){
        uint h=proc_hash(s*0x9E3779B1u+uint(i)*0x85EBCA6Bu+0x27D4EB2Fu);
        float3 c=float3(proc_unit(h),proc_unit(h+1u),proc_unit(h+2u))-0.5;
        float r=0.16+0.22*proc_unit(h+3u);
        acc=proc_smin(acc,length(q-c*0.66)-r,0.16);
    }
    return acc;
}
float proc_atom(float4 A,float4 B,float4 C,float3 q){
    float kind=A.x;
    if(kind==0.0){float3 w=q-float3(A.z,A.w,B.x);return length(w)-A.y;}
    if(kind==1.0){float3 w=q-float3(A.w,B.x,B.y);float t=sqrt(w.x*w.x+w.z*w.z)-A.y;return sqrt(t*t+w.y*w.y)-A.z;}
    if(kind==2.0){float3 w=q-float3(A.w,B.x,B.y);return proc_capped2(sqrt(w.x*w.x+w.z*w.z)-A.y,abs(w.y)-A.z);}
    if(kind==3.0){float3 w=q-float3(B.x,B.y,B.z);return proc_box3(abs(w)-float3(A.y,A.z,A.w));}
    if(kind==4.0)return proc_bulb(q,A.y);
    if(kind==5.0)return proc_menger(q,A.y);
    if(kind==6.0)return proc_blob(q,A.y);
    return 1e9;
}
float proc_chain_at(uint base,float3 q){
    float4 A0=procedurals[base],B0=procedurals[base+1];
    float d=proc_atom(A0,B0,procedurals[base+2],q);
    float4 A1=procedurals[base+3];
    if(A1.x<0.0)return d;
    float4 B1=procedurals[base+4],C1=procedurals[base+5];
    d=proc_join(d,proc_atom(A1,B1,C1,q),B1.w,C1.x);
    float4 A2=procedurals[base+6];
    if(A2.x<0.0)return d;
    float4 B2=procedurals[base+7],C2=procedurals[base+8];
    return proc_join(d,proc_atom(A2,B2,C2,q),B2.w,C2.x);
}
// Object-space march from the candidate's box entry; step is scaled by |od| so world t
// advances evenly (safety keeps smooth/fractal chains conservative). The stop epsilon is
// far tighter than sdf.rs HIT_EPS (1e-3, picks only): the commit lands on the true
// surface, inside the 0.0003-world secondary-ray offset — a commit short of the surface
// self-hits the next ray at t~0 (moiré rings and self-occlusion on every surface).
float proc_hit(uint base,float3 oo,float3 od,float t0,float tmax){
    float4 C0=procedurals[base+2];float safety=C0.x,budget=C0.y;
    float speed=length(od);if(speed<1e-9)return -1.0;
    float t=max(t0,0.0001);
    for(float it=0.0;it<budget;it+=1.0){
        float d=proc_chain_at(base,oo+od*t);
        if(d<0.00001)return d>0.0?t+d/speed:t; // land on the true surface when approached from
        t+=max(d,1e-4)*safety/speed;   // the secondary-ray offset (0.0003 world), so a shadow ray
        if(t>tmax)return -1.0;         // restarted from a surface point never re-hits at t~0
    }
    return -1.0;
}
// Whole-chain gradient at the hit (central differences, GRAD_STEP as sdf.rs) — analytic
// normals for every chain kind, fractal included.
float3 proc_grad(uint base,float3 p){
    float3 g=0.0;
    for(int a=0;a<3;a++){
        for(float s=-1.0;s<=1.0;s+=2.0){
            float3 d=0.0;
            if(a==0)d.x=s*0.015;else if(a==1)d.y=s*0.015;else d.z=s*0.015;
            g+=proc_chain_at(base,p+d)*d;
        }
    }
    return dot(g,g)<1e-8?float3(0.0,1.0,0.0):g;
}
#endif
bool intersect(float3 o,float3 d,float limit,out Hit h,bool shadow,bool camera_ray){
    RayDesc ray;ray.Origin=o;ray.Direction=d;ray.TMin=0.0001;ray.TMax=limit;
    // Nonopaque geometry allows the light quad and optional glass to be ignored for shadow rays.
    RayQuery<RAY_FLAG_NONE> q;q.TraceRayInline(scene,RAY_FLAG_NONE,255,ray);
    while(q.Proceed()){proceed_steps++;
        uint m=candidate_material(q.CandidateInstanceID(),q.CandidatePrimitiveIndex());
#ifdef PROCEDURALS
        if(q.CandidateType()==CANDIDATE_PROCEDURAL_PRIMITIVE){
            Instance ci=instances[q.CandidateInstanceID()];
            if(ci.procedural==0u)continue;
            float tt=proc_hit((ci.procedural-1u)*9u,q.CandidateObjectRayOrigin(),q.CandidateObjectRayDirection(),0.0001,limit);
            if(tt<0.0)continue;
            if(camera_ray && (((uint)materials[m].lobes.w&2u)!=0))continue;
            if(!shadow || jitter.w>0.5 || materials[m].lobes.z<0.5)q.CommitProceduralPrimitiveHit(tt);
            continue;
        }
#endif
        if(flagged(m,8u) && alpha_hole(q.CandidateInstanceID(),q.CandidatePrimitiveIndex(),q.CandidateTriangleBarycentrics(),m))continue;
        if(camera_ray && (((uint)materials[m].lobes.w&2u)!=0))continue;
        if(!shadow || jitter.w>0.5 || materials[m].lobes.z<0.5)q.CommitNonOpaqueTriangleHit();
    }
    h=(Hit)0;
    uint status=q.CommittedStatus();
    if(status!=COMMITTED_TRIANGLE_HIT
#ifdef PROCEDURALS
        &&status!=COMMITTED_PROCEDURAL_PRIMITIVE_HIT
#endif
    )return false;
    Instance inst=instances[q.CommittedInstanceID()];uint prim=q.CommittedPrimitiveIndex();float2 b=q.CommittedTriangleBarycentrics();
    // uv_scale: texture units per world unit at the hit; T, B: world dP/du and dP/dv (both for
    // the footprint; T zero when only uv_scale is known).
    h.t=q.CommittedRayT();h.p=o+d*h.t;float2 uv=0;float4 tangent=0;bool mapped;float uv_scale=0;float3 T=0,B=0;
#ifdef PROCEDURALS
    if(status==COMMITTED_PROCEDURAL_PRIMITIVE_HIT){
        uint slot=(inst.procedural-1u)*9u;
        h.material=inst.material;
        float3 po=q.CommittedObjectRayOrigin()+q.CommittedObjectRayDirection()*h.t;
        float3 g=proc_grad(slot,po);
        h.n=normalize(mul(g,(float3x3)q.CommittedWorldToObject3x4()));
        // Procedural surfaces only take a texture set (world box mapping); a mesh-style
        // map with own UVs has no UVs to read and keeps the plain albedo (not done).
        uv_scale=materials[h.material].base_scale.w;
        mapped=flagged(h.material,16u)&&uv_scale>0;
        if(mapped){
            float3 t,bt;
            box_map(h.p,h.n,uv_scale,uv,t,bt);
            tangent=float4(t,tangent_sign(h.n,t,bt));T=t/uv_scale;B=bt/uv_scale;
        }
        // Analytic surfaces are not in the triangle NEE table: h.emitter=0, but emission
        // on a BSDF hit still lights the path through the material.
        h.emitter=0;h.front=dot(d,h.n)<0;if(!h.front)h.n=-h.n;h.ng=h.n;
        if(mapped)sample_maps(h,uv,tangent,footprint(d,h.ng,cone_width+cone_spread*h.t,T,B,uv_scale));
        return true;
    }
#endif
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
    // Footprint: the cone's width at the hit, stretched along the ray by the grazing angle
    // (capped at 20×).
    if(mapped)sample_maps(h,uv,tangent,footprint(d,h.ng,cone_width+cone_spread*h.t,T,B,uv_scale));
    h.emitter=inst.emitter_base==0xffffffffu?0:emitter_lookup[inst.emitter_base+prim];return true;
}
// Shadow test: any blocker ends the search, and no hit data is reconstructed.
bool occluded(float3 o,float3 d,float limit){
    RayDesc ray;ray.Origin=o;ray.Direction=d;ray.TMin=0.0001;ray.TMax=limit;
    RayQuery<RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH> q;q.TraceRayInline(scene,RAY_FLAG_NONE,255,ray);
    while(q.Proceed()){proceed_steps++;uint m=candidate_material(q.CandidateInstanceID(),q.CandidatePrimitiveIndex());
#ifdef PROCEDURALS
        if(q.CandidateType()==CANDIDATE_PROCEDURAL_PRIMITIVE){
            Instance ci=instances[q.CandidateInstanceID()];
            if(ci.procedural==0u)continue;
            float tt=proc_hit((ci.procedural-1u)*9u,q.CandidateObjectRayOrigin(),q.CandidateObjectRayDirection(),0.0001,limit);
            if(tt<0.0)continue;
            if(jitter.w>0.5 || materials[m].lobes.z<0.5)q.CommitProceduralPrimitiveHit(tt);
            continue;
        }
#endif
        if(flagged(m,8u) && alpha_hole(q.CandidateInstanceID(),q.CandidatePrimitiveIndex(),q.CandidateTriangleBarycentrics(),m))continue;
        if(jitter.w>0.5 || materials[m].lobes.z<0.5)q.CommitNonOpaqueTriangleHit();}
#ifdef PROCEDURALS
    uint st=q.CommittedStatus();return st==COMMITTED_TRIANGLE_HIT||st==COMMITTED_PROCEDURAL_PRIMITIVE_HIT;
#else
    return q.CommittedStatus()==COMMITTED_TRIANGLE_HIT;
#endif
}
Material material_at(uint id,float3 p){
    Material m=materials[id];uint pattern=(uint)m.optical.z;float t=0;
    if(pattern==1)t=fmod(abs(floor(p.x*m.secondary.w)+floor(p.z*m.secondary.w)),2);
    if(pattern==2)t=step(0,sin((p.x+p.z)*m.secondary.w*PI));
    // Marble: soft veins where a turbulent sine crosses zero, at most 75% secondary color.
    if(pattern==3){float s=m.secondary.w;float vein=sin((p.x+p.z)*s*0.6+3*sin(p.y*3+p.z*2)+1.5*sin(p.x*5-p.z*2)+0.6*sin(p.x*13+p.y*11)+0.3*sin(p.z*27-p.y*21));t=0.75*pow(1-abs(vein),6);}
    m.color_rough.rgb=lerp(m.color_rough.rgb,m.secondary.rgb,t);
    if(pattern&&((uint)m.lobes.w&4u)==0u)m.secondary.w=t; // stash for layered reads; portals keep the pair id in this slot
    return m;
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
// First index whose CDF value exceeds u, among `count` entries of row `row`.
uint cdf_search(uint row,uint count,float u){uint lo=0,hi=count-1;while(lo<hi){uint mid=(lo+hi)/2;if(u<env_cdf[uint2(mid,row)])hi=mid;else lo=mid+1;}return lo;}
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
float ggx_d(float nh,float a){float d=nh*nh*(a*a-1)+1;return a*a/(PI*d*d);}
float g1(float nv,float a){return 2*nv/max(1e-6,nv+sqrt(a*a+(1-a*a)*nv*nv));}
float3 f0_of(Material m){float f=pow((m.optical.x-1)/(m.optical.x+1),2);return lerp(lerp(f.xxx,1.0.xxx,m.lobes.y),m.color_rough.rgb,m.lobes.x);}
float spec_probability(Material m){return clamp(0.25+0.5*m.lobes.x+0.25*m.lobes.y,0.1,0.95);}
// Track 11 (roadmap 11): layered and exotic materials. Material fields (common.hlsli):
// coating=(clearcoat, coat roughness, sheen, sheen roughness) with the two roughness values
// perceptual like color_rough.a; effects=(thin-film IOR, film thickness nm, diffraction period
// nm, 0); weave=(anisotropy, groove rotation degrees, 0, 0). Layer order follows glTF: the thin
// film and the grating ride on the base specular, then sheen, then clearcoat; the transmittance
// pairs below attenuate each deeper layer. With every field zero the guarded branch is skipped
// and the arithmetic above is exactly what the old path was (bit-identical).
float ggx_s(float mu,float a){ // Kulla-Conty complement albedo 1 - E, from the 32x32 lattice table
  // (tools/build_ggx_e_data.py integrates the shader's own D * G1 * G1 / (4 no nv) form; rows are
  // alpha, columns mu, both in [0.02,1], bilinear). Only the layered branch reads the table.
  float A=(clamp(a,0.02,1.0)-0.02)*31.0/0.98,M=(clamp(mu,0.02,1.0)-0.02)*31.0/0.98;
  int j0=(int)A,j1=min(j0+1,31),i0=(int)M,i1=min(i0+1,31);float f=M-(float)i0,g=A-(float)j0;
  return lerp(lerp(ggx_table[j0*32+i0],ggx_table[j0*32+i1],f),lerp(ggx_table[j1*32+i0],ggx_table[j1*32+i1],f),g);
}
float ggx_eavg(float a){return PI+a*(-0.446898+a*(-5.72019+a*(6.61848-2.41727*a)));} // published E_avg fit
float3 ggx_ms(float nv,float nl,float a,float3 f0){ // multiple scattering recovers lost energy
  float3 fm=(0.04+f0*(0.66+0.3*f0))*f0;return ggx_s(nv,a)*ggx_s(nl,a)*fm/max(PI-ggx_eavg(a),1e-3);}
float ggx_da(float3 h,float3 T,float3 B,float3 n,float at,float ab){ // aniso GGX (Burley via Filament);
  // at=ab=a reduces it to ggx_d exactly, which is what keeps the isotropic family consistent.
  float x=dot(h,T),y=dot(h,B),z=dot(h,n);float b=x*x/(at*at)+y*y/(ab*ab)+z*z;return 1.0/max(PI*at*ab*b*b,1e-20);}
float g1a(float3 v,float3 T,float3 B,float3 n,float at,float ab){ // slope-space Smith G1: the sqrt
  // equals g1's when at=ab=a (a^2 x^2 + a^2 y^2 + z^2 = a^2 + (1-a^2) z^2 for unit v).
  float x=dot(v,T),y=dot(v,B),z=saturate(dot(v,n));return 2.0*z/max(1e-6,z+sqrt(at*at*x*x+ab*ab*y*y+z*z));}
// Charlie sheen (Estevez and Kulla's 2017 production-friendly CC sheen; the log-lambda V fit
// coefficients are the published set quoted by Patapom's GodComplex BRDF.cs). a = alpha again.
float charlie_d(float nh,float a){float s=max(1e-4,1.0-nh*nh);return (1.0/a+2.0)*pow(s,0.5/a)/(2.0*PI);}
float charlie_l(float x,float a){ // log lambda; mirrored above x = 0.5
  float r=1.0-a*a,t=x<0.5?x:1.0-x;
  float A=lerp(25.3245,21.5473,r),B=lerp(3.32435,3.82987,r),C=lerp(0.16801,0.19823,r),D=lerp(-1.27393,-1.97760,r),E=lerp(-4.85967,-4.32054,r);
  float h=A/(1.0+B*pow(0.5,C))+0.5*D+E,lt=A/(1.0+B*pow(t,C))+D*t+E;return x<0.5?lt:2.0*h-lt;}
// Thin-film interference (Belcour 2017, the form glTF KHR_materials_iridescence adopted):
// per-channel cos^2 envelope of optical depth over wavelength, phase flipped when exactly one
// boundary enters a denser medium; the film sits on the base, so its incidence is the macro
// normal's view cosine. Grooves: the diffraction grating equation OPD = p (v.T - l.T) riding
// the same per-channel machinery, with wavelength peaks at integer multiples of the period.
float3 film_env(float nv,float ior,float d,float base_ior){
  float s2=(1.0-nv*nv)/(ior*ior),ct=sqrt(max(0.0,1.0-s2)),opd=2.0*ior*d*ct; // optical depth, nm
  float ph=((ior>1.0)!=(base_ior>ior))?0.5:0.0;
  float3 o=opd*float3(0.0015384615,0.0019607843,0.0021052632)+ph; // 1/650, 1/510, 1/475 per nm
  float3 c=cos(PI*o);return c*c;}
float3 grating_env(float3 v,float3 l,float3 T,float3 n,float p){
  float opd=p*(dot(v,T)-dot(l,T));float3 o=opd*float3(0.0015384615,0.0019607843,0.0021052632);
  float3 c=cos(PI*o);return c*c;}
float film_favg(float ior){return clamp((ior-1.0)/(4.08567+1.00071*ior),0.0,1.0);} // mean reflectance
float3 eval_bsdf(Material m,float3 n,float3 v,float3 l,float3 p,out float pdf){
    float nl=saturate(dot(n,l)),nv=saturate(dot(n,v));float3 h=normalize(v+l);float nh=saturate(dot(n,h)),vh=saturate(dot(v,h));float a=max(0.001,m.color_rough.a*m.color_rough.a);float d=ggx_d(nh,a);float3 f=fresnel(vh,f0_of(m));float ps=spec_probability(m);
    pdf=(1-m.lobes.z)*((1-ps)*nl/PI+ps*d*nh/max(4*vh,1e-6));
    float3 diff=(1-f)*(1-m.lobes.x)*m.color_rough.rgb/PI;
    if(((uint)m.lobes.w&64u)!=0u&&m.lobes.z<0.5){ // bit6 from pack: a layer field is set (track 11)
        float pc=m.coating.x,sh=m.coating.z,an=m.weave.x,ft=m.effects.y,gp=m.effects.z;
        float ac=max(1e-3,m.coating.y*m.coating.y),as=max(1e-3,m.coating.w*m.coating.w),wbase=(1.0-pc)*(1.0-sh);
        float3 fs0=f0_of(m);
        float3 cpair=(1.0-pc*fresnel(nv,0.04.xxx))*(1.0-pc*fresnel(nl,0.04.xxx)); // through the coat
        float3 spair=(1.0-sh*fresnel(nv,fs0))*(1.0-sh*fresnel(nl,fs0));            // through the sheen
        float3 env=1.0.xxx; // per-channel film / grating envelope
        float d_b=d,g_b=g1(nv,a)*g1(nl,a),a_geo=a,ink=1.0,difffac=1.0;
        if(an>0.0||gp>0.0){ // groove frame: recomputed from the box map at the hit (registers: not stored in Hit)
            float2 du;float3 tb,bb;box_map(p,n,1.0,du,tb,bb);
            tb=normalize(tb);bb=normalize(bb);
            float rot=radians(m.weave.y),cr=cos(rot),sr=sin(rot);
            float3 Tr=cr*tb+sr*bb;float3 Br=cross(n,Tr);
            if(an>0.0){
                float asp=sqrt(max(0.01,1.0-0.9*an)),at=a/asp,ab=a*asp;a_geo=sqrt(at*ab);
                d_b=ggx_da(h,Tr,Br,n,at,ab);g_b=g1a(v,Tr,Br,n,at,ab)*g1a(l,Tr,Br,n,at,ab);
            }
            if(gp>0.0){float opd=gp*(dot(v,Tr)-dot(l,Tr));float3 c=cos(PI*opd*float3(0.0015384615,0.0019607843,0.0021052632));env*=c*c;ink=(m.optical.z>0.5?m.secondary.w:0.0);} // grooves: diffuse only under ink
        }
        if(ft>0.0){env*=film_env(nv,m.effects.x,ft*(m.optical.z>0.5?lerp(0.5,1.5,m.secondary.w):1.0),m.optical.x);difffac=1.0-film_favg(m.effects.x);}
        float dc=ggx_d(nh,ac);
        float3 coat=fresnel(vh,0.04.xxx)*dc*g1(nv,ac)*g1(nl,ac)/max(4*nv*nl,1e-6)+ggx_ms(nv,nl,ac,0.04.xxx);
        float3 sheen=fresnel(nh,fs0)*charlie_d(nh,as)/max((1.0+exp(charlie_l(nv,as))+exp(charlie_l(nl,as)))*4.0*max(nv,1e-3)*max(nl,1e-3),1e-6);
        pdf=(1-m.lobes.z)*(wbase*((1-ps)*nl/PI+ps*d_b*nh/max(4*vh,1e-6))+pc*dc*nh/max(4*vh,1e-6)+sh*(1.0-pc)*nl/PI);
        return pc*coat+sh*cpair*sheen+wbase*(cpair*spair)*(diff*(ink*difffac)+f*env*d_b*g_b/max(4*nv*nl,1e-6)+ggx_ms(nv,nl,a_geo,fs0)*env);
    }
    return (1-m.lobes.z)*(diff+f*d*g1(nv,a)*g1(nl,a)/max(4*nv*nl,1e-6)); // old text verbatim: layer-free scenes stay bit-identical
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
Emitter select_emitter(float u){uint lo=0,hi=scene_info.x-1;while(lo<hi){uint mid=(lo+hi)/2;if(u<emitters[mid].edge1_cdf.w)hi=mid;else lo=mid+1;}return emitters[lo];}
float3 camera_direction(float2 pixel){float2 ndc=(pixel/float2(dimensions.xy))*2-1;ndc.y=-ndc.y;float4 p=mul(inverse_vp,float4(ndc,1,1));return normalize(p.xyz/p.w-eye.xyz);}
// A point in the unit aperture: a disk, or an n-gon (one triangle of the fan, then uniform in it).
float2 aperture_point(){
    if(lens.z<2.5){float r=sqrt(random()),a=2*PI*random();return r*float2(cos(a),sin(a));}
    float n=lens.z,k=floor(random()*n);float a0=lens.w+2*PI*k/n,a1=lens.w+2*PI*(k+1)/n;
    float u=random(),v=random();if(u+v>1){u=1-u;v=1-v;}
    return u*float2(cos(a0),sin(a0))+v*float2(cos(a1),sin(a1));
}
// Camera rays for every model (roadmap 5c). `lens_sample` adds depth of field; a ray the
// cat-eye vignette blocks comes back with valid=false and contributes nothing (optical vignetting).
void camera_ray(float2 pixel,bool lens_sample,out float3 o,out float3 d,out bool valid){
    float3 right=world_view[0].xyz,up=world_view[1].xyz,forward=-world_view[2].xyz;
    float2 ndc=(pixel/float2(dimensions.xy))*2-1;ndc.y=-ndc.y;float aspect=float(dimensions.x)/dimensions.y;
    uint model=(uint)camera_model.x;o=eye.xyz;valid=true;
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
// Participating media (track 12, see docs/research/track-12-volumes.md): homogeneous
// sigma_a/sigma_s per metre inside meshes (material volume_a/volume_s) and an ambient
// medium for the whole space (Frame.medium_a/medium_s, fog and underwater). Membership
// rides the same interior list as the dielectrics, so the containing medium is its top.
bool has_volume(Material m){return max(max(m.volume_s.x,m.volume_s.y),m.volume_s.z)>0||max(max(m.volume_a.x,m.volume_a.y),m.volume_a.z)>0;}
// Isotropic phase: the cosine to the incoming direction is uniform on [-1,1] and the
// value/pdf is 1 for exact sampling; pdf = 1/2PI is also the balance partner in vol_nee.
float3 vol_dir(float3 in_dir,out float pdf){
    float cs=1-2*random();float sc=sqrt(max(0,1-cs*cs));float az=2*PI*random();
    pdf=1/(2*PI);return basis(in_dir,float3(sc*cos(az),sc*sin(az),cs));
}
// Next-event estimation at a scattering point: same table as the surface NEE but with the
// scattering integrand sig_s·phase (phase = 1/2PI) and the arriving radiance carrying the
// full extinction exp(-(sig_a+sig_s)·len) along the shadow segment; the balance partner
// for the continuation is the phase pdf. Environment and suns sit at the far plane /
// their nominal 50 units, so the extinction uses those reaches.
float3 vol_nee(float3 q,float3 sig_a,float3 sig_s){
    uint mis=(scene_info.w>>4)&3;
    bool env_nee=hdri_on() && mis!=2,tri_nee=scene_info.x>0 && mis!=2;
    float p_env=env_nee?(tri_nee?0.5:1):0;
    float ps=1/(2*PI);float3 ext=sig_a+sig_s;float3 acc=0;
    if(env_nee && random()<p_env){
        float pe;float3 ld=sample_env(pe);
        if(pe>0)acc+=sig_s*ps*exp(-ext*500)*environment_radiance(ld)/max(mis==1?p_env*pe:p_env*pe+ps,1e-8);
    }else if(tri_nee){
        Emitter light=select_emitter(random());
        if(light.position_area.w<0){
            float om=0,pc=0,lim=1e4;float3 ld=float3(0,1,0);
            if(light.position_area.w==-1){
                float3 c=light.position_area.xyz-q;float d2=dot(c,c),r=light.edge1_cdf.x;om=sphere_om(d2,r);
                if(om>0){ld=cone_sample(c*rsqrt(d2),om,pc);float b=dot(c,ld);lim=b-sqrt(max(0,r*r-(d2-b*b)));}
            }else{om=light.edge1_cdf.x;ld=cone_sample(light.position_area.xyz,om,pc);}
            if(pc>0 && lim>0 && !occluded(q+ld*0.0003,ld,max(0.0002,lim-0.0006))){
                float reach=(light.position_area.w==-1)?lim:50.;
                float pl=(1-p_env)*light.edge2_probability.w*pc;
                acc+=sig_s*ps*exp(-ext*reach)*light.emission_material.rgb/max(mis==1?pl:pl+ps,1e-8);
            }
        }else{
            float su=sqrt(random()),v=random();float3 lp=light.position_area.xyz+light.edge1_cdf.xyz*(1-su)+light.edge2_probability.xyz*(su*v);float3 ld=lp-q;float len=length(ld);ld/=max(len,1e-8);
            Material lm=material_at((uint)light.emission_material.w,lp);float3 ln=normalize(cross(light.edge1_cdf.xyz,light.edge2_probability.xyz));
            if((lm.optical.w<0.5 || dot(ln,-ld)>0) && !occluded(q+ld*0.0003,ld,max(0.0002,len-0.0006))){
                float pl=(1-p_env)*emitter_pdf(q,lp,light);
                acc+=sig_s*ps*exp(-ext*len)*lm.color_rough.rgb*lm.optical.y*spot(lm,dot(ln,-ld))/max(mis==1?pl:pl+ps,1e-8);
            }
        }
    }
    return acc;
}
[numthreads(8,8,1)]void trace(uint3 tid:SV_DispatchThreadID){
    uint2 p=tid.xy;if(any(p>=dimensions.xy))return;
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
    Hit guide;float3 go,gd;bool gvalid;camera_ray(float2(p)+0.5+jitter.xy,false,go,gd,gvalid);
    if(intersect(go,gd,1e4,guide,false,true)){
        Material m=material_at(guide.material,guide.p);apply_maps(guide,m);object_id[p]=guide.material+1;
        if(((uint)m.lobes.w&64u)!=0u){ // layered coverage guide (track 11); else keeps the original writes verbatim
            float pc=m.coating.x,sh=m.coating.z,wb=(1.0-pc)*(1.0-sh);
            normal_roughness[p]=float4(guide.n,pc>0.0?m.coating.y:m.color_rough.a);
            float3 dalb=m.color_rough.rgb*(1-m.lobes.x)*(1-m.lobes.z)*wb;
            if(m.effects.y>0.0)dalb*=1.0-film_favg(m.effects.x);
            if(m.effects.z>0.0)dalb*=(m.optical.z>0.5?m.secondary.w:0.0);
            diffuse_albedo[p]=float4(dalb,1);
            float nvg=abs(dot(-gd,guide.n));
            specular_albedo[p]=float4(pc*fresnel(nvg,0.04.xxx)+sh*fresnel(nvg,f0_of(m))+wb*fresnel(nvg,f0_of(m)),1);
        }else{
            normal_roughness[p]=float4(guide.n,m.color_rough.a);
            diffuse_albedo[p]=float4(m.color_rough.rgb*(1-m.lobes.x)*(1-m.lobes.z),1);
            specular_albedo[p]=float4(fresnel(abs(dot(-gd,guide.n)),f0_of(m)),1);
        }
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
    float3 batch=0;
    for(uint sample_id=0;sample_id<sampling.y;sample_id++){
        // The base changes on every reset, so a moving camera never reuses the previous frame's points.
        sequence_index=scene_info.z+sampling.x+sample_id;dimension_index=0;
        // RR and the temporal upscalers need every sample of a frame at that frame's jitter.
        float2 subpixel=(scene_info.w&1)!=0?jitter.xy:filter_offset(float2(random(),random()));
        float3 origin,direction;bool lens_valid;camera_ray(float2(p)+0.5+subpixel,true,origin,direction,lens_valid);
        cone_width=pixel_width;cone_spread=pixel_angle;
        float3 throughput=lens_valid?1:0,radiance=0,direct=0;float4 first_diffuse=0;float3 first_specular=0;uint portal_crossings=0;bool first_spec=false,previous_delta=true;float previous_pdf=0;float3 previous_position=origin;float3 absorption=medium_a.rgb; // ambient medium (track 12): zero keeps every pre-track-12 path bit-identical
        // Nested dielectrics (track 10): the volumes this path is inside, in the order they
        // were entered, so a boundary bends by the two media it actually touches. Slot 0 is
        // the ambient medium (air) and never matches a material id.
        // Only the ids are stored: this kernel's register budget decides its occupancy and three
        // 8-slot arrays cost every pixel, so the optical index and the Beer-Lambert colour come
        // back from the material table on demand (apply_maps never touches optical.x, so a push
        // keeps the maps of the boundary being crossed while a pop takes the table value of the
        // medium it returns to, since that medium's maps belong to its own surface).
        uint med_id[MED_DEPTH];uint med_n=1u;
        [unroll] for(uint ms=0;ms<MED_DEPTH;ms++)med_id[ms]=0xffffffffu;
        for(uint bounce=0;bounce<sampling.z && lens_valid;bounce++){
            bounces_used+=1;
            uint mis=(scene_info.w>>4)&3;
            // Media context this bounce (track 12): the containing medium is the top of the
            // interior list (ambient = the constants when the list holds only the sentinel).
            // cur_s drives the free-flight race; extinction above the sampling bound (absorption
            // is the Beer-Lambert/sigma_a of the containing medium the push/pop below keeps
            // current) is what every segment and shadow ray multiplies by.
            uint cur_id=0xffffffffu;[unroll]for(uint s=0;s<MED_DEPTH;s++){if(s+1u==med_n)cur_id=med_id[s];}
            float3 cur_s=cur_id==0xffffffffu?medium_s.rgb:materials[cur_id].volume_s.rgb;
            float sbar=max(max(cur_s.x,cur_s.y),cur_s.z);
            bool vol=sbar>0.0;
            bool med=vol||max(max(medium_a.x,medium_a.y),medium_a.z)>0||max(max(medium_s.x,medium_s.y),medium_s.z)>0; // some medium touches this path segment
            // Light selection: the HDRI and the emissive triangles share NEE half and half.
            bool env_nee=hdri_on() && mis!=2,tri_nee=scene_info.x>0 && mis!=2;
            float p_env=env_nee?(tri_nee?0.5:1):0;
            Hit h;bool hit=intersect(origin,direction,1e4,h,false,bounce==0);
            float3 ext=absorption+cur_s-sbar; // arrival extinction minus sampling bound: what media segments multiply by (equals absorption when the bound is uniform)
            if(vol){
                // Free-flight race ahead of the miss test: sky-pointing rays through fog scatter
                // along the way, which is what makes beams and haze visible. The distance is
                // sampled with the conservative scalar sigma_bar and raced against the first
                // thing on the ray: a surface hit, or the environment / sun reach on a miss.
                float dfree=-log(max(random(),1e-30))/sbar;
                if(dfree<(hit?h.t:500.)){
                    // Scatter before any surface: arrival extinction divided by the sampling
                    // survival = (sigma_s_c/sigma_bar)·exp(-ext·d) per channel; direct scattering
                    // by NEE at the event point; isotropic phase continuation from there (the
                    // race reruns at the new origin, so multiple scattering comes for free).
                    float3 q=origin+direction*dfree;
                    throughput*=exp(-ext*dfree)*(cur_s/sbar);
                    float3 ve=vol_nee(q,absorption,cur_s);
                    radiance+=throughput*ve;if(bounce==0)direct+=throughput*ve;
                    float pdf;direction=vol_dir(direction,pdf);
                    origin=q+direction*0.0005;previous_position=origin;previous_delta=false;previous_pdf=pdf;
                    cone_width+=cone_spread*dfree;
                    continue;
                }
            }
            if(!hit){
                if(bounce==0)first_diffuse.a=guide.material==0xffffffffu?1:0;
                // The camera sees the background colour when the HDRI is hidden from it.
                float3 env=bounce==0 && hdri_on() && ((uint)environment_map.z&2u)==0?background.rgb:environment_radiance(direction);
                float weight=1;if(hdri_on() && bounce>0 && !previous_delta)weight=mis==1?0:mis==2?1:previous_pdf/(previous_pdf+p_env*env_pdf(direction));
                float3 e=throughput*env*weight*(med?exp(-ext*500.):float3(1.,1.,1.)); // the environment sits at the far plane
                // Suns (they lead the emitter table): seen inside their disk, weighted against
                // light sampling as triangles are; a hidden one only from bounce 1 on.
                for(uint i=0;i<(uint)lights.x;i++){Emitter sun=emitters[i];
                    // Inside the disk by sin² (from a cross product), which keeps its precision.
                    float3 x=cross(direction,sun.position_area.xyz);float om=sun.edge1_cdf.x;
                    if(dot(direction,sun.position_area.xyz)<=0 || dot(x,x)>om*(2-om))continue;
                    if(bounce==0 && (((uint)materials[(uint)sun.emission_material.w].lobes.w&2u)!=0))continue;
                    float w=1;if(bounce>0 && !previous_delta)w=mis==1?0:mis==2?1:previous_pdf/(previous_pdf+(1-p_env)*sun.edge2_probability.w/(2*PI*sun.edge1_cdf.x));
                    e+=throughput*sun.emission_material.rgb*w*(med?exp(-ext*50.):float3(1.,1.,1.));} // suns sit at their nominal 50 units
                radiance+=e;if(bounce<=1)direct+=e;break;}
            if(vol)throughput*=exp(-ext*h.t); // no event before the surface: arrival ÷ survival
            else throughput*=exp(-absorption*h.t); // old text verbatim: layer- and media-free paths
            cone_width+=cone_spread*h.t;
            // Portal: continue from the matching point of the paired object, without a bounce.
            // Light sampling cannot see through portals, so emission found after one has weight 1.
            Material m=material_at(h.material,h.p);
            if(((uint)m.lobes.w&4u)!=0 && portal_crossings<16){
                portal_crossings++;
                // The map lands on the far face of the pair; inside a portal object the ray just
                // passes through, so it leaves through the pair's near face (portals work both ways).
                if(!h.front){origin=h.p+direction*0.001;bounce--;continue;}
                Material pm=materials[(uint)m.secondary.w];
                float4 P=float4(h.p,1),D=float4(direction,0);
                direction=normalize(float3(dot(pm.color_rough,D),dot(pm.lobes,D),dot(pm.optical,D)));
                origin=float3(dot(pm.color_rough,P),dot(pm.lobes,P),dot(pm.optical,P))+direction*0.001;
                previous_delta=true;bounce--;continue;
            }
            float3 emitted=m.color_rough.rgb*m.optical.y;
            if(m.optical.w>0.5 && !h.front)emitted=0;
            emitted*=spot(m,-dot(direction,h.n));
            // Emission keeps the flat colour: light sampling has no texture lookup.
            apply_maps(h,m);
            if(bounce==0 && sample_albedo){
                if(((uint)m.lobes.w&64u)!=0u){ // same layer coverage as the layered eval (track 11)
                    float pc=m.coating.x,sh=m.coating.z,wb=(1.0-pc)*(1.0-sh);
                    float3 dalb=m.color_rough.rgb*(1-m.lobes.x)*(1-m.lobes.z)*wb;
                    if(m.effects.y>0.0)dalb*=1.0-film_favg(m.effects.x);
                    if(m.effects.z>0.0)dalb*=(m.optical.z>0.5?m.secondary.w:0.0);
                    float ng=abs(dot(-direction,h.n));
                    first_diffuse=float4(dalb,h.material==guide.material?1:0);
                    first_specular=pc*fresnel(ng,0.04.xxx)+sh*fresnel(ng,f0_of(m))+wb*fresnel(ng,f0_of(m));
                }else{
                    first_diffuse=float4(m.color_rough.rgb*(1-m.lobes.x)*(1-m.lobes.z),h.material==guide.material?1:0);
                    first_specular=fresnel(abs(dot(-direction,h.n)),f0_of(m));
                }
            }
            // Emission found by BSDF sampling: MIS weight, or all / nothing in the single-strategy modes.
            float weight=1;if(h.emitter!=0 && bounce>0 && !previous_delta)weight=mis==1?0:mis==2?1:previous_pdf/(previous_pdf+(1-p_env)*emitter_pdf(previous_position,h.p,emitters[h.emitter-1]));
            float3 e=throughput*emitted*weight;radiance+=e;if(bounce<=1)direct+=e;
            if(((uint)m.lobes.w&1u)!=0)break;
            if(env_nee && random()<p_env){
                float pe;float3 ld=sample_env(pe);float nl=dot(h.n,ld),pdf;float3 f=eval_bsdf(m,h.n,-direction,ld,h.p,pdf);
                if(nl>0 && pe>0 && !occluded(h.p+h.ng*0.0003,ld,1e4)){
                    float3 e=throughput*f*nl*(med?exp(-(absorption+cur_s)*500.):float3(1.,1.,1.))*environment_radiance(ld)/max(mis==1?p_env*pe:p_env*pe+pdf,1e-8);radiance+=e;if(bounce==0)direct+=e;
                }
            }else if(tri_nee){
            Emitter light=select_emitter(random());
            if(light.position_area.w<0){
                // Sphere or sun: a direction in the cone it covers, then a shadow ray up to the
                // sphere's true surface (its inscribed mesh lies beyond) or to infinity.
                float om=0,pc=0,limit=1e4;float3 ld=float3(0,1,0);
                if(light.position_area.w==-1){
                    float3 c=light.position_area.xyz-h.p;float d2=dot(c,c),r=light.edge1_cdf.x;om=sphere_om(d2,r);
                    if(om>0){ld=cone_sample(c*rsqrt(d2),om,pc);float b=dot(c,ld);limit=b-sqrt(max(0,r*r-(d2-b*b)));}
                }else{om=light.edge1_cdf.x;ld=cone_sample(light.position_area.xyz,om,pc);}
                float nl=dot(h.n,ld),pdf;float3 f=eval_bsdf(m,h.n,-direction,ld,h.p,pdf);
                if(pc>0 && nl>0 && !occluded(h.p+h.ng*0.0003,ld,max(0.0002,limit-0.0006))){
                    float pl=(1-p_env)*light.edge2_probability.w*pc;float3 e=throughput*f*nl*(med?exp(-(absorption+cur_s)*(light.position_area.w==-1?limit:50.)):float3(1.,1.,1.))*light.emission_material.rgb/max(mis==1?pl:pl+pdf,1e-8);radiance+=e;if(bounce==0)direct+=e;
                }
            }else{
            float su=sqrt(random()),v=random();float3 lp=light.position_area.xyz+light.edge1_cdf.xyz*(1-su)+light.edge2_probability.xyz*(su*v);float3 ld=lp-h.p;float len=length(ld);ld/=max(len,1e-8);
            Material lm=material_at((uint)light.emission_material.w,lp);float3 ln=normalize(cross(light.edge1_cdf.xyz,light.edge2_probability.xyz));
            float nl=dot(h.n,ld),pdf;float3 f=eval_bsdf(m,h.n,-direction,ld,h.p,pdf);
            if(nl>0 && (lm.optical.w<0.5 || dot(ln,-ld)>0)){
                if(!occluded(h.p+h.ng*0.0003,ld,max(0.0002,len-0.0006))){
                    float pl=(1-p_env)*emitter_pdf(h.p,lp,light);float3 e=throughput*f*nl*(med?exp(-(absorption+cur_s)*len):float3(1.,1.,1.))*(lm.color_rough.rgb*lm.optical.y*spot(lm,dot(ln,-ld)))/max(mis==1?pl:pl+pdf,1e-8);radiance+=e;if(bounce==0)direct+=e;
                }
            }}}
            float3 next;bool delta=false;float bsdf_pdf=0;
            if(random()<m.lobes.z){
                // Nested dielectrics (track 10): the medium the path is in comes off the
                // interior list, so a liquid's wall refracts against the glass it touches, an
                // ice cube in a drink bends the tea behind it, and the drink's Beer-Lambert
                // absorption is put back after leaving the ice. Fixed arrays read with
                // unrolled constant-index loops, because DXC has no dynamically indexed local
                // memory. A surface of a volume the path never entered (modelling where a
                // liquid pierces the glass wall, or a mesh that is not a closed manifold)
                // passes through unchanged; the general priority-based answer is the Schmidt
                // and Budge interior list (Journal of Graphics Tools 7(2), 2002), kept with
                // the inside test for a path that starts inside a volume on the not-done list.
                uint hid=h.material;float mior=m.optical.x;
                uint top_id=0xffffffffu,under=0u,below_id=0xffffffffu;
                [unroll] for(uint s=0;s<MED_DEPTH;s++){if(s+1u==med_n)top_id=med_id[s];if(s<med_n&&med_id[s]==hid)under=s;}
                if(under!=0u){[unroll] for(uint s=0;s<MED_DEPTH;s++){if(s+1u==under)below_id=med_id[s];}}
                bool entered=h.front&&under==0u,leaving=!h.front&&under!=0u;
                float inc=mior,tra=mior;
                if(entered)inc=top_id==0xffffffffu?medium_a.w:materials[top_id].optical.x;
                if(leaving)tra=below_id==0xffffffffu?medium_a.w:materials[below_id].optical.x;
                float eta=inc/tra;float cosi=saturate(dot(-direction,h.n));float cost2=1-eta*eta*(1-cosi*cosi);float r0=(inc-tra)/(inc+tra);float fr=r0*r0+(1-r0*r0)*pow(1-(h.front?cosi:sqrt(max(0,cost2))),5);
                bool reflected=cost2<=0 || random()<fr;
                next=reflected?reflect(direction,h.n):refract(direction,h.n,eta);
                if(!reflected){
                    // Only a transmission event edits the list and the medium: a total internal
                    // reflection off the inner wall of a glass keeps the path in the same medium,
                    // and so does a surface of a volume the path never entered. The medium being
                    // returned to is looked up by id, so its own surface maps (which belong to
                    // that surface, not to the boundary being crossed here) do not tint the
                    // segment between two volumes.
                    if(entered){if(med_n<MED_DEPTH){[unroll] for(uint s=0;s<MED_DEPTH;s++)if(s==med_n)med_id[s]=hid;med_n=min(med_n+1u,MED_DEPTH);}absorption=has_volume(m)?m.volume_a:-log(max(m.color_rough.rgb,0.001));}
                    else if(leaving){med_n=under;if(below_id==0xffffffffu)absorption=medium_a.rgb;else absorption=has_volume(materials[below_id])?materials[below_id].volume_a:-log(max(materials[below_id].color_rough.rgb,0.001));}
                    throughput*=eta*eta;
                }
                if(m.color_rough.a>0.001){float3 pert=cosine_sample(reflected?h.n:-h.n);next=normalize(lerp(next,pert,m.color_rough.a*m.color_rough.a));}
                delta=true;
            }else{
                bool layered=((uint)m.lobes.w&64u)!=0u&&m.lobes.z<0.5; // pack bit6: branch guard without extra loads
                float ps=spec_probability(m);
                if(layered){
                    // Lobe mixture matching eval_bsdf's weights (track 11): coat, then sheen, then
                    // the base pair under (1-pc)(1-ps); the anisotropic base GGX samples the
                    // stretched-hemisphere sampler (the disk projection of the (at,ab,1) ellipsoid,
                    // which at at=ab=a is the sampler below rewritten).
                    float pc=m.coating.x,sh=m.coating.z,wbase=(1.0-pc)*(1.0-sh),an=m.weave.x;
                    float3 Tr,Br;
                    if(an>0.0||m.effects.z>0.0){
                        float2 du;float3 tb,bb;box_map(h.p,h.n,1.0,du,tb,bb);
                        tb=normalize(tb);bb=normalize(bb);
                        float rot=radians(m.weave.y),cr=cos(rot),sr=sin(rot);
                        Tr=cr*tb+sr*bb;Br=cross(h.n,Tr);
                    }
                    float sel=random();
                    if(sel<pc){
                        float ac=max(1e-3,m.coating.y*m.coating.y);float u=random(),phi=2*PI*random();float r=sqrt(u/(1.0-u));
                        float3 hv=normalize(basis(h.n,float3(ac*r*cos(phi),ac*r*sin(phi),1.0)));next=reflect(direction,hv);if(bounce==0)first_spec=true;cone_spread+=0.5*ac;
                    }else if(sel<pc+sh*(1.0-pc)){
                        next=cosine_sample(h.n);if(bounce==0)first_spec=true;cone_spread+=0.5;
                    }else if(sel<pc+sh*(1.0-pc)+wbase*ps){
                        if(an>0.0){
                            float a=max(0.001,m.color_rough.a*m.color_rough.a),asp=sqrt(max(0.01,1.0-0.9*an)),at=a/asp,ab=a*asp;
                            float u=random(),phi=2*PI*random();float r=sqrt(u/(1.0-u));
                            float3 hv=normalize(at*r*cos(phi)*Tr+ab*r*sin(phi)*Br+h.n);next=reflect(direction,hv);if(bounce==0)first_spec=true;cone_spread+=0.5*(at*ab+1.0)*0.5;
                        }else{
                            float a=max(0.001,m.color_rough.a*m.color_rough.a);float u=random(),phi=2*PI*random();float c=sqrt((1-u)/(1+(a*a-1)*u));float3 hvec=basis(h.n,float3(sqrt(max(0,1-c*c))*cos(phi),sqrt(max(0,1-c*c))*sin(phi),c));next=reflect(direction,hvec);if(bounce==0)first_spec=true;cone_spread+=0.5*a;
                        }
                    }else{next=cosine_sample(h.n);cone_spread+=0.5;}
                }else if(random()<ps){float a=max(0.001,m.color_rough.a*m.color_rough.a);float u=random(),phi=2*PI*random();float c=sqrt((1-u)/(1+(a*a-1)*u));float3 hvec=basis(h.n,float3(sqrt(max(0,1-c*c))*cos(phi),sqrt(max(0,1-c*c))*sin(phi),c));next=reflect(direction,hvec);if(bounce==0)first_spec=true;
                    cone_spread+=0.5*a;
                }else{next=cosine_sample(h.n);cone_spread+=0.5;}
                if(dot(h.n,next)<=0)break;
                float3 bsdf=eval_bsdf(m,h.n,-direction,next,h.p,bsdf_pdf);throughput*=bsdf*abs(dot(h.n,next))/max(bsdf_pdf,1e-7);
            }
            if(bounce==0 && delta)first_spec=true;
            previous_pdf=bsdf_pdf;previous_delta=delta;previous_position=h.p;
            direction=normalize(next);origin=h.p+h.ng*(dot(direction,h.ng)>0?0.0003:-0.0003);
            if(bounce>=3){float survival=clamp(max(throughput.r,max(throughput.g,throughput.b)),0.05,0.95);if(random()>survival)break;throughput/=survival;}
        }
        if(!all(isfinite(radiance))){radiance=0;direct=0;}
        radiance=max(radiance,0);if(options.x>0 && sampling.x>4){float l=luminance(radiance);float scale=min(1,options.x*max(luminance(mean),0.05)/max(l,1e-6));radiance*=scale;direct*=scale;}
        float count=sampling.x+sample_id+1;float delta_l=luminance(radiance)-luminance(mean);mean+=(radiance-mean)/count;m2+=delta_l*(luminance(radiance)-luminance(mean));
        dmean+=(direct-dmean)/count;smean+=((first_spec?radiance:0)-smean)/count;batch+=radiance/sampling.y;
        damean+=(first_diffuse-damean)/count;samean+=(first_specular-samean)/count;
    }
    if(sample_albedo){diffuse_albedo[p]=damean;specular_albedo[p]=float4(samean,1);}
    trace_stats[p]=float2(bounces_used/max(1,sampling.y),proceed_steps);
    noisy[p]=float4(batch,1);accumulation[p]=float4(mean,max(0,m2));direct_light[p]=float4(dmean,1);specular_light[p]=float4(smean,1);
}

