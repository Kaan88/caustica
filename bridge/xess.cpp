// Intel XeSS Super Resolution behind a small C ABI. libxess.dll is loaded at runtime, so the
// app still starts when it is missing. XeSS runs on any DX12 GPU with Shader Model 6.4+.
#include <d3d12.h>
#include <windows.h>
#include "xess/xess_d3d12.h"

typedef xess_result_t (*CreateFn)(ID3D12Device*,xess_context_handle_t*);
typedef xess_result_t (*InitFn)(xess_context_handle_t,const xess_d3d12_init_params_t*);
typedef xess_result_t (*ExecuteFn)(xess_context_handle_t,ID3D12GraphicsCommandList*,const xess_d3d12_execute_params_t*);
typedef xess_result_t (*DestroyFn)(xess_context_handle_t);
typedef xess_result_t (*ScaleFn)(xess_context_handle_t,float,float);
struct Xess { HMODULE module=nullptr; CreateFn create=nullptr; InitFn init=nullptr; ExecuteFn execute=nullptr; DestroyFn destroy=nullptr; ScaleFn velocity=nullptr; xess_context_handle_t ctx=nullptr; };
extern "C" {
unsigned caustica_xess_init(ID3D12Device* device, Xess** out) {
    *out=nullptr;
    wchar_t path[MAX_PATH]; GetModuleFileNameW(nullptr,path,MAX_PATH);
    wchar_t* end=wcsrchr(path,L'\\'); if(end) end[1]=0;
    wcscat_s(path,L"libxess.dll");
    HMODULE m=LoadLibraryW(path); if(!m) return 1;
    Xess* x=new Xess; x->module=m;
    x->create=(CreateFn)GetProcAddress(m,"xessD3D12CreateContext"); x->init=(InitFn)GetProcAddress(m,"xessD3D12Init");
    x->execute=(ExecuteFn)GetProcAddress(m,"xessD3D12Execute"); x->destroy=(DestroyFn)GetProcAddress(m,"xessDestroyContext");
    x->velocity=(ScaleFn)GetProcAddress(m,"xessSetVelocityScale");
    if(!x->create||!x->init||!x->execute||!x->destroy||!x->velocity){FreeLibrary(m);delete x;return 2;}
    xess_result_t r=x->create(device,&x->ctx);
    if(r!=XESS_RESULT_SUCCESS){FreeLibrary(m);delete x;return 0x100u|(unsigned)(r&0xff);}
    *out=x; return 0;
}
void caustica_xess_destroy(Xess* x) { if(!x) return; if(x->ctx) x->destroy(x->ctx); FreeLibrary(x->module); delete x; }
// Initializing again with new sizes replaces the previous setup.
unsigned caustica_xess_create(Xess* x,unsigned w,unsigned ow,unsigned oh,float velocity_scale) {
    float ratio=float(ow)/w;
    xess_d3d12_init_params_t p={}; p.outputResolution={ow,oh}; p.initFlags=XESS_INIT_FLAG_ENABLE_AUTOEXPOSURE;
    p.qualitySetting=ratio<=1.01f?XESS_QUALITY_SETTING_AA:ratio<=1.4f?XESS_QUALITY_SETTING_ULTRA_QUALITY:ratio<=1.6f?XESS_QUALITY_SETTING_QUALITY:ratio<=1.85f?XESS_QUALITY_SETTING_BALANCED:ratio<=2.5f?XESS_QUALITY_SETTING_PERFORMANCE:XESS_QUALITY_SETTING_ULTRA_PERFORMANCE;
    xess_result_t r=x->init(x->ctx,&p); if(r!=XESS_RESULT_SUCCESS) return (unsigned)r;
    return (unsigned)x->velocity(x->ctx,velocity_scale,velocity_scale);
}
unsigned caustica_xess_eval(Xess* x,ID3D12GraphicsCommandList* cmd,ID3D12Resource* color,ID3D12Resource* depth,ID3D12Resource* motion,ID3D12Resource* output,
                           unsigned w,unsigned h,float jx,float jy,int reset) {
    xess_d3d12_execute_params_t p={}; p.pColorTexture=color; p.pVelocityTexture=motion; p.pDepthTexture=depth; p.pOutputTexture=output;
    p.jitterOffsetX=-jx; p.jitterOffsetY=-jy; p.exposureScale=1.f; p.resetHistory=reset?1:0; p.inputWidth=w; p.inputHeight=h;
    return (unsigned)x->execute(x->ctx,cmd,&p);
}
}
