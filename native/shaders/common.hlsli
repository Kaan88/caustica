struct Material {float4 color_rough;float4 lobes;float4 optical;float4 secondary;};
cbuffer Frame:register(b0) {
    float4x4 inverse_vp;
    float4x4 current_vp;
    float4x4 previous_vp;
    float4x4 world_view;
    float4 eye;
    uint4 dimensions; // render w,h,output w,h
    uint4 sampling; // accumulated samples,spp,bounces,frame
    float4 light_position_size;
    float4 light_color_aspect;
    float4 background;
    float4 display_settings; // exposure,gamma,direct,indirect
    uint4 modes; // view,tonemap,denoise,RR active
    float4 options; // firefly,denoise strength,compare,split
    float4 jitter; // x,y,sobol,glass shadows
    uint4 filter_settings; // pass (0 reproject, 1 integrate, 2–6 à-trous),temporal,no history,reference valid
    uint4 scene_info; // emissive triangle count, object count, Sobol sequence base, reserved
    float4 environment; // sky intensity, view-space ray scale x, y (tan of half field of view)
};
StructuredBuffer<Material> materials:register(t3);
struct Emitter {float4 position_area;float4 edge1_cdf;float4 edge2_probability;float4 emission_material;};
StructuredBuffer<Emitter> emitters:register(t4);
StructuredBuffer<uint> emitter_lookup:register(t5);
RaytracingAccelerationStructure scene:register(t0);
struct Vertex {float3 position;uint material;float3 normal;uint pad;};
StructuredBuffer<Vertex> vertices:register(t1);
RWTexture2D<float4> noisy:register(u0);
RWTexture2D<float4> accumulation:register(u1);
RWTexture2D<float4> normal_roughness:register(u2);
RWTexture2D<float4> diffuse_albedo:register(u3);
RWTexture2D<float4> specular_albedo:register(u4);
RWTexture2D<float> depth:register(u5);
RWTexture2D<float2> motion:register(u6);
RWTexture2D<float> specular_distance:register(u7);
RWTexture2D<float4> ping:register(u8);
RWTexture2D<float4> pong:register(u9);
RWTexture2D<float4> output_image:register(u10);
RWTexture2D<float4> direct_light:register(u11);
RWTexture2D<float4> specular_light:register(u12);
RWTexture2D<float4> reference_image:register(u13);
RWTexture2D<float4> reconstruction:register(u14);
RWTexture2D<float4> history:register(u15);
RWTexture2D<float4> history_normal:register(u16);
RWTexture2D<float> history_depth:register(u17);
RWTexture2D<float4> linear_output:register(u18);
RWTexture2D<float> object_id:register(u19); // first-hit material id + 1, 0 = miss
RWTexture2D<float> history_id:register(u20);
RWTexture2D<float4> integrated:register(u21); // temporally integrated raw color, a = effective samples
RWTexture2D<float4> moments:register(u22); // demodulated luminance E[l], E[l^2] per sample, variance of the integrated mean
RWTexture2D<float4> history_moments:register(u23);
RWTexture2D<float4> filter_guide:register(u24); // octahedral normal xy, depth, object id: one read per filter tap
float luminance(float3 c){return dot(c,float3(0.2126,0.7152,0.0722));}
static const float PI=3.14159265359;

