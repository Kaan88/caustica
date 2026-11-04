//! A/B references, captures and readbacks, D3D12 debug messages.
use super::*;

impl Gpu {
    /// Puts a linear RGBA image of the output size (the CPU reference) where the A/B compare
    /// reads its reference.
    pub unsafe fn upload_reference(&mut self, rgba: &[f32]) -> Result<()> {
        anyhow::ensure!(
            rgba.len() == self.width as usize * self.height as usize * 4,
            "reference image size"
        );
        let t = Texture {
            resource: self.textures[13].resource.clone(),
            format: self.textures[13].format,
        };
        self.upload_texture(&t, bytemuck::cast_slice(rgba), self.width as usize * 16)?;
        self.reference_valid = true;
        Ok(())
    }
    pub unsafe fn capture_reference(&mut self) -> Result<()> {
        self.begin()?;
        copy_texture(
            &self.list,
            &self.textures[18].resource,
            &self.textures[13].resource,
        );
        self.submit()?;
        self.reference_valid = true;
        Ok(())
    }
    pub unsafe fn capture(
        &mut self,
        path: &std::path::Path,
        hdr: bool,
        include_ui: bool,
    ) -> Result<()> {
        // `name.layers.exr`: every AOV as a layer (`--capture`, scripts).
        if !include_ui
            && path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().to_lowercase().ends_with(".layers.exr"))
        {
            let layers = self.read_layers()?;
            return crate::video::write_layers(path, &layers);
        }
        let index = if hdr { 18 } else { 10 };
        let resource = if include_ui {
            self.ui_frame
                .clone()
                .context("UI capture needs keep_ui_frame set before rendering")?
        } else {
            self.textures[index].resource.clone()
        };
        let state = if include_ui {
            D3D12_RESOURCE_STATE_COPY_DEST
        } else {
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS
        };
        let (layout, bytes) = footprint(&self.device, &resource);
        let staging = buffer(
            &self.device,
            bytes,
            D3D12_HEAP_TYPE_READBACK,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_COPY_DEST,
        )?;
        self.begin()?;
        transition(
            &self.list,
            &resource,
            state,
            D3D12_RESOURCE_STATE_COPY_SOURCE,
        );
        let mut src = location(&resource);
        let mut dst = D3D12_TEXTURE_COPY_LOCATION {
            pResource: ManuallyDrop::new(Some(staging.clone())),
            Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                PlacedFootprint: layout,
            },
        };
        self.list.CopyTextureRegion(&dst, 0, 0, 0, &src, None);
        ManuallyDrop::drop(&mut src.pResource);
        ManuallyDrop::drop(&mut dst.pResource);
        transition(
            &self.list,
            &resource,
            D3D12_RESOURCE_STATE_COPY_SOURCE,
            state,
        );
        self.submit()?;
        let mut p = null_mut();
        staging.Map(0, None, Some(&mut p))?;
        // A UI capture in HDR mode copies the RGBA16F scRGB backbuffer (HDR10: R10G10B10A2 PQ).
        let ui_hdr = include_ui && self.hdr_active;
        let ui_hdr10 = ui_hdr && self.hdr10;
        // The UI frame has the swapchain's size, the output textures the output's.
        let (width, height) = if include_ui {
            (self.swap_size[0], self.swap_size[1])
        } else {
            (self.width, self.height)
        };
        let row_size = width as usize
            * if hdr {
                16
            } else if ui_hdr && !ui_hdr10 {
                8
            } else {
                4
            };
        let mut packed = Vec::with_capacity(row_size * height as usize);
        for y in 0..height as usize {
            packed.extend_from_slice(std::slice::from_raw_parts(
                (p as *const u8).add(y * layout.Footprint.RowPitch as usize),
                row_size,
            ));
        }
        staging.Unmap(0, None);
        if ui_hdr {
            // Back to SDR: paper white becomes 1.0, then sRGB-encode.
            let scale = 80. / self.hdr_levels[0];
            let scrgb: Vec<f32> = if ui_hdr10 {
                packed
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .flat_map(|b| crate::display::hdr10_to_scrgb(u32::from_le_bytes(*b)))
                    .collect()
            } else {
                packed
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| half_to_f32(u16::from_le_bytes(*b)))
                    .collect()
            };
            packed = scrgb
                .iter()
                .enumerate()
                .map(|(i, &v)| {
                    if i % 4 == 3 {
                        (v.clamp(0., 1.) * 255.).round() as u8
                    } else {
                        let v = (v * scale).clamp(0., 1.);
                        let e = if v <= 0.0031308 {
                            v * 12.92
                        } else {
                            1.055 * v.powf(1. / 2.4) - 0.055
                        };
                        (e * 255.).round() as u8
                    }
                })
                .collect();
        }
        if hdr {
            let floats: Vec<f32> = packed
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect();
            let rgb: Vec<f32> = floats
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|c| [c[0], c[1], c[2]])
                .collect();
            image::Rgb32FImage::from_raw(width, height, rgb)
                .unwrap()
                .save(path)?;
        } else {
            image::save_buffer(path, &packed, width, height, image::ColorType::Rgba8)?;
        }
        Ok(())
    }
    /// Reads an output-resolution float texture (18 linear RGBA32F, 26 scRGB RGBA16F) as RGBA f32.
    /// Reads the RGBA8 output (texture 10) as tightly packed rows.
    pub unsafe fn read_rgba8(&mut self, index: usize) -> Result<Vec<u8>> {
        ensure!(
            self.textures[index].format == DXGI_FORMAT_R8G8B8A8_UNORM,
            "Texture {index} is not RGBA8"
        );
        let resource = self.textures[index].resource.clone();
        let (layout, bytes) = footprint(&self.device, &resource);
        let staging = buffer(
            &self.device,
            bytes,
            D3D12_HEAP_TYPE_READBACK,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_COPY_DEST,
        )?;
        self.begin()?;
        transition(
            &self.list,
            &resource,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_COPY_SOURCE,
        );
        let mut src = location(&resource);
        let mut dst = D3D12_TEXTURE_COPY_LOCATION {
            pResource: ManuallyDrop::new(Some(staging.clone())),
            Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                PlacedFootprint: layout,
            },
        };
        self.list.CopyTextureRegion(&dst, 0, 0, 0, &src, None);
        ManuallyDrop::drop(&mut src.pResource);
        ManuallyDrop::drop(&mut dst.pResource);
        transition(
            &self.list,
            &resource,
            D3D12_RESOURCE_STATE_COPY_SOURCE,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
        );
        self.submit()?;
        let mut p = null_mut();
        staging.Map(0, None, Some(&mut p))?;
        let row = self.width as usize * 4;
        let mut out = Vec::with_capacity(row * self.height as usize);
        for y in 0..self.height as usize {
            out.extend_from_slice(std::slice::from_raw_parts(
                (p as *const u8).add(y * layout.Footprint.RowPitch as usize),
                row,
            ));
        }
        staging.Unmap(0, None);
        Ok(out)
    }
    /// Any float texture at its own size: width, height, channels per texel (R32F 1, RG32F 2,
    /// RGBA32F or RGBA16F 4) and the samples, rows top first.
    pub unsafe fn read_texture(&mut self, index: usize) -> Result<(u32, u32, usize, Vec<f32>)> {
        let resource = self.textures[index].resource.clone();
        let format = self.textures[index].format;
        let (channels, bytes_per) = match format {
            DXGI_FORMAT_R32_FLOAT => (1, 4),
            DXGI_FORMAT_R32G32_FLOAT => (2, 4),
            DXGI_FORMAT_R32G32B32A32_FLOAT => (4, 4),
            DXGI_FORMAT_R16G16B16A16_FLOAT => (4, 2),
            _ => anyhow::bail!("Texture {index} is not a float texture"),
        };
        let desc = resource.GetDesc();
        let (width, height) = (desc.Width as usize, desc.Height as usize);
        let (layout, bytes) = footprint(&self.device, &resource);
        let staging = buffer(
            &self.device,
            bytes,
            D3D12_HEAP_TYPE_READBACK,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_COPY_DEST,
        )?;
        self.begin()?;
        transition(
            &self.list,
            &resource,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_COPY_SOURCE,
        );
        let mut src = location(&resource);
        let mut dst = D3D12_TEXTURE_COPY_LOCATION {
            pResource: ManuallyDrop::new(Some(staging.clone())),
            Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                PlacedFootprint: layout,
            },
        };
        self.list.CopyTextureRegion(&dst, 0, 0, 0, &src, None);
        ManuallyDrop::drop(&mut src.pResource);
        ManuallyDrop::drop(&mut dst.pResource);
        transition(
            &self.list,
            &resource,
            D3D12_RESOURCE_STATE_COPY_SOURCE,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
        );
        self.submit()?;
        let mut p = null_mut();
        staging.Map(0, None, Some(&mut p))?;
        let mut out = Vec::with_capacity(width * height * channels);
        for y in 0..height {
            let row = (p as *const u8).add(y * layout.Footprint.RowPitch as usize);
            for i in 0..width * channels {
                out.push(if bytes_per == 2 {
                    half_to_f32((row.add(i * 2) as *const u16).read_unaligned())
                } else {
                    (row.add(i * 4) as *const f32).read_unaligned()
                });
            }
        }
        staging.Unmap(0, None);
        Ok((width as u32, height as u32, channels, out))
    }
    /// Every AOV for a multi-layer EXR (wave 3): the beauty (linear, output size) and, at render
    /// size, the denoiser guides (diffuse and specular albedo, normal, roughness, linear depth,
    /// motion), the direct and specular light AOVs, the object ids, and each light path
    /// expression in use.
    pub unsafe fn read_layers(&mut self) -> Result<Vec<AovLayer>> {
        let mut layers = vec![];
        let mut add = |name: &str,
                       (w, h, n, data): (u32, u32, usize, Vec<f32>),
                       pick: &[(usize, &'static str)]| {
            let data = data
                .chunks_exact(n)
                .flat_map(|t| pick.iter().map(|(c, _)| t[*c]).collect::<Vec<_>>())
                .collect();
            layers.push(AovLayer {
                name: name.into(),
                width: w,
                height: h,
                channels: pick.iter().map(|(_, c)| *c).collect(),
                data,
            });
        };
        const RGB: [(usize, &str); 3] = [(0, "R"), (1, "G"), (2, "B")];
        add(
            "beauty",
            self.read_texture(18)?,
            &[(0, "R"), (1, "G"), (2, "B"), (3, "A")],
        );
        add("diffuse_albedo", self.read_texture(3)?, &RGB);
        add("specular_albedo", self.read_texture(4)?, &RGB);
        let normal = self.read_texture(2)?;
        add("normal", normal.clone(), &[(0, "X"), (1, "Y"), (2, "Z")]);
        add("roughness", normal, &[(3, "Y")]);
        add("depth", self.read_texture(5)?, &[(0, "Z")]);
        add("motion", self.read_texture(6)?, &[(0, "X"), (1, "Y")]);
        add("direct", self.read_texture(11)?, &RGB);
        add("specular", self.read_texture(12)?, &RGB);
        add("object_id", self.read_texture(19)?, &[(0, "Y")]);
        let (w, h) = (layers[1].width, layers[1].height);
        for slot in 0..self.lpe_slots {
            let rgb = self.read_lpe(slot)?;
            if rgb.len() == (w * h * 3) as usize {
                layers.push(AovLayer {
                    name: lpe_layer_name(
                        slot,
                        self.lpe_uploaded
                            .as_ref()
                            .and_then(|(e, _)| e.get(slot as usize)),
                    ),
                    width: w,
                    height: h,
                    channels: vec!["R", "G", "B"],
                    data: rgb,
                });
            }
        }
        Ok(layers)
    }
    pub unsafe fn read_floats(&mut self, index: usize) -> Result<Vec<f32>> {
        let resource = self.textures[index].resource.clone();
        let half = self.textures[index].format == DXGI_FORMAT_R16G16B16A16_FLOAT;
        let packed10 = self.textures[index].format == DXGI_FORMAT_R10G10B10A2_UNORM;
        let (layout, bytes) = footprint(&self.device, &resource);
        let staging = buffer(
            &self.device,
            bytes,
            D3D12_HEAP_TYPE_READBACK,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_COPY_DEST,
        )?;
        self.begin()?;
        transition(
            &self.list,
            &resource,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_COPY_SOURCE,
        );
        let mut src = location(&resource);
        let mut dst = D3D12_TEXTURE_COPY_LOCATION {
            pResource: ManuallyDrop::new(Some(staging.clone())),
            Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                PlacedFootprint: layout,
            },
        };
        self.list.CopyTextureRegion(&dst, 0, 0, 0, &src, None);
        ManuallyDrop::drop(&mut src.pResource);
        ManuallyDrop::drop(&mut dst.pResource);
        transition(
            &self.list,
            &resource,
            D3D12_RESOURCE_STATE_COPY_SOURCE,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
        );
        self.submit()?;
        let mut p = null_mut();
        staging.Map(0, None, Some(&mut p))?;
        let mut out = Vec::with_capacity(self.width as usize * self.height as usize * 4);
        for y in 0..self.height as usize {
            let row = (p as *const u8).add(y * layout.Footprint.RowPitch as usize);
            if packed10 {
                for x in 0..self.width as usize {
                    let word = (row.add(x * 4) as *const u32).read_unaligned();
                    out.extend_from_slice(&crate::display::hdr10_to_scrgb(word));
                }
                continue;
            }
            for i in 0..self.width as usize * 4 {
                out.push(if half {
                    half_to_f32((row.add(i * 2) as *const u16).read_unaligned())
                } else {
                    (row.add(i * 4) as *const f32).read_unaligned()
                });
            }
        }
        staging.Unmap(0, None);
        Ok(out)
    }
    pub unsafe fn debug_messages(&self) -> Vec<String> {
        let mut result = vec![];
        if let Ok(q) = self.device.cast::<ID3D12InfoQueue>() {
            for i in 0..q.GetNumStoredMessagesAllowedByRetrievalFilter() {
                let mut len = 0;
                let _ = q.GetMessage(i, None, &mut len);
                let mut bytes = vec![0u64; len.div_ceil(8)];
                let p = bytes.as_mut_ptr() as *mut D3D12_MESSAGE;
                if q.GetMessage(i, Some(p), &mut len).is_ok() {
                    let msg = &*p;
                    let text: String = String::from_utf8_lossy(std::slice::from_raw_parts(
                        msg.pDescription,
                        msg.DescriptionByteLength.saturating_sub(1),
                    ))
                    .into();
                    // The Agility SDK's layer (track 20) flags every CheckFeatureSupport as
                    // superseded by ID3D12Device16::CheckFeatureSupport1, which the windows
                    // crate doesn't bind; the old call still answers correctly.
                    // It also notes every buffer created in a state other than COMMON, which
                    // the upscaler SDKs (DLSS, FSR, XeSS) do for theirs; the runtime uses COMMON.
                    let superseded = text
                        .contains("superseded by ID3D12Device16::CheckFeatureSupport1")
                        || (text.contains("Ignoring InitialState")
                            && text.contains("Buffers are effectively created in state"));
                    if msg.Severity.0 <= D3D12_MESSAGE_SEVERITY_WARNING.0 && !superseded {
                        result.push(text);
                    }
                }
            }
            q.ClearStoredMessages();
        }
        result
    }
}
/// An LPE layer's EXR name: `lpe<slot>_` and the expression with everything but letters and
/// digits as `_` (a dot would nest layers), at most 48 characters.
fn lpe_layer_name(slot: u32, expr: Option<&String>) -> String {
    let mut name = format!("lpe{}", slot + 1);
    if let Some(e) = expr {
        let mut last = '_';
        let mut tail = String::new();
        for c in e.chars() {
            let c = if c.is_ascii_alphanumeric() { c } else { '_' };
            if c != '_' || last != '_' {
                tail.push(c);
            }
            last = c;
        }
        let tail = tail.trim_matches('_');
        if !tail.is_empty() {
            name = format!("{name}_{tail}");
        }
    }
    name.chars().take(48).collect()
}
#[cfg(test)]
mod tests {
    #[test]
    fn lpe_layers_are_named_by_their_expression() {
        let e = "C D'red wall' .* L".to_string();
        assert_eq!(super::lpe_layer_name(0, Some(&e)), "lpe1_C_D_red_wall_L");
        assert_eq!(super::lpe_layer_name(2, None), "lpe3");
    }
}
