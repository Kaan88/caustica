#include "common.hlsli"
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
struct Hit{float t;float3 p;float3 n;uint material;uint emitter;bool front;};
bool intersect(float3 o,float3 d,float limit,out Hit h,bool shadow,bool camera_ray){
    RayDesc ray;ray.Origin=o;ray.Direction=d;ray.TMin=0.0001;ray.TMax=limit;
    // Nonopaque geometry allows the light quad and optional glass to be ignored for shadow rays.
    RayQuery<RAY_FLAG_NONE> q;q.TraceRayInline(scene,RAY_FLAG_NONE,255,ray);
    while(q.Proceed()){proceed_steps++;
        uint m=vertices[q.CandidatePrimitiveIndex()*3].material;
        if(camera_ray && (((uint)materials[m].lobes.w&2u)!=0))continue;
        if(!shadow || jitter.w>0.5 || materials[m].lobes.z<0.5)q.CommitNonOpaqueTriangleHit();
    }
    h=(Hit)0;
    if(q.CommittedStatus()!=COMMITTED_TRIANGLE_HIT)return false;
    uint idx=q.CommittedPrimitiveIndex()*3;float2 b=q.CommittedTriangleBarycentrics();
    Vertex a=vertices[idx],v=vertices[idx+1],c=vertices[idx+2];h.t=q.CommittedRayT();h.p=o+d*h.t;
    h.n=normalize(a.normal*(1-b.x-b.y)+v.normal*b.x+c.normal*b.y);h.front=dot(d,h.n)<0;if(!h.front)h.n=-h.n;h.material=a.material;h.emitter=emitter_lookup[q.CommittedPrimitiveIndex()];return true;
}
// Shadow test: any blocker ends the search, and no hit data is reconstructed.
bool occluded(float3 o,float3 d,float limit){
    RayDesc ray;ray.Origin=o;ray.Direction=d;ray.TMin=0.0001;ray.TMax=limit;
    RayQuery<RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH> q;q.TraceRayInline(scene,RAY_FLAG_NONE,255,ray);
    while(q.Proceed()){proceed_steps++;uint m=vertices[q.CandidatePrimitiveIndex()*3].material;if(jitter.w>0.5 || materials[m].lobes.z<0.5)q.CommitNonOpaqueTriangleHit();}
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
float3 environment_radiance(float3 d){return background.rgb+environment.x*lerp(float3(0.8,0.87,1.0),float3(0.16,0.35,0.72),saturate(d.y*0.5+0.5));}
float3 fresnel(float c,float3 f0){return f0+(1-f0)*pow(1-saturate(c),5);}
float ggx_d(float nh,float a){float d=nh*nh*(a*a-1)+1;return a*a/(PI*d*d);}
float g1(float nv,float a){return 2*nv/max(1e-6,nv+sqrt(a*a+(1-a*a)*nv*nv));}
float3 f0_of(Material m){float f=pow((m.optical.x-1)/(m.optical.x+1),2);return lerp(lerp(f.xxx,1.0.xxx,m.lobes.y),m.color_rough.rgb,m.lobes.x);}
float spec_probability(Material m){return clamp(0.25+0.5*m.lobes.x+0.25*m.lobes.y,0.1,0.95);}
float3 eval_bsdf(Material m,float3 n,float3 v,float3 l,out float pdf){
    float nl=saturate(dot(n,l)),nv=saturate(dot(n,v));float3 h=normalize(v+l);float nh=saturate(dot(n,h)),vh=saturate(dot(v,h));float a=max(0.001,m.color_rough.a*m.color_rough.a);float d=ggx_d(nh,a);float3 f=fresnel(vh,f0_of(m));float ps=spec_probability(m);
    pdf=(1-m.lobes.z)*((1-ps)*nl/PI+ps*d*nh/max(4*vh,1e-6));
    float3 diff=(1-f)*(1-m.lobes.x)*m.color_rough.rgb/PI;
    return (1-m.lobes.z)*(diff+f*d*g1(nv,a)*g1(nl,a)/max(4*nv*nl,1e-6));
}
float emitter_pdf(float3 from,float3 at,Emitter light){float3 d=at-from;float dist2=dot(d,d);float3 n=normalize(cross(light.edge1_cdf.xyz,light.edge2_probability.xyz));float c=abs(dot(n,normalize(d)));return light.edge2_probability.w*dist2/max(1e-8,light.position_area.w*c);}
Emitter select_emitter(float u){uint lo=0,hi=scene_info.x-1;while(lo<hi){uint mid=(lo+hi)/2;if(u<emitters[mid].edge1_cdf.w)hi=mid;else lo=mid+1;}return emitters[lo];}
float3 camera_direction(float2 pixel){float2 ndc=(pixel/float2(dimensions.xy))*2-1;ndc.y=-ndc.y;float4 p=mul(inverse_vp,float4(ndc,1,1));return normalize(p.xyz/p.w-eye.xyz);}
[numthreads(8,8,1)]void trace(uint3 tid:SV_DispatchThreadID){
    uint2 p=tid.xy;if(any(p>=dimensions.xy))return;
    pixel_seed=hash(p.x+p.y*dimensions.x);rng_state=hash(pixel_seed+sampling.w*747796405);proceed_steps=0;float bounces_used=0;
    float3 mean=sampling.x>0?accumulation[p].rgb:0;float m2=sampling.x>0?accumulation[p].a:0;
    float3 dmean=sampling.x>0?direct_light[p].rgb:0,smean=sampling.x>0?specular_light[p].rgb:0;
    Hit guide;float3 gd=camera_direction(float2(p)+0.5+jitter.xy);
    if(intersect(eye.xyz,gd,1e4,guide,false,true)){
        Material m=material_at(guide.material,guide.p);normal_roughness[p]=float4(guide.n,m.color_rough.a);object_id[p]=guide.material+1;
        diffuse_albedo[p]=float4(m.color_rough.rgb*(1-m.lobes.x)*(1-m.lobes.z),1);
        specular_albedo[p]=float4(fresnel(abs(dot(-gd,guide.n)),f0_of(m)),1);
        depth[p]=-mul(world_view,float4(guide.p,1)).z;
        float4 projected=mul(current_vp,float4(guide.p,1));device_depth[p]=projected.z/projected.w;
        float4 prev=mul(previous_vp,float4(guide.p,1)),curr=mul(current_vp,float4(guide.p,1));
        float2 delta=prev.xy/prev.w-curr.xy/curr.w;motion[p]=delta*float2(0.5,-0.5)*dimensions.xy;
        // A stable primary reflection ray gives RR consistent reflection-motion
        // guidance, instead of alternating random-lobe / zero / sky distances.
        // Only RR reads it; the native path skips the extra ray.
        Hit reflected;float3 reflected_direction=reflect(gd,guide.n);
        specular_distance[p]=modes.w!=0 && intersect(guide.p+guide.n*0.0003,reflected_direction,1e4,reflected,false,false)?reflected.t:65504.0;
    }else{normal_roughness[p]=float4(0,0,0,1);object_id[p]=0;diffuse_albedo[p]=0;specular_albedo[p]=0;depth[p]=1e4;device_depth[p]=1;motion[p]=0;specular_distance[p]=65504.0;}
    float3 batch=0;
    for(uint sample_id=0;sample_id<sampling.y;sample_id++){
        // The base changes on every reset, so a moving camera never reuses the previous frame's points.
        sequence_index=scene_info.z+sampling.x+sample_id;dimension_index=0;
        // RR and the temporal upscalers need every sample of a frame at that frame's jitter.
        float2 subpixel=(scene_info.w&1)!=0?jitter.xy:float2(random(),random())-0.5;
        float3 direction=camera_direction(float2(p)+0.5+subpixel),origin=eye.xyz,throughput=1,radiance=0,direct=0;bool first_spec=false,previous_delta=true;float previous_pdf=0;float3 previous_position=origin;float3 absorption=0;
        for(uint bounce=0;bounce<sampling.z;bounce++){
            bounces_used+=1;
            Hit h;if(!intersect(origin,direction,1e4,h,false,bounce==0)){float3 e=throughput*environment_radiance(direction);radiance+=e;if(bounce<=1)direct+=e;break;}
            throughput*=exp(-absorption*h.t);
            Material m=material_at(h.material,h.p);float3 emitted=m.color_rough.rgb*m.optical.y;
            if(m.optical.w>0.5 && !h.front)emitted=0;
            float weight=1;if(h.emitter!=0 && bounce>0 && !previous_delta)weight=previous_pdf/(previous_pdf+emitter_pdf(previous_position,h.p,emitters[h.emitter-1]));
            float3 e=throughput*emitted*weight;radiance+=e;if(bounce<=1)direct+=e;
            if(((uint)m.lobes.w&1u)!=0)break;
            if(scene_info.x>0){
            Emitter light=select_emitter(random());float su=sqrt(random()),v=random();float3 lp=light.position_area.xyz+light.edge1_cdf.xyz*(1-su)+light.edge2_probability.xyz*(su*v);float3 ld=lp-h.p;float len=length(ld);ld/=max(len,1e-8);
            Material lm=material_at((uint)light.emission_material.w,lp);float3 ln=normalize(cross(light.edge1_cdf.xyz,light.edge2_probability.xyz));
            float nl=dot(h.n,ld),pdf;float3 f=eval_bsdf(m,h.n,-direction,ld,pdf);
            if(nl>0 && (lm.optical.w<0.5 || dot(ln,-ld)>0)){
                if(!occluded(h.p+h.n*0.0003,ld,max(0.0002,len-0.0006))){
                    float pl=emitter_pdf(h.p,lp,light);float3 e=throughput*f*nl*(lm.color_rough.rgb*lm.optical.y)/max(pl+pdf,1e-8);radiance+=e;if(bounce==0)direct+=e;
                }
            }}
            float3 next;bool delta=false;float bsdf_pdf=0;
            if(random()<m.lobes.z){
                float eta=h.front?1/m.optical.x:m.optical.x;float cosi=saturate(dot(-direction,h.n));float cost2=1-eta*eta*(1-cosi*cosi);float f=pow((m.optical.x-1)/(m.optical.x+1),2);float fr=f+(1-f)*pow(1-(h.front?cosi:sqrt(max(0,cost2))),5);
                bool reflected=cost2<=0 || random()<fr;
                next=reflected?reflect(direction,h.n):refract(direction,h.n,eta);
                if(!reflected){absorption=h.front?-log(max(m.color_rough.rgb,0.001)):0;throughput*=eta*eta;}
                if(m.color_rough.a>0.001){float3 pert=cosine_sample(reflected?h.n:-h.n);next=normalize(lerp(next,pert,m.color_rough.a*m.color_rough.a));}
                delta=true;
            }else{
                float ps=spec_probability(m);if(random()<ps){float a=max(0.001,m.color_rough.a*m.color_rough.a);float u=random(),phi=2*PI*random();float c=sqrt((1-u)/(1+(a*a-1)*u));float3 hvec=basis(h.n,float3(sqrt(max(0,1-c*c))*cos(phi),sqrt(max(0,1-c*c))*sin(phi),c));next=reflect(direction,hvec);if(bounce==0)first_spec=true;
                }else next=cosine_sample(h.n);
                if(dot(h.n,next)<=0)break;
                float3 bsdf=eval_bsdf(m,h.n,-direction,next,bsdf_pdf);throughput*=bsdf*abs(dot(h.n,next))/max(bsdf_pdf,1e-7);
            }
            if(bounce==0 && delta)first_spec=true;
            previous_pdf=bsdf_pdf;previous_delta=delta;previous_position=h.p;
            direction=normalize(next);origin=h.p+h.n*(dot(direction,h.n)>0?0.0003:-0.0003);
            if(bounce>=3){float survival=clamp(max(throughput.r,max(throughput.g,throughput.b)),0.05,0.95);if(random()>survival)break;throughput/=survival;}
        }
        if(!all(isfinite(radiance))){radiance=0;direct=0;}
        radiance=max(radiance,0);if(options.x>0 && sampling.x>4){float l=luminance(radiance);float scale=min(1,options.x*max(luminance(mean),0.05)/max(l,1e-6));radiance*=scale;direct*=scale;}
        float count=sampling.x+sample_id+1;float delta_l=luminance(radiance)-luminance(mean);mean+=(radiance-mean)/count;m2+=delta_l*(luminance(radiance)-luminance(mean));
        dmean+=(direct-dmean)/count;smean+=((first_spec?radiance:0)-smean)/count;batch+=radiance/sampling.y;
    }
    trace_stats[p]=float2(bounces_used/max(1,sampling.y),proceed_steps);
    noisy[p]=float4(batch,1);accumulation[p]=float4(mean,max(0,m2));direct_light[p]=float4(dmean,1);specular_light[p]=float4(smean,1);
}

