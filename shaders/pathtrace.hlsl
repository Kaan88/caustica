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
// tangent: the brushing direction of an anisotropic material (layers.w > 0, else zero), not
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
bool intersect(float3 o,float3 d,float limit,out Hit h,bool shadow,bool camera_ray){
    RayDesc ray;ray.Origin=o;ray.Direction=d;ray.TMin=0.0001;ray.TMax=limit;
    // Nonopaque geometry allows the light quad and optional glass to be ignored for shadow rays.
    RayQuery<RAY_FLAG_NONE> q;q.TraceRayInline(scene,RAY_FLAG_NONE,255,ray);
    while(q.Proceed()){proceed_steps++;
        uint m=candidate_material(q.CandidateInstanceID(),q.CandidatePrimitiveIndex());
        if(flagged(m,8u) && alpha_hole(q.CandidateInstanceID(),q.CandidatePrimitiveIndex(),q.CandidateTriangleBarycentrics(),m))continue;
        if(camera_ray && (((uint)materials[m].lobes.w&2u)!=0))continue;
        if(!shadow || jitter.w>0.5 || materials[m].lobes.z<0.5)q.CommitNonOpaqueTriangleHit();
    }
    h=(Hit)0;
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
    if(materials[h.material].layers.w>0){
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
    RayQuery<RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH> q;q.TraceRayInline(scene,RAY_FLAG_NONE,255,ray);
    while(q.Proceed()){proceed_steps++;uint m=candidate_material(q.CandidateInstanceID(),q.CandidatePrimitiveIndex());
        if(flagged(m,8u) && alpha_hole(q.CandidateInstanceID(),q.CandidatePrimitiveIndex(),q.CandidateTriangleBarycentrics(),m))continue;
        if(jitter.w>0.5 || materials[m].lobes.z<0.5)q.CommitNonOpaqueTriangleHit();}
    return q.CommittedStatus()==COMMITTED_TRIANGLE_HIT;
}
Material material_at(uint id,float3 p){
    Material m=materials[id];uint pattern=(uint)m.optical.z;float t=0;
    if(pattern==1)t=fmod(abs(floor(p.x*m.secondary.w)+floor(p.z*m.secondary.w)),2);
    if(pattern==2)t=step(0,sin((p.x+p.z)*m.secondary.w*PI));
    // Marble: soft veins where a turbulent sine crosses zero, at most 75% secondary color.
    if(pattern==3){float s=m.secondary.w;float vein=sin((p.x+p.z)*s*0.6+3*sin(p.y*3+p.z*2)+1.5*sin(p.x*5-p.z*2)+0.6*sin(p.x*13+p.y*11)+0.3*sin(p.z*27-p.y*21));t=0.75*pow(1-abs(vein),6);}
    m.color_rough.rgb=lerp(m.color_rough.rgb,m.secondary.rgb,t);return m;
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
Material medium_material(uint64_t s,int top){return materials[(medium_at(s,top)&0x1fffu)-1];}
float medium_ior(uint64_t s,int top){return top>=0?medium_material(s,top).optical.x:1;}
float3 medium_absorption(uint64_t s,int top){return top>=0?-log(max(medium_material(s,top).color_rough.rgb,0.001)):float3(0,0,0);}
float ggx_d(float nh,float a){float d=nh*nh*(a*a-1)+1;return a*a/(PI*d*d);}
float g1(float nv,float a){return 2*nv/max(1e-6,nv+sqrt(a*a+(1-a*a)*nv*nv));}
float3 f0_of(Material m){float f=pow((m.optical.x-1)/(m.optical.x+1),2);return lerp(lerp(f.xxx,1.0.xxx,m.lobes.y),m.color_rough.rgb,m.lobes.x);}
float spec_probability(Material m){return clamp(0.25+0.5*m.lobes.x+0.25*m.lobes.y,0.1,0.95);}
// Layers (track 11, glTF's KHR_materials_clearcoat / _sheen model): a sheen over the base and
// a clear coat over both. layers = (coat, coat roughness, sheen roughness, anisotropy), sheen.rgb
// = the sheen colour (times its strength). Each layer dims what lies below by the energy it takes
// from the view direction, as glTF's spec has it: the coat by its compensated albedo (glTF: its
// Fresnel), the sheen by its directional albedo (layers.hlsli). Dimming by both directions (reciprocal) lost 8–12% of a
// white base in the furnace; the view-only form keeps it, and NEE and BSDF sampling still
// evaluate the same function, so MIS stays consistent.
float max3(float3 c){return max(c.r,max(c.g,c.b));}
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
float3 eval_bsdf(Material m,float3 n,float3 t,float3 v,float3 l,out float pdf){
    float nl=saturate(dot(n,l)),nv=saturate(dot(n,v));float3 h=normalize(v+l);float nh=saturate(dot(n,h)),vh=saturate(dot(v,h));float a=max(0.001,m.color_rough.a*m.color_rough.a);float d=ggx_d(nh,a);float3 f0=f0_of(m),f=fresnel(vh,f0);float ps=spec_probability(m);
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
    float3 r=diff+f*d*gv*gl/max(4*nv*nl,1e-6)*(1+f0*(1/ess-1));
    // The sheen is sampled with the diffuse lobe's cosine directions (it adds no pdf term).
    float s=max3(m.sheen.rgb);
    if(s>0){float sr=m.layers.z;r=r*(1-s*min(1,sheen_albedo(nv,sr)))+m.sheen.rgb*sheen_lobe(sr,nh,nv,nl);}
    float cc=m.layers.x;
    if(cc>0){
        float ac=max(0.001,m.layers.y*m.layers.y),dc=ggx_d(nh,ac);
        // The coat is compensated in full (1/E), and the layers below are dimmed by exactly the
        // albedo that leaves the coat (coat_albedo, = coat_fresnel(nv) when smooth): F(nv) nears 1
        // at grazing, where a rough coat's facets mostly see far smaller F(vh).
        r=r*(1-cc*coat_albedo(nv,m.layers.y))+cc*coat_fresnel(vh)*dc*g1(nv,ac)*g1(nl,ac)/max(4*nv*nl,1e-6)/ggx_albedo(nv,m.layers.y);
        pdf=lerp(pdf,dc*nh/max(4*vh,1e-6),coat_probability(m));
    }
    pdf*=1-m.lobes.z;return (1-m.lobes.z)*r;
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
        Material m=material_at(guide.material,guide.p);apply_maps(guide,m);normal_roughness[p]=float4(guide.n,m.color_rough.a);object_id[p]=guide.material+1;
        diffuse_albedo[p]=float4(m.color_rough.rgb*(1-m.lobes.x)*(1-m.lobes.z),1);
        specular_albedo[p]=float4(fresnel(abs(dot(-gd,guide.n)),f0_of(m)),1);
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
        float3 throughput=lens_valid?1:0,radiance=0,direct=0;float4 first_diffuse=0;float3 first_specular=0;uint portal_crossings=0,false_hits=0;uint64_t media=0;bool first_spec=false,previous_delta=true;float previous_pdf=0;float3 previous_position=origin;float3 absorption=0;
        for(uint bounce=0;bounce<sampling.z && lens_valid;bounce++){
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
                media=h.front?medium_push(media,e):medium_remove(media,e);
                throughput*=exp(-absorption*h.t);cone_width+=cone_spread*h.t;
                false_hits++;origin=h.p-h.ng*0.0003;
            }
            if(!found){
                if(bounce==0)first_diffuse.a=guide.material==0xffffffffu?1:0;
                // The camera sees the background colour when the HDRI is hidden from it.
                float3 env=bounce==0 && hdri_on() && ((uint)environment_map.z&2u)==0?background.rgb:environment_radiance(direction);
                float weight=1;if(hdri_on() && bounce>0 && !previous_delta)weight=mis==1?0:mis==2?1:previous_pdf/(previous_pdf+p_env*env_pdf(direction));
                float3 e=throughput*env*weight;
                // Suns (they lead the emitter table): seen inside their disk, weighted against
                // light sampling as triangles are; a hidden one only from bounce 1 on.
                for(uint i=0;i<(uint)lights.x;i++){Emitter sun=emitters[i];
                    // Inside the disk by sin² (from a cross product), which keeps its precision.
                    float3 x=cross(direction,sun.position_area.xyz);float om=sun.edge1_cdf.x;
                    if(dot(direction,sun.position_area.xyz)<=0 || dot(x,x)>om*(2-om))continue;
                    if(bounce==0 && (((uint)materials[(uint)sun.emission_material.w].lobes.w&2u)!=0))continue;
                    float w=1;if(bounce>0 && !previous_delta)w=mis==1?0:mis==2?1:previous_pdf/(previous_pdf+(1-p_env)*sun.edge2_probability.w/(2*PI*sun.edge1_cdf.x));
                    e+=throughput*sun.emission_material.rgb*w;}
                radiance+=e;if(bounce<=1)direct+=e;break;}
            throughput*=exp(-absorption*h.t);cone_width+=cone_spread*h.t;
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
            if(bounce==0 && sample_albedo){first_diffuse=float4(m.color_rough.rgb*(1-m.lobes.x)*(1-m.lobes.z),h.material==guide.material?1:0);first_specular=fresnel(abs(dot(-direction,h.n)),f0_of(m));}
            // Emission found by BSDF sampling: MIS weight, or all / nothing in the single-strategy modes.
            float weight=1;if(h.emitter!=0 && bounce>0 && !previous_delta)weight=mis==1?0:mis==2?1:previous_pdf/(previous_pdf+(1-p_env)*emitter_pdf(previous_position,h.p,emitters[h.emitter-1]));
            float3 e=throughput*emitted*weight;radiance+=e;if(bounce<=1)direct+=e;
            if(((uint)m.lobes.w&1u)!=0)break;
            // Light sampling: each strategy picks a direction ld with its radiance le, its pdf pl
            // (solid angle) and a shadow-ray limit; one shared BSDF evaluation follows, so
            // eval_bsdf is inlined once here instead of once per strategy (code size and
            // registers cost even scenes that never take the anisotropic branch).
            bool nee=false;float3 ld=float3(0,1,0),le=0;float pl=0,limit=1e4;
            if(env_nee && random()<p_env){
                float pe;ld=sample_env(pe);
                if(pe>0){le=environment_radiance(ld);pl=p_env*pe;nee=true;}
            }else if(tri_nee){
            Emitter light=select_emitter(random());
            if(light.position_area.w<0){
                // Sphere or sun: a direction in the cone it covers, then a shadow ray up to the
                // sphere's true surface (its inscribed mesh lies beyond) or to infinity.
                float om=0,pc=0;
                if(light.position_area.w==-1){
                    float3 c=light.position_area.xyz-h.p;float d2=dot(c,c),r=light.edge1_cdf.x;om=sphere_om(d2,r);
                    if(om>0){ld=cone_sample(c*rsqrt(d2),om,pc);float b=dot(c,ld);limit=b-sqrt(max(0,r*r-(d2-b*b)));}
                }else{om=light.edge1_cdf.x;ld=cone_sample(light.position_area.xyz,om,pc);}
                if(pc>0){le=light.emission_material.rgb;pl=(1-p_env)*light.edge2_probability.w*pc;limit=max(0.0002,limit-0.0006);nee=true;}
            }else{
            float su=sqrt(random()),v=random();float3 lp=light.position_area.xyz+light.edge1_cdf.xyz*(1-su)+light.edge2_probability.xyz*(su*v);ld=lp-h.p;float len=length(ld);ld/=max(len,1e-8);
            Material lm=material_at((uint)light.emission_material.w,lp);float3 ln=normalize(cross(light.edge1_cdf.xyz,light.edge2_probability.xyz));
            if(lm.optical.w<0.5 || dot(ln,-ld)>0){le=lm.color_rough.rgb*lm.optical.y*spot(lm,dot(ln,-ld));pl=(1-p_env)*emitter_pdf(h.p,lp,light);limit=max(0.0002,len-0.0006);nee=true;}
            }}
            float nl=dot(h.n,ld);
            if(nee && nl>0){
                float pdf;float3 f=eval_bsdf(m,h.n,h.tangent,-direction,ld,pdf);
                if(!occluded(h.p+h.ng*0.0003,ld,limit)){float3 e=throughput*f*nl*le/max(mis==1?pl:pl+pdf,1e-8);radiance+=e;if(bounce==0)direct+=e;}
            }
            float3 next;bool delta=false;float bsdf_pdf=0;
            if(random()<m.lobes.z){
                // The media on both sides: entering, the current one and this; leaving, this and
                // the one the stack falls back to (air when empty, as before nesting).
                uint e=medium_entry(h.material,m.lobes.w);uint64_t after=h.front?medium_push(media,e):medium_remove(media,e);
                float n1=h.front?medium_ior(media,medium_top(media)):m.optical.x,n2=h.front?m.optical.x:medium_ior(after,medium_top(after));
                float eta=n1/n2;float fr=fresnel_dielectric(saturate(dot(-direction,h.n)),eta);
                bool reflected=random()<fr;
                next=reflected?reflect(direction,h.n):refract(direction,h.n,eta);
                if(!reflected){media=after;absorption=medium_absorption(media,medium_top(media));throughput*=eta*eta;}
                if(m.color_rough.a>0.001){float3 pert=cosine_sample(reflected?h.n:-h.n);next=normalize(lerp(next,pert,m.color_rough.a*m.color_rough.a));}
                delta=true;
            }else{
                float pc=coat_probability(m),ps=spec_probability(m);
                if(pc>0 && random()<pc){float a=max(0.001,m.layers.y*m.layers.y);next=ggx_sample(h.n,direction,a);if(bounce==0)first_spec=true;cone_spread+=0.5*a;}
                else if(random()<ps){float a=max(0.001,m.color_rough.a*m.color_rough.a);
                    if(m.layers.w>0){float3 T,B;aniso_frame(h.n,h.tangent,m.sheen.w,T,B);next=ggx_sample_aniso(h.n,T,B,direction,aniso_alpha(m));}else next=ggx_sample(h.n,direction,a);
                    if(bounce==0)first_spec=true;
                    cone_spread+=0.5*a;
                }else{next=cosine_sample(h.n);cone_spread+=0.5;}
                if(dot(h.n,next)<=0)break;
                float3 bsdf=eval_bsdf(m,h.n,h.tangent,-direction,next,bsdf_pdf);throughput*=bsdf*abs(dot(h.n,next))/max(bsdf_pdf,1e-7);
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

