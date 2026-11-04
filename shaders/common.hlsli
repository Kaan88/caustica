// maps: heap slot + 1 of base colour, normal, metallic-roughness (0 = none), alpha cutoff bits.
// base_scale.rgb and surface_scale.xy (roughness, metallic) = value / map average (core track 5);
// surface_scale.zw the thin film (track 11: strength, thickness + 2048 × IOR in hundredths).
// layers: clear coat, coat roughness, sheen roughness, -; sheen: colour times strength, - (track 11).
struct Material {float4 color_rough;float4 lobes;float4 optical;float4 secondary;uint4 maps;float4 base_scale;float4 surface_scale;float4 layers;float4 sheen;float4 medium;float4 phase;};
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
    uint4 scene_info; // emissive triangle count, object count, Sobol sequence base, flags (bit 0 jittered primaries, bit 1 show upscaled texture 14, bits 2-3 spatial upscale: 1 bilinear, 2 bicubic; bits 4-5 light sampling: 0 MIS, 1 NEE only, 2 BSDF only; bit 6 equiangular sampling in fog)
    float4 environment; // sky intensity, view-space ray scale x, y (tan of half field of view), the medium-stack entry of the object the camera is inside (0 = air; geometry.rs camera_medium)
    float4 hdr; // HDR output on (1/0), paper white nits, peak nits, HDR10 PQ encoding (1/0)
    float4 tools; // zebras on, scopes on, heat-map scale for bounces, for BVH steps per pixel
    float4 lens; // aperture radius, focus distance, blades (0 round), blade rotation (radians)
    float4 camera_model; // projection (0 persp, 1 ortho, 2 equirect, 3 little planet, 4 tilt-shift), cat-eye, tilt (radians), ortho half height
    float4 look; // halation, grain, vignette, LUT size (0 = none)
    float4 lift; // rgb, w = grain size in pixels
    float4 grade_gamma; // rgb, w = LUT strength
    float4 gain; // rgb, w = frame noise seed
    float4 environment_map; // HDRI strength, rotation (radians), flags (bit 0 loaded, bit 1 seen by the camera), pdf normalization (environment.rs)
    float4 fog; // track 12: density, phase g, top height, half extent on x / z
    float4 fog_color; // albedo rgb
    uint4 path_record; // track 14, the PATH_RECORD variant of trace only: render pixel x, y, paths, record capacity
    uint4 lpe; // track 14: light path expression slots in use (0 = off), the slot the LPE view shows
    uint4 restir; // track 15 ReSTIR DI: flags (bit 0 on, bit 1 history valid), candidates, spatial neighbours, history cap
    uint4 region; // track 15: the render pixels this dispatch covers, x0, y0, x1, y1 (battle mode quadrants; else the whole image)
    uint4 integrator; // track 15 (integrators.hlsl): photons emitted this frame, photon capacity, splat offset in aux (bytes), flags (bit 0 light tracing)
    float4 photon; // track 15: gather radius, image-plane area at distance 1 times the region's share of the image, SDF photon records (count, first emitter record; gpu.rs sdf_emitter_records)
    float4 bounds; // track 15: the scene's bounding sphere (centre, radius) for sun and sky photons
    float4 real_lens; // track 16: lens system surfaces (0 = off), dispersion, scene units per mm, -
    uint4 glare; // track 16 glare pass: mode (0 highlights, 1 aperture, 2 rows FFT, 3 columns FFT, 4 PSF), region, flags (bit 0 inverse, bit 1 convolve), image size in the grid (w | h << 16)
    float4 glare_settings; // strength (0 = off), threshold, aperture diameter in grid texels, -
    float4 env_sun; // the HDRI's split sun (environment.rs Sun): world direction, 1 − cos of the half angle (0 = none)
    float4 env_sun_color; // its radiance times the HDRI strength, the share of environment samples it takes
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
// Light path expressions (track 14, lpe.rs): per slot, state and event, the next DFA state | its
// accept flag << 8 (255 = no match possible); and each slot's AOV, the running mean per render
// pixel (index (y * render width + x) * slots + slot), written by the LPE variant of trace.
StructuredBuffer<uint> lpe_table:register(t8);
// Track 16 lens system (lens.rs `pack`): header, surfaces, exit-pupil bounds per film band.
StructuredBuffer<float4> lens_data:register(t9);
// Track 16 glare (post.hlsl): complex values, region r channel c at ((r*3+c)*512+y)*512+x;
// region 0 the image being convolved, region 1 the kernel's spectrum.
RWStructuredBuffer<float2> glare_data:register(u4,space2);
RWStructuredBuffer<float4> lpe_aov:register(u1,space2);
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
float2 sign_nz(float2 v){return float2(v.x>=0?1:-1,v.y>=0?1:-1);}
float2 oct_encode(float3 n){n/=abs(n.x)+abs(n.y)+abs(n.z)+1e-9;return n.z>=0?n.xy:(1-abs(n.yx))*sign_nz(n.xy);}
float3 oct_decode(float2 e){float3 n=float3(e,1-abs(e.x)-abs(e.y));n.xy-=saturate(-n.z)*sign_nz(n.xy);return normalize(n);}
static const float PI=3.14159265359;
// The aperture (roadmap 5c, track 16): a disk of radius r, or the n-gon of lens.z blades turned
// by lens.w (vertices at lens.w + 2πk/n, as pathtrace's aperture_point).
bool inside_aperture(float2 q,float r){
    if(lens.z<2.5)return dot(q,q)<=r*r;
    float n=lens.z,sector=2*PI/n,a=atan2(q.y,q.x)-lens.w-PI/n;a-=round(a/sector)*sector;
    return length(q)*cos(a)<=r*cos(PI/n);
}

