// Track 15 part 2: bidirectional path tracing (BDPT) and progressive photon mapping beside the
// path tracer, built from its pieces (intersect, eval_bsdf, the light table). See engine-notes
// "BDPT and photon mapping". Both use white noise, see glass as solid in shadow rays, ignore
// portals, nested-dielectric false hits and the light-sampling mode, and use a box pixel filter.
// Media (track 12's homogeneous media in transmissive objects and the fog box) are walked by
// free-flight sampling as trace walks them: BDPT gets scatter vertices with HG phase functions,
// photon mapping volume photons (see engine-notes "Media in BDPT and photon mapping").
#include "pathtrace.hlsl"
// BDPT: light-tracing splats (rgb floats per render pixel at integrator.z). Photon mapping: a
// counter, HASH list heads, then 32-byte photons (position, incoming direction, power, next).
RWByteAddressBuffer aux:register(u3,space2);
static const uint HASH=1u<<20,PHOTON_BASE=16+4*HASH;
// A path vertex. info: material id (bits 0-23) and the flags below. beta: the subpath's
// throughput arriving there; pdf_fwd / pdf_rev: area densities of reaching it from the
// subpath's own side / from the other side (PBRT-v3's BDPT notation).
// V_MEDIUM: a scatter point in a medium (no surface: n = 0, g in rough_metal.x). medium: the
// stack's top entry (medium_entry, 0 = none, so the fog box applies) on the side the walk
// arrived from; connections leave on that side (glass blocks them), so it is the connection's.
static const uint V_DELTA=1u<<24,V_TEXTURED=1u<<25,V_NO_BSDF=1u<<26,V_LIGHT=1u<<27,V_CAMERA=1u<<28,V_FRONT=1u<<29,V_MEDIUM=1u<<30;
struct BVertex{float3 p;uint info;float3 n;float pdf_fwd;float3 beta;float pdf_rev;float3 albedo;uint emitter;float3 tangent;float2 rough_metal;uint medium;};
BVertex make_vertex(Hit h,float3 beta,float pdf_fwd){
    BVertex x;x.p=h.p;x.n=h.n;x.beta=beta;x.pdf_fwd=pdf_fwd;x.pdf_rev=0;
    x.info=h.material|(h.textured?V_TEXTURED:0)|(h.front?V_FRONT:0)|(((uint)materials[h.material].lobes.w&1u)!=0?V_NO_BSDF:0)|(((uint)materials[h.material].lobes.w&4096u)!=0?V_DELTA:0);
    x.albedo=h.albedo;x.rough_metal=h.rough_metal;x.tangent=h.tangent;x.emitter=h.emitter;x.medium=0;return x;
}
BVertex medium_vertex(float3 p,float3 beta,float pdf_fwd,uint medium,float g){
    BVertex x=(BVertex)0;x.p=p;x.beta=beta;x.pdf_fwd=pdf_fwd;x.info=V_MEDIUM;x.medium=medium;x.rough_metal.x=g;return x;
}
uint top_entry(uint64_t media){int t=medium_top(media);return t>=0?medium_at(media,t):0;}
// The cosine a density or a geometry term takes at x (1 at a scatter point).
float vcos(BVertex x,float3 w){return (x.info&V_MEDIUM)!=0?1:abs(dot(x.n,w));}
// Transmittance along len of w from a in a vertex's medium: an object's extinction plus its
// Beer colour, else the fog box's part of the segment.
float3 segment_tr(uint medium,float3 a,float3 w,float len){
    if(medium!=0){Material m=materials[(medium&0x1fffu)-1];return exp(-(m.medium.w-log(max(m.color_rough.rgb,0.001)))*len);}
    float t0,t1;return fog_interval(a,w,len,t0,t1)?exp(-fog.x*(t1-t0)).xxx:float3(1,1,1);
}
// A walk's medium state. After a vertex whose sampled direction crosses a dielectric (n faces
// the arriving walk), enter or leave its medium as trace does (no false-hit test).
struct Walk{uint64_t media;float3 absorption;float density;};
Walk new_walk(){Walk k;k.media=0;k.absorption=0;k.density=0;return k;}
void cross_surface(BVertex x,float3 next,inout Walk k){
    uint id=x.info&0xffffffu;if((x.info&V_MEDIUM)!=0 || materials[id].lobes.z<=0 || dot(next,x.n)>=0)return;
    uint e=medium_entry(id,materials[id].lobes.w);k.media=(x.info&V_FRONT)!=0?medium_push(k.media,e):medium_remove(k.media,e);
    int top=medium_top(k.media);k.absorption=medium_absorption(k.media,top);k.density=top>=0?medium_material(k.media,top).medium.w:0;
}
// Free-flight sampling on the grey extinction up to tmax (the next surface), as trace samples
// it: a scatter at ts (the pdf cancels the transmittance, so the caller multiplies by albedo).
bool free_flight(Walk k,float3 o,float3 d,float tmax,out float ts,out float3 albedo,out float g,out float sigma){
    ts=tmax;albedo=1;g=0;sigma=0;if(!(k.density>0 || (fog.x>0 && k.media==0)))return false;
    float t0,t1;if(!medium_params(k.media,o,d,tmax,sigma,albedo,g,t0,t1))return false;
    ts=t0-log(1-random())/sigma;return ts<t1;
}
// A nested-dielectric false hit (see trace's medium_top): entering a dielectric ranked below the
// current medium, or leaving one that is not the current medium. Updates the stack.
bool false_hit(Hit h,inout Walk k){
    if(materials[h.material].lobes.z<=0)return false;
    int top=medium_top(k.media);uint e=medium_entry(h.material,materials[h.material].lobes.w);bool skip;
    if(h.front)skip=top>=0 && (e>>13)<(medium_at(k.media,top)>>13);
    else skip=medium_find(k.media,e)>=0 && medium_at(k.media,top)!=e;
    if(!skip)return false;
    k.media=h.front?medium_push(k.media,e):medium_remove(k.media,e);
    top=medium_top(k.media);k.absorption=medium_absorption(k.media,top);k.density=top>=0?medium_material(k.media,top).medium.w:0;
    return true;
}
// A walk's next segment from o along d: free flight in each medium crossed, false hits passed
// straight through (at most 32), absorption applied to beta. 0: escaped; 1: surface hit h;
// 2: a scatter at xs (the caller multiplies by alb). o ends at the last segment's start.
uint walk_step(inout float3 o,float3 d,bool camera,inout Walk k,inout float3 beta,out Hit h,out float3 xs,out float3 alb,out float g,out float sigma){
    xs=o;
    for(uint f=0;;f++){
        bool found=intersect(o,d,1e4,h,false,camera);float ts;
        if(free_flight(k,o,d,found?h.t:1e4,ts,alb,g,sigma)){beta*=exp(-k.absorption*ts);xs=o+d*ts;return 2;}
        if(!found)return 0;
        beta*=exp(-k.absorption*h.t);
        if(f>=32 || !false_hit(h,k))return 1;
        o=h.p-h.ng*0.0003;
    }
}
float area_pdf(float pdf_w,float3 from,float3 at,float3 n,bool medium){float3 w=at-from;float d2=max(dot(w,w),1e-12);return pdf_w*(medium?1:abs(dot(n,w))*rsqrt(d2))/d2;}
// The vertex's material as trace builds it: pattern, texture maps, thin film at the cosine of v,
// and a grating wavelength fixed per vertex (so connections and samples agree).
Material vertex_material(BVertex x,float3 v){
    Material m=material_at(x.info&0xffffffu,x.p);
    if((x.info&V_TEXTURED)!=0){m.color_rough.rgb=x.albedo;m.color_rough.a=x.rough_metal.x;m.lobes.x=x.rough_metal.y;}
    if(m.surface_scale.z>0){bool front=(x.info&V_FRONT)!=0;float c=saturate(dot(v,x.n));
        if(m.lobes.z>0)prepare_film(m,c,front?1:m.optical.x,front?m.optical.x:1);else prepare_film(m,c,1,0);}
    if(has_grating(m))grating_wavelength(hash(asuint(x.p.x))^hash(asuint(x.p.y)+0x68e31da4u)^hash(asuint(x.p.z)+0x1b873593u));
    return m;
}
// The emission a hit vertex sends toward w, as trace counts it (flat colour, one-sided, spot).
float3 vertex_emission(BVertex x,float3 w){
    Material m=material_at(x.info&0xffffffu,x.p);float3 e=m.color_rough.rgb*m.optical.y;
    if(m.optical.w>0.5 && (x.info&V_FRONT)==0)e=0;return e*spot(m,dot(x.n,w));
}
// f(v, l) with v toward the camera side and l toward the light side, and the density of sampling
// l given v; 0 below the surface (n faces the ray that found the vertex).
float3 vertex_f(BVertex x,float3 v,float3 l,out float pdf){
    if((x.info&V_MEDIUM)!=0){pdf=hg_pdf(dot(-v,l),x.rough_metal.x);return pdf;}
    pdf=0;if((x.info&(V_NO_BSDF|V_CAMERA|V_LIGHT))!=0 || dot(x.n,v)<=0 || dot(x.n,l)<=0)return 0;
    Material m=vertex_material(x,v);return eval_bsdf(m,x.n,x.tangent,v,l,pdf);
}
// The solid-angle density of sampling wout at x given win.
float vertex_pdf(BVertex x,float3 win,float3 wout){float pdf;vertex_f(x,win,wout,pdf);return pdf;}
float to_area(float pdf,float3 from,BVertex to){float3 w=to.p-from;float d2=max(dot(w,w),1e-12);return pdf*((to.info&V_MEDIUM)!=0?1:abs(dot(to.n,w))*rsqrt(d2))/d2;}
// Samples the next direction of a walk arriving along d, the way trace samples it. Camera
// subpaths (radiance) carry the eta² of refraction, light subpaths do not. mode 0: the whole
// material; 1: its dielectric lobe only; 2: everything but the dielectric (pdfs divided by
// 1 - transmission). pf: density of next given -d; pr: of -d given next (both 0 when delta).
// The medium stack of the walk calling sample_vertex (set before each call).
static uint64_t sv_media;
bool sample_vertex(inout BVertex x,float3 d,bool radiance,uint mode,out float3 next,out float pf,out float pr,inout float3 beta,inout float3 absorption){
    pf=0;pr=0;next=d;
    // A scatter point: the HG phase function, sampled exactly (f / pdf = 1, symmetric pdfs).
    if((x.info&V_MEDIUM)!=0){next=hg_sample(d,x.rough_metal.x,pf);pr=pf;return pf>0;}
    if((x.info&V_NO_BSDF)!=0)return false;
    Material m=vertex_material(x,-d);bool front=(x.info&V_FRONT)!=0;float z=m.lobes.z;
    // Beam splitter (track 17): reflect with probability `reflectivity`, else pass straight on.
    if(((uint)m.lobes.w&4096u)!=0){if(front){beta*=m.color_rough.rgb;if(random()<m.lobes.y)next=reflect(d,x.n);}return true;}
    if(mode==1 || (mode==0 && random()<z)){
        if(z<=0)return false;
        // Both sides' indices from the walk's medium stack (sv_media), as trace takes them.
        uint e=medium_entry(x.info&0xffffffu,m.lobes.w);uint64_t after=front?medium_push(sv_media,e):medium_remove(sv_media,e);
        float n1=front?medium_ior(sv_media,medium_top(sv_media)):m.optical.x,n2=front?m.optical.x:medium_ior(after,medium_top(after));
        float eta=n1/n2,fr=fresnel_dielectric(saturate(dot(-d,x.n)),eta);
        float3 frc=fr.xxx;
        if(m.surface_scale.z>0 && fr<1){frc=lerp(frc,m.base_scale.rgb,m.surface_scale.z);fr=clamp((frc.r+frc.g+frc.b)/3,1e-4,1-1e-4);}
        bool reflected=random()<fr;
        next=reflected?reflect(d,x.n):refract(d,x.n,eta);
        if(m.surface_scale.z>0)beta*=reflected?frc/fr:(1-frc)/(1-fr);
        if(!reflected){absorption=front?-log(max(m.color_rough.rgb,0.001)):0;if(radiance)beta*=eta*eta;}
        if(m.color_rough.a>0.001){float3 pert=cosine_sample(reflected?x.n:-x.n);next=normalize(lerp(next,pert,m.color_rough.a*m.color_rough.a));}
        x.info|=V_DELTA;return true;
    }
    if(mode==2 && z>=1)return false;
    float pc=coat_probability(m),ps=spec_probability(m),a=max(0.001,m.color_rough.a*m.color_rough.a);
    if(pc>0 && random()<pc)next=ggx_sample(x.n,d,max(0.001,m.layers.y*m.layers.y));
    else if(random()<ps){
        int o=-1;float3 T,B;
        if(has_grating(m)){aniso_frame(x.n,x.tangent,m.sheen.w,T,B);float4 w=order_shares(m,x.n,B,-d);float u=random(),c=0;[unroll]for(int i=0;i<4;i++){if(o<0 && u<c+w[i])o=i;c+=w[i];}}
        if(o>=0){next=ggx_sample(x.n,d,a);if(dot(x.n,next)<=0 || !grating_move(x.n,B,next,GRATING_ORDER[o]*grating_shift(m),next))return false;}
        else if(m.layers.w>0){aniso_frame(x.n,x.tangent,m.sheen.w,T,B);next=ggx_sample_aniso(x.n,T,B,d,aniso_alpha(m));}
        else next=ggx_sample(x.n,d,a);
    }else next=cosine_sample(x.n);
    if(dot(x.n,next)<=0)return false;
    float3 f;
    if(radiance){f=eval_bsdf(m,x.n,x.tangent,-d,next,pf);eval_bsdf(m,x.n,x.tangent,next,-d,pr);}
    else{f=eval_bsdf(m,x.n,x.tangent,next,-d,pr);eval_bsdf(m,x.n,x.tangent,-d,next,pf);}
    if(mode==2){pf/=1-z;pr/=1-z;}
    if(pf<=0)return false;
    beta*=f*dot(x.n,next)/pf;return true;
}
// Table emitters as light-path origins (triangles and spheres; suns start no subpath): chosen
// with the probability next-event estimation gives them, uniform over their area, cosine
// emission (both sides of a two-sided panel, half each).
LightPoint hit_light(uint i,float3 p){
    Emitter e=emitters[i];LightPoint l;l.p=p;l.le=e.emission_material.rgb;l.lum=0;l.source=0;
    if(e.position_area.w==-1){l.n=normalize(p-e.position_area.xyz);l.kind=1;}
    else{l.n=normalize(cross(e.edge1_cdf.xyz,e.edge2_probability.xyz));l.kind=0;}
    return l;
}
float origin_pdf(uint i,float p_env){Emitter e=emitters[i];float area=e.position_area.w==-1?4*PI*e.edge1_cdf.x*e.edge1_cdf.x:e.position_area.w;return (1-p_env)*e.edge2_probability.w/max(area,1e-12);}
bool two_sided(uint i,LightPoint l){return l.kind==0 && material_at((uint)emitters[i].emission_material.w,l.p).optical.w<0.5;}
// A spot light's cone: emission is 0 below cos = secondary.x, so light subpaths sample the cone
// uniformly instead of the hemisphere (track 17: a 4° beam got 0.1% of the photons). 0 = no spot.
float spot_cone(uint i,LightPoint l){if(l.kind!=0)return 0;Material lm=material_at((uint)emitters[i].emission_material.w,l.p);return ((uint)lm.lobes.w&32u)!=0 && lm.optical.w>=0.5?clamp(lm.secondary.x,-1,0.9999):0;}
float light_dir_pdf(uint i,LightPoint l,float3 w){float c=dot(l.n,w);float k=spot_cone(i,l);if(k!=0)return c>=k?1/(2*PI*(1-k)):0;if(two_sided(i,l))return abs(c)/(2*PI);return c>0?c/PI:0;}
// The first direction of a light subpath from l (n: the side chosen) and its density.
float3 emit_dir(uint i,LightPoint l,float3 n,bool two,out float pd){
    float k=spot_cone(i,l);
    if(k!=0){float c=lerp(1,k,random()),r=sqrt(max(0,1-c*c)),a=2*PI*random();pd=1/(2*PI*(1-k));return basis(n,float3(r*cos(a),r*sin(a),c));}
    float3 w=cosine_sample(n);pd=dot(n,w)/PI*(two?0.5:1);return w;
}
float3 emission_toward(uint i,LightPoint l,float3 w){
    float c=dot(l.n,w);if(l.kind==1)return c>0?l.le:0;
    Material lm=material_at((uint)emitters[i].emission_material.w,l.p);
    return lm.optical.w>=0.5 && c<=0?0:lm.color_rough.rgb*lm.optical.y*spot(lm,c);
}
// What a ray leaving along d finds at infinity: the sky (the camera may see the background
// instead) and the suns, weighted against light sampling by MIS as trace weights them.
float3 escape(float3 d,bool camera,bool prev_delta,float prev_pdf,float p_env){
    float3 e=camera && hdri_on() && ((uint)environment_map.z&2u)==0?background.rgb:environment_radiance(d);
    if(hdri_on() && !camera && !prev_delta)e*=prev_pdf/(prev_pdf+p_env*env_pdf(d));
    for(uint i=0;i<(uint)lights.x;i++){Emitter sun=emitters[i];
        float3 x=cross(d,sun.position_area.xyz);float om=sun.edge1_cdf.x;
        if(dot(d,sun.position_area.xyz)<=0 || dot(x,x)>om*(2-om))continue;
        if(camera && (((uint)materials[(uint)sun.emission_material.w].lobes.w&2u)!=0))continue;
        float w=1;if(!camera && !prev_delta)w=prev_pdf/(prev_pdf+(1-p_env)*sun.edge2_probability.w/(2*PI*om));
        e+=sun.emission_material.rgb*w;}
    return e;
}
float3 camera_forward(){return -world_view[2].xyz;}
// The pinhole's directional density (whole image, times the region's share: photon.y) and the
// render pixel a point projects to inside this dispatch's region.
float camera_pdf_w(float3 w){float c=dot(w,camera_forward());return c>0?1/(photon.y*c*c*c):0;}
bool project(float3 q,out uint2 pix){
    pix=0;float4 c=mul(current_vp,float4(q,1));if(c.w<=0)return false;
    float2 f=float2(c.x/c.w*0.5+0.5,0.5-c.y/c.w*0.5)*float2(dimensions.xy);
    if(any(f<float2(region.xy)) || any(f>=float2(min(dimensions.xy,region.zw))))return false;
    pix=uint2(f);return true;
}
// Visibility between two surface points, each moved off its surface toward the other. A shadow
// ray that stopped a fixed 0.6 mm short of the far point (as next-event estimation's do) still
// hit that point's own surface at grazing angles: veach-ajar's door lit the wall 4% darker.
bool connected(float3 a,float3 na,float3 b,float3 nb){
    float3 w=b-a;a+=na*(dot(na,w)>0?0.0003:-0.0003);b+=nb*(dot(nb,w)<0?0.0003:-0.0003);
    float3 e=b-a;float l=length(e);return l>0 && !occluded(a,e/l,l);
}
// --- BDPT -------------------------------------------------------------------------------
// Veach's bidirectional estimator in PBRT-v3's form, with the balance heuristic over every
// strategy (s light vertices, t camera vertices). Up to BDPT_BOUNCES bounces. Light tracing (t = 1)
// splats into other pixels and needs a pinhole perspective camera without RR's jitter
// (integrator.w bit 0); without it the camera counts as a delta vertex. The sky, the HDRI and
// the suns start no light subpath: they keep trace's two strategies (hits and NEE).
// Up to BDPT_BOUNCES bounces (mirror rooms and the Ganzfeld set 16-24; at 10 they were 15-26%
// dark). The arrays live in local memory; only the vertices a sample reaches are touched.
#define BDPT_BOUNCES 24
#define CAMERA_VERTICES (BDPT_BOUNCES+2)
#define LIGHT_VERTICES (BDPT_BOUNCES+1)
// Photon walks and gathers: deposits beyond the buffer's photons × 10 are dropped, which Russian
// roulette keeps rare.
#define PHOTON_WALK 32u
static BVertex cam[CAMERA_VERTICES],lig[LIGHT_VERTICES];
float remap0(float x){return x!=0?x:1;}
// One strategy's weighted contribution. A non-finite term (a half vector of opposite directions,
// densities overflowing at near-coincident vertices) is dropped on its own: zeroing the whole
// sample instead lost the large terms of the same path, 4% of veach-ajar's light.
void add_term(inout float3 L,float3 c){if(all(isfinite(c)))L+=max(c,0);}
// The balance-heuristic weight of strategy (s, t) from the ratios of each vertex's densities;
// the connection's endpoints and their neighbours take the reverse densities passed in.
float mis_weight(uint s,uint t,float pt_rev,float ptm_rev,float qs_rev,float qsm_rev,float qs_fwd1){
    if(s+t==2)return 1;
    float sum=0,ri=1;
    for(int i=(int)t-1;i>0;i--){
        float rev=i==(int)t-1?pt_rev:i==(int)t-2?ptm_rev:cam[i].pdf_rev;
        ri*=remap0(rev)/remap0(cam[i].pdf_fwd);
        bool di=i!=(int)t-1 && (cam[i].info&V_DELTA)!=0,dp=(cam[i-1].info&V_DELTA)!=0;
        if(!di && !dp)sum+=ri;
    }
    ri=1;
    for(int j=(int)s-1;j>=0;j--){
        float rev=j==(int)s-1?qs_rev:j==(int)s-2?qsm_rev:lig[j].pdf_rev;
        float fwd=s==1 && j==0?qs_fwd1:lig[j].pdf_fwd;
        ri*=remap0(rev)/remap0(fwd);
        bool dj=j!=(int)s-1 && (lig[j].info&V_DELTA)!=0,dl=j>0 && (lig[j-1].info&V_DELTA)!=0;
        if(!dj && !dl)sum+=ri;
    }
    return isfinite(sum)?1/(1+sum):0;
}
void atomic_add_float(uint a,float v){
    if(v==0)return;uint old=aux.Load(a);
    for(uint n=0;n<1024;n++){uint prev;aux.InterlockedCompareExchange(a,old,asuint(asfloat(old)+v),prev);if(prev==old)break;old=prev;}
}
void splat(uint2 q,float3 c){
    if(!all(isfinite(c)) || !any(c>0))return;uint a=integrator.z+(q.y*dimensions.x+q.x)*12;
    atomic_add_float(a,c.r);atomic_add_float(a+4,c.g);atomic_add_float(a+8,c.b);
}
float3 bdpt_sample(uint2 p,uint D,bool lt,float p_env,float pixel_angle,float pixel_width){
    float3 L=0;bool tri=scene_info.x>0;
    float2 sub=(scene_info.w&1)!=0?jitter.xy:float2(random(),random())-0.5;
    float3 o,d;bool valid;camera_ray(float2(p)+0.5+sub,true,o,d,valid);if(!valid)return 0;
    // Camera subpath.
    BVertex x0=(BVertex)0;x0.p=o;x0.beta=camera_weight;x0.info=V_CAMERA|(lt?0:V_DELTA);cam[0]=x0;
    uint T=1;float3 beta=camera_weight,absorption=0;float pdf_w=camera_pdf_w(d),prev_pdf=0;bool prev_delta=true;Walk k=new_walk();
    cone_width=pixel_width;cone_spread=pixel_angle;
    for(;;){
        Hit h;float3 xs,alb;float g,sigma;uint step=walk_step(o,d,T==1,k,beta,h,xs,alb,g,sigma);
        if(step==2){
            beta*=alb;
            BVertex x=medium_vertex(xs,beta,area_pdf(pdf_w,cam[T-1].p,xs,0,true),top_entry(k.media),g);cam[T]=x;T++;
            if(T>D+1)break;
            float3 next;float pf,pr;if(!sample_vertex(x,d,true,0,next,pf,pr,beta,absorption))break;
            if(T-2>0)cam[T-2].pdf_rev=to_area(pr,x.p,cam[T-2]);
            pdf_w=pf;prev_delta=false;prev_pdf=pf;o=x.p;d=next;continue;
        }
        if(step==0){add_term(L,beta*escape(d,T==1,prev_delta,prev_pdf,p_env));break;}
        BVertex x=make_vertex(h,beta,area_pdf(pdf_w,cam[T-1].p,h.p,h.n,false));x.medium=top_entry(k.media);
        // Emission outside the light table (procedural emitters) is found by hits alone.
        if(h.emitter==0)add_term(L,beta*vertex_emission(x,-d));
        cam[T]=x;T++;
        if(T>D+1 || (x.info&V_NO_BSDF)!=0)break;
        float3 next;float pf,pr;
        // A local copy: inout on an element of a static array did not reliably write back.
        sv_media=k.media;bool sampled=sample_vertex(x,d,true,0,next,pf,pr,beta,absorption);cam[T-1]=x;
        if(!sampled)break;
        cross_surface(x,next,k);
        bool delta=(x.info&V_DELTA)!=0;
        if(T-2>0)cam[T-2].pdf_rev=to_area(pr,cam[T-1].p,cam[T-2]);
        pdf_w=delta?0:pf;prev_delta=delta;prev_pdf=pf;
        o=h.p+h.ng*(dot(next,h.ng)>0?0.0003:-0.0003);d=next;
    }
    // Light subpath, from a table emitter picked as NEE picks one (none for the sky or a sun).
    uint S=0;
    if(tri && random()>=p_env){
        uint i=select_index(random());Emitter e=emitters[i];
        if(e.position_area.w>=0 || e.position_area.w==-1){
            LightPoint l=light_point(i,pack_uv(float2(random(),random())));
            float po=(1-p_env)*l.source;bool two=two_sided(i,l);float3 n=l.n;if(two && random()<0.5)n=-n;
            float pd;float3 w=emit_dir(i,l,n,two,pd);float c=dot(n,w);float3 le=emission_toward(i,l,w);
            if(po>0 && pd>0 && any(le>0)){
                BVertex y=(BVertex)0;y.p=l.p;y.n=n;y.beta=1/po;y.pdf_fwd=po;y.info=V_LIGHT;y.emitter=i+1;lig[0]=y;S=1;
                float3 b=le*c/(po*pd),ab=0,lo=l.p+n*0.0003;float pw=pd;Walk lk=new_walk();
                cone_width=0;cone_spread=0;
                for(;;){
                    Hit h;float3 xs,alb;float g,sigma;uint step=walk_step(lo,w,false,lk,b,h,xs,alb,g,sigma);
                    if(step==2){
                        b*=alb;
                        BVertex x=medium_vertex(xs,b,area_pdf(pw,lig[S-1].p,xs,0,true),top_entry(lk.media),g);lig[S]=x;S++;
                        if(S>D)break;
                        float3 next;float pf,pr;if(!sample_vertex(x,w,false,0,next,pf,pr,b,ab))break;
                        lig[S-2].pdf_rev=to_area(pr,x.p,lig[S-2]);
                        pw=pf;lo=x.p;w=next;continue;
                    }
                    if(step==0)break;
                    BVertex x=make_vertex(h,b,area_pdf(pw,lig[S-1].p,h.p,h.n,false));x.medium=top_entry(lk.media);lig[S]=x;S++;
                    if(S>D || (x.info&V_NO_BSDF)!=0)break;
                    float3 next;float pf,pr;
                    sv_media=lk.media;bool sampled=sample_vertex(x,w,false,0,next,pf,pr,b,ab);lig[S-1]=x;
                    if(!sampled)break;
                    cross_surface(x,next,lk);
                    lig[S-2].pdf_rev=to_area(pr,x.p,lig[S-2]);
                    pw=(x.info&V_DELTA)!=0?0:pf;
                    lo=h.p+h.ng*(dot(next,h.ng)>0?0.0003:-0.0003);w=next;
                }
            }
        }
    }
    // Every strategy (s, t) with s + t - 2 bounces.
    for(uint t=1;t<=T;t++){
        for(uint s=0;s<=max(S,1u);s++){
            if(s+t<2 || s+t-2>D || (s==1 && t==1))continue;
            if(s>=2 && s>S)continue;
            if(s==0){
                // The camera subpath ends on a table emitter.
                BVertex pt=cam[t-1];if(pt.emitter==0)continue;uint i=pt.emitter-1;
                float3 w=normalize(cam[t-2].p-pt.p);float3 le=vertex_emission(pt,w);if(!any(le>0))continue;
                LightPoint l=hit_light(i,pt.p);
                float ptm_rev=t>2?to_area(light_dir_pdf(i,l,w),pt.p,cam[t-2]):0;
                add_term(L,pt.beta*le*mis_weight(0,t,origin_pdf(i,p_env),ptm_rev,0,0,0));
            }else if(t==1){
                // Light tracing: the light subpath's last vertex seen by the camera.
                if(!lt || s>S)continue;
                BVertex qs=lig[s-1];if((qs.info&(V_DELTA|V_NO_BSDF))!=0)continue;
                uint2 pix;if(!project(qs.p,pix))continue;
                float3 w=eye.xyz-qs.p;float d2=dot(w,w),dist=sqrt(d2);w/=dist;
                float c=vcos(qs,w),ct=dot(-w,camera_forward());if(ct<=0)continue;
                float3 f;float qsm_rev=0;
                if(s==1)f=emission_toward(qs.emitter-1,hit_light(qs.emitter-1,qs.p),w);
                else{float3 wq=normalize(lig[s-2].p-qs.p);float pq;f=vertex_f(qs,w,wq,pq);qsm_rev=to_area(vertex_pdf(qs,w,wq),qs.p,lig[s-2]);}
                float3 C=qs.beta*f*c*segment_tr(qs.medium,qs.p,w,dist)/(d2*photon.y*ct*ct*ct)/max(sampling.y,1u);
                if(!any(C>0))continue;
                float3 q=qs.p+qs.n*(dot(qs.n,w)>0?0.0003:-0.0003)-eye.xyz;float lq=length(q);
                Hit hb;if(intersect(eye.xyz,q/lq,lq,hb,false,true))continue;
                splat(pix,C*mis_weight(s,1,0,0,camera_pdf_w(-w)*c/d2,qsm_rev,0));
            }else if(s==1){
                // Next-event estimation: one light choice per camera vertex, as trace makes it.
                BVertex pt=cam[t-1];if((pt.info&(V_DELTA|V_NO_BSDF))!=0)continue;
                float3 wm=normalize(cam[t-2].p-pt.p);
                if(random()<p_env){
                    float pe;float3 ld=sample_env(pe);if(pe<=0)continue;float pdf;float3 f=vertex_f(pt,wm,ld,pdf);
                    if(any(f>0) && !occluded(pt.p+pt.n*0.0003,ld,1e4))add_term(L,pt.beta*f*vcos(pt,ld)*segment_tr(pt.medium,pt.p,ld,1e4)*environment_radiance(ld)/(p_env*pe+pdf));
                    continue;
                }
                if(!tri)continue;
                uint i=select_index(random());Emitter e=emitters[i];
                if(e.position_area.w<0 && e.position_area.w!=-1){
                    float pc;float3 ld=cone_sample(e.position_area.xyz,e.edge1_cdf.x,pc);float pdf;float3 f=vertex_f(pt,wm,ld,pdf);
                    if(any(f>0) && !occluded(pt.p+pt.n*0.0003,ld,1e4))add_term(L,pt.beta*f*vcos(pt,ld)*segment_tr(pt.medium,pt.p,ld,1e4)*e.emission_material.rgb/((1-p_env)*e.edge2_probability.w*pc+pdf));
                    continue;
                }
                LightPoint l=light_point(i,pack_uv(float2(random(),random())));float po=(1-p_env)*l.source;
                float3 ld;float g,limit;float3 le=light_toward(i,l,pt.p,ld,g,limit);
                float pdf;float3 f=vertex_f(pt,wm,ld,pdf);
                float3 dl=l.p-pt.p;float d2=max(dot(dl,dl),1e-12);
                float3 C=pt.beta*f*vcos(pt,ld)*g*le*segment_tr(pt.medium,pt.p,ld,sqrt(d2))/max(po,1e-30);
                if(!any(C>0) || (l.kind==0?!connected(pt.p,pt.n,l.p,l.n):occluded(pt.p+pt.n*0.0003,ld,limit)))continue;
                float pt_rev=light_dir_pdf(i,l,-ld)*vcos(pt,ld)/d2;
                float ptm_rev=t>2?to_area(vertex_pdf(pt,ld,wm),pt.p,cam[t-2]):0;
                float qs_rev=pdf*abs(dot(l.n,ld))/d2;
                add_term(L,C*mis_weight(1,t,pt_rev,ptm_rev,qs_rev,0,po));
            }else{
                // Both subpaths joined by one edge.
                BVertex qs=lig[s-1],pt=cam[t-1];if(((qs.info|pt.info)&(V_DELTA|V_NO_BSDF))!=0)continue;
                float3 w=qs.p-pt.p;float d2=max(dot(w,w),1e-12),dist=sqrt(d2);w/=dist;
                float3 wm=normalize(cam[t-2].p-pt.p),wq=normalize(lig[s-2].p-qs.p);
                float pa,pb;float3 fa=vertex_f(pt,wm,w,pa),fb=vertex_f(qs,-w,wq,pb);
                float3 C=pt.beta*fa*qs.beta*fb*vcos(pt,w)*vcos(qs,w)*segment_tr(pt.medium,pt.p,w,dist)/d2;
                if(!any(C>0) || !connected(pt.p,pt.n,qs.p,qs.n))continue;
                float pt_rev=vertex_pdf(qs,wq,-w)*vcos(pt,w)/d2;
                float ptm_rev=t>2?to_area(vertex_pdf(pt,w,wm),pt.p,cam[t-2]):0;
                float qs_rev=pa*vcos(qs,w)/d2;
                float qsm_rev=to_area(vertex_pdf(qs,-w,wq),qs.p,lig[s-2]);
                add_term(L,C*mis_weight(s,t,pt_rev,ptm_rev,qs_rev,qsm_rev,0));
            }
        }
    }
    return L;
}
// The frame's estimate b (a mean over sampling.y samples) into the running mean, as trace's
// tail does; the direct-light AOV holds the beauty (the direct / indirect sliders do nothing).
void finish_frame(uint2 p,float3 b){
    if(!all(isfinite(b)))b=0;b=max(b,0);
    float n=max(sampling.y,1u),count=sampling.x+n;
    float3 mean=sampling.x>0?accumulation[p].rgb:0;float m2=sampling.x>0?accumulation[p].a:0;
    float dl=luminance(b)-luminance(mean);mean+=(b-mean)*n/count;m2+=n*dl*(luminance(b)-luminance(mean));
    accumulation[p]=float4(mean,max(0,m2));noisy[p]=float4(b,1);direct_light[p]=float4(mean,1);specular_light[p]=0;trace_stats[p]=0;
}
float pixel_angle_now(){return camera_model.x==1?0:2*environment.z/dimensions.y;}
float pixel_width_now(){return camera_model.x==1?2*camera_model.w/dimensions.y:0;}
float env_share(){return hdri_on()?(scene_info.x>0?0.5:1):0;}
[numthreads(8,8,1)]void bdpt(uint3 tid:SV_DispatchThreadID){
    uint2 p=tid.xy+region.xy;if(any(p>=min(dimensions.xy,region.zw)))return;
    white_noise=true;glass_blocks=true;pixel_seed=hash(p.x+p.y*dimensions.x);rng_state=hash(pixel_seed^hash(sampling.w*0x2c1b3c6du+0x297a2d39u));
    cone_width=pixel_width_now();cone_spread=pixel_angle_now();
    Hit guide;write_guides(p,guide);
    float3 sum=0;
    for(uint k=0;k<sampling.y;k++)sum+=bdpt_sample(p,min(sampling.z,(uint)BDPT_BOUNCES),(integrator.w&1)!=0,env_share(),pixel_angle_now(),pixel_width_now());
    noisy[p]=float4(sum/max(sampling.y,1u),1);
}
// After bdpt: this frame's camera strategies plus the light-tracing splats, which are cleared.
[numthreads(8,8,1)]void bdpt_resolve(uint3 tid:SV_DispatchThreadID){
    uint2 p=tid.xy+region.xy;if(any(p>=min(dimensions.xy,region.zw)))return;
    float3 b=noisy[p].rgb;
    if((integrator.w&1)!=0){uint a=integrator.z+(p.y*dimensions.x+p.x)*12;b+=asfloat(aux.Load3(a));aux.Store3(a,uint3(0,0,0));}
    finish_frame(p,b);
}
// --- Photon mapping -----------------------------------------------------------------------
// Probabilistic progressive photon mapping (Knaus and Zwicker 2011): every frame emits
// integrator.x photons into a hash grid of cells one radius wide and gathers them with the
// radius photon.x, which the CPU shrinks as r²·(i + 2/3)/(i + 1) per frame; each frame is then
// an independent estimate and the running mean converges. Camera paths pass delta surfaces and
// near-mirror metals (gather_surface), then take direct light by NEE plus one BSDF ray (MIS)
// and indirect light from the photons, deposited from their second hit on.
uint cell_hash(int3 c){return hash((uint)c.x*73856093u^(uint)c.y*19349663u^(uint)c.z*83492791u)&(HASH-1);}
// Beam splitters (bit 4096) are delta: no photons, no gather; the walk passes them on.
bool gather_surface(Material m){return m.lobes.z<1 && ((uint)m.lobes.w&4096u)==0 && !(m.color_rough.a<0.2 && m.lobes.x>0.5);}
// Emitters keep no photons: a small bright emitter lit by its neighbours (birthday-cake's flames)
// holds a dense cluster that surfaces within the radius gathered as their own, 87% too red.
// Their reflected light is lost, negligible beside their emission.
bool keeps_photons(Material m){return gather_surface(m) && m.optical.y<=0;}
[numthreads(64,1,1)]void photon_clear(uint3 tid:SV_DispatchThreadID){
    if(tid.x<HASH)aux.Store(16+4*tid.x,0xffffffffu);
    if(tid.x==0)aux.Store(0,0);
}
// A volume photon (deposited at a scatter in a medium) carries its power's red with the sign
// bit set; surface gathers skip those and volume gathers take only those.
void deposit(float3 p,float3 wi,float3 b,bool volume){
    uint idx;aux.InterlockedAdd(0,1,idx);if(idx>=integrator.y)return;
    uint prev;aux.InterlockedExchange(16+4*cell_hash((int3)floor(p/photon.x)),idx,prev);
    uint a=PHOTON_BASE+idx*32;aux.Store4(a,uint4(asuint(p),pack_normal(wi)));aux.Store4(a+16,uint4(asuint(b.r)|(volume?0x80000000u:0),asuint(b.g),asuint(b.b),prev));
}
// A point on the disk of the bounding sphere facing direction ld (toward the sun or sky).
float3 disk_origin(float3 ld){float r=bounds.w*sqrt(random()),a=2*PI*random();return bounds.xyz+basis(ld,float3(r*cos(a),r*sin(a),bounds.w*1.01));}
[numthreads(64,1,1)]void photon_trace(uint3 tid:SV_DispatchThreadID){
    uint i=tid.x;if(i>=integrator.x)return;
    white_noise=true;glass_blocks=true;rng_state=hash(i^hash(sampling.w*0x9e3779b9u+0x7f4a7c15u));cone_width=0;cone_spread=0;
    bool tri=scene_info.x>0,sky=hdri_on() || any(background.rgb>0) || environment.x>0;float p_env=sky?(tri?0.5:1):0;
    float3 o,w,b;float disk=PI*bounds.w*bounds.w;
    if(random()<p_env){
        float pw;float3 ld;
        if(hdri_on())ld=sample_env(pw);else{float z=1-2*random(),s=sqrt(max(0,1-z*z)),a=2*PI*random();ld=float3(s*cos(a),z,s*sin(a));pw=1/(4*PI);}
        if(pw<=0)return;
        o=disk_origin(ld);w=-ld;b=environment_radiance(ld)*disk/(p_env*pw);
    }else{
        if(!tri)return;
        uint k=select_index(random());Emitter e=emitters[k];
        if(e.position_area.w<0 && e.position_area.w!=-1){
            float pc;float3 ld=cone_sample(e.position_area.xyz,e.edge1_cdf.x,pc);
            o=disk_origin(ld);w=-ld;b=e.emission_material.rgb*disk/((1-p_env)*e.edge2_probability.w*pc);
        }else{
            LightPoint l=light_point(k,pack_uv(float2(random(),random())));float po=(1-p_env)*l.source;
            bool two=two_sided(k,l);float3 n=l.n;if(two && random()<0.5)n=-n;
            float pd;w=emit_dir(k,l,n,two,pd);float c=dot(n,w);if(po<=0 || pd<=0)return;
            b=emission_toward(k,l,w)*c/(po*pd);o=l.p+n*0.0003;
        }
    }
    float3 ab=0;Walk wk=new_walk();
    for(uint k=0;k<min(sampling.z,PHOTON_WALK);k++){
        Hit h;float3 x,alb;float g,sigma;uint step=walk_step(o,w,false,wk,b,h,x,alb,g,sigma);
        if(step==2){
            // A scatter in a medium: a volume photon from the second interaction on (the first
            // is direct light, which the gather takes by next-event estimation).
            if(k>0)deposit(x,-w,b,true);
            float before=max3(b),pf;b*=alb;float3 next=hg_sample(w,g,pf);
            if(k>=2){float survival=clamp(max3(b)/max(before,1e-30),0.05,0.95);if(random()>survival)return;b/=survival;}
            o=x;w=next;continue;
        }
        if(step==0)return;
        BVertex y=make_vertex(h,b,0);if((y.info&V_NO_BSDF)!=0)return;
        if(k>0 && keeps_photons(vertex_material(y,-w)))deposit(h.p,-w,b,false);
        float before=max3(b);float3 next;float pf,pr;
        sv_media=wk.media;if(!sample_vertex(y,w,false,0,next,pf,pr,b,ab))return;
        cross_surface(y,next,wk);
        if(k>=2){float survival=clamp(max3(b)/max(before,1e-30),0.05,0.95);if(random()>survival)return;b/=survival;}
        o=h.p+h.ng*(dot(next,h.ng)>0?0.0003:-0.0003);w=next;
    }
}
// The photons within the radius, through the BSDF toward v (flux over the gather disk).
float3 gather(BVertex x,Material m,float3 v){
    float r=photon.x;float3 sum=0;int3 c0=(int3)floor(x.p/r);
    for(int k=0;k<27;k++){
        int3 c=c0+int3(k%3,(k/3)%3,k/9)-1;uint j=aux.Load(16+4*cell_hash(c));float3 cell=0;uint guard=0;
        for(;j!=0xffffffffu && guard<4096;guard++){
            uint a=PHOTON_BASE+j*32;uint4 A=aux.Load4(a),B=aux.Load4(a+16);float3 q=asfloat(A.xyz),dq=q-x.p;
            if((B.x>>31)==0 && all((int3)floor(q/r)==c) && dot(dq,dq)<r*r){float3 wi=unpack_normal(A.w);float pdf;
                if(dot(x.n,wi)>0)cell+=eval_bsdf(m,x.n,x.tangent,v,wi,pdf)*asfloat(B.xyz);}
            j=B.w;
        }
        // A chain longer than the walk: count the rest and scale up the walked part (the chain's
        // order is random, so the first 4096 stand for the whole cell) instead of going dark.
        uint total=guard;for(;j!=0xffffffffu && total<(1u<<20);total++)j=aux.Load(PHOTON_BASE+j*32+28);
        sum+=cell*(float(total)/max(guard,1u));
    }
    return sum/(max(integrator.x,1u)*PI*r*r);
}
// In-scattered radiance toward -d at a scatter point x from the volume photons within the
// radius: the phase function times their power over the sphere's volume and the extinction
// (photons were deposited at sigma_t's rate along their flights).
float3 gather_volume(float3 x,float3 d,float g,float sigma){
    float r=photon.x;float3 sum=0;int3 c0=(int3)floor(x/r);
    for(int k=0;k<27;k++){
        int3 c=c0+int3(k%3,(k/3)%3,k/9)-1;uint j=aux.Load(16+4*cell_hash(c));float3 cell=0;uint guard=0;
        for(;j!=0xffffffffu && guard<4096;guard++){
            uint a=PHOTON_BASE+j*32;uint4 A=aux.Load4(a),B=aux.Load4(a+16);float3 q=asfloat(A.xyz),dq=q-x;
            if((B.x>>31)!=0 && all((int3)floor(q/r)==c) && dot(dq,dq)<r*r)
                cell+=hg_pdf(dot(unpack_normal(A.w),d),g)*float3(asfloat(B.x&0x7fffffffu),asfloat(B.y),asfloat(B.z));
            j=B.w;
        }
        uint total=guard;for(;j!=0xffffffffu && total<(1u<<20);total++)j=aux.Load(PHOTON_BASE+j*32+28);
        sum+=cell*(float(total)/max(guard,1u));
    }
    return sum/(max(integrator.x,1u)*(4.0/3.0)*PI*r*r*r*max(sigma,1e-8));
}
// Direct light at a scatter point: NEE weighted by the phase pdf and one phase-sampled ray
// (balance heuristic), both through the medium's transmittance.
float3 volume_direct(float3 x,float3 d,float g,uint medium,float p_env){
    float3 L=0,ld,le;float pl,limit;
    if(sample_light(x,hdri_on(),scene_info.x>0,p_env,ld,le,pl,limit)){
        float pp=hg_pdf(dot(d,ld),g);
        if(!occluded(x,ld,limit))L+=pp*le*segment_tr(medium,x,ld,limit)/max(pl+pp,1e-8);
    }
    float pf;float3 next=hg_sample(d,g,pf);Hit h;
    if(!intersect(x,next,1e4,h,false,false))L+=escape(next,false,false,pf,p_env)*segment_tr(medium,x,next,1e4);
    else{float3 e=vertex_emission(make_vertex(h,1,0),-next);
        if(any(e>0)){float w=h.emitter!=0?pf/(pf+(1-p_env)*emitter_pdf(x,h.p,emitters[h.emitter-1])):1;L+=e*w*segment_tr(medium,x,next,h.t);}}
    return L;
}
// Direct light at a gather point: NEE and one BSDF sample (no transmission), balance-weighted.
float3 direct_light_at(BVertex x,Material m,float3 v,float p_env){
    float3 L=0;float z=m.lobes.z;
    float3 ld,le;float pl,limit;
    if(sample_light(x.p,hdri_on(),scene_info.x>0,p_env,ld,le,pl,limit) && dot(x.n,ld)>0){
        float pdf;float3 f=eval_bsdf(m,x.n,x.tangent,v,ld,pdf);pdf/=1-z;
        if(any(f>0) && !occluded(x.p+x.n*0.0003,ld,limit))L+=f*dot(x.n,ld)*le*segment_tr(x.medium,x.p,ld,limit)/max(pl+pdf,1e-8);
    }
    BVertex y=x;float3 b=1,ab=0,next;float pf,pr;
    if(sample_vertex(y,-v,true,2,next,pf,pr,b,ab)){
        Hit h;
        if(!intersect(x.p+x.n*0.0003,next,1e4,h,false,false))L+=b*escape(next,false,false,pf,p_env)*segment_tr(x.medium,x.p,next,1e4);
        else{float3 e=vertex_emission(make_vertex(h,1,0),-next);
            if(any(e>0)){float w=h.emitter!=0?pf/(pf+(1-p_env)*emitter_pdf(x.p,h.p,emitters[h.emitter-1])):1;L+=b*e*w*segment_tr(x.medium,x.p,next,h.t);}}
    }
    return L;
}
[numthreads(8,8,1)]void photon_gather(uint3 tid:SV_DispatchThreadID){
    uint2 p=tid.xy+region.xy;if(any(p>=min(dimensions.xy,region.zw)))return;
    white_noise=true;glass_blocks=true;pixel_seed=hash(p.x+p.y*dimensions.x);rng_state=hash(pixel_seed^hash(sampling.w*0x6c8e9cf5u+0x3c6ef372u));
    cone_width=pixel_width_now();cone_spread=pixel_angle_now();
    Hit guide;write_guides(p,guide);
    float p_env=env_share();float3 sum=0;
    for(uint k=0;k<sampling.y;k++){
        float2 sub=(scene_info.w&1)!=0?jitter.xy:float2(random(),random())-0.5;
        float3 o,d;bool valid;camera_ray(float2(p)+0.5+sub,true,o,d,valid);if(!valid)continue;
        cone_width=pixel_width_now();cone_spread=pixel_angle_now();
        float3 beta=camera_weight,ab=0,L=0;Walk wk=new_walk();
        for(uint bounce=0;bounce<min(sampling.z,PHOTON_WALK);bounce++){
            Hit h;float3 xs,alb;float g,sigma;uint step=walk_step(o,d,bounce==0,wk,beta,h,xs,alb,g,sigma);
            if(step==2){
                // A scatter: direct light plus the volume photons, and the path ends there.
                beta*=alb;
                L+=beta*(volume_direct(xs,d,g,top_entry(wk.media),p_env)+gather_volume(xs,d,g,sigma));break;
            }
            if(step==0){L+=beta*escape(d,bounce==0,true,0,p_env);break;}
            BVertex x=make_vertex(h,beta,0);x.medium=top_entry(wk.media);
            // Reached through the camera, delta surfaces or near mirrors: no other strategy finds it.
            L+=beta*vertex_emission(x,-d);
            if((x.info&V_NO_BSDF)!=0)break;
            Material m=vertex_material(x,-d);float3 next;float pf,pr;
            if(gather_surface(m)){
                L+=beta*(direct_light_at(x,m,-d,p_env)+(keeps_photons(m)?gather(x,m,-d):0));
                // The dielectric part of a partly transmissive surface goes on.
                sv_media=wk.media;if(m.lobes.z<=0 || random()>=m.lobes.z || !sample_vertex(x,d,true,1,next,pf,pr,beta,ab))break;
            }else{sv_media=wk.media;if(!sample_vertex(x,d,true,0,next,pf,pr,beta,ab))break;}
            cross_surface(x,next,wk);
            o=h.p+h.ng*(dot(next,h.ng)>0?0.0003:-0.0003);d=next;
        }
        if(all(isfinite(L)))sum+=max(L,0);
    }
    finish_frame(p,sum/max(sampling.y,1u));
}
