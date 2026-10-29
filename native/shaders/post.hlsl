#include "common.hlsli"
// Iteration 0 integrates the unfiltered samples over time (rgb = color, a = effective
// sample count). Iterations 1–5 are à-trous passes: integrated -> ping -> pong -> ... -> ping.
// History holds integrated color, never filtered output, so features are not re-blurred every frame.
float3 source(uint2 p,uint iteration){if(iteration==1)return integrated[p].rgb;if((iteration&1)==0)return ping[p].rgb;return pong[p].rgb;}
void integrate(uint2 p){
    float3 n=normal_roughness[p].xyz;float z=depth[p];float count=max(1,sampling.x+sampling.y);
    float3 c=accumulation[p].rgb;float effective=count;
    if(filter_settings.y!=0 && filter_settings.z==0 && count<256){
        int2 q=int2(round(float2(p)+motion[p]));
        if(all(q>=0)&&all(q<int2(dimensions.xy)) && history_id[q]==object_id[p] && dot(n,history_normal[q].xyz)>0.95 && abs(z-history_depth[q])<max(0.025,z*0.03)){
            // Glossy surfaces keep little history: their reflections move with the view.
            float4 h=history[q];float w=lerp(0.85,0.1,1-normal_roughness[p].w)*saturate(1-count/256);
            c=lerp(c,h.rgb,w);effective=1/((1-w)*(1-w)/count+w*w/max(h.a,1));
        }
    }
    integrated[p]=float4(c,effective);
}
[numthreads(8,8,1)]void filter(uint3 tid:SV_DispatchThreadID){
    uint2 p=tid.xy;if(any(p>=dimensions.xy))return;uint iteration=filter_settings.x;
    if(iteration==0){integrate(p);return;}
    float3 n=normal_roughness[p].xyz;float z=depth[p];float id=object_id[p];float3 center=source(p,iteration);float count=max(1,sampling.x+sampling.y);float effective=max(1,integrated[p].a);
    float var=accumulation[p].a/max(1,count-1)/effective;float sigma=max(sqrt(var),max(0.02,abs(luminance(center))*0.08/sqrt(effective)))*max(0.01,options.y)*6;
    float3 sum=0;float total=0;int step=int(1u<<(iteration-1));
    for(int y=-2;y<=2;y++)for(int x=-2;x<=2;x++){
        // Never mix surfaces: the light sits 3.5 cm under the ceiling with the same normal.
        int2 q=int2(p)+int2(x,y)*step;if(any(q<0)||any(q>=int2(dimensions.xy))||object_id[q]!=id)continue;
        float3 c=source(q,iteration);float3 a=max(diffuse_albedo[p].rgb+specular_albedo[p].rgb,0.05);float3 b=max(diffuse_albedo[q].rgb+specular_albedo[q].rgb,0.05);
        float norm=pow(saturate(dot(n,normal_roughness[q].xyz)),64);float dw=exp(-abs(z-depth[q])/max(0.005,0.008*z*step));float lum=exp(-abs(luminance(c/b)-luminance(center/a))/max(sigma,1e-5));float kernel=(x==0?6:(abs(x)==1?4:1))*(y==0?6:(abs(y)==1?4:1));float w=kernel*norm*dw*lum;
        sum+=c/b*a*w;total+=w;
    }
    float3 c=total>1e-6?sum/total:center;
    if((iteration&1)==1)ping[p]=float4(c,1);else pong[p]=float4(c,1);
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
float3 beauty(uint2 p,uint2 op){
    float3 raw=accumulation[p].rgb;float3 c=modes.w!=0?reconstruction[op].rgb:(modes.z!=0?ping[p].rgb:raw);
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
    else {if(options.z>0 && filter_settings.w!=0 && float(op.x)/dimensions.z<options.w)c=reference_image[op].rgb;c=tonemap(c);if(options.z>0 && abs(float(op.x)/dimensions.z-options.w)<1.0/dimensions.z)c=float3(0.3,0.8,1);}
    output_image[op]=float4(c,1);
}

