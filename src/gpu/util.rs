//! D3D12 helpers: resources, pipelines, barriers and copies.
use super::*;

/// A UAV texture that lives across resizes (the environment and its CDFs).
pub(super) unsafe fn env_texture(
    device: &ID3D12Device5,
    w: u32,
    h: u32,
    format: DXGI_FORMAT,
) -> Result<Texture> {
    texture(
        device,
        w,
        h,
        format,
        D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
        D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
    )
}
/// A cubic RGBA32F UAV texture (the film-look LUT).
pub(super) unsafe fn texture_3d(device: &ID3D12Device5, size: u32) -> Result<Texture> {
    let mut resource: Option<ID3D12Resource> = None;
    device.CreateCommittedResource(
        &D3D12_HEAP_PROPERTIES {
            Type: D3D12_HEAP_TYPE_DEFAULT,
            ..Default::default()
        },
        D3D12_HEAP_FLAG_NONE,
        &D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE3D,
            Width: size as u64,
            Height: size,
            DepthOrArraySize: size as u16,
            MipLevels: 1,
            Format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Flags: D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
            ..Default::default()
        },
        D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
        None,
        &mut resource,
    )?;
    Ok(Texture {
        resource: resource.context("3D texture")?,
        format: DXGI_FORMAT_R32G32B32A32_FLOAT,
    })
}
pub(super) unsafe fn create_compute_pipeline(
    device: &ID3D12Device5,
    root: &ID3D12RootSignature,
    code: &[u8],
) -> Result<ID3D12PipelineState> {
    let mut d = D3D12_COMPUTE_PIPELINE_STATE_DESC {
        pRootSignature: ManuallyDrop::new(Some(root.clone())),
        CS: bytecode(code),
        // Opacity micromaps for the cs_6_5 tracers' RayQuery too (track 20a (a)):
        // D3D12_PIPELINE_STATE_FLAG_RAYQUERY_ALLOW_OPACITY_MICROMAPS_ON_PRE_SM69_SHADERS.
        Flags: if omm_on() {
            D3D12_PIPELINE_STATE_FLAGS(0x80)
        } else {
            D3D12_PIPELINE_STATE_FLAG_NONE
        },
        ..Default::default()
    };
    let p = device.CreateComputePipelineState(&d);
    ManuallyDrop::drop(&mut d.pRootSignature);
    Ok(p?)
}
/// Clears the current back buffer to the UI's dark grey and presents it, so the window isn't
/// white while the pipelines, NGX and the scene are made (seconds on a cold shader cache).
pub(super) unsafe fn present_blank(
    device: &ID3D12Device5,
    queue: &ID3D12CommandQueue,
    swap: &IDXGISwapChain3,
    allocator: &ID3D12CommandAllocator,
    list: &ID3D12GraphicsCommandList4,
    rtv: &ID3D12DescriptorHeap,
) -> Result<()> {
    unsafe {
        let back: ID3D12Resource = swap.GetBuffer(swap.GetCurrentBackBufferIndex())?;
        let handle = rtv.GetCPUDescriptorHandleForHeapStart();
        device.CreateRenderTargetView(&back, None, handle);
        allocator.Reset()?;
        list.Reset(allocator, None)?;
        transition(
            list,
            &back,
            D3D12_RESOURCE_STATE_PRESENT,
            D3D12_RESOURCE_STATE_RENDER_TARGET,
        );
        list.ClearRenderTargetView(handle, &[0.1, 0.1, 0.1, 1.], None);
        transition(
            list,
            &back,
            D3D12_RESOURCE_STATE_RENDER_TARGET,
            D3D12_RESOURCE_STATE_PRESENT,
        );
        list.Close()?;
        queue.ExecuteCommandLists(&[Some(list.cast()?)]);
        swap.Present(0, DXGI_PRESENT(0)).ok()?;
        let fence: ID3D12Fence = device.CreateFence(0, D3D12_FENCE_FLAG_NONE)?;
        let event = CreateEventW(None, false, false, None)?;
        let waited = queue
            .Signal(&fence, 1)
            .and_then(|()| fence.SetEventOnCompletion(1, event))
            .map(|()| WaitForSingleObject(event, 30_000));
        let _ = windows::Win32::Foundation::CloseHandle(event);
        ensure!(
            waited? == windows::Win32::Foundation::WAIT_OBJECT_0,
            "GPU fence timeout on the first frame"
        );
        Ok(())
    }
}
/// Raw D3D12 objects shared with the pipeline threads (`create_compute_pipelines`): the device
/// is free-threaded and the root signature is immutable.
pub(super) struct Threaded<T>(pub(super) T);
unsafe impl<T> Send for Threaded<T> {}
unsafe impl<T> Sync for Threaded<T> {}
/// Runs `make` for every input on its own thread and keeps the order.
pub(super) fn in_parallel<I: Sync, O>(inputs: &[I], make: impl Fn(&I) -> O + Sync) -> Vec<O> {
    let make = &make;
    std::thread::scope(|scope| {
        let handles: Vec<_> = inputs
            .iter()
            .map(|i| scope.spawn(move || Threaded(make(i))))
            .collect();
        handles
            .into_iter()
            .map(|h| match h.join() {
                Ok(o) => o.0,
                Err(e) => std::panic::resume_unwind(e),
            })
            .collect()
    })
}
/// `create_compute_pipeline` for every shader at once (in order).
pub(super) unsafe fn create_compute_pipelines<const N: usize>(
    device: &ID3D12Device5,
    root: &ID3D12RootSignature,
    code: [&[u8]; N],
) -> Result<[ID3D12PipelineState; N]> {
    let shared = Threaded((device, root));
    let made = in_parallel(&code, |c| {
        let (device, root) = shared.0;
        unsafe { create_compute_pipeline(device, root, c) }
    });
    let made: Vec<ID3D12PipelineState> = made.into_iter().collect::<Result<_>>()?;
    made.try_into()
        .map_err(|_| anyhow::anyhow!("one pipeline per shader"))
}
/// A SER tracer (track 20): a ray tracing state object with just the `trace_rg` ray
/// generation shader of a DXIL library (it traces with inline RayQuery, so no hit groups),
/// on the compute root signature as its global one, and the one-record shader table.
pub struct SerPipeline {
    pub(super) state: ID3D12StateObject,
    pub(super) table: ID3D12Resource,
}
pub(super) unsafe fn create_ser_pipeline(
    device: &ID3D12Device5,
    root: &ID3D12RootSignature,
    code: &[u8],
) -> Result<SerPipeline> {
    unsafe {
        let library = D3D12_DXIL_LIBRARY_DESC {
            DXILLibrary: bytecode(code),
            ..Default::default()
        };
        let mut global = D3D12_GLOBAL_ROOT_SIGNATURE {
            pGlobalRootSignature: ManuallyDrop::new(Some(root.clone())),
        };
        // Inline ray queries only: no TraceRay, so the smallest payload and recursion.
        let shader = D3D12_RAYTRACING_SHADER_CONFIG {
            MaxPayloadSizeInBytes: 4,
            MaxAttributeSizeInBytes: 8,
        };
        let pipeline = D3D12_RAYTRACING_PIPELINE_CONFIG {
            MaxTraceRecursionDepth: 1,
        };
        let sub = |t: D3D12_STATE_SUBOBJECT_TYPE, d: *const std::ffi::c_void| {
            D3D12_STATE_SUBOBJECT { Type: t, pDesc: d }
        };
        let subobjects = [
            sub(
                D3D12_STATE_SUBOBJECT_TYPE_DXIL_LIBRARY,
                &library as *const _ as _,
            ),
            sub(
                D3D12_STATE_SUBOBJECT_TYPE_GLOBAL_ROOT_SIGNATURE,
                &global as *const _ as _,
            ),
            sub(
                D3D12_STATE_SUBOBJECT_TYPE_RAYTRACING_SHADER_CONFIG,
                &shader as *const _ as _,
            ),
            sub(
                D3D12_STATE_SUBOBJECT_TYPE_RAYTRACING_PIPELINE_CONFIG,
                &pipeline as *const _ as _,
            ),
        ];
        let desc = D3D12_STATE_OBJECT_DESC {
            Type: D3D12_STATE_OBJECT_TYPE_RAYTRACING_PIPELINE,
            NumSubobjects: subobjects.len() as u32,
            pSubobjects: subobjects.as_ptr(),
        };
        let state = device.CreateStateObject::<ID3D12StateObject>(&desc);
        ManuallyDrop::drop(&mut global.pGlobalRootSignature);
        let state = state?;
        let props: ID3D12StateObjectProperties = state.cast()?;
        let id = props.GetShaderIdentifier(windows::core::w!("trace_rg"));
        anyhow::ensure!(!id.is_null(), "trace_rg has no shader identifier");
        let id = std::slice::from_raw_parts(
            id as *const u8,
            D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES as usize,
        );
        // Upload heaps start 64 KiB aligned, past the 64-byte shader table alignment.
        let table = upload(device, id)?;
        Ok(SerPipeline { state, table })
    }
}
/// The SER tracers, when the device supports SER and CAUSTICA_NO_SER isn't set (A/B
/// measurements); a creation failure falls back to the compute tracers with a message.
pub(super) unsafe fn make_ser(
    device: &ID3D12Device5,
    root: &ID3D12RootSignature,
    support: SerSupport,
    code: [&[u8]; 7],
) -> Option<[SerPipeline; 7]> {
    if !support.available() || std::env::var_os("CAUSTICA_NO_SER").is_some() {
        return None;
    }
    let shared = Threaded((device, root));
    let made: Result<Vec<SerPipeline>> = in_parallel(&code, |c| {
        let (device, root) = shared.0;
        unsafe { create_ser_pipeline(device, root, c) }
    })
    .into_iter()
    .collect();
    match made {
        Ok(v) => v.try_into().ok(),
        Err(e) => {
            eprintln!("SER tracers unavailable, using the compute tracers: {e:#}");
            None
        }
    }
}
pub(super) unsafe fn create_gui_pipeline(
    device: &ID3D12Device5,
    root: &ID3D12RootSignature,
    format: DXGI_FORMAT,
    vs: &[u8],
    ps: &[u8],
) -> Result<ID3D12PipelineState> {
    let input = [
        D3D12_INPUT_ELEMENT_DESC {
            SemanticName: PCSTR(c"POSITION".as_ptr() as _),
            Format: DXGI_FORMAT_R32G32_FLOAT,
            InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
            ..Default::default()
        },
        D3D12_INPUT_ELEMENT_DESC {
            SemanticName: PCSTR(c"TEXCOORD".as_ptr() as _),
            Format: DXGI_FORMAT_R32G32_FLOAT,
            AlignedByteOffset: 8,
            InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
            ..Default::default()
        },
        D3D12_INPUT_ELEMENT_DESC {
            SemanticName: PCSTR(c"COLOR".as_ptr() as _),
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            AlignedByteOffset: 16,
            InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
            ..Default::default()
        },
    ];
    let blend = D3D12_RENDER_TARGET_BLEND_DESC {
        BlendEnable: true.into(),
        SrcBlend: D3D12_BLEND_ONE,
        DestBlend: D3D12_BLEND_INV_SRC_ALPHA,
        BlendOp: D3D12_BLEND_OP_ADD,
        SrcBlendAlpha: D3D12_BLEND_ONE,
        DestBlendAlpha: D3D12_BLEND_INV_SRC_ALPHA,
        BlendOpAlpha: D3D12_BLEND_OP_ADD,
        LogicOp: D3D12_LOGIC_OP_NOOP,
        RenderTargetWriteMask: D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8,
        ..Default::default()
    };
    let mut d = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
        pRootSignature: ManuallyDrop::new(Some(root.clone())),
        VS: bytecode(vs),
        PS: bytecode(ps),
        BlendState: D3D12_BLEND_DESC {
            RenderTarget: [blend; 8],
            ..Default::default()
        },
        SampleMask: u32::MAX,
        RasterizerState: D3D12_RASTERIZER_DESC {
            FillMode: D3D12_FILL_MODE_SOLID,
            CullMode: D3D12_CULL_MODE_NONE,
            DepthClipEnable: true.into(),
            ..Default::default()
        },
        DepthStencilState: D3D12_DEPTH_STENCIL_DESC {
            DepthEnable: false.into(),
            DepthFunc: D3D12_COMPARISON_FUNC_ALWAYS,
            ..Default::default()
        },
        InputLayout: D3D12_INPUT_LAYOUT_DESC {
            pInputElementDescs: input.as_ptr(),
            NumElements: input.len() as _,
        },
        PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
        NumRenderTargets: 1,
        RTVFormats: [
            format,
            DXGI_FORMAT_UNKNOWN,
            DXGI_FORMAT_UNKNOWN,
            DXGI_FORMAT_UNKNOWN,
            DXGI_FORMAT_UNKNOWN,
            DXGI_FORMAT_UNKNOWN,
            DXGI_FORMAT_UNKNOWN,
            DXGI_FORMAT_UNKNOWN,
        ],
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        ..Default::default()
    };
    let result = device.CreateGraphicsPipelineState(&d);
    ManuallyDrop::drop(&mut d.pRootSignature);
    Ok(result?)
}
pub(super) unsafe fn buffer(
    device: &ID3D12Device5,
    size: u64,
    heap: D3D12_HEAP_TYPE,
    flags: D3D12_RESOURCE_FLAGS,
    state: D3D12_RESOURCE_STATES,
) -> Result<ID3D12Resource> {
    let desc = D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
        Width: size.max(256).next_multiple_of(256),
        Height: 1,
        DepthOrArraySize: 1,
        MipLevels: 1,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
        Flags: flags,
        ..Default::default()
    };
    let mut r = None;
    device.CreateCommittedResource(
        &D3D12_HEAP_PROPERTIES {
            Type: heap,
            ..Default::default()
        },
        D3D12_HEAP_FLAG_NONE,
        &desc,
        // Default-heap buffers start in COMMON whatever is asked (the Agility SDK's layer
        // warns about any other state, track 20) and promote to UAV / shader-resource use on
        // their own; upload, readback and acceleration-structure states still apply.
        if heap == D3D12_HEAP_TYPE_DEFAULT
            && (state == D3D12_RESOURCE_STATE_UNORDERED_ACCESS
                || state == D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE)
        {
            D3D12_RESOURCE_STATE_COMMON
        } else {
            state
        },
        None,
        &mut r,
    )?;
    Ok(r.unwrap())
}
pub(super) unsafe fn texture(
    device: &ID3D12Device5,
    w: u32,
    h: u32,
    format: DXGI_FORMAT,
    flags: D3D12_RESOURCE_FLAGS,
    state: D3D12_RESOURCE_STATES,
) -> Result<Texture> {
    let desc = D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
        Width: w as u64,
        Height: h,
        DepthOrArraySize: 1,
        MipLevels: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Flags: flags,
        ..Default::default()
    };
    let mut r = None;
    device.CreateCommittedResource(
        &D3D12_HEAP_PROPERTIES {
            Type: D3D12_HEAP_TYPE_DEFAULT,
            ..Default::default()
        },
        D3D12_HEAP_FLAG_NONE,
        &desc,
        state,
        None,
        &mut r,
    )?;
    Ok(Texture {
        resource: r.unwrap(),
        format,
    })
}
pub(super) unsafe fn upload(device: &ID3D12Device5, bytes: &[u8]) -> Result<ID3D12Resource> {
    let r = buffer(
        device,
        bytes.len() as u64,
        D3D12_HEAP_TYPE_UPLOAD,
        D3D12_RESOURCE_FLAG_NONE,
        D3D12_RESOURCE_STATE_GENERIC_READ,
    )?;
    write_buffer(&r, 0, bytes)?;
    Ok(r)
}
pub(super) unsafe fn write_buffer(r: &ID3D12Resource, offset: u64, bytes: &[u8]) -> Result<()> {
    let mut p = null_mut();
    r.Map(0, Some(&D3D12_RANGE::default()), Some(&mut p))?;
    std::ptr::copy_nonoverlapping(
        bytes.as_ptr(),
        (p as *mut u8).add(offset as usize),
        bytes.len(),
    );
    r.Unmap(0, None);
    Ok(())
}
pub(super) unsafe fn transition(
    cmd: &ID3D12GraphicsCommandList4,
    r: &ID3D12Resource,
    before: D3D12_RESOURCE_STATES,
    after: D3D12_RESOURCE_STATES,
) {
    let mut b = D3D12_RESOURCE_BARRIER {
        Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
        Anonymous: D3D12_RESOURCE_BARRIER_0 {
            Transition: ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                pResource: ManuallyDrop::new(Some(r.clone())),
                Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                StateBefore: before,
                StateAfter: after,
            }),
        },
        ..Default::default()
    };
    cmd.ResourceBarrier(std::slice::from_ref(&b));
    ManuallyDrop::drop(&mut (*b.Anonymous.Transition).pResource);
}
pub(super) unsafe fn uav(cmd: &ID3D12GraphicsCommandList4, r: Option<&ID3D12Resource>) {
    let mut b = D3D12_RESOURCE_BARRIER {
        Type: D3D12_RESOURCE_BARRIER_TYPE_UAV,
        Anonymous: D3D12_RESOURCE_BARRIER_0 {
            UAV: ManuallyDrop::new(D3D12_RESOURCE_UAV_BARRIER {
                pResource: ManuallyDrop::new(r.cloned()),
            }),
        },
        ..Default::default()
    };
    cmd.ResourceBarrier(std::slice::from_ref(&b));
    ManuallyDrop::drop(&mut (*b.Anonymous.UAV).pResource);
}
pub(super) unsafe fn copy_texture(
    cmd: &ID3D12GraphicsCommandList4,
    from: &ID3D12Resource,
    to: &ID3D12Resource,
) {
    transition(
        cmd,
        from,
        D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
        D3D12_RESOURCE_STATE_COPY_SOURCE,
    );
    transition(
        cmd,
        to,
        D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
        D3D12_RESOURCE_STATE_COPY_DEST,
    );
    cmd.CopyResource(to, from);
    transition(
        cmd,
        from,
        D3D12_RESOURCE_STATE_COPY_SOURCE,
        D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
    );
    transition(
        cmd,
        to,
        D3D12_RESOURCE_STATE_COPY_DEST,
        D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
    );
}
pub(super) unsafe fn location(r: &ID3D12Resource) -> D3D12_TEXTURE_COPY_LOCATION {
    D3D12_TEXTURE_COPY_LOCATION {
        pResource: ManuallyDrop::new(Some(r.clone())),
        Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
        Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
            SubresourceIndex: 0,
        },
    }
}
pub(super) unsafe fn footprint(
    device: &ID3D12Device5,
    r: &ID3D12Resource,
) -> (D3D12_PLACED_SUBRESOURCE_FOOTPRINT, u64) {
    let mut layout = D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default();
    let mut bytes = 0;
    device.GetCopyableFootprints(
        &r.GetDesc(),
        0,
        1,
        0,
        Some(&mut layout),
        None,
        None,
        Some(&mut bytes),
    );
    (layout, bytes)
}
pub(super) fn halton(mut index: u32, base: u32) -> f32 {
    let (mut f, mut result) = (1., 0.);
    while index > 0 {
        f /= base as f32;
        result += f * (index % base) as f32;
        index /= base;
    }
    result
}

pub(super) fn root_srv(register: u32) -> D3D12_ROOT_PARAMETER {
    D3D12_ROOT_PARAMETER {
        ParameterType: D3D12_ROOT_PARAMETER_TYPE_SRV,
        Anonymous: D3D12_ROOT_PARAMETER_0 {
            Descriptor: D3D12_ROOT_DESCRIPTOR {
                ShaderRegister: register,
                RegisterSpace: 0,
            },
        },
        ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
    }
}
