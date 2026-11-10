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
    // max: the spectral tracer's mean may hold negative components (engine-notes "Spectral
    // rendering"); RGB means never do.
    float count=max(1,sampling.x+sampling.y);float4 acc=accumulation[p];acc.rgb=max(acc.rgb,0);float id=object_id[p];
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
        for(int y=-2;y<=2;y++)for(int x=-2;x<=2;x++){int2 q=int2(p)+int2(x,y);if(any(q<0)||any(q>=int2(dimensions.xy))||object_id[q]!=id)continue;float l=luminance(max(accumulation[q].rgb,0)/albedo_at(q));s1+=l;s2+=l*l;k++;}
        variance=max(0,s2/k-(s1/k)*(s1/k))*count/effective;
    }else variance=max(0,m.y-m.x*m.x)/effective;
    integrated[p]=float4(c,effective);moments[p]=float4(m,variance,0);
    filter_guide[p]=float4(oct_encode(normal_roughness[p].xyz),depth[p],id);
}
float4 source(int2 q,uint pass){return pass==2?float4(integrated[q].rgb,moments[q].z):((pass&1)==1?ping[q]:pong[q]);}
// The last pass gives edge pixels (coverage below 1: their samples hit several surfaces, which
// the surface stops can't tell apart) back their unfiltered share, then restores albedo.
void store(uint2 p,uint pass,float4 v){
    if(pass==6)v.rgb=lerp(integrated[p].rgb,v.rgb,saturate(diffuse_albedo[p].a))*albedo_at(int2(p));
    if((pass&1)==0)ping[p]=v;else pong[p]=v;
}
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
// Exposure meter and adaptation (E3; reference/display.rs is the CPU twin with the reasoning,
// render.rs METER_* the layout). The meter pass is this file's display entry dispatched once more
// before the display (meter_ctrl.x bit 1), one thread per 4 x 4 pixel cell: it bins the display
// input's log2 luminance, weighted, into row 260 of u28; the last group to finish (a counter at
// row 261 column 0) reduces it and adapts the state in row 261 from column 8, which the display
// reads. No CPU readback in the loop.
#define METER_ROW 260
#define METER_STATE 261
#define METER_MAGIC 0x4d455452
static const float METER_LOG_MIN=-40,METER_LOG_MAX=24,METER_FRAC=64,METER_LOW=0.01,METER_HIGH=0.97;
static const float ADAPT_BRIGHTER=0.2,ADAPT_DARKER=3,ADAPT_SCOTOPIC=10,ADAPT_SPEED=2,SCOTOPIC_BELOW=3e-7;
static const float EXPOSURE_EV_MIN=-8,EXPOSURE_EV_MAX=30;
groupshared uint meter_hist[256];
groupshared uint meter_last;
bool meter_valid(){return scopes[uint2(8,METER_STATE)]==METER_MAGIC;}
float meter_word(uint i){return asfloat(scopes[uint2(8+i,METER_STATE)]);}
bool auto_exposure(){return (meter_ctrl.x&1)!=0 && meter_valid();}
// The metered exposure (no bias), and the displayed one; the manual exposure before a reading.
float metered_exposure(){return auto_exposure()?exp2(meter_word(2)):display_settings.x;}
float exposure_now(){return auto_exposure()?exp2(meter_word(2)+meter.y):display_settings.x;}
// A film / lens threshold given in exposed units (1 = white after exposure), in scene units: so
// halation and glare catch the same highlights whatever the meter picks (E7b). Exposure 1 (the
// manual default) leaves it as it was.
float exposed_threshold(float t){return t/max(exposure_now(),1e-30);}
uint meter_weight(uint2 q,uint2 size){
    float2 uv=(float2(q)+0.5)/float2(size)*2-1;float r2=uv.x*uv.x+uv.y*uv.y;
    return meter_ctrl.z==1?(r2<0.04?8:0):meter_ctrl.z==2?8:1+uint(7*saturate(1-r2)+0.5);
}
float meter_pos(uint b){return b+(float(meter_hist[128+b])/float(meter_hist[b])+0.5)/METER_FRAC;}
// Thread 0 of the last group: the reading from meter_hist (when `reading`), then the step.
void meter_update(bool reading){
    bool valid=meter_valid();
    float base=meter_word(1),cur=meter_word(2),target=meter_word(3),scene=meter_word(4);
    bool got=false;
    if(reading){
        float total=0;for(uint b=0;b<128;b++)total+=meter_hist[b];
        if(total>0){
            float at=0;
            if(meter_ctrl.z==2){float cum=0;for(uint b=0;b<128;b++){cum+=meter_hist[b];if(meter_hist[b]>0 && cum>=total*0.5){at=meter_pos(b);break;}}}
            else{float lo=total*METER_LOW,hi=total*METER_HIGH,cum=0,acc=0;
                for(uint b=0;b<128;b++){float c=meter_hist[b];if(c>0){float kept=min(cum+c,hi)-max(cum,lo);if(kept>0)acc+=kept*meter_pos(b);}cum+=c;}
                at=acc/(hi-lo);}
            scene=METER_LOG_MIN+at*((METER_LOG_MAX-METER_LOG_MIN)/128);
            target=clamp(log2(0.18)-scene,EXPOSURE_EV_MIN,EXPOSURE_EV_MAX);got=true;
        }
    }
    if(!valid){if(!got)return;base=target;cur=target;}
    else{
        if((meter_ctrl.x&4)!=0)base=cur;
        if(meter_ctrl.y==1){if(got){base=target;cur=target;}}
        else{float tau=(target<base?ADAPT_BRIGHTER:scene<log2(SCOTOPIC_BELOW)?ADAPT_SCOTOPIC:ADAPT_DARKER)*ADAPT_SPEED/max(meter.z,1e-3);
            cur=base+(target-base)*(1-exp(-max(meter.x,0)/tau));}
    }
    scopes[uint2(8,METER_STATE)]=METER_MAGIC;scopes[uint2(9,METER_STATE)]=asuint(base);scopes[uint2(10,METER_STATE)]=asuint(cur);
    scopes[uint2(11,METER_STATE)]=asuint(target);scopes[uint2(12,METER_STATE)]=asuint(scene);
}
// The Eyes look (E4; reference/display.rs `eyes_mix` / `eyes_look` is the CPU twin with the
// reasoning): toward rod vision, the scotopic luminance in a Purkinje blue, between an adapted
// 3 and 0.01 cd/m2. Nothing at all above, so daylight stays bit-identical.
static const float EYES_PHOTOPIC_CD=3,EYES_SCOTOPIC_CD=0.01;
static const float3 EYES_ROD=float3(0.04,0.53,0.43),EYES_TINT=float3(0.80,1.00,1.30);
float eyes_mix(){
    if(meter.w<=0)return 0;
    float cd=SCENE_CD_M2*(0.18/max(metered_exposure(),1e-30));
    float t=saturate((log10(EYES_PHOTOPIC_CD)-log10(max(cd,1e-30)))/(log10(EYES_PHOTOPIC_CD)-log10(EYES_SCOTOPIC_CD)));
    return t*t*(3-2*t)*meter.w;
}
float3 eyes(float3 c){
    float m=eyes_mix();if(m<=0)return c;
    float rod=dot(max(c,0),EYES_ROD);
    return c+(EYES_TINT/luminance(EYES_TINT)*rod-c)*m;
}
float filmic(float x){return ((x*(0.15*x+0.05)+0.004)/(x*(0.15*x+0.5)+0.06))-0.06666667;}
// The tone curves on exposed scene light; the result is display light, 1 = white.
float3 tone_curve(float3 c,uint mode){
    if(mode==1)c=c/(1+c);
    if(mode==2)c=c*(1+c/16)/(1+c);
    if(mode==3)c=saturate(c*(2.51*c+0.03)/(c*(2.43*c+0.59)+0.14));
    if(mode==4)c=float3(filmic(c.r*2),filmic(c.g*2),filmic(c.b*2))/filmic(11.2);
    if(mode==5){c=saturate(c*(2.51*c+0.03)/(c*(2.43*c+0.59)+0.14));c=smoothstep(0,1,c);}
    return saturate(c);
}
// Tone maps 6 and 7. AgX: Benjamin Wrensch's polynomial fit of Troy Sobotka's AgX base.
float3 agx(float3 c){
    const float3x3 inset=float3x3(0.842479,0.0784336,0.0792237,0.0423282,0.878468,0.0791661,0.0423756,0.0784336,0.879142);
    const float3x3 outset=float3x3(1.19688,-0.0980209,-0.0990297,-0.0528969,1.15190,-0.0989612,-0.0529716,-0.0980435,1.15107);
    c=mul(inset,c);c=clamp(log2(max(c,1e-10)),-12.47393,4.026069);c=(c+12.47393)/16.5;
    float3 x2=c*c,x4=x2*x2;
    c=15.5*x4*x2-40.14*x4*c+31.96*x4-6.868*x2*c+0.4298*x2+0.1191*c-0.00232;
    return saturate(pow(max(mul(outset,c),0),2.2));
}
// Khronos PBR Neutral: keeps base colours, compresses only highlights.
float3 pbr_neutral(float3 c){
    const float start=0.8-0.04,desaturation=0.15;float x=min(c.r,min(c.g,c.b));float offset=x<0.08?x-6.25*x*x:0.04;c-=offset;
    float peak=max(c.r,max(c.g,c.b));if(peak<start)return c;float d=1-start;float np=1-d*d/(peak+d-start);c*=np/peak;
    float g=1-1/(desaturation*(peak-np)+1);return lerp(c,np.xxx,g);
}
// Film look on display light t (0..1): lift / gamma / gain on the encoded value, then the LUT.
float3 lut_at(int3 q){int n=(int)look.w;return lut[clamp(q,0,n-1)].rgb;}
float3 apply_lut(float3 e){
    float n=look.w;float3 x=saturate(e)*(n-1);int3 b=int3(floor(x));float3 f=x-b;
    float3 c00=lerp(lut_at(b),lut_at(b+int3(1,0,0)),f.x),c10=lerp(lut_at(b+int3(0,1,0)),lut_at(b+int3(1,1,0)),f.x);
    float3 c01=lerp(lut_at(b+int3(0,0,1)),lut_at(b+int3(1,0,1)),f.x),c11=lerp(lut_at(b+int3(0,1,1)),lut_at(b+int3(1,1,1)),f.x);
    return lerp(lerp(c00,c10,f.y),lerp(c01,c11,f.y),f.z);
}
float3 film_look(float3 t){
    float3 e=pow(saturate(t),1/2.2);
    e=gain.rgb*(e+lift.rgb*(1-e));e=pow(saturate(e),1/max(grade_gamma.rgb,0.05));
    if(look.w>1)e=lerp(e,apply_lut(e),grade_gamma.w);
    return pow(saturate(e),2.2);
}
float3 tonemap(float3 c){
    c=max(eyes(c)*exposure_now(),0);uint mode=modes.y;
    float3 t=mode==6?agx(c):mode==7?saturate(pbr_neutral(c)):tone_curve(c,mode);
    return pow(film_look(t),1/max(0.1,display_settings.y));
}
// ST.2084 (PQ) encode/decode in nits, for the BT.2390 roll-off below.
float pq(float nits){float y=pow(clamp(nits,0,10000)/10000,0.1593017578125);return pow((0.8359375+18.8515625*y)/(1+18.6875*y),78.84375);}
float pq_inverse(float e){float p=pow(saturate(e),1/78.84375);return 10000*pow(max(p-0.8359375,0)/(18.8515625-18.6875*p),1/0.1593017578125);}
// HDR display transform: scene-linear x exposure x paper white (nits), left untouched up to the
// BT.2390 knee and rolled off into the panel peak. Applied to the brightest channel, so hue is kept.
// Returns scRGB (1.0 = 80 nits).
float3 hdr_map(float3 c){
    // A tone map other than Linear keeps its SDR look: below half of white the display light is
    // exactly the SDR one at paper white. Above that knee the highlights expand into the panel's
    // headroom r = peak/white with g(u) = u/(1-u+u/R), slope-continuous at the knee, reaching r at 1.
    if(modes.y!=0){
        float3 x=max(eyes(c)*exposure_now(),0);
        float3 t=pow(film_look(modes.y==6?agx(x):modes.y==7?saturate(pbr_neutral(x)):tone_curve(x,modes.y)),2.2/max(0.1,display_settings.y));
        float r=max(hdr.z/hdr.y,1),R=(r-0.5)/0.5;float3 u=max(t-0.5,0)/0.5;
        return (min(t,0.5)+0.5*u/(1-u+u/R))*hdr.y/80;
    }
    c=max(eyes(c)*exposure_now(),0)*hdr.y;float m=max(c.r,max(c.g,c.b));if(m<=0)return 0;
    float peak=pq(hdr.z),knee=1.5*peak-0.5,e=pq(m);
    if(e>knee){float t=(e-knee)/(1-knee),t2=t*t,t3=t2*t;e=(2*t3-3*t2+1)*knee+(t3-2*t2+t)*(1-knee)+(-2*t3+3*t2)*peak;}
    return c*(min(pq_inverse(e),hdr.z)/m)/80;
}
// scRGB (BT.709, 1.0 = 80 nits) to HDR10 code values: BT.2020 primaries, PQ per channel.
float3 hdr10_encode(float3 c){
    float3 b=max(mul(float3x3(0.627404,0.329282,0.043314,0.069097,0.919540,0.011361,0.016392,0.088013,0.895595),c),0)*80;
    return float3(pq(b.r),pq(b.g),pq(b.b));
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
// Turbo-like heat map for 0..1.
float3 heat(float t){t=saturate(t);return saturate(float3(1.5-abs(4*t-3),1.5-abs(4*t-2),1.5-abs(4*t-1)))*smoothstep(0,0.08,t+0.02);}
// False colour: exposed scene luminance in stops around middle grey (0.18); in auto exposure at
// the metered exposure, so it shows stops relative to the meter (the bias not included).
float3 false_color(float3 c){
    float ev=log2(max(luminance(max(c*metered_exposure(),0)),1e-6)/0.18);
    if(ev<-6)return float3(0.25,0.0,0.35);if(ev<-4)return float3(0.1,0.15,0.75);if(ev<-2)return float3(0.0,0.5,0.6);
    if(ev<-0.5)return float3(0.35,0.35,0.35);if(ev<0.5)return float3(0.35,0.75,0.3);if(ev<2)return float3(0.7,0.7,0.7);
    if(ev<4)return float3(0.95,0.85,0.2);if(ev<6)return float3(1.0,0.5,0.1);return float3(1,0.1,0.1);
}
// Scopes: histogram, waveform and vectorscope of the displayed (SDR) image.
void bin_scopes(uint2 op,float3 c){
    c=saturate(c);float l=luminance(c);uint4 b=uint4(c*255+0.5,l*255+0.5);
    uint dummy;InterlockedAdd(scopes[uint2(b.x,0)],1,dummy);InterlockedAdd(scopes[uint2(b.y,1)],1,dummy);
    InterlockedAdd(scopes[uint2(b.z,2)],1,dummy);InterlockedAdd(scopes[uint2(b.w,3)],1,dummy);
    InterlockedAdd(scopes[uint2(op.x*256/dimensions.z,4+uint((1-l)*127+0.5))],1,dummy);
    float cb=(c.b-l)/1.8556,cr=(c.r-l)/1.5748;
    InterlockedAdd(scopes[uint2(uint(saturate(cb+0.5)*127+0.5),132+uint(saturate(0.5-cr)*127+0.5))],1,dummy);
}
// Scene light at an output pixel without the display's other work (for halation taps).
// The software view (Render tab): a split past the edge makes the whole image the CPU's, in what's
// saved and what the lens effects see too. The A/B split never goes past 1.
bool cpu_view(){return options.z>0 && filter_settings.w!=0 && options.w>1;}
float3 scene_at(int2 op){
    op=clamp(op,0,int2(dimensions.zw)-1);
    if(cpu_view())return reference_image[op].rgb;
    if((scene_info.w&2)!=0)return reconstruction[op].rgb;
    return native_at(int2((float2(op)+0.5)*float2(dimensions.xy)/float2(dimensions.zw)));
}
#define GN 512
// Track 16 glare: the image region of the convolved grid at an output pixel (bilinear).
float3 glare_at(uint2 op){
    float2 size=float2(glare.w&0xffff,glare.w>>16),g=(float2(op)+0.5)/float2(dimensions.zw)*size-0.5;
    int2 b=int2(floor(g));float2 f=g-b;float3 sum=0;
    for(int i=0;i<4;i++){int2 o=int2(i&1,i>>1),q=clamp(b+o,0,int2(size)-1);float w=(o.x!=0?f.x:1-f.x)*(o.y!=0?f.y:1-f.y);
        sum+=w*float3(glare_data[q.y*GN+q.x].x,glare_data[(GN+q.y)*GN+q.x].x,glare_data[(2*GN+q.y)*GN+q.x].x);}
    return max(sum,0);
}
// Halation: red-orange glow from highlights scattering back through film; vignette: cos^4-like falloff.
float3 lens_and_film(float3 c,uint2 op){
    if(look.x>0){
        float r=dimensions.w*0.012,th=exposed_threshold(1);float3 glow=0;
        for(int i=0;i<12;i++){float a=i*PI/6;for(int k=1;k<=2;k++)glow+=max(scene_at(int2(op)+int2(float2(cos(a),sin(a))*r*k))-th,0);}
        c+=glow/24*look.x*float3(1.0,0.3,0.12);
    }
    if(glare_settings.x>0)c+=glare_at(op)*glare_settings.x;
    if(look.z>0){float2 q=(float2(op)/dimensions.zw-0.5)*float2(float(dimensions.z)/dimensions.w,1);float v=1/(1+dot(q,q)*1.6);c*=lerp(1,v*v,look.z);}
    return c;
}
// Track 16 glare: the aperture's diffraction pattern (|FFT|² of its shape, scaled per wavelength)
// convolved with the frame's highlights by FFT on a GN² grid. One dispatch of GN groups per mode;
// a group is one row (or column) and each thread two of its texels.
groupshared float2 fft_a[3][GN],fft_b[3][GN];
uint gidx(uint r,uint c,uint x,uint y){return ((r*3+c)*GN+y)*GN+x;}
float2 cmul(float2 a,float2 b){return float2(a.x*b.x-a.y*b.y,a.x*b.y+a.y*b.x);}
// One Stockham radix-2 stage (natural order out) for the three channels; sign -1 forward.
#define FFT_STAGE(SRC,DST,NS) {[unroll]for(uint c=0;c<3;c++){float2 v0=SRC[c][t],v1=SRC[c][t+GN/2];float a=sign*PI*float(t&(NS-1))/NS;v1=cmul(v1,float2(cos(a),sin(a)));uint d=(t/NS)*NS*2+(t&(NS-1));DST[c][d]=v0+v1;DST[c][d+NS]=v0-v1;}GroupMemoryBarrierWithGroupSync();}
void fft(uint t,float sign){
    FFT_STAGE(fft_a,fft_b,1)FFT_STAGE(fft_b,fft_a,2)FFT_STAGE(fft_a,fft_b,4)FFT_STAGE(fft_b,fft_a,8)
    FFT_STAGE(fft_a,fft_b,16)FFT_STAGE(fft_b,fft_a,32)FFT_STAGE(fft_a,fft_b,64)FFT_STAGE(fft_b,fft_a,128)
    FFT_STAGE(fft_a,fft_b,256)
    [unroll]for(uint c=0;c<3;c++){fft_a[c][t]=fft_b[c][t];fft_a[c][t+GN/2]=fft_b[c][t+GN/2];}GroupMemoryBarrierWithGroupSync();
}
// |F|² of the aperture's spectrum (region 0, channel 0) at a signed, fractional frequency (bilinear).
float spectrum_power(float2 k){
    int2 b=int2(floor(k));float2 f=k-b;float sum=0;
    for(int i=0;i<4;i++){int2 o=int2(i&1,i>>1);uint2 q=uint2((b+o)&(GN-1));float w=(o.x!=0?f.x:1-f.x)*(o.y!=0?f.y:1-f.y);float2 v=glare_data[gidx(0,0,q.x,q.y)];sum+=w*dot(v,v);}
    return sum;
}
[numthreads(256,1,1)]void glare_pass(uint3 gid:SV_GroupID,uint3 tid:SV_GroupThreadID){
    uint t=tid.x,row=gid.x,mode=glare.x,r=glare.y,W=glare.w&0xffff,H=glare.w>>16;
    if(mode==0){
        // The highlights: light above the threshold (by luminance), box-filtered into the grid.
        float2 cell=float2(dimensions.zw)/float2(W,H);float th=exposed_threshold(glare_settings.y);
        for(uint k=0;k<2;k++){
            uint x=t+k*GN/2;float3 v=0;
            if(x<W && row<H){
                for(int i=0;i<16;i++){float3 s=scene_at(int2((float2(x,row)+(float2(i&3,i>>2)+0.5)/4)*cell));float l=luminance(s);v+=s*max(l-th,0)/max(l,1e-6);}
                v/=16;
            }
            [unroll]for(uint c=0;c<3;c++)glare_data[gidx(0,c,x,row)]=float2(v[c],0);
        }
        return;
    }
    if(mode==1){
        // The aperture shape, glare_settings.z texels across (4×4 supersampled), into region 0.
        float R=glare_settings.z*0.5;
        for(uint k=0;k<2;k++){
            uint x=t+k*GN/2;float a=0;
            for(int i=0;i<16;i++)a+=inside_aperture(float2(x,row)+(float2(i&3,i>>2)+0.5)/4-GN/2,R)?1.0/16:0;
            glare_data[gidx(0,0,x,row)]=float2(a,0);glare_data[gidx(0,1,x,row)]=0;glare_data[gidx(0,2,x,row)]=0;
        }
        return;
    }
    if(mode==4){
        // The diffraction pattern per channel: |F(k / s)|² / s² with s = λ / 550 nm (610, 550 and
        // 465 nm), normalized to unit sum (Parseval: Σ|F|² = GN² · aperture area), centred at 0.
        float n=lens.z,R=glare_settings.z*0.5,area=n<2.5?PI*R*R:0.5*n*R*R*sin(2*PI/n),norm=1/(float(GN)*GN*area);
        float3 scale=float3(610,550,465)/550;
        for(uint k=0;k<2;k++){
            uint x=t+k*GN/2;float2 q=float2(int2(uint2(x,row)+GN/2)&(GN-1))-GN/2;
            [unroll]for(uint c=0;c<3;c++)glare_data[gidx(1,c,x,row)]=float2(spectrum_power(q/scale[c])/(scale[c]*scale[c])*norm,0);
        }
        return;
    }
    // FFT of rows (mode 2) or columns (mode 3) of region r; flags: bit 0 inverse, bit 1 convolve
    // (columns: forward, times the kernel's spectrum, inverse).
    bool rows=mode==2,inverse=(glare.z&1)!=0;
    [unroll]for(uint c=0;c<3;c++){[unroll]for(uint k=0;k<2;k++){uint i=t+k*GN/2;fft_a[c][i]=glare_data[rows?gidx(r,c,i,row):gidx(r,c,row,i)];}}
    GroupMemoryBarrierWithGroupSync();
    fft(t,inverse?1:-1);
    if((glare.z&2)!=0){
        [unroll]for(uint c=0;c<3;c++){[unroll]for(uint k=0;k<2;k++){uint i=t+k*GN/2;fft_a[c][i]=cmul(fft_a[c][i],glare_data[gidx(1,c,row,i)]);}}
        GroupMemoryBarrierWithGroupSync();
        fft(t,1);
    }
    // The 1 / GN² of the inverse transform, once, in the last (row) pass.
    float s=rows && inverse?1.0/(float(GN)*GN):1;
    [unroll]for(uint c=0;c<3;c++){[unroll]for(uint k=0;k<2;k++){uint i=t+k*GN/2;glare_data[rows?gidx(r,c,i,row):gidx(r,c,row,i)]=fft_a[c][i]*s;}}
}
float film_grain(uint2 op){
    uint2 cell=op/max(1,(uint)lift.w);uint h=cell.x*1973+cell.y*9277+(uint)gain.w*26699;h^=h>>13;h*=0x5bd1e995;h^=h>>15;
    return (h&0xffff)/65535.0-0.5;
}
uint2 render_pixel(uint2 op){return min(uint2((float2(op)+0.5)*float2(dimensions.xy)/float2(dimensions.zw)),dimensions.xy-1);}
// What the display shows before the look: the software view's image or the beauty.
float3 display_input(uint2 op){return cpu_view()?reference_image[op].rgb:beauty(render_pixel(op),op);}
void meter_pass(uint2 tid,uint gi,uint2 gid){
    if(meter_ctrl.y==3)return;
    if(meter_ctrl.y==2){if(gi==0 && all(gid==0))meter_update(false);return;}
    uint2 size=dimensions.zw,grid=(size+3)/4,groups=(grid+7)/8;
    for(uint i=gi;i<256;i+=64)meter_hist[i]=0;
    GroupMemoryBarrierWithGroupSync();
    if(all(tid<grid)){
        uint2 q=min(tid*4+2,size-1);uint w=meter_weight(q,size);float l=luminance(max(display_input(q),0));
        if(w>0 && l>exp2(METER_LOG_MIN)){
            float t=clamp((log2(l)-METER_LOG_MIN)*(128/(METER_LOG_MAX-METER_LOG_MIN)),0,128-0.001);uint b=uint(t);uint f=min(uint((t-b)*METER_FRAC),uint(METER_FRAC)-1);
            InterlockedAdd(meter_hist[b],w);InterlockedAdd(meter_hist[128+b],w*f);
        }
    }
    GroupMemoryBarrierWithGroupSync();
    for(uint i=gi;i<256;i+=64)if(meter_hist[i]>0)InterlockedAdd(scopes[uint2(i,METER_ROW)],meter_hist[i]);
    DeviceMemoryBarrierWithGroupSync();
    if(gi==0){uint old;InterlockedAdd(scopes[uint2(0,METER_STATE)],1,old);meter_last=old==groups.x*groups.y-1?1:0;}
    GroupMemoryBarrierWithGroupSync();
    if(meter_last==0)return;
    // The last group: take the totals (and clear them for the next frame), then reduce.
    for(uint i=gi;i<256;i+=64){uint v;InterlockedExchange(scopes[uint2(i,METER_ROW)],0,v);meter_hist[i]=v;}
    GroupMemoryBarrierWithGroupSync();
    if(gi==0){uint v;InterlockedExchange(scopes[uint2(0,METER_STATE)],0,v);meter_update(true);}
}
// Pre-exposure (E5, meter_ctrl.x bit 3): rgb of texture meter_ctrl.y times asfloat(meter_ctrl.w), a
// power of two, so the scale and its inverse are exact. Alpha (accumulation's M2) is untouched.
void scale_texture(RWTexture2D<float4> t,uint2 p,float k){uint w,h;t.GetDimensions(w,h);if(p.x<w && p.y<h){float4 v=t[p];t[p]=float4(v.rgb*k,v.a);}}
void scale_pass(uint2 p){
    float k=asfloat(meter_ctrl.w);
    if(meter_ctrl.y==0)scale_texture(noisy,p,k);else if(meter_ctrl.y==1)scale_texture(accumulation,p,k);
    else if(meter_ctrl.y==8)scale_texture(ping,p,k);else scale_texture(reconstruction,p,k);
}
[numthreads(8,8,1)]void display(uint3 tid:SV_DispatchThreadID,uint gi:SV_GroupIndex,uint3 gid:SV_GroupID){
    if((meter_ctrl.x&8)!=0){scale_pass(tid.xy);return;}
    if((meter_ctrl.x&2)!=0){meter_pass(tid.xy,gi,gid.xy);return;}
    uint2 op=tid.xy;if(any(op>=dimensions.zw))return;uint2 p=render_pixel(op);
    float3 c=display_input(op);linear_output[op]=float4(c,1);
    if(modes.x==1)c=normal_roughness[p].xyz*0.5+0.5;
    else if(modes.x==2)c=pow(diffuse_albedo[p].rgb,1/2.2);
    else if(modes.x==3)c=exp(-depth[p]*0.3).xxx;
    else if(modes.x==4)c=tonemap(direct_light[p].rgb);
    else if(modes.x==5)c=tonemap(max(0,accumulation[p].rgb-direct_light[p].rgb));
    else if(modes.x==6)c=tonemap(specular_light[p].rgb);
    else if(modes.x==7){float n=max(2,sampling.x+sampling.y);float se=sqrt(accumulation[p].a/(n*(n-1)))/max(0.01,luminance(accumulation[p].rgb));float t=saturate((log2(max(se,0.001))-log2(0.001))/(log2(0.3)-log2(0.001)));c=saturate(float3(t*2,1-abs(t*2-1),2-t*2));}
    else if(modes.x==8)c=heat((trace_stats[p].x-1)/max(1,tools.z-1));
    // BVH steps per sample on a log scale: tools.w steps per sample is the top of the map.
    else if(modes.x==9)c=heat(log2(1+trace_stats[p].y/max(1,sampling.y))/log2(1+tools.w));
    else if(modes.x==10){float n=max(2,sampling.x+sampling.y);c=heat(log2(1+accumulation[p].a/(n-1))/4);}
    else if(modes.x==11)c=false_color(c);
    // A light path expression's AOV, raw (no denoising); it also replaces the beauty in EXR captures.
    else if(modes.x==12){float3 a=lpe.y<lpe.x?lpe_aov[(p.y*dimensions.x+p.x)*lpe.x+lpe.y].rgb:0;linear_output[op]=float4(a,1);c=tonemap(a);}
    float3 h;bool beauty_view=modes.x==0;
    if(beauty_view){if(options.z>0 && filter_settings.w!=0 && float(op.x)/dimensions.z<options.w)c=reference_image[op].rgb;c=lens_and_film(c,op);h=hdr_map(c);c=tonemap(c);
        // Grain on the displayed value, strongest in the midtones.
        if(look.y>0){float g=film_grain(op)*look.y*0.18;float l=luminance(saturate(c));float w=4*l*(1-l)+0.15;c=saturate(c+g*w);h*=1+g*w*2;}if(options.z>0 && abs(float(op.x)/dimensions.z-options.w)<1.0/dimensions.z){c=float3(0.3,0.8,1);h=sdr_in_hdr(c);}}
    else h=sdr_in_hdr(c);
    // Zebras: diagonal stripes where the displayed image clips.
    if(tools.x>0 && beauty_view && max(c.r,max(c.g,c.b))>0.97 && ((op.x+op.y)/6)%2==0){c=float3(0.05,0.05,0.05);h=sdr_in_hdr(c);}
    if(tools.y>0)bin_scopes(op,c);
    output_image[op]=float4(c,1);
    // HDR10 (hdr.w): PQ code values in BT.2020 primaries, for the R10G10B10A2 swapchain.
    if(hdr.x>0)hdr_output[op]=float4(hdr.w>0?hdr10_encode(h):h,1);
}

