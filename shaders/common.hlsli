// maps: heap slot + 1 of base colour, normal, metallic-roughness (0 = none), alpha cutoff bits.
// base_scale.rgb and surface_scale.xy (roughness, metallic) = value / map average (core track 5).
struct Material {float4 color_rough;float4 lobes;float4 optical;float4 secondary;uint4 maps;float4 base_scale;float4 surface_scale;};
cbuffer Frame:register(b0) {
    float4x4 inverse_vp;
    float4x4 current_vp;
    float4x4 previous_vp;
    float4x4 world_view;
    float4 eye;
    uint4 dimensions; // render w,h,output w,h
    uint4 sampling; // accumulated samples,spp,bounces,frame
    float4 lights; // suns at the start of the emitter table (track 7), pixel filter (0 box, 1 Gaussian, 2 Blackman-Harris), its width in pixels, texture anisotropy (1-16)
    float4 light_color_aspect;
    float4 background;
    float4 display_settings; // exposure,gamma,direct,indirect
    uint4 modes; // view,tonemap,denoise,RR active
    float4 options; // firefly,denoise strength,compare,split
    float4 jitter; // x,y,sobol,glass shadows
    uint4 filter_settings; // pass (0 reproject, 1 integrate, 2–6 à-trous),temporal,no history,reference valid
    uint4 scene_info; // emissive triangle count, object count, Sobol sequence base, flags (bit 0 jittered primaries, bit 1 show upscaled texture 14, bits 2-3 spatial upscale: 1 bilinear, 2 bicubic; bits 4-5 light sampling: 0 MIS, 1 NEE only, 2 BSDF only)
    float4 environment; // sky intensity, view-space ray scale x, y (tan of half field of view), first triangle of the opaque BLAS range
    float4 hdr; // scRGB output on (1/0), paper white nits, peak nits
    float4 tools; // zebras on, scopes on, heat-map scale for bounces, for BVH steps per pixel
    float4 lens; // aperture radius, focus distance, blades (0 round), blade rotation (radians)
    float4 camera_model; // projection (0 persp, 1 ortho, 2 equirect, 3 little planet, 4 tilt-shift), cat-eye, tilt (radians), ortho half height
    float4 look; // halation, grain, vignette, LUT size (0 = none)
    float4 lift; // rgb, w = grain size in pixels
    float4 grade_gamma; // rgb, w = LUT strength
    float4 gain; // rgb, w = frame noise seed
    float4 environment_map; // HDRI strength, rotation (radians), flags (bit 0 loaded, bit 1 seen by the camera), pdf normalization (environment.rs)
};
StructuredBuffer<Material> materials:register(t3);
struct Emitter {float4 position_area;float4 edge1_cdf;float4 edge2_probability;float4 emission_material;};
StructuredBuffer<Emitter> emitters:register(t4);
StructuredBuffer<uint> emitter_lookup:register(t5);
RaytracingAccelerationStructure scene:register(t0);
struct Vertex {float3 position;uint material;float3 normal;uint pad;float2 uv;};
StructuredBuffer<Vertex> vertices:register(t1); // every mesh in use, object space (core track 4)
StructuredBuffer<Vertex> static_vertices:register(t6); // floor, room and lights, world space
// One per TLAS instance (InstanceID). material 0xffffffff = the static instance (per-vertex ids).
struct Instance{uint vertex_base;uint material;uint emitter_base;uint object;};
StructuredBuffer<Instance> instances:register(t7);
Texture2D<float4> maps[]:register(t0,space1); // mesh texture maps (gpu.rs MAX_MAPS)
SamplerState map_sampler:register(s1); // linear, wrap
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
RWTexture2D<float> device_depth:register(u25); // projected depth 0 (near) to 1 (far), for DLSS SR / FSR / XeSS
RWTexture2D<float4> hdr_output:register(u26); // scRGB (1.0 = 80 nits) for an HDR swapchain, output resolution
RWTexture2D<float2> trace_stats:register(u27); // this frame: mean bounces per sample, BVH steps (Proceed calls) per pixel
RWTexture3D<float4> lut:register(u29); // film-look .cube LUT (look.w cubed); 2x2x2 when unused
RWTexture2D<float4> env_map:register(u30); // equirectangular HDRI, RGBA16F, linear RGB / scale (folded into environment_map.x and .w); 1x1 when unused
RWTexture2D<float> env_cdf:register(u31); // per-row conditional CDFs, then the marginal CDF over rows in an extra last row
RWTexture2D<uint> scopes:register(u28); // 256 x 260 counters: rows 0-3 histogram R,G,B,L; 4-131 waveform; 132-259 vectorscope (128 wide)
float luminance(float3 c){return dot(c,float3(0.2126,0.7152,0.0722));}
static const float PI=3.14159265359;

