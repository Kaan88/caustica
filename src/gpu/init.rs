//! Device, swap chain, pipelines and resources (`Gpu::new`).
use super::*;

impl Gpu {
    pub unsafe fn new(
        hwnd: HWND,
        width: u32,
        height: u32,
        s: &Settings,
        debug: bool,
    ) -> Result<Self> {
        if debug {
            let mut d: Option<ID3D12Debug> = None;
            D3D12GetDebugInterface(&mut d)?;
            d.unwrap().EnableDebugLayer();
            // DRED: on a device removal, `removed_reason` reports the last GPU operations and a
            // page fault's address (self-test failures name the pass that hung or faulted).
            let mut dred: Option<ID3D12DeviceRemovedExtendedDataSettings> = None;
            if D3D12GetDebugInterface(&mut dred).is_ok()
                && let Some(dred) = dred
            {
                dred.SetAutoBreadcrumbsEnablement(D3D12_DRED_ENABLEMENT_FORCED_ON);
                dred.SetPageFaultEnablement(D3D12_DRED_ENABLEMENT_FORCED_ON);
            }
            if let Ok(q) = DXGIGetDebugInterface1::<IDXGIInfoQueue>(0) {
                q.SetBreakOnSeverity(
                    DXGI_DEBUG_ALL,
                    DXGI_INFO_QUEUE_MESSAGE_SEVERITY_ERROR,
                    false,
                )?;
                q.SetBreakOnSeverity(
                    DXGI_DEBUG_ALL,
                    DXGI_INFO_QUEUE_MESSAGE_SEVERITY_CORRUPTION,
                    false,
                )?;
            }
        }
        let factory: IDXGIFactory6 = CreateDXGIFactory2(if debug {
            DXGI_CREATE_FACTORY_DEBUG
        } else {
            DXGI_CREATE_FACTORY_FLAGS(0)
        })?;
        if debug && let Ok(q) = DXGIGetDebugInterface1::<IDXGIInfoQueue>(0) {
            q.SetBreakOnSeverity(
                DXGI_DEBUG_ALL,
                DXGI_INFO_QUEUE_MESSAGE_SEVERITY_ERROR,
                false,
            )?;
            q.SetBreakOnSeverity(
                DXGI_DEBUG_ALL,
                DXGI_INFO_QUEUE_MESSAGE_SEVERITY_CORRUPTION,
                false,
            )?;
        }
        let mut selected = None;
        for i in 0..32 {
            let Ok(adapter) = factory.EnumAdapterByGpuPreference::<IDXGIAdapter1>(
                i,
                DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE,
            ) else {
                break;
            };
            let mut device = None;
            if D3D12CreateDevice::<_, ID3D12Device5>(&adapter, D3D_FEATURE_LEVEL_12_1, &mut device)
                .is_err()
            {
                continue;
            }
            let dev = device.unwrap();
            let mut opt = D3D12_FEATURE_DATA_D3D12_OPTIONS5::default();
            dev.CheckFeatureSupport(
                D3D12_FEATURE_D3D12_OPTIONS5,
                &mut opt as *mut _ as _,
                size_of_val(&opt) as u32,
            )?;
            if opt.RaytracingTier.0 < D3D12_RAYTRACING_TIER_1_1.0 {
                continue;
            }
            let desc = adapter.GetDesc1()?;
            let end = desc.Description.iter().position(|v| *v == 0).unwrap_or(128);
            selected = Some((dev, String::from_utf16_lossy(&desc.Description[..end])));
            break;
        }
        let (device,adapter_name)=selected.context("No D3D12 adapter with DXR tier 1.1. Update the GPU driver and use a ray-tracing-capable GPU.")?;
        let ser = ser_support(&device);
        OMM.store(
            ser.tier >= 12 && std::env::var_os("CAUSTICA_NO_OMM").is_none_or(|v| v != "1"),
            std::sync::atomic::Ordering::Relaxed,
        );
        eprintln!(
            "D3D12: shader model {}.{}, raytracing tier {}.{}; SER {}",
            ser.shader_model >> 4,
            ser.shader_model & 15,
            ser.tier / 10,
            ser.tier % 10,
            match (ser.available(), ser.reorders) {
                (false, _) => "unavailable",
                (true, false) => "accepted, no reordering",
                (true, true) => "reorders",
            }
        );
        let queue: ID3D12CommandQueue = device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
            Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
            ..Default::default()
        })?;
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width,
            Height: height,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            ..Default::default()
        };
        let swap: IDXGISwapChain3 = factory
            .CreateSwapChainForHwnd(&queue, hwnd, &desc, None, None)?
            .cast()?;
        factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER)?;
        let allocator = device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)?;
        let list: ID3D12GraphicsCommandList4 =
            device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &allocator, None)?;
        list.Close()?;
        let fence = device.CreateFence(0, D3D12_FENCE_FLAG_NONE)?;
        let event = CreateEventW(None, false, false, None)?;
        let heap = device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
            Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
            NumDescriptors: (HEAP_SIZE + MAX_MAPS) as u32,
            Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
            NodeMask: 0,
        })?;
        let rtv = device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
            Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
            NumDescriptors: 2,
            ..Default::default()
        })?;
        // Cosmetic: a failure here only leaves the window white until the first frame.
        if let Err(e) = present_blank(&device, &queue, &swap, &allocator, &list, &rtv) {
            eprintln!("First blank frame skipped: {e:#}");
        }
        let ranges = [
            D3D12_DESCRIPTOR_RANGE {
                RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_UAV,
                // The per-resize textures, the LUT (u29), the environment and its CDFs (u30, u31).
                NumDescriptors: TEXTURES as u32 + 3,
                BaseShaderRegister: 0,
                RegisterSpace: 0,
                OffsetInDescriptorsFromTableStart: 0,
            },
            D3D12_DESCRIPTOR_RANGE {
                RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
                NumDescriptors: 1,
                BaseShaderRegister: 2,
                RegisterSpace: 0,
                OffsetInDescriptorsFromTableStart: 0,
            },
            // Texture maps: only slots a material points at are read, so unused ones may stay
            // empty (root signature 1.0 descriptors are volatile).
            D3D12_DESCRIPTOR_RANGE {
                RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
                NumDescriptors: MAX_MAPS as u32,
                BaseShaderRegister: 0,
                RegisterSpace: 1,
                OffsetInDescriptorsFromTableStart: 0,
            },
        ];
        let params = [
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_CBV,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    Descriptor: D3D12_ROOT_DESCRIPTOR {
                        ShaderRegister: 0,
                        RegisterSpace: 0,
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
            },
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_SRV,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    Descriptor: D3D12_ROOT_DESCRIPTOR {
                        ShaderRegister: 0,
                        RegisterSpace: 0,
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
            },
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_SRV,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    Descriptor: D3D12_ROOT_DESCRIPTOR {
                        ShaderRegister: 1,
                        RegisterSpace: 0,
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
            },
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE {
                        NumDescriptorRanges: 1,
                        pDescriptorRanges: &ranges[0],
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
            },
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE {
                        NumDescriptorRanges: 1,
                        pDescriptorRanges: &ranges[1],
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
            },
            root_srv(3),
            root_srv(4),
            root_srv(5),
            root_srv(6),
            root_srv(7),
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE {
                        NumDescriptorRanges: 1,
                        pDescriptorRanges: &ranges[2],
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
            },
            // Track 14: the path visualizer's records (u0 space 2, the recording trace only).
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_UAV,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    Descriptor: D3D12_ROOT_DESCRIPTOR {
                        ShaderRegister: 0,
                        RegisterSpace: 2,
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
            },
            // Light path expressions: the AOVs (u1 space 2) and the DFA table (t8).
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_UAV,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    Descriptor: D3D12_ROOT_DESCRIPTOR {
                        ShaderRegister: 1,
                        RegisterSpace: 2,
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
            },
            root_srv(8),
            // ReSTIR DI: the reservoirs (u2 space 2).
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_UAV,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    Descriptor: D3D12_ROOT_DESCRIPTOR {
                        ShaderRegister: 2,
                        RegisterSpace: 2,
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
            },
            // Track 15 integrators: BDPT splats and the photon grid (u3 space 2, raw).
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_UAV,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    Descriptor: D3D12_ROOT_DESCRIPTOR {
                        ShaderRegister: 3,
                        RegisterSpace: 2,
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
            },
            // Track 16: the lens system (t9).
            root_srv(9),
            // Track 16: glare (u4 space 2).
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_UAV,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    Descriptor: D3D12_ROOT_DESCRIPTOR {
                        ShaderRegister: 4,
                        RegisterSpace: 2,
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
            },
        ];
        let gui_sampler = D3D12_STATIC_SAMPLER_DESC {
            Filter: D3D12_FILTER_MIN_MAG_MIP_LINEAR,
            AddressU: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
            AddressV: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
            AddressW: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
            MaxLOD: f32::MAX,
            ShaderRegister: 0,
            ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
            ComparisonFunc: D3D12_COMPARISON_FUNC_ALWAYS,
            ..Default::default()
        };
        // Texture maps repeat (glTF's default wrap mode) and are filtered anisotropically; the
        // shader limits the ratio to Settings::anisotropy through the gradients it passes.
        let samplers = [
            gui_sampler,
            D3D12_STATIC_SAMPLER_DESC {
                Filter: D3D12_FILTER_ANISOTROPIC,
                MaxAnisotropy: 16,
                AddressU: D3D12_TEXTURE_ADDRESS_MODE_WRAP,
                AddressV: D3D12_TEXTURE_ADDRESS_MODE_WRAP,
                AddressW: D3D12_TEXTURE_ADDRESS_MODE_WRAP,
                ShaderRegister: 1,
                ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
                ..gui_sampler
            },
        ];
        let rd = D3D12_ROOT_SIGNATURE_DESC {
            NumParameters: params.len() as _,
            pParameters: params.as_ptr(),
            NumStaticSamplers: samplers.len() as _,
            pStaticSamplers: samplers.as_ptr(),
            Flags: D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT,
        };
        let mut blob = None;
        let mut errors = None;
        D3D12SerializeRootSignature(
            &rd,
            D3D_ROOT_SIGNATURE_VERSION_1,
            &mut blob,
            Some(&mut errors),
        )?;
        let blob = blob.unwrap();
        let root: ID3D12RootSignature = device.CreateRootSignature(
            0,
            std::slice::from_raw_parts(blob.GetBufferPointer() as _, blob.GetBufferSize()),
        )?;
        crate::startup("device");
        // NGX takes over a second to start; it needs only the device, so it starts while the
        // pipelines are made (the thread holds its own reference to the device).
        let ngx_thread = {
            let device = Threaded(device.clone());
            std::thread::spawn(move || {
                let device = device;
                let mut ngx = null_mut();
                let code = unsafe { caustica_ngx_init(device.0.as_raw(), &mut ngx) };
                Threaded((ngx, code))
            })
        };
        // The driver compiles each pipeline to GPU code (seconds on a cold shader cache), so
        // they are created on all cores at once; the device is free-threaded.
        let [
            trace,
            trace_maps,
            trace_lean,
            trace_maps_lean,
            trace_spectral,
            trace_spectral_lean,
            trace_paths,
            trace_lpe,
            glare,
            restir_presample,
            restir_initial,
            restir_spatial,
            bdpt,
            bdpt_resolve,
            photon_clear,
            photon_trace,
            photon_trace_spectral,
            photon_gather_spectral,
            bdpt_spectral,
            photon_gather,
            filter,
            display,
        ] = create_compute_pipelines(
            &device,
            &root,
            [
                include_bytes!(concat!(env!("OUT_DIR"), "/trace.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/trace_maps.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/trace_lean.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/trace_maps_lean.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/trace_spectral.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/trace_spectral_lean.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/trace_paths.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/trace_lpe.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/glare.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/restir_presample.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/restir_initial.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/restir_spatial.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/bdpt.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/bdpt_resolve.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/photon_clear.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/photon_trace.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/photon_trace_spectral.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/photon_gather_spectral.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/bdpt_spectral.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/photon_gather.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/filter.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/display.dxil")),
            ],
        )?;
        let ser_pipelines = make_ser(
            &device,
            &root,
            ser,
            [
                include_bytes!(concat!(env!("OUT_DIR"), "/trace_ser.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/trace_maps_ser.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/trace_lean_ser.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/trace_maps_lean_ser.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/trace_lpe_ser.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/trace_spectral_ser.dxil")),
                include_bytes!(concat!(env!("OUT_DIR"), "/trace_spectral_lean_ser.dxil")),
            ],
        );
        let path_size = (PATH_CAPACITY * size_of::<crate::paths::Record>()) as u64;
        let path_buffer = buffer(
            &device,
            path_size,
            D3D12_HEAP_TYPE_DEFAULT,
            D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
            // Buffers start in COMMON and are promoted to UAV by the first dispatch.
            D3D12_RESOURCE_STATE_COMMON,
        )?;
        let path_readback = buffer(
            &device,
            path_size,
            D3D12_HEAP_TYPE_READBACK,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_COPY_DEST,
        )?;
        let lpe_table = buffer(
            &device,
            ((crate::lpe::SLOTS * crate::lpe::STATES * crate::lpe::SYMBOLS + crate::lpe::MATERIALS)
                * 4) as u64,
            D3D12_HEAP_TYPE_UPLOAD,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_GENERIC_READ,
        )?;
        let glare_buffer = buffer(
            &device,
            2 * 3 * GLARE_N * GLARE_N * 8,
            D3D12_HEAP_TYPE_DEFAULT,
            D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_COMMON,
        )?;
        let lens_buffer = buffer(
            &device,
            (crate::lens::FLOAT4S * 16) as u64,
            D3D12_HEAP_TYPE_UPLOAD,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_GENERIC_READ,
        )?;
        let (vs, ps): (&[u8], &[u8]) = (
            include_bytes!(concat!(env!("OUT_DIR"), "/vs_main.dxil")),
            include_bytes!(concat!(env!("OUT_DIR"), "/ps_main.dxil")),
        );
        let gui = create_gui_pipeline(&device, &root, DXGI_FORMAT_R8G8B8A8_UNORM, vs, ps)?;
        let gui_hdr = create_gui_pipeline(&device, &root, DXGI_FORMAT_R16G16B16A16_FLOAT, vs, ps)?;
        let gui_hdr10 = create_gui_pipeline(&device, &root, DXGI_FORMAT_R10G10B10A2_UNORM, vs, ps)?;
        crate::startup("pipelines");
        let lut = texture_3d(&device, 2)?;
        let env = env_texture(&device, 1, 1, DXGI_FORMAT_R32G32B32A32_FLOAT)?;
        let env_cdf = env_texture(&device, 1, 2, DXGI_FORMAT_R32_FLOAT)?;
        let constants = buffer(
            &device,
            CB_STRIDE * CB_SLOTS,
            D3D12_HEAP_TYPE_UPLOAD,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_GENERIC_READ,
        )?;
        let geometry = buffer(
            &device,
            256,
            D3D12_HEAP_TYPE_UPLOAD,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_GENERIC_READ,
        )?;
        let mut query = None;
        device.CreateQueryHeap(
            &D3D12_QUERY_HEAP_DESC {
                Type: D3D12_QUERY_HEAP_TYPE_TIMESTAMP,
                Count: TIMESTAMPS,
                NodeMask: 0,
            },
            &mut query,
        )?;
        let query = query.unwrap();
        let timing = buffer(
            &device,
            8 * TIMESTAMPS as u64,
            D3D12_HEAP_TYPE_READBACK,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_COPY_DEST,
        )?;
        let (ngx, code) = match ngx_thread.join() {
            Ok(t) => t.0,
            Err(e) => std::panic::resume_unwind(e),
        };
        let rr_status = if ngx_ok(code) {
            "DLSS Ray Reconstruction available".into()
        } else {
            format!("NGX unavailable (0x{code:08x}); using native denoiser")
        };
        let mut fsr = null_mut();
        let fsr_code = caustica_fsr_init(device.as_raw(), &mut fsr);
        let mut xess = null_mut();
        let xess_code = caustica_xess_init(device.as_raw(), &mut xess);
        let init_reasons = {
            let nvidia = adapter_name.to_ascii_lowercase().contains("nvidia");
            let caps = if ngx.is_null() {
                0
            } else {
                caustica_ngx_caps(ngx)
            };
            let ngx_reason = |bit: u32, what: &str| {
                if !ngx_ok(code) && !nvidia {
                    format!("needs an NVIDIA RTX GPU (this GPU: {adapter_name})")
                } else if !ngx_ok(code) {
                    format!(
                        "NGX did not start (0x{code:08x}); update the driver, and check that nvngx_dlss*.dll is next to the exe"
                    )
                } else if caps & bit == 0 {
                    format!("this GPU or driver does not offer {what}")
                } else {
                    String::new()
                }
            };
            [
                ngx_reason(1, "Ray Reconstruction"),
                ngx_reason(2, "DLSS Super Resolution"),
                match fsr_code {
                    _ if !fsr.is_null() => String::new(),
                    1 => "amd_fidelityfx_loader_dx12.dll is missing next to the exe".into(),
                    c => format!("the FidelityFX loader did not start ({c})"),
                },
                match xess_code {
                    _ if !xess.is_null() => String::new(),
                    1 => "libxess.dll is missing next to the exe".into(),
                    2 => "libxess.dll lacks a needed function (an old version?)".into(),
                    c => format!("XeSS did not start (result {})", c & 0xff),
                },
            ]
        };
        eprintln!(
            "GPU: {adapter_name}; {rr_status}; DLSS SR {}; FSR {}; XeSS {}",
            if !ngx.is_null() && caustica_ngx_caps(ngx) & 2 != 0 {
                "available".to_string()
            } else {
                "unavailable".into()
            },
            if fsr.is_null() {
                format!("unavailable ({fsr_code})")
            } else {
                "available".into()
            },
            if xess.is_null() {
                format!("unavailable (0x{xess_code:x})")
            } else {
                "available".into()
            }
        );
        let mut gpu = Self {
            device,
            queue,
            swap,
            allocator,
            list,
            fence,
            event,
            fence_value: 0,
            heap,
            rtv,
            root,
            trace,
            trace_maps,
            trace_lean,
            trace_maps_lean,
            trace_spectral,
            trace_spectral_lean,
            ser: ser_pipelines,
            trace_paths,
            path_buffer,
            path_readback,
            trace_lpe,
            lpe_table,
            lpe_uploaded: None,
            lens_buffer,
            lens_params: None,
            lens_surfaces: 0,
            glare,
            glare_buffer,
            glare_kernel: None,
            lpe_aov: None,
            lpe_aov_len: 0,
            restir_presample,
            restir_initial,
            restir_spatial,
            reservoirs: None,
            reservoirs_len: 0,
            restir_history: false,
            restir_layers: 0,
            restir_ran: false,
            bdpt,
            bdpt_resolve,
            photon_clear,
            photon_trace,
            photon_trace_spectral,
            photon_gather_spectral,
            bdpt_spectral,
            photon_gather,
            aux: None,
            aux_len: 0,
            splat_offset: 0,
            photon_frames: 0,
            photon_scale: 1.,
            bounds: [0., 0., 0., 2.],
            bounds_valid: false,
            textured: false,
            lean_scene: false,
            filter,
            display,
            gui,
            gui_hdr,
            gui_hdr10,
            lut,
            lut_size: 0,
            env,
            env_cdf,
            env_inv_norm: 0.,
            env_sun: None,
            env_scale: 1.,
            present_count: 0,
            readback: None,
            scope_zero: None,
            color_space_ok: true,
            buffers: vec![],
            textures: vec![],
            constants,
            material_buffer: geometry.clone(),
            emitter_buffer: geometry.clone(),
            emitter_lookup: geometry.clone(),
            mesh_vertices: geometry.clone(),
            vertex_ranges: Default::default(),
            vertex_used: 0,
            vertex_capacity: 0,
            static_vertices: geometry.clone(),
            instance_table: geometry,
            static_cache: vec![],
            static_groups: vec![],
            sdf_emitters: [0; 2],
            sdf_blas: None,
            meshes: Default::default(),
            instances: vec![],
            tlas: None,
            tlas_cache: None,
            compact_queue: vec![],
            gui_textures: Default::default(),
            free_slots: (FIRST_GUI_SLOT..HEAP_SIZE).rev().collect(),
            maps: Default::default(),
            free_maps: (0..MAX_MAPS as u32).rev().collect(),
            ui_frame: None,
            ngx,
            fsr,
            xess,
            sr_created: None,
            sr_failed: vec![],
            sr_reset: true,
            sr_fresh: 0,
            rr_created: false,
            rr_failed: false,
            init_reasons,
            previous_vp: Mat4::IDENTITY,
            rr_reset: true,
            restarts: 0,
            sequence_base: 0,
            history_valid: false,
            integrated_valid: false,
            camera_moved: false,
            query,
            timing,
            st: GpuState {
                path_request: None,
                path_result: None,
                lpe_slots: 0,
                battle: [BattleQuadrant::default(); 4],
                hdr_active: false,
                hdr10: false,
                hdr10_refused: false,
                hdr_levels: [200., 1000.],
                painting: false,
                zebras: false,
                scopes_on: false,
                scopes: vec![],
                probe: None,
                probe_result: None,
                triangle_count: 0,
                emitter_count: 0,
                sun_count: 0,
                keep_ui_frame: false,
                rr_status,
                upscale_status: String::new(),
                rr_active: false,
                width,
                height,
                swap_size: [width, height],
                rw: 0,
                rh: 0,
                samples: 0,
                frame: 0,
                gpu_ms: 0.,
                fence_wait_ms: 0.,
                pass_ms: [0.; 4],
                adapter_name,
                denoiser_activity: "",
                reference_valid: false,
            },
        };
        gpu.resize(width, height, s)?;
        gpu.rebuild_scene(s)?;
        Ok(gpu)
    }
}
