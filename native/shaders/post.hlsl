#include "common.hlsli"
// Native SVGF-style denoiser, display only (the progressive accumulation stays unbiased).
//   0 reproject: on a camera move, warp last frame's integrated color + moments into this
//     view as the history (bilinear, each tap checked by object id, normal and depth).
//   1 integrate: blend this frame's raw samples with that history. While the camera is
//     still, the history stays frozen, so accumulated samples are never counted twice.
//   2–6 à-trous: integrated -> ping -> pong -> ping -> pong -> ping (display reads ping).
// Everything is in demodulated (albedo-divided) light until the last pass. Variance is of
// demodulated luminance, carried in moments.z then ping/pong alpha.
float3 albedo_at(int2 q){return max(diffuse_albedo[q].rgb+specular_albedo[q].rgb,0.05);}
float3 view_dir(float2 pixel){float2 ndc=(pixel/float2(dimensions.xy))*2-1;ndc.y=-ndc.y;float4 h=mul(inverse_vp,float4(ndc,1,1));return normalize(h.xyz/h.w-eye.xyz);}
float3 world_at(int2 q,float3 d){return eye.xyz+d*(depth[q]/max(-mul(world_view,float4(d,0)).z,1e-4));}
float3 view_pos(int2 q,float z){float2 ndc=((float2(q)+0.5)/float2(dimensions.xy))*2-1;return float3(ndc.x*environment.y,-ndc.y*environment.z,-1)*z;}
float2 sign_nz(float2 v){return float2(v.x>=0?1:-1,v.y>=0?1:-1);}
float2 oct_encode(float3 n){n/=abs(n.x)+abs(n.y)+abs(n.z)+1e-9;return n.z>=0?n.xy:(1-abs(n.yx))*sign_nz(n.xy);}
float3 oct_decode(float2 e){float3 n=float3(e,1-abs(e.x)-abs(e.y));n.xy-=saturate(-n.z)*sign_nz(n.xy);return normalize(n);}
void reproject(uint2 p){
    float3 n=normal_roughness[p].xyz;float3 P=world_at(int2(p),view_dir(float2(p)+0.5));float4 previous=mul(previous_vp,float4(P,1));float id=object_id[p];
    float2 at=float2(p)+motion[p];int2 base=int2(floor(at));float2 f=at-base;
    float4 color=0,moment=0;float total=0;
    for(int i=0;i<4;i++){
        int2 o=int2(i&1,i>>1);int2 q=base+o;if(any(q<0)||any(q>=int2(dimensions.xy)))continue;
        if(history_id[q]!=id || dot(n,history_normal[q].xyz)<0.9 || abs(history_depth[q]-previous.w)>0.03*previous.w+0.01)continue;
        float w=(o.x!=0?f.x:1-f.x)*(o.y!=0?f.y:1-f.y);color+=w*integrated[q];moment+=w*moments[q];total+=w;
    }
    // Partly covered pixels get proportionally less trust; mostly uncovered ones start over.
    if(total<0.25){history[p]=0;history_moments[p]=0;return;}
    color/=total;color.a*=saturate(total);history[p]=color;history_moments[p]=moment/total;
}
void integrate(uint2 p){
    float count=max(1,sampling.x+sampling.y);float4 acc=accumulation[p];float id=object_id[p];
    float3 alb=albedo_at(int2(p));float ld=luminance(acc.rgb/alb),lr=luminance(acc.rgb);float s=lr>1e-6?ld/lr:1/luminance(alb);
    float2 m=float2(ld,acc.a/count*s*s+ld*ld);float3 c=acc.rgb/alb;float effective=count;
    if(filter_settings.y!=0 && filter_settings.z==0){
        // History cap down to 2 where the first bounce is specular (reflections move with the
        // view). History fades out over 256 fresh samples.
        float spec=0,all_light=0;
        for(int y=-1;y<=1;y++)for(int x=-1;x<=1;x++){int2 q=clamp(int2(p)+int2(x,y),0,int2(dimensions.xy)-1);spec+=luminance(specular_light[q].rgb);all_light+=luminance(accumulation[q].rgb);}
        // Motionless pixels reproject exactly, so they may keep a much longer history (this keeps
        // per-frame upscaler input clean on a still camera); moving ones are capped at 24 samples.
        float4 h=history[p];float cap=dot(motion[p],motion[p])<1e-4?256:24;float h_eff=min(h.a,lerp(cap,2,saturate(spec/max(all_light,1e-4))))*saturate(1-count/256);
        float w=h_eff/(h_eff+count);c=lerp(c,h.rgb,w);m=lerp(m,history_moments[p].xy,w);effective=h_eff+count;
    }
    float variance;
    if(effective<4){
        // Too few samples for moments: use the spread of same-surface neighbors instead.
        float s1=0,s2=0,k=0;
        for(int y=-2;y<=2;y++)for(int x=-2;x<=2;x++){int2 q=int2(p)+int2(x,y);if(any(q<0)||any(q>=int2(dimensions.xy))||object_id[q]!=id)continue;float l=luminance(accumulation[q].rgb/albedo_at(q));s1+=l;s2+=l*l;k++;}
        variance=max(0,s2/k-(s1/k)*(s1/k))*count/effective;
    }else variance=max(0,m.y-m.x*m.x)/effective;
    integrated[p]=float4(c,effective);moments[p]=float4(m,variance,0);
    filter_guide[p]=float4(oct_encode(normal_roughness[p].xyz),depth[p],id);
}
float4 source(int2 q,uint pass){return pass==2?float4(integrated[q].rgb,moments[q].z):((pass&1)==1?ping[q]:pong[q]);}
void store(uint2 p,uint pass,float4 v){if(pass==6)v.rgb*=albedo_at(int2(p));if((pass&1)==0)ping[p]=v;else pong[p]=v;}
[numthreads(8,8,1)]void filter(uint3 tid:SV_DispatchThreadID){
    uint2 p=tid.xy;if(any(p>=dimensions.xy))return;uint pass=filter_settings.x;
    if(pass==0){reproject(p);return;}
    if(pass==1){integrate(p);return;}
    // À-trous over demodulated light. Each tap reads two textures: color + variance, and the
    // packed guide (octahedral normal, depth, object id). The last pass restores albedo.
    float4 center=source(int2(p),pass);float4 g=filter_guide[p];float id=g.w;
    if(id==0){store(p,pass,center);return;}
    float3 n=oct_decode(g.xy);float3 nv=mul(world_view,float4(n,0)).xyz;float3 P=view_pos(int2(p),g.z);float pixel=2*environment.z*g.z/dimensions.y;
    // Luminance stop from a 3x3 Gaussian of the variance (one lucky low-variance pixel no longer becomes a speck).
    float gv=0,gw=0;
    for(int y=-1;y<=1;y++)for(int x=-1;x<=1;x++){int2 q=int2(p)+int2(x,y);if(any(q<0)||any(q>=int2(dimensions.xy))||filter_guide[q].w!=id)continue;float k=(x==0?2:1)*(y==0?2:1);gv+=k*source(q,pass).a;gw+=k;}
    float stop=8*max(0.01,options.y)*sqrt(gv/gw)+1e-4;float l=luminance(center.rgb);
    float3 sum=0;float total=0,variance=0;int step=int(1u<<(pass-2));
    for(int y=-2;y<=2;y++)for(int x=-2;x<=2;x++){
        // Never mix surfaces: the light sits 3.5 cm under the ceiling with the same normal.
        int2 q=int2(p)+int2(x,y)*step;if(any(q<0)||any(q>=int2(dimensions.xy)))continue;
        float4 h=filter_guide[q];if(h.w!=id)continue;float4 c=source(q,pass);
        // Plane distance in view space: positions are a ray scale times depth.
        float plane=exp(-abs(dot(nv,view_pos(q,h.z)-P))/(2*pixel*length(float2(x,y))*step+1e-4));
        float w=(x==0?6:(abs(x)==1?4:1))*(y==0?6:(abs(y)==1?4:1))*pow(saturate(dot(n,oct_decode(h.xy))),64)*plane*exp(-abs(luminance(c.rgb)-l)/stop);
        sum+=c.rgb*w;total+=w;variance+=w*w*c.a;
    }
    store(p,pass,total>1e-6?float4(sum/total,variance/(total*total)):center);
}
float filmic(float x){return ((x*(0.15*x+0.05)+0.004)/(x*(0.15*x+0.5)+0.06))-0.06666667;}
float3 tonemap(float3 c){c=max(c*display_settings.x,0);uint mode=modes.y;
    if(mode==1)c=c/(1+c);
    if(mode==2)c=c*(1+c/16)/(1+c);
    if(mode==3)c=saturate(c*(2.51*c+0.03)/(c*(2.43*c+0.59)+0.14));
    if(mode==4)c=float3(filmic(c.r*2),filmic(c.g*2),filmic(c.b*2))/filmic(11.2);
    if(mode==5){c=saturate(c*(2.51*c+0.03)/(c*(2.43*c+0.59)+0.14));c=smoothstep(0,1,c);}
    return pow(saturate(c),1/max(0.1,display_settings.y));
}
// ST.2084 (PQ) encode/decode in nits, for the BT.2390 roll-off below.
float pq(float nits){float y=pow(clamp(nits,0,10000)/10000,0.1593017578125);return pow((0.8359375+18.8515625*y)/(1+18.6875*y),78.84375);}
float pq_inverse(float e){float p=pow(saturate(e),1/78.84375);return 10000*pow(max(p-0.8359375,0)/(18.8515625-18.6875*p),1/0.1593017578125);}
// HDR display transform: scene-linear x exposure x paper white (nits), left untouched up to the
// BT.2390 knee and rolled off into the panel peak. Applied to the brightest channel, so hue is kept.
// Returns scRGB (1.0 = 80 nits).
float3 hdr_map(float3 c){
    c=max(c*display_settings.x,0)*hdr.y;float m=max(c.r,max(c.g,c.b));if(m<=0)return 0;
    float peak=pq(hdr.z),knee=1.5*peak-0.5,e=pq(m);
    if(e>knee){float t=(e-knee)/(1-knee),t2=t*t,t3=t2*t;e=(2*t3-3*t2+1)*knee+(t3-2*t2+t)*(1-knee)+(-2*t3+3*t2)*peak;}
    return c*(min(pq_inverse(e),hdr.z)/m)/80;
}
// An SDR-encoded value (debug views, overlays) shown at paper white in HDR.
float3 sdr_in_hdr(float3 c){return pow(saturate(c),max(0.1,display_settings.y))*hdr.y/80;}
float3 native_at(int2 q){q=clamp(q,0,int2(dimensions.xy)-1);return modes.z!=0?ping[q].rgb:accumulation[q].rgb;}
float catmull_rom(float t,int i){float a=abs(t-i);return a<1?1.5*a*a*a-2.5*a*a+1:(a<2?-0.5*a*a*a+2.5*a*a-4*a+2:0);}
// Spatial upscale of the native image (render resolution) to output pixel op: bilinear or
// Catmull-Rom bicubic. At render scale 1 this is an exact copy.
float3 native_upscaled(uint2 op){
    if(all(dimensions.xy==dimensions.zw))return native_at(int2(op));
    float2 at=(float2(op)+0.5)*float2(dimensions.xy)/float2(dimensions.zw)-0.5;int2 b=int2(floor(at));float2 f=at-b;
    float3 sum=0;float total=0;
    if(((scene_info.w>>2)&3)==1){for(int i=0;i<4;i++){int2 o=int2(i&1,i>>1);float w=(o.x!=0?f.x:1-f.x)*(o.y!=0?f.y:1-f.y);sum+=w*native_at(b+o);total+=w;}}
    else{
        for(int y=-1;y<=2;y++)for(int x=-1;x<=2;x++){float w=catmull_rom(f.x,x)*catmull_rom(f.y,y);sum+=w*native_at(b+int2(x,y));total+=w;}
        // Catmull-Rom overshoots at HDR edges (a dark rim around the light); clamp to the 2x2 footprint.
        float3 lo=native_at(b),hi=lo;for(int i=1;i<4;i++){float3 v=native_at(b+int2(i&1,i>>1));lo=min(lo,v);hi=max(hi,v);}
        return clamp(sum/total,lo,hi);
    }
    return max(sum/total,0);
}
float3 beauty(uint2 p,uint2 op){
    float3 raw=accumulation[p].rgb;float3 c=(scene_info.w&2)!=0?reconstruction[op].rgb:native_upscaled(op);
    if(display_settings.z!=1 || display_settings.w!=1){float3 composed=direct_light[p].rgb*display_settings.z+(raw-direct_light[p].rgb)*display_settings.w;c*=max(composed,0)/max(raw,0.0001);}
    return max(c,0);
}
[numthreads(8,8,1)]void display(uint3 tid:SV_DispatchThreadID){
    uint2 op=tid.xy;if(any(op>=dimensions.zw))return;uint2 p=min(uint2((float2(op)+0.5)*float2(dimensions.xy)/float2(dimensions.zw)),dimensions.xy-1);
    float3 c=beauty(p,op);linear_output[op]=float4(c,1);
    if(modes.x==1)c=normal_roughness[p].xyz*0.5+0.5;
    else if(modes.x==2)c=pow(diffuse_albedo[p].rgb,1/2.2);
    else if(modes.x==3)c=exp(-depth[p]*0.3).xxx;
    else if(modes.x==4)c=tonemap(direct_light[p].rgb);
    else if(modes.x==5)c=tonemap(max(0,accumulation[p].rgb-direct_light[p].rgb));
    else if(modes.x==6)c=tonemap(specular_light[p].rgb);
    else if(modes.x==7){float n=max(2,sampling.x+sampling.y);float se=sqrt(accumulation[p].a/(n*(n-1)))/max(0.01,luminance(accumulation[p].rgb));float t=saturate((log2(max(se,0.001))-log2(0.001))/(log2(0.3)-log2(0.001)));c=saturate(float3(t*2,1-abs(t*2-1),2-t*2));}
    float3 h;bool beauty_view=modes.x==0;
    if(beauty_view){if(options.z>0 && filter_settings.w!=0 && float(op.x)/dimensions.z<options.w)c=reference_image[op].rgb;h=hdr_map(c);c=tonemap(c);if(options.z>0 && abs(float(op.x)/dimensions.z-options.w)<1.0/dimensions.z){c=float3(0.3,0.8,1);h=sdr_in_hdr(c);}}
    else h=sdr_in_hdr(c);
    output_image[op]=float4(c,1);
    if(hdr.x>0)hdr_output[op]=float4(h,1);
}

