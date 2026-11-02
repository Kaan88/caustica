cbuffer Frame:register(b0){float4x4 unused_matrices[4];float4 eye;uint4 dimensions;}; // eye.w: HDR paper white / 80, 0 in SDR, negative for HDR10 (PQ)
Texture2D<float4> atlas:register(t2);SamplerState linear_sampler:register(s0);
struct VIn{float2 pos:POSITION;float2 uv:TEXCOORD;float4 color:COLOR;};
struct VOut{float4 pos:SV_Position;float2 uv:TEXCOORD;float4 color:COLOR;};
VOut vs_main(VIn i){VOut o;o.pos=float4(i.pos/float2(dimensions.zw)*float2(2,-2)+float2(-1,1),0,1);o.uv=i.uv;o.color=i.color;return o;}
float3 srgb_to_linear(float3 c){return select(c<=0.04045,c/12.92,pow((c+0.055)/1.055,2.4));}
float4 ps_main(VOut i):SV_Target{float4 c=i.color*atlas.Sample(linear_sampler,i.uv);
    // egui colors are premultiplied sRGB. In HDR, linearize and place white at the SDR white level.
    // Some (Color32::from_white_alpha) have rgb above alpha: clamp, or dividing blows them up to white.
    if(eye.w>0&&c.a>0)c.rgb=srgb_to_linear(saturate(c.rgb/c.a))*c.a*eye.w;
    // HDR10: encode the unpremultiplied colour as PQ / BT.2020 (blending then happens on code values).
    if(eye.w<0&&c.a>0){float3 l=max(mul(float3x3(0.627404,0.329282,0.043314,0.069097,0.919540,0.011361,0.016392,0.088013,0.895595),srgb_to_linear(saturate(c.rgb/c.a))),0)*(-eye.w*80/10000);
        float3 y=pow(l,0.1593017578125);c.rgb=pow((0.8359375+18.8515625*y)/(1+18.6875*y),78.84375)*c.a;}
    return c;}
