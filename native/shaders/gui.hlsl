cbuffer Frame:register(b0){float4x4 unused_matrices[4];float4 unused_eye;uint4 dimensions;};
Texture2D<float4> atlas:register(t2);SamplerState linear_sampler:register(s0);
struct VIn{float2 pos:POSITION;float2 uv:TEXCOORD;float4 color:COLOR;};
struct VOut{float4 pos:SV_Position;float2 uv:TEXCOORD;float4 color:COLOR;};
VOut vs_main(VIn i){VOut o;o.pos=float4(i.pos/float2(dimensions.zw)*float2(2,-2)+float2(-1,1),0,1);o.uv=i.uv;o.color=i.color;return o;}
float4 ps_main(VOut i):SV_Target{return i.color*atlas.Sample(linear_sampler,i.uv);}
