// Small C ABI over NVIDIA's C++ parameter interface. Rendering remains in Rust/HLSL.
#include <d3d12.h>
#include <windows.h>
#include <initializer_list>
#include "nvsdk_ngx.h"
#include "nvsdk_ngx_helpers_d3d.h"
#include "nvsdk_ngx_helpers_dlssd_d3d.h"

// handle = Ray Reconstruction (DLSSD); sr = Super Resolution (DLSS), with its own parameter block.
struct Ngx { NVSDK_NGX_Parameter* params=nullptr; NVSDK_NGX_Parameter* sr_params=nullptr; NVSDK_NGX_Handle* handle=nullptr; NVSDK_NGX_Handle* sr=nullptr; ID3D12Device* device=nullptr; bool initialized=false; unsigned caps=0; };
static NVSDK_NGX_PerfQuality_Value quality_for(unsigned w,unsigned ow) {
    float ratio=float(w)/ow;
    return ratio>=0.99f?NVSDK_NGX_PerfQuality_Value_DLAA:ratio>=0.62f?NVSDK_NGX_PerfQuality_Value_MaxQuality:ratio>=0.54f?NVSDK_NGX_PerfQuality_Value_Balanced:ratio>=0.42f?NVSDK_NGX_PerfQuality_Value_MaxPerf:NVSDK_NGX_PerfQuality_Value_UltraPerformance;
}
extern "C" {
void caustica_ngx_release(Ngx* n) {
    if(n && n->handle){NVSDK_NGX_D3D12_ReleaseFeature(n->handle);n->handle=nullptr;}
    if(n && n->sr){NVSDK_NGX_D3D12_ReleaseFeature(n->sr);n->sr=nullptr;}
}
// Bit 0: Ray Reconstruction available, bit 1: Super Resolution available.
unsigned caustica_ngx_caps(Ngx* n) { return n ? n->caps : 0; }
void caustica_ngx_destroy(Ngx* n) {
    if(!n) return;
    caustica_ngx_release(n);
    if(n->sr_params) NVSDK_NGX_D3D12_DestroyParameters(n->sr_params);
    if(n->params) NVSDK_NGX_D3D12_DestroyParameters(n->params);
    if(n->initialized) NVSDK_NGX_D3D12_Shutdown1(n->device);
    delete n;
}
unsigned caustica_ngx_init(ID3D12Device* device, Ngx** result) {
    *result=nullptr;
    Ngx* n=new Ngx; n->device=device;
    wchar_t path[MAX_PATH]; GetModuleFileNameW(nullptr,path,MAX_PATH);
    wchar_t* end=wcsrchr(path,L'\\'); if(end)*end=0;
    const wchar_t* paths[]={path}; NVSDK_NGX_FeatureCommonInfo info={};info.PathListInfo.Path=paths;info.PathListInfo.Length=1;
    // The app's own identifier: without an application ID from NVIDIA, the DLSS guide (5.2.1) has
    // the app supply one here, GUID-like, for the custom engine type.
    auto r=NVSDK_NGX_D3D12_Init_with_ProjectID("e9641daa-48c9-4e65-a593-c3a11259bb40",NVSDK_NGX_ENGINE_TYPE_CUSTOM,"0.1.0",path,device,&info);
    if(NVSDK_NGX_FAILED(r)){delete n;return r;} n->initialized=true;
    r=NVSDK_NGX_D3D12_GetCapabilityParameters(&n->params);
    if(NVSDK_NGX_FAILED(r)){caustica_ngx_destroy(n);return r;}
    int rr=0,sr=0; n->params->Get(NVSDK_NGX_Parameter_SuperSamplingDenoising_Available,&rr); n->params->Get(NVSDK_NGX_Parameter_SuperSampling_Available,&sr);
    n->caps=(rr?1u:0u)|(sr?2u:0u);
    if(!n->caps){caustica_ngx_destroy(n);return NVSDK_NGX_Result_FAIL_FeatureNotSupported;}
    if(sr && NVSDK_NGX_FAILED(NVSDK_NGX_D3D12_AllocateParameters(&n->sr_params))) n->caps&=~2u;
    *result=n;return NVSDK_NGX_Result_Success;
}
unsigned caustica_ngx_create(Ngx* n, ID3D12GraphicsCommandList* cmd,unsigned w,unsigned h,unsigned ow,unsigned oh,unsigned preset) {
    if(!(n->caps&1u)) return NVSDK_NGX_Result_FAIL_FeatureNotSupported;
    if(n->handle){NVSDK_NGX_D3D12_ReleaseFeature(n->handle);n->handle=nullptr;}
    NVSDK_NGX_DLSSD_Create_Params p={};p.InDenoiseMode=NVSDK_NGX_DLSS_Denoise_Mode_DLUnified;
    p.InRoughnessMode=NVSDK_NGX_DLSS_Roughness_Mode_Packed;p.InUseHWDepth=NVSDK_NGX_DLSS_Depth_Type_Linear;
    p.InWidth=w;p.InHeight=h;p.InTargetWidth=ow;p.InTargetHeight=oh;
    p.InPerfQualityValue=quality_for(w,ow);
    for(const char* key:{NVSDK_NGX_Parameter_RayReconstruction_Hint_Render_Preset_DLAA,NVSDK_NGX_Parameter_RayReconstruction_Hint_Render_Preset_Quality,NVSDK_NGX_Parameter_RayReconstruction_Hint_Render_Preset_Balanced,NVSDK_NGX_Parameter_RayReconstruction_Hint_Render_Preset_Performance,NVSDK_NGX_Parameter_RayReconstruction_Hint_Render_Preset_UltraPerformance}) n->params->Set(key,preset);
    p.InFeatureCreateFlags=NVSDK_NGX_DLSS_Feature_Flags_IsHDR|NVSDK_NGX_DLSS_Feature_Flags_MVLowRes;
    return NGX_D3D12_CREATE_DLSSD_EXT(cmd,1,1,&n->handle,n->params,&p);
}
unsigned caustica_ngx_eval(Ngx* n,ID3D12GraphicsCommandList* cmd,ID3D12Resource** resources,unsigned w,unsigned h,float jx,float jy,int reset,float dt,float* view,float* projection,float pre_exposure) {
    NVSDK_NGX_D3D12_DLSSD_Eval_Params p={};
    p.pInColor=resources[0];p.pInOutput=resources[14];p.pInNormals=resources[2];p.pInDiffuseAlbedo=resources[3];p.pInSpecularAlbedo=resources[4];p.pInDepth=resources[5];p.pInMotionVectors=resources[6];p.pInSpecularHitDistance=resources[7];
    // Sampling pixel center + jitter shifts image features by -jitter. NGX
    // expects that image displacement in top-left pixel coordinates.
    p.InRenderSubrectDimensions={w,h};p.InJitterOffsetX=-jx;p.InJitterOffsetY=-jy;p.InReset=reset;p.InMVScaleX=1.;p.InMVScaleY=1.;p.InPreExposure=pre_exposure;p.InExposureScale=1.;p.InFrameTimeDeltaInMsec=dt;
    p.pInWorldToViewMatrix=view;p.pInViewToClipMatrix=projection;
    return NGX_D3D12_EVALUATE_DLSSD_EXT(cmd,n->handle,n->params,&p);
}
// DLSS Super Resolution over the native renderer's denoised color: device depth, low-res pixel motion.
unsigned caustica_ngx_create_sr(Ngx* n, ID3D12GraphicsCommandList* cmd,unsigned w,unsigned h,unsigned ow,unsigned oh,unsigned preset) {
    if(!(n->caps&2u)) return NVSDK_NGX_Result_FAIL_FeatureNotSupported;
    if(n->sr){NVSDK_NGX_D3D12_ReleaseFeature(n->sr);n->sr=nullptr;}
    // 0 lets NVIDIA pick per quality mode; 10-13 are presets J, K, L and M.
    for(const char* key:{NVSDK_NGX_Parameter_DLSS_Hint_Render_Preset_DLAA,NVSDK_NGX_Parameter_DLSS_Hint_Render_Preset_Quality,NVSDK_NGX_Parameter_DLSS_Hint_Render_Preset_Balanced,NVSDK_NGX_Parameter_DLSS_Hint_Render_Preset_Performance,NVSDK_NGX_Parameter_DLSS_Hint_Render_Preset_UltraPerformance,NVSDK_NGX_Parameter_DLSS_Hint_Render_Preset_UltraQuality}) n->sr_params->Set(key,preset);
    NVSDK_NGX_DLSS_Create_Params p={};p.Feature.InWidth=w;p.Feature.InHeight=h;p.Feature.InTargetWidth=ow;p.Feature.InTargetHeight=oh;p.Feature.InPerfQualityValue=quality_for(w,ow);
    p.InFeatureCreateFlags=NVSDK_NGX_DLSS_Feature_Flags_IsHDR|NVSDK_NGX_DLSS_Feature_Flags_MVLowRes|NVSDK_NGX_DLSS_Feature_Flags_AutoExposure;
    return NGX_D3D12_CREATE_DLSS_EXT(cmd,1,1,&n->sr,n->sr_params,&p);
}
unsigned caustica_ngx_eval_sr(Ngx* n,ID3D12GraphicsCommandList* cmd,ID3D12Resource* color,ID3D12Resource* depth,ID3D12Resource* motion,ID3D12Resource* output,unsigned w,unsigned h,float jx,float jy,int reset,float dt,float pre_exposure) {
    NVSDK_NGX_D3D12_DLSS_Eval_Params p={};
    p.Feature.pInColor=color;p.Feature.pInOutput=output;p.pInDepth=depth;p.pInMotionVectors=motion;
    // Same convention as Ray Reconstruction: the image displacement, i.e. the negated ray offset.
    p.InJitterOffsetX=-jx;p.InJitterOffsetY=-jy;p.InRenderSubrectDimensions={w,h};p.InReset=reset;p.InMVScaleX=1.;p.InMVScaleY=1.;p.InPreExposure=pre_exposure;p.InExposureScale=1.;p.InFrameTimeDeltaInMsec=dt;
    return NGX_D3D12_EVALUATE_DLSS_EXT(cmd,n->sr,n->sr_params,&p);
}
}
