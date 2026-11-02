// AMD FSR (FidelityFX API) upscaler behind a small C ABI. The signed loader DLL is loaded at
// runtime, so the app still starts when it is missing.
#define _WINDOWS
#include <d3d12.h>
#include <windows.h>
#include "ffx_api.h"
#include "ffx_api_loader.h"
#include "dx12/ffx_api_dx12.h"
#include "ffx_upscale.h"

struct Fsr { HMODULE module=nullptr; ffxFunctions f={}; ffxContext ctx=nullptr; ID3D12Device* device=nullptr; };
static HMODULE load_next_to_exe(const wchar_t* name) {
    wchar_t path[MAX_PATH]; GetModuleFileNameW(nullptr,path,MAX_PATH);
    wchar_t* end=wcsrchr(path,L'\\'); if(end) end[1]=0;
    wcscat_s(path,name); return LoadLibraryW(path);
}
extern "C" {
unsigned caustica_fsr_init(ID3D12Device* device, Fsr** out) {
    *out=nullptr;
    HMODULE m=load_next_to_exe(L"amd_fidelityfx_loader_dx12.dll"); if(!m) return 1;
    Fsr* f=new Fsr; f->module=m; f->device=device; ffxLoadFunctions(&f->f,m);
    if(!f->f.CreateContext||!f->f.Dispatch||!f->f.DestroyContext){FreeLibrary(m);delete f;return 2;}
    *out=f; return 0;
}
void caustica_fsr_release(Fsr* f) { if(f && f->ctx){f->f.DestroyContext(&f->ctx,nullptr);f->ctx=nullptr;} }
void caustica_fsr_destroy(Fsr* f) { if(!f) return; caustica_fsr_release(f); FreeLibrary(f->module); delete f; }
unsigned caustica_fsr_create(Fsr* f,unsigned w,unsigned h,unsigned ow,unsigned oh) {
    caustica_fsr_release(f);
    ffxCreateContextDescUpscaleVersion version={}; version.header.type=FFX_API_CREATE_CONTEXT_DESC_TYPE_UPSCALE_VERSION; version.version=FFX_UPSCALER_VERSION;
    ffxCreateBackendDX12Desc backend={}; backend.header.type=FFX_API_CREATE_CONTEXT_DESC_TYPE_BACKEND_DX12; backend.header.pNext=&version.header; backend.device=f->device;
    ffxCreateContextDescUpscale d={}; d.header.type=FFX_API_CREATE_CONTEXT_DESC_TYPE_UPSCALE; d.header.pNext=&backend.header;
    d.maxRenderSize={w,h}; d.maxUpscaleSize={ow,oh}; d.flags=FFX_UPSCALE_ENABLE_HIGH_DYNAMIC_RANGE|FFX_UPSCALE_ENABLE_AUTO_EXPOSURE;
    return f->f.CreateContext(&f->ctx,&d.header,nullptr);
}
unsigned caustica_fsr_eval(Fsr* f,ID3D12GraphicsCommandList* cmd,ID3D12Resource* color,ID3D12Resource* depth,ID3D12Resource* motion,ID3D12Resource* output,
                          unsigned w,unsigned h,unsigned ow,unsigned oh,float jx,float jy,int reset,float dt,float fov_y,float near_plane,float far_plane) {
    ffxDispatchDescUpscale d={}; d.header.type=FFX_API_DISPATCH_DESC_TYPE_UPSCALE; d.commandList=cmd;
    d.color=ffxApiGetResourceDX12(color,FFX_API_RESOURCE_STATE_COMPUTE_READ);
    d.depth=ffxApiGetResourceDX12(depth,FFX_API_RESOURCE_STATE_COMPUTE_READ);
    d.motionVectors=ffxApiGetResourceDX12(motion,FFX_API_RESOURCE_STATE_COMPUTE_READ);
    d.output=ffxApiGetResourceDX12(output,FFX_API_RESOURCE_STATE_UNORDERED_ACCESS);
    // Like NGX: FSR's jitter is the projection offset, the negated ray offset.
    d.jitterOffset={-jx,-jy}; d.motionVectorScale={1.f,1.f}; d.renderSize={w,h}; d.upscaleSize={ow,oh};
    d.frameTimeDelta=dt; d.preExposure=1.f; d.reset=reset!=0; d.cameraNear=near_plane; d.cameraFar=far_plane; d.cameraFovAngleVertical=fov_y; d.viewSpaceToMetersFactor=1.f;
    return f->f.Dispatch(&f->ctx,&d.header);
}
}
