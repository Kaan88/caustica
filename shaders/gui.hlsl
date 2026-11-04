cbuffer Frame:register(b0){float4x4 unused_matrices[4];float4 eye;uint4 dimensions;}; // eye.w: HDR paper white / 80, 0 in SDR, negative for HDR10 (PQ)
Texture2D<float4> atlas:register(t2);SamplerState linear_sampler:register(s0);
struct VIn{float2 pos:POSITION;float2 uv:TEXCOORD;float4 color:COLOR;};
struct VOut{float4 pos:SV_Position;float2 uv:TEXCOORD;float4 color:COLOR;};
VOut vs_main(VIn i){VOut o;o.pos=float4(i.pos/float2(dimensions.zw)*float2(2,-2)+float2(-1,1),0,1);o.uv=i.uv;o.color=i.color;return o;}
float3 srgb_to_linear(float3 c){return select(c<=0.04045,c/12.92,pow((c+0.055)/1.055,2.4));}
RWTexture2D<float> object_id:register(u19); // first-hit material id + 1, at render size
// Silhouette outlines (ui.rs, a quad with uv.x < 0): uv.x = -1 - width in pixels, uv.y = the
// object id. Covers the pixels just outside the object's visible pixels, so occluders cut it.
float silhouette(float2 pos,float width,float id){
    float2 scale=float2(dimensions.xy)/float2(dimensions.zw);int2 top=int2(dimensions.xy)-1;
    // Ids are whole numbers; the tolerance covers any drift in the interpolated uv.
    if(abs(object_id[clamp(int2(pos*scale),0,top)]-id)<0.5)return 0;
    float hits=0;
    for(int k=0;k<16;k++){float a=k*0.3926991;float2 q=pos+float2(cos(a),sin(a))*((k&1)!=0?width:width*0.5);hits+=abs(object_id[clamp(int2(q*scale),0,top)]-id)<0.5?1:0;}
    return saturate(hits*0.5);}
float4 ps_main(VOut i):SV_Target{float4 c=i.color*atlas.Sample(linear_sampler,i.uv);
    if(i.uv.x<0)c=i.color*silhouette(i.pos.xy,-1-i.uv.x,i.uv.y);
    // egui colors are premultiplied sRGB. In HDR, linearize and place white at the SDR white level.
    // Some (Color32::from_white_alpha) have rgb above alpha: clamp, or dividing blows them up to white.
    if(eye.w>0&&c.a>0)c.rgb=srgb_to_linear(saturate(c.rgb/c.a))*c.a*eye.w;
    // HDR10: encode the unpremultiplied colour as PQ / BT.2020 (blending then happens on code values).
    if(eye.w<0&&c.a>0){float3 l=max(mul(float3x3(0.627404,0.329282,0.043314,0.069097,0.919540,0.011361,0.016392,0.088013,0.895595),srgb_to_linear(saturate(c.rgb/c.a))),0)*(-eye.w*80/10000);
        float3 y=pow(l,0.1593017578125);c.rgb=pow((0.8359375+18.8515625*y)/(1+18.6875*y),78.84375)*c.a;}
    return c;}
