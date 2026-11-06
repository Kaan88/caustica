// The upscalers on Vulkan: NVIDIA NGX (Ray Reconstruction, DLSS Super Resolution) and Intel XeSS
// behind a small C ABI, as ngx.cpp and xess.cpp do for D3D12. Rendering stays in Rust/HLSL;
// images are passed as (image, view, format, width, height) and must be in GENERAL layout.
#include <initializer_list>
#ifdef _WIN32
#include <windows.h>
typedef HMODULE Module;
static void* sym(Module m, const char* name) { return (void*)GetProcAddress(m, name); }
#else
// Linux (engine-notes "Linux DLSS"): the same ABI; NGX links statically, XeSS is dlopen'd.
#include <dlfcn.h>
#include <string.h>
#include <unistd.h>
#define MAX_PATH 4096
typedef void* Module;
static void* sym(Module m, const char* name) { return dlsym(m, name); }
// The executable's folder (/proc/self/exe; in the AppImage, the mount's usr/bin).
static void exe_dir_utf8(char* path) {
    ssize_t n = readlink("/proc/self/exe", path, MAX_PATH - 1);
    path[n > 0 ? n : 0] = 0;
    char* end = strrchr(path, '/'); if (end) *end = 0;
}
#endif
#include <vulkan/vulkan.h>
#include "nvsdk_ngx_vk.h"
#include "nvsdk_ngx_helpers_vk.h"
#include "nvsdk_ngx_helpers_dlssd_vk.h"
#include "xess/xess_vk.h"

struct VkImageRef { VkImage image; VkImageView view; VkFormat format; unsigned width, height; };

static NVSDK_NGX_Resource_VK resource(const VkImageRef& r, bool write) {
    VkImageSubresourceRange range{VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1};
    return NVSDK_NGX_Create_ImageView_Resource_VK(r.view, r.image, range, r.format, r.width, r.height, write);
}
static NVSDK_NGX_PerfQuality_Value quality_for(unsigned w, unsigned ow) {
    float ratio = float(w) / ow;
    return ratio >= 0.99f ? NVSDK_NGX_PerfQuality_Value_DLAA : ratio >= 0.62f ? NVSDK_NGX_PerfQuality_Value_MaxQuality
         : ratio >= 0.54f ? NVSDK_NGX_PerfQuality_Value_Balanced : ratio >= 0.42f ? NVSDK_NGX_PerfQuality_Value_MaxPerf
         : NVSDK_NGX_PerfQuality_Value_UltraPerformance;
}
// The executable's folder, wide: NGX takes wchar_t paths on both platforms.
static void exe_dir(wchar_t* path) {
#ifdef _WIN32
    GetModuleFileNameW(nullptr, path, MAX_PATH);
    wchar_t* end = wcsrchr(path, L'\\'); if (end) *end = 0;
#else
    // UTF-8 decoded by hand: mbstowcs would depend on the C locale.
    char u[MAX_PATH]; exe_dir_utf8(u);
    size_t o = 0;
    for (const unsigned char* c = (const unsigned char*)u; *c && o < MAX_PATH - 1; o++) {
        unsigned v = *c++, extra = v >= 0xf0 ? 3 : v >= 0xe0 ? 2 : v >= 0xc0 ? 1 : 0;
        if (extra) v &= 0x3f >> extra;
        for (; extra && (*c & 0xc0) == 0x80; extra--) v = v << 6 | (*c++ & 0x3f);
        path[o] = (wchar_t)v;
    }
    path[o] = 0;
#endif
}

struct NgxVk { NVSDK_NGX_Parameter* params=nullptr; NVSDK_NGX_Parameter* sr_params=nullptr; NVSDK_NGX_Handle* handle=nullptr; NVSDK_NGX_Handle* sr=nullptr; VkDevice device=VK_NULL_HANDLE; bool initialized=false; unsigned caps=0; };

typedef xess_result_t (*XCreate)(VkInstance, VkPhysicalDevice, VkDevice, xess_context_handle_t*);
typedef xess_result_t (*XInit)(xess_context_handle_t, const xess_vk_init_params_t*);
typedef xess_result_t (*XExecute)(xess_context_handle_t, VkCommandBuffer, const xess_vk_execute_params_t*);
typedef xess_result_t (*XDestroy)(xess_context_handle_t);
typedef xess_result_t (*XScale)(xess_context_handle_t, float, float);
typedef xess_result_t (*XInstanceExt)(uint32_t*, const char* const**, uint32_t*);
typedef xess_result_t (*XDeviceExt)(VkInstance, VkPhysicalDevice, uint32_t*, const char* const**);
struct XessVk { XCreate create=nullptr; XInit init=nullptr; XExecute execute=nullptr; XDestroy destroy=nullptr; XScale velocity=nullptr; xess_context_handle_t ctx=nullptr; };

// libxess from the executable's folder, loaded once (null when missing). Intel ships no Linux
// libxess.so (SDK 3.0.2): on Linux this stays null unless a later SDK's is placed there.
static Module xess_module() {
    static Module m = [] {
#ifdef _WIN32
        wchar_t path[MAX_PATH]; exe_dir(path); wcscat_s(path, L"\\libxess.dll");
        return LoadLibraryW(path);
#else
        char path[MAX_PATH + 16]; exe_dir_utf8(path); strcat(path, "/libxess.so");
        return dlopen(path, RTLD_NOW | RTLD_LOCAL);
#endif
    }();
    return m;
}

extern "C" {
// Extensions NGX and XeSS need: instance (before vkCreateInstance) or device ones, as a list
// of names valid for the process. Returns the count; the caller enables those it finds.
unsigned caustica_vk_upscaler_extensions(int device, VkInstance instance, VkPhysicalDevice phys, const char** out, unsigned cap) {
    unsigned n = 0;
    auto add = [&](const char* e) { if (n < cap) out[n++] = e; };
    unsigned ic = 0, dc = 0; const char** ie = nullptr; const char** de = nullptr;
    if (NVSDK_NGX_SUCCEED(NVSDK_NGX_VULKAN_RequiredExtensions(&ic, &ie, &dc, &de))) {
        if (device) { for (unsigned i = 0; i < dc; i++) add(de[i]); } else { for (unsigned i = 0; i < ic; i++) add(ie[i]); }
    }
    if (Module m = xess_module()) {
        uint32_t count = 0; const char* const* names = nullptr;
        if (device) {
            auto f = (XDeviceExt)sym(m, "xessVKGetRequiredDeviceExtensions");
            if (f && f(instance, phys, &count, &names) == XESS_RESULT_SUCCESS) for (uint32_t i = 0; i < count; i++) add(names[i]);
        } else {
            auto f = (XInstanceExt)sym(m, "xessVKGetRequiredInstanceExtensions");
            uint32_t api = 0;
            if (f && f(&count, &names, &api) == XESS_RESULT_SUCCESS) for (uint32_t i = 0; i < count; i++) add(names[i]);
        }
    }
    return n;
}

void caustica_ngxvk_release(NgxVk* n) {
    if (n && n->handle) { NVSDK_NGX_VULKAN_ReleaseFeature(n->handle); n->handle = nullptr; }
    if (n && n->sr) { NVSDK_NGX_VULKAN_ReleaseFeature(n->sr); n->sr = nullptr; }
}
unsigned caustica_ngxvk_caps(NgxVk* n) { return n ? n->caps : 0; }
void caustica_ngxvk_destroy(NgxVk* n) {
    if (!n) return;
    caustica_ngxvk_release(n);
    if (n->sr_params) NVSDK_NGX_VULKAN_DestroyParameters(n->sr_params);
    if (n->params) NVSDK_NGX_VULKAN_DestroyParameters(n->params);
    if (n->initialized) NVSDK_NGX_VULKAN_Shutdown1(n->device);
    delete n;
}
unsigned caustica_ngxvk_init(VkInstance instance, VkPhysicalDevice phys, VkDevice device, PFN_vkGetInstanceProcAddr gipa, PFN_vkGetDeviceProcAddr gdpa, NgxVk** result) {
    *result = nullptr;
    NgxVk* n = new NgxVk; n->device = device;
    wchar_t path[MAX_PATH]; exe_dir(path);
    const wchar_t* paths[] = {path}; NVSDK_NGX_FeatureCommonInfo info = {}; info.PathListInfo.Path = paths; info.PathListInfo.Length = 1;
    // The app's own identifier, the same as the D3D12 bridge's.
    auto r = NVSDK_NGX_VULKAN_Init_with_ProjectID("e9641daa-48c9-4e65-a593-c3a11259bb40", NVSDK_NGX_ENGINE_TYPE_CUSTOM, "0.1.0", path, instance, phys, device, gipa, gdpa, &info, NVSDK_NGX_Version_API);
    if (NVSDK_NGX_FAILED(r)) { delete n; return r; } n->initialized = true;
    r = NVSDK_NGX_VULKAN_GetCapabilityParameters(&n->params);
    if (NVSDK_NGX_FAILED(r)) { caustica_ngxvk_destroy(n); return r; }
    int rr = 0, sr = 0; n->params->Get(NVSDK_NGX_Parameter_SuperSamplingDenoising_Available, &rr); n->params->Get(NVSDK_NGX_Parameter_SuperSampling_Available, &sr);
    n->caps = (rr ? 1u : 0u) | (sr ? 2u : 0u);
    if (!n->caps) { caustica_ngxvk_destroy(n); return NVSDK_NGX_Result_FAIL_FeatureNotSupported; }
    if (sr && NVSDK_NGX_FAILED(NVSDK_NGX_VULKAN_AllocateParameters(&n->sr_params))) n->caps &= ~2u;
    *result = n; return NVSDK_NGX_Result_Success;
}
// Ray Reconstruction, as caustica_ngx_create / _eval (ngx.cpp).
unsigned caustica_ngxvk_create(NgxVk* n, VkCommandBuffer cmd, unsigned w, unsigned h, unsigned ow, unsigned oh, unsigned preset) {
    if (!(n->caps & 1u)) return NVSDK_NGX_Result_FAIL_FeatureNotSupported;
    if (n->handle) { NVSDK_NGX_VULKAN_ReleaseFeature(n->handle); n->handle = nullptr; }
    NVSDK_NGX_DLSSD_Create_Params p = {}; p.InDenoiseMode = NVSDK_NGX_DLSS_Denoise_Mode_DLUnified;
    p.InRoughnessMode = NVSDK_NGX_DLSS_Roughness_Mode_Packed; p.InUseHWDepth = NVSDK_NGX_DLSS_Depth_Type_Linear;
    p.InWidth = w; p.InHeight = h; p.InTargetWidth = ow; p.InTargetHeight = oh; p.InPerfQualityValue = quality_for(w, ow);
    for (const char* key : {NVSDK_NGX_Parameter_RayReconstruction_Hint_Render_Preset_DLAA, NVSDK_NGX_Parameter_RayReconstruction_Hint_Render_Preset_Quality, NVSDK_NGX_Parameter_RayReconstruction_Hint_Render_Preset_Balanced, NVSDK_NGX_Parameter_RayReconstruction_Hint_Render_Preset_Performance, NVSDK_NGX_Parameter_RayReconstruction_Hint_Render_Preset_UltraPerformance}) n->params->Set(key, preset);
    p.InFeatureCreateFlags = NVSDK_NGX_DLSS_Feature_Flags_IsHDR | NVSDK_NGX_DLSS_Feature_Flags_MVLowRes;
    return NGX_VULKAN_CREATE_DLSSD_EXT1(n->device, cmd, 1, 1, &n->handle, n->params, &p);
}
// images: the render textures by index (0 colour, 2 normals/roughness, 3 diffuse albedo,
// 4 specular albedo, 5 depth, 6 motion, 7 specular hit distance, 14 output).
unsigned caustica_ngxvk_eval(NgxVk* n, VkCommandBuffer cmd, const VkImageRef* images, unsigned w, unsigned h, float jx, float jy, int reset, float dt, float* view, float* projection) {
    NVSDK_NGX_Resource_VK color = resource(images[0], false), out = resource(images[14], true), normals = resource(images[2], false),
        diffuse = resource(images[3], false), specular = resource(images[4], false), depth = resource(images[5], false),
        motion = resource(images[6], false), hit = resource(images[7], false);
    NVSDK_NGX_VK_DLSSD_Eval_Params p = {};
    p.pInColor = &color; p.pInOutput = &out; p.pInNormals = &normals; p.pInDiffuseAlbedo = &diffuse; p.pInSpecularAlbedo = &specular;
    p.pInDepth = &depth; p.pInMotionVectors = &motion; p.pInSpecularHitDistance = &hit;
    // Image displacement, the negated ray jitter (see ngx.cpp).
    p.InRenderSubrectDimensions = {w, h}; p.InJitterOffsetX = -jx; p.InJitterOffsetY = -jy; p.InReset = reset; p.InMVScaleX = 1.; p.InMVScaleY = 1.;
    p.InPreExposure = 1.; p.InExposureScale = 1.; p.InFrameTimeDeltaInMsec = dt;
    p.pInWorldToViewMatrix = view; p.pInViewToClipMatrix = projection;
    return NGX_VULKAN_EVALUATE_DLSSD_EXT(cmd, n->handle, n->params, &p);
}
// DLSS Super Resolution, as caustica_ngx_create_sr / _eval_sr.
unsigned caustica_ngxvk_create_sr(NgxVk* n, VkCommandBuffer cmd, unsigned w, unsigned h, unsigned ow, unsigned oh, unsigned preset) {
    if (!(n->caps & 2u)) return NVSDK_NGX_Result_FAIL_FeatureNotSupported;
    if (n->sr) { NVSDK_NGX_VULKAN_ReleaseFeature(n->sr); n->sr = nullptr; }
    for (const char* key : {NVSDK_NGX_Parameter_DLSS_Hint_Render_Preset_DLAA, NVSDK_NGX_Parameter_DLSS_Hint_Render_Preset_Quality, NVSDK_NGX_Parameter_DLSS_Hint_Render_Preset_Balanced, NVSDK_NGX_Parameter_DLSS_Hint_Render_Preset_Performance, NVSDK_NGX_Parameter_DLSS_Hint_Render_Preset_UltraPerformance, NVSDK_NGX_Parameter_DLSS_Hint_Render_Preset_UltraQuality}) n->sr_params->Set(key, preset);
    NVSDK_NGX_DLSS_Create_Params p = {}; p.Feature.InWidth = w; p.Feature.InHeight = h; p.Feature.InTargetWidth = ow; p.Feature.InTargetHeight = oh; p.Feature.InPerfQualityValue = quality_for(w, ow);
    p.InFeatureCreateFlags = NVSDK_NGX_DLSS_Feature_Flags_IsHDR | NVSDK_NGX_DLSS_Feature_Flags_MVLowRes | NVSDK_NGX_DLSS_Feature_Flags_AutoExposure;
    return NGX_VULKAN_CREATE_DLSS_EXT1(n->device, cmd, 1, 1, &n->sr, n->sr_params, &p);
}
unsigned caustica_ngxvk_eval_sr(NgxVk* n, VkCommandBuffer cmd, const VkImageRef* color, const VkImageRef* depth, const VkImageRef* motion, const VkImageRef* output, unsigned w, unsigned h, float jx, float jy, int reset, float dt) {
    NVSDK_NGX_Resource_VK c = resource(*color, false), d = resource(*depth, false), m = resource(*motion, false), o = resource(*output, true);
    NVSDK_NGX_VK_DLSS_Eval_Params p = {};
    p.Feature.pInColor = &c; p.Feature.pInOutput = &o; p.pInDepth = &d; p.pInMotionVectors = &m;
    p.InJitterOffsetX = -jx; p.InJitterOffsetY = -jy; p.InRenderSubrectDimensions = {w, h}; p.InReset = reset; p.InMVScaleX = 1.; p.InMVScaleY = 1.;
    p.InPreExposure = 1.; p.InExposureScale = 1.; p.InFrameTimeDeltaInMsec = dt;
    return NGX_VULKAN_EVALUATE_DLSS_EXT(cmd, n->sr, n->sr_params, &p);
}

// XeSS, as xess.cpp.
unsigned caustica_xessvk_init(VkInstance instance, VkPhysicalDevice phys, VkDevice device, XessVk** out) {
    *out = nullptr;
    Module m = xess_module(); if (!m) return 1;
    XessVk* x = new XessVk;
    x->create = (XCreate)sym(m, "xessVKCreateContext"); x->init = (XInit)sym(m, "xessVKInit");
    x->execute = (XExecute)sym(m, "xessVKExecute"); x->destroy = (XDestroy)sym(m, "xessDestroyContext");
    x->velocity = (XScale)sym(m, "xessSetVelocityScale");
    if (!x->create || !x->init || !x->execute || !x->destroy || !x->velocity) { delete x; return 2; }
    xess_result_t r = x->create(instance, phys, device, &x->ctx);
    if (r != XESS_RESULT_SUCCESS) { delete x; return 0x100u | (unsigned)(r & 0xff); }
    *out = x; return 0;
}
void caustica_xessvk_destroy(XessVk* x) { if (!x) return; if (x->ctx) x->destroy(x->ctx); delete x; }
unsigned caustica_xessvk_create(XessVk* x, unsigned w, unsigned ow, unsigned oh, float velocity_scale) {
    float ratio = float(ow) / w;
    xess_vk_init_params_t p = {}; p.outputResolution = {ow, oh}; p.initFlags = XESS_INIT_FLAG_ENABLE_AUTOEXPOSURE;
    p.qualitySetting = ratio <= 1.01f ? XESS_QUALITY_SETTING_AA : ratio <= 1.4f ? XESS_QUALITY_SETTING_ULTRA_QUALITY : ratio <= 1.6f ? XESS_QUALITY_SETTING_QUALITY
                     : ratio <= 1.85f ? XESS_QUALITY_SETTING_BALANCED : ratio <= 2.5f ? XESS_QUALITY_SETTING_PERFORMANCE : XESS_QUALITY_SETTING_ULTRA_PERFORMANCE;
    xess_result_t r = x->init(x->ctx, &p); if (r != XESS_RESULT_SUCCESS) return (unsigned)r;
    return (unsigned)x->velocity(x->ctx, velocity_scale, velocity_scale);
}
unsigned caustica_xessvk_eval(XessVk* x, VkCommandBuffer cmd, const VkImageRef* color, const VkImageRef* depth, const VkImageRef* motion, const VkImageRef* output, unsigned w, unsigned h, float jx, float jy, int reset) {
    auto info = [](const VkImageRef& r) { xess_vk_image_view_info i = {}; i.imageView = r.view; i.image = r.image; i.subresourceRange = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1}; i.format = r.format; i.width = r.width; i.height = r.height; return i; };
    xess_vk_execute_params_t p = {}; p.colorTexture = info(*color); p.velocityTexture = info(*motion); p.depthTexture = info(*depth); p.outputTexture = info(*output);
    p.jitterOffsetX = -jx; p.jitterOffsetY = -jy; p.exposureScale = 1.f; p.resetHistory = reset ? 1 : 0; p.inputWidth = w; p.inputHeight = h;
    return (unsigned)x->execute(x->ctx, cmd, &p);
}
}
