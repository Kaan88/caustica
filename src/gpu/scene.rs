//! Scene upload: vertices, BLAS / TLAS, opacity micromaps, materials.
use super::*;

impl Gpu {
    pub unsafe fn rebuild_scene(&mut self, s: &Settings) -> Result<()> {
        let packed = crate::lighting::materials(s);
        let needs_any_hit = |m: u32| {
            let m = &packed[m as usize];
            (m.lobes[3] as u32 & 2) != 0 || m.lobes[2] > 0.
        };
        self.begin()?;
        let mut keep: Vec<ID3D12Resource> = vec![];
        // Static geometry (floor, room, lights) in world space, rebuilt only when it changes.
        // Its triangles are grouped by `instance_mask` like objects (track 20a (a)): lights
        // hidden from the camera and clear glass become opaque instances with their mask bit,
        // so only triangles under both rules still run the shader per candidate. Each group is
        // its own BLAS and instance; the instance record's vertex_base is the group's first
        // triangle in `static_vertices`.
        let soup = crate::geometry::static_geometry(s);
        // Measured by photon mapping when it next runs: it walks every object's vertices, which
        // made each frame of a timeline export (a rebuild per frame) 10 ms slower.
        self.bounds_valid = false;
        type Group<'a> = ((u32, bool), Vec<&'a [Vertex; 3]>);
        let mut groups: Vec<Group> = vec![];
        for t in soup.as_chunks::<3>().0 {
            let key = instance_mask(&packed[t[0].material as usize]);
            match groups.iter_mut().find(|g| g.0 == key) {
                Some(g) => g.1.push(t),
                None => groups.push((key, vec![t])),
            }
        }
        // Opaque 0xff first, so plain scenes keep one static instance at InstanceID 0.
        groups.sort_by_key(|g| g.0 != (0xff, false));
        let layout: Vec<StaticGroup> = {
            let mut first = 0;
            groups
                .iter()
                .map(|g| {
                    let group = StaticGroup {
                        mask: g.0.0,
                        any_hit: g.0.1,
                        first,
                        count: g.1.len() as u32,
                        blas: None,
                    };
                    first += group.count;
                    group
                })
                .collect()
        };
        let statics: Vec<Vertex> = groups
            .into_iter()
            .flat_map(|g| g.1)
            .flatten()
            .copied()
            .collect();
        let same_layout = layout.len() == self.static_groups.len()
            && layout.iter().zip(&self.static_groups).all(|(a, b)| {
                (a.mask, a.any_hit, a.first, a.count) == (b.mask, b.any_hit, b.first, b.count)
            });
        if self.static_groups.is_empty()
            || !same_layout
            || bytemuck::cast_slice::<Vertex, u8>(&statics)
                != bytemuck::cast_slice::<Vertex, u8>(&self.static_cache)
        {
            // An empty room still needs a vertex buffer to bind.
            let bytes: &[u8] = if statics.is_empty() {
                &[0; size_of::<Vertex>()]
            } else {
                bytemuck::cast_slice(&statics)
            };
            self.static_vertices = self.gpu_buffer(bytes, &mut keep)?;
            let base = self.static_vertices.GetGPUVirtualAddress();
            let mut layout = layout;
            for g in &mut layout {
                let flags = if g.any_hit {
                    D3D12_RAYTRACING_GEOMETRY_FLAG_NONE
                } else {
                    D3D12_RAYTRACING_GEOMETRY_FLAG_OPAQUE
                };
                let address = base + (g.first as usize * 3 * size_of::<Vertex>()) as u64;
                g.blas = Some(self.build_blas(
                    &[triangles(address, g.count as usize, flags)],
                    &mut keep,
                    true,
                )?);
            }
            self.static_groups = layout;
            self.static_cache = statics;
        }
        // One BLAS per distinct mesh in object space (core track 4). They are rebuilt only when
        // the set of meshes changes; the shared vertex buffer then holds every mesh in use.
        // Track 20a (a): an alpha-tested object whose material needs no other per-candidate
        // test gets a BLAS with opacity micromaps, keyed by mesh, base map and cutoff.
        let omm_of = |i: usize, o: &crate::scene::Object, key: &str| {
            if !omm_on() || !o.material.texture.is_empty() {
                return None;
            }
            let maps = crate::mesh::maps(o)?;
            let base = maps.base.filter(|_| maps.cutoff > 0.)?;
            (instance_mask(&packed[5 + i]) == (0xff, false)).then(|| {
                let omm_key = format!(
                    "{key}#omm{:x}/{}",
                    std::sync::Arc::as_ptr(&base) as usize,
                    maps.cutoff.to_bits()
                );
                (omm_key, base, maps.cutoff)
            })
        };
        let visible: Vec<VisibleMesh> = s
            .objects
            .iter()
            .enumerate()
            .filter(|(_, o)| o.visible && o.sdf.is_none())
            .map(|(i, o)| {
                let (key, source) = crate::geometry::object_source(o);
                let omm = omm_of(i, o, &key);
                VisibleMesh {
                    index: i,
                    blas_key: omm.as_ref().map_or_else(|| key.clone(), |m| m.0.clone()),
                    key,
                    source,
                    omm: omm.map(|m| (m.1, m.2)),
                }
            })
            .collect();
        let mut wanted: Vec<&str> = visible.iter().map(|v| v.blas_key.as_str()).collect();
        wanted.sort_unstable();
        wanted.dedup();
        let mut have: Vec<&str> = self.meshes.keys().map(String::as_str).collect();
        have.sort_unstable();
        if wanted != have {
            self.upload_mesh_vertices(s, &visible, &mut keep)?;
            let base = self.mesh_vertices.GetGPUVirtualAddress();
            let mut meshes = std::collections::HashMap::new();
            for v in &visible {
                if meshes.contains_key(&v.blas_key) {
                    continue;
                }
                let (first, count) = self.vertex_ranges[&v.key];
                let address = base + first as u64 * size_of::<Vertex>() as u64;
                // A BLAS holds its own copy of the triangles, so ones already built are reused.
                let (blas, omm) = match self.meshes.remove(&v.blas_key) {
                    Some(old) => (old.blas, old._omm),
                    None => match &v.omm {
                        Some((map, cutoff)) => {
                            let uvs = mesh_uvs(&s.objects[v.index], v.source.len());
                            self.omm_blas(address, count, &uvs, map, *cutoff, &mut keep)?
                        }
                        None => (
                            self.build_blas(
                                &[triangles(
                                    address,
                                    count,
                                    D3D12_RAYTRACING_GEOMETRY_FLAG_NONE,
                                )],
                                &mut keep,
                                // A simulating cloth gets a new mesh each step: no compaction submit.
                                !v.blas_key.contains("cloth:"),
                            )?,
                            None,
                        ),
                    },
                };
                meshes.insert(
                    v.blas_key.clone(),
                    MeshBlas {
                        blas,
                        first,
                        count,
                        _omm: omm,
                        _map: v.omm.as_ref().map(|(m, _)| m.clone()),
                    },
                );
            }
            self.meshes = meshes;
        }
        self.compact_blases(&mut keep)?;
        // Instances: the static BLAS (InstanceID 0), then one per visible object. Flags decide
        // opacity per object: glass runs the any-hit loop so shadow rays may pass it.
        let cull = D3D12_RAYTRACING_INSTANCE_FLAG_TRIANGLE_CULL_DISABLE.0 as u32;
        let mut descs: Vec<u8> = vec![];
        let mut table: Vec<InstanceRecord> = vec![];
        let identity = [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0.];
        let mut push =
            |transform: &[f32; 12], id: u32, flags: u32, blas: &ID3D12Resource, mask: u32| {
                descs.extend_from_slice(bytemuck::cast_slice(transform));
                descs.extend_from_slice(&(id | mask << 24).to_le_bytes());
                descs.extend_from_slice(&(flags << 24).to_le_bytes());
                descs.extend_from_slice(&blas.GetGPUVirtualAddress().to_le_bytes());
            };
        // Static instances first: emitter_base 0, so emitter_lookup takes the static triangle
        // index (vertex_base + primitive) directly.
        for g in &self.static_groups {
            let opacity = if g.any_hit {
                D3D12_RAYTRACING_INSTANCE_FLAG_FORCE_NON_OPAQUE.0
            } else {
                D3D12_RAYTRACING_INSTANCE_FLAG_FORCE_OPAQUE.0
            } as u32;
            push(
                &identity,
                table.len() as u32,
                cull | opacity,
                g.blas.as_ref().unwrap(),
                g.mask,
            );
            table.push(InstanceRecord {
                vertex_base: g.first,
                material: u32::MAX,
                emitter_base: 0,
                object: u32::MAX,
            });
        }
        let mut triangle_count = self.static_cache.len() / 3;
        for VisibleMesh {
            index: i,
            blas_key,
            omm,
            ..
        } in &visible
        {
            let mesh = &self.meshes[blas_key];
            let material = 5 + *i as u32;
            // Masked maps (alpha cutoff) are tested in the any-hit loop as well.
            let o = &s.objects[*i];
            let masked = o.material.texture.is_empty()
                && crate::mesh::maps(o).is_some_and(|m| m.cutoff != 0.);
            // Track 20a (a): an object's two skip rules (hidden from camera rays, glass that
            // shadow rays pass) become instance masks, which the rays leave out (pathtrace.hlsl
            // MASK_*), so the RT cores commit its hits without running the shader per candidate.
            // Only alpha-tested objects, and ones under both rules, stay non-opaque.
            let (mask, any_hit) = instance_mask(&packed[material as usize]);
            // With micromaps the geometry's own non-opaque flag stands, so the RT cores read
            // them (a FORCE flag would override them).
            let opacity = if omm.is_some() {
                0
            } else if any_hit || masked {
                D3D12_RAYTRACING_INSTANCE_FLAG_FORCE_NON_OPAQUE.0
            } else {
                D3D12_RAYTRACING_INSTANCE_FLAG_FORCE_OPAQUE.0
            } as u32;
            push(
                &crate::geometry::object_transform(&s.objects[*i]),
                table.len() as u32,
                cull | opacity,
                &mesh.blas,
                if masked { 0xff } else { mask },
            );
            table.push(InstanceRecord {
                vertex_base: mesh.first,
                material,
                emitter_base: u32::MAX,
                object: *i as u32,
            });
            triangle_count += mesh.count;
        }
        // Procedural objects (track 13): instances of the unit-cube AABB; the shader intersects
        // the surface (kind in vertex_base). Glass and hidden ones are non-opaque like meshes,
        // though the shader sees every procedural candidate either way.
        if s.objects.iter().any(|o| o.visible && !o.sdf.is_none()) {
            if self.sdf_blas.is_none() {
                let aabb = self.gpu_buffer(
                    bytemuck::cast_slice(&[-1f32, -1., -1., 1., 1., 1.]),
                    &mut keep,
                )?;
                let desc = D3D12_RAYTRACING_GEOMETRY_DESC {
                    Type: D3D12_RAYTRACING_GEOMETRY_TYPE_PROCEDURAL_PRIMITIVE_AABBS,
                    Flags: D3D12_RAYTRACING_GEOMETRY_FLAG_NONE,
                    Anonymous: D3D12_RAYTRACING_GEOMETRY_DESC_0 {
                        AABBs: D3D12_RAYTRACING_GEOMETRY_AABBS_DESC {
                            AABBCount: 1,
                            AABBs: D3D12_GPU_VIRTUAL_ADDRESS_AND_STRIDE {
                                StartAddress: aabb.GetGPUVirtualAddress(),
                                StrideInBytes: 24,
                            },
                        },
                    },
                };
                self.sdf_blas = Some(self.build_blas(&[desc], &mut keep, false)?);
                keep.push(aabb);
            }
            let blas = self.sdf_blas.clone().unwrap();
            for (i, o) in s.objects.iter().enumerate() {
                if !o.visible || o.sdf.is_none() {
                    continue;
                }
                let material = 5 + i as u32;
                let opacity = if needs_any_hit(material) {
                    D3D12_RAYTRACING_INSTANCE_FLAG_FORCE_NON_OPAQUE.0
                } else {
                    D3D12_RAYTRACING_INSTANCE_FLAG_FORCE_OPAQUE.0
                } as u32;
                push(
                    &crate::geometry::object_transform(o),
                    table.len() as u32,
                    cull | opacity,
                    &blas,
                    0xff,
                );
                table.push(InstanceRecord {
                    vertex_base: o.sdf.id(o.phase, o.detail),
                    material,
                    emitter_base: u32::MAX,
                    object: i as u32,
                });
            }
        }
        // TLAS refit (wave 3): when only instance transforms changed (a drag, an animation or
        // physics step: the same instances with the same ids, masks, flags and BLAS), the TLAS
        // is updated in place, reusing its buffers; a full build every 64 refits keeps the tree
        // from degrading as objects move far.
        let tails: Vec<u8> = descs
            .chunks(64)
            .flat_map(|d| d[48..64].iter().copied())
            .collect();
        let refit = self.tlas.is_some()
            && self
                .tlas_cache
                .as_ref()
                .is_some_and(|c| c.tails == tails && c.refits < 64);
        let flags = D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_PREFER_FAST_TRACE
            | D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_ALLOW_UPDATE
            | if refit {
                D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_PERFORM_UPDATE
            } else {
                D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_NONE
            };
        if refit {
            // One frame in flight: the GPU is idle here, so the descs can be rewritten.
            let cache = self.tlas_cache.as_mut().unwrap();
            write_buffer(&cache.instances, 0, &descs)?;
            cache.refits += 1;
        } else {
            self.tlas_cache = None;
        }
        let instances = match &self.tlas_cache {
            Some(c) => c.instances.clone(),
            None => upload(&self.device, &descs)?,
        };
        let ti = D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS {
            Type: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL,
            Flags: flags,
            NumDescs: table.len() as u32,
            DescsLayout: D3D12_ELEMENTS_LAYOUT_ARRAY,
            Anonymous: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS_0 {
                InstanceDescs: instances.GetGPUVirtualAddress(),
            },
        };
        let tlas = if refit {
            self.tlas.clone().unwrap()
        } else {
            let mut info = D3D12_RAYTRACING_ACCELERATION_STRUCTURE_PREBUILD_INFO::default();
            self.device
                .GetRaytracingAccelerationStructurePrebuildInfo(&ti, &mut info);
            let tlas = buffer(
                &self.device,
                info.ResultDataMaxSizeInBytes,
                D3D12_HEAP_TYPE_DEFAULT,
                D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
                D3D12_RESOURCE_STATE_RAYTRACING_ACCELERATION_STRUCTURE,
            )?;
            let scratch = buffer(
                &self.device,
                info.ScratchDataSizeInBytes
                    .max(info.UpdateScratchDataSizeInBytes)
                    .max(256),
                D3D12_HEAP_TYPE_DEFAULT,
                D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
                D3D12_RESOURCE_STATE_COMMON,
            )?;
            self.tlas_cache = Some(TlasCache {
                scratch,
                instances: instances.clone(),
                tails,
                refits: 0,
            });
            tlas
        };
        let scratch = self
            .tlas_cache
            .as_ref()
            .unwrap()
            .scratch
            .GetGPUVirtualAddress();
        // Each BLAS build above ends with a UAV barrier.
        self.list.BuildRaytracingAccelerationStructure(
            &D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_DESC {
                DestAccelerationStructureData: tlas.GetGPUVirtualAddress(),
                Inputs: ti,
                SourceAccelerationStructureData: if refit {
                    tlas.GetGPUVirtualAddress()
                } else {
                    0
                },
                ScratchAccelerationStructureData: scratch,
            },
            None,
        );
        uav(&self.list, Some(&tlas));
        self.submit()?;
        drop((keep, instances));
        self.tlas = Some(tlas);
        self.instances = table;
        self.triangle_count = triangle_count;
        self.update_materials(s)?;
        if !self.painting {
            self.reset(true);
        }
        Ok(())
    }
    /// A DEFAULT-heap buffer filled through a staging copy recorded on the open command list
    /// (the staging buffer goes into `keep` until the list has run).
    /// Incremental vertex upload (wave 3) into the mesh vertex buffer shared by every object
    /// BLAS and the shading lookups: meshes already there keep their ranges (a removed or hidden
    /// one leaves a hole, so showing it again is free), new ones are appended, and the buffer
    /// grows by a quarter when they don't fit, copying the old ranges on the GPU. It is repacked
    /// from scratch when the holes would outweigh the meshes in use. Before, adding or hiding
    /// one object rebuilt and uploaded every mesh's vertices.
    pub(super) unsafe fn upload_mesh_vertices(
        &mut self,
        s: &Settings,
        visible: &[VisibleMesh],
        keep: &mut Vec<ID3D12Resource>,
    ) -> Result<()> {
        let stride = size_of::<Vertex>() as u64;
        let mut seen = std::collections::HashSet::new();
        let live: Vec<&VisibleMesh> = visible
            .iter()
            .filter(|v| seen.insert(v.key.as_str()))
            .collect();
        let live_total: u64 = live.iter().map(|v| v.source.len() as u64).sum();
        let new_total = |ranges: &std::collections::HashMap<String, (u32, usize)>| -> u64 {
            live.iter()
                .filter(|v| !ranges.contains_key(&v.key))
                .map(|v| v.source.len() as u64)
                .sum()
        };
        if (self.vertex_used + new_total(&self.vertex_ranges)).saturating_sub(live_total)
            > live_total
        {
            self.vertex_ranges.clear();
            self.vertex_used = 0;
        }
        let fresh_total = new_total(&self.vertex_ranges);
        if fresh_total == 0 {
            return Ok(());
        }
        let mut fresh: Vec<Vertex> = Vec::with_capacity(fresh_total as usize);
        for v in &live {
            if self.vertex_ranges.contains_key(&v.key) {
                continue;
            }
            let first = self.vertex_used + fresh.len() as u64;
            self.vertex_ranges
                .insert(v.key.clone(), (first as u32, v.source.len() / 3));
            let uv = mesh_uvs(&s.objects[v.index], v.source.len());
            fresh.extend(v.source.iter().zip(uv).map(|((p, n), uv)| Vertex {
                position: p.to_array(),
                material: 0,
                normal: n.to_array(),
                pad: 0,
                uv,
            }));
        }
        let needed = self.vertex_used + fresh_total;
        let offset = self.vertex_used * stride;
        let staging = upload(&self.device, bytemuck::cast_slice(&fresh))?;
        if needed > self.vertex_capacity {
            let capacity = if self.vertex_used == 0 {
                needed
            } else {
                needed.max(self.vertex_capacity + self.vertex_capacity / 4)
            };
            // Buffers start in COMMON; the copies promote it to COPY_DEST.
            let grown = buffer(
                &self.device,
                capacity * stride,
                D3D12_HEAP_TYPE_DEFAULT,
                D3D12_RESOURCE_FLAG_NONE,
                D3D12_RESOURCE_STATE_COMMON,
            )?;
            if self.vertex_used > 0 {
                transition(
                    &self.list,
                    &self.mesh_vertices,
                    D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
                    D3D12_RESOURCE_STATE_COPY_SOURCE,
                );
                self.list
                    .CopyBufferRegion(&grown, 0, &self.mesh_vertices, 0, offset);
            }
            // The old buffer (or the shared placeholder) stays alive until the copy has run.
            keep.push(std::mem::replace(&mut self.mesh_vertices, grown));
            self.vertex_capacity = capacity;
        } else {
            transition(
                &self.list,
                &self.mesh_vertices,
                D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
                D3D12_RESOURCE_STATE_COPY_DEST,
            );
        }
        self.list.CopyBufferRegion(
            &self.mesh_vertices,
            offset,
            &staging,
            0,
            fresh_total * stride,
        );
        transition(
            &self.list,
            &self.mesh_vertices,
            D3D12_RESOURCE_STATE_COPY_DEST,
            D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
        );
        keep.push(staging);
        self.vertex_used = needed;
        Ok(())
    }
    pub(super) unsafe fn gpu_buffer(
        &mut self,
        bytes: &[u8],
        keep: &mut Vec<ID3D12Resource>,
    ) -> Result<ID3D12Resource> {
        let staging = upload(&self.device, bytes)?;
        // Buffers start in COMMON; the copy promotes it to COPY_DEST.
        let b = buffer(
            &self.device,
            bytes.len() as u64,
            D3D12_HEAP_TYPE_DEFAULT,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_COMMON,
        )?;
        self.list
            .CopyBufferRegion(&b, 0, &staging, 0, bytes.len() as u64);
        transition(
            &self.list,
            &b,
            D3D12_RESOURCE_STATE_COPY_DEST,
            D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
        );
        keep.push(staging);
        Ok(b)
    }
    /// Track 20a (a): a BLAS whose triangles link to opacity micromaps built from the base
    /// map's alpha (`omm::build`), with the micromap array it needs kept alive beside it.
    /// Falls back to a plain BLAS when there is nothing to build.
    pub(super) unsafe fn omm_blas(
        &mut self,
        address: u64,
        count: usize,
        uvs: &[[f32; 2]],
        map: &crate::mesh::Map,
        cutoff: f32,
        keep: &mut Vec<ID3D12Resource>,
    ) -> Result<(ID3D12Resource, Option<ID3D12Resource>)> {
        let plain = triangles(address, count, D3D12_RAYTRACING_GEOMETRY_FLAG_NONE);
        let Some(mut maps) = crate::omm::build(uvs, map.width, map.height, &map.rgba, cutoff)
            .filter(|m| m.index.len() == count)
        else {
            return Ok((self.build_blas(&[plain], keep, true)?, None));
        };
        // Every triangle settled by a special index still needs an array to link: one unused
        // level-1 micromap.
        if maps.is_trivial() {
            maps.data.extend([0; 4]);
            maps.descs.push((0, 1));
        }
        let data = self.gpu_buffer(&maps.data, keep)?;
        let descs: Vec<u32> = maps
            .descs
            .iter()
            .flat_map(|&(offset, level)| [offset, level | OMM_FORMAT_4_STATE << 16])
            .collect();
        let descs = self.gpu_buffer(bytemuck::cast_slice(&descs), keep)?;
        let index = self.gpu_buffer(bytemuck::cast_slice(&maps.index), keep)?;
        let histogram: Vec<OmmHistogramEntry> = maps
            .histogram()
            .into_iter()
            .map(|(level, count)| OmmHistogramEntry {
                count,
                level,
                format: OMM_FORMAT_4_STATE,
            })
            .collect();
        let array_desc = OmmArrayDesc {
            histogram_entries: histogram.len() as u32,
            histogram: histogram.as_ptr(),
            input: data.GetGPUVirtualAddress(),
            descs: D3D12_GPU_VIRTUAL_ADDRESS_AND_STRIDE {
                StartAddress: descs.GetGPUVirtualAddress(),
                StrideInBytes: 8,
            },
        };
        let inputs = D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS {
            Type: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE(OMM_ARRAY_TYPE),
            Flags: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_PREFER_FAST_TRACE,
            NumDescs: 1,
            DescsLayout: D3D12_ELEMENTS_LAYOUT_ARRAY,
            Anonymous: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS_0 {
                pGeometryDescs: &array_desc as *const OmmArrayDesc as *const _,
            },
        };
        let mut info = D3D12_RAYTRACING_ACCELERATION_STRUCTURE_PREBUILD_INFO::default();
        self.device
            .GetRaytracingAccelerationStructurePrebuildInfo(&inputs, &mut info);
        ensure!(
            info.ResultDataMaxSizeInBytes > 0,
            "the driver reports no size for an opacity micromap array"
        );
        let array = buffer(
            &self.device,
            info.ResultDataMaxSizeInBytes,
            D3D12_HEAP_TYPE_DEFAULT,
            D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_RAYTRACING_ACCELERATION_STRUCTURE,
        )?;
        let scratch = buffer(
            &self.device,
            info.ScratchDataSizeInBytes.max(256),
            D3D12_HEAP_TYPE_DEFAULT,
            D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_COMMON,
        )?;
        self.list.BuildRaytracingAccelerationStructure(
            &D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_DESC {
                DestAccelerationStructureData: array.GetGPUVirtualAddress(),
                Inputs: inputs,
                ScratchAccelerationStructureData: scratch.GetGPUVirtualAddress(),
                ..Default::default()
            },
            None,
        );
        uav(&self.list, Some(&array));
        keep.extend([data, descs, scratch]);
        // The triangles, linked to the array: per triangle a micromap or a special index.
        let tri = plain.Anonymous.Triangles;
        let linkage = OmmLinkage {
            index: D3D12_GPU_VIRTUAL_ADDRESS_AND_STRIDE {
                StartAddress: index.GetGPUVirtualAddress(),
                StrideInBytes: 4,
            },
            index_format: DXGI_FORMAT_R32_UINT,
            base_location: 0,
            array: array.GetGPUVirtualAddress(),
        };
        let mut desc = D3D12_RAYTRACING_GEOMETRY_DESC {
            Type: D3D12_RAYTRACING_GEOMETRY_TYPE(OMM_GEOMETRY_TYPE),
            Flags: D3D12_RAYTRACING_GEOMETRY_FLAG_NONE,
            Anonymous: std::mem::zeroed(),
        };
        std::ptr::write(
            &mut desc.Anonymous as *mut _ as *mut OmmTriangles,
            OmmTriangles {
                triangles: &tri,
                linkage: &linkage,
            },
        );
        let blas = self.build_blas(&[desc], keep, true)?;
        keep.push(index);
        Ok((blas, Some(array)))
    }
    /// Frame timestamp k (0 start, 1 after the trace, 2 after reconstruction, 3 after display,
    /// 4 after the UI) and named events between them, so Nsight and PIX split their metrics per
    /// pass (track 20a step 1).
    pub(super) unsafe fn stamp(&self, k: u32) {
        const PASSES: [&str; 4] = ["trace", "reconstruction", "display", "ui"];
        if k > 0 {
            self.list.EndEvent();
        }
        self.list
            .EndQuery(&self.query, D3D12_QUERY_TYPE_TIMESTAMP, k);
        if let Some(name) = PASSES.get(k as usize) {
            // Metadata 0: a UTF-16 string (WINPIX_EVENT_UNICODE_VERSION; 1 would be ANSI).
            let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
            self.list
                .BeginEvent(0, Some(wide.as_ptr().cast()), (wide.len() * 2) as u32);
        }
    }
    /// Records a BLAS build on the open command list (scratch goes into `keep`).
    /// A BLAS for the open command list. With `compact`, it is queued for compaction
    /// (`compact_blases`, before the TLAS is built): the driver writes its compacted size.
    pub(super) unsafe fn build_blas(
        &mut self,
        descs: &[D3D12_RAYTRACING_GEOMETRY_DESC],
        keep: &mut Vec<ID3D12Resource>,
        compact: bool,
    ) -> Result<ID3D12Resource> {
        let inputs = D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS {
            Type: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_BOTTOM_LEVEL,
            Flags: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_PREFER_FAST_TRACE
                | if compact {
                    D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_ALLOW_COMPACTION
                } else {
                    D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_NONE
                },
            NumDescs: descs.len() as u32,
            DescsLayout: D3D12_ELEMENTS_LAYOUT_ARRAY,
            Anonymous: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS_0 {
                pGeometryDescs: descs.as_ptr(),
            },
        };
        let mut info = D3D12_RAYTRACING_ACCELERATION_STRUCTURE_PREBUILD_INFO::default();
        self.device
            .GetRaytracingAccelerationStructurePrebuildInfo(&inputs, &mut info);
        let blas = buffer(
            &self.device,
            info.ResultDataMaxSizeInBytes,
            D3D12_HEAP_TYPE_DEFAULT,
            D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_RAYTRACING_ACCELERATION_STRUCTURE,
        )?;
        let scratch = buffer(
            &self.device,
            info.ScratchDataSizeInBytes.max(256),
            D3D12_HEAP_TYPE_DEFAULT,
            D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_COMMON,
        )?;
        let info = if compact {
            Some(buffer(
                &self.device,
                8,
                D3D12_HEAP_TYPE_DEFAULT,
                D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            )?)
        } else {
            None
        };
        let postbuild = info.as_ref().map(|b| {
            [
                D3D12_RAYTRACING_ACCELERATION_STRUCTURE_POSTBUILD_INFO_DESC {
                    DestBuffer: b.GetGPUVirtualAddress(),
                    InfoType: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_POSTBUILD_INFO_COMPACTED_SIZE,
                },
            ]
        });
        self.list.BuildRaytracingAccelerationStructure(
            &D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_DESC {
                DestAccelerationStructureData: blas.GetGPUVirtualAddress(),
                Inputs: inputs,
                ScratchAccelerationStructureData: scratch.GetGPUVirtualAddress(),
                ..Default::default()
            },
            postbuild.as_ref().map(|p| p.as_slice()),
        );
        uav(&self.list, Some(&blas));
        keep.push(scratch);
        if let Some(info) = info {
            self.compact_queue.push((blas.clone(), info));
        }
        Ok(blas)
    }
    /// BLAS compaction (wave 3): the BLASes built on the open list are copied into buffers of
    /// their compacted size (typically about half) and swapped into the mesh and static-group
    /// tables before the TLAS records their addresses. It costs one more submit, only
    /// on rebuilds that built a BLAS (a scene load or a new mesh, not a drag).
    pub(super) unsafe fn compact_blases(&mut self, keep: &mut Vec<ID3D12Resource>) -> Result<()> {
        if self.compact_queue.is_empty() {
            return Ok(());
        }
        let queue = std::mem::take(&mut self.compact_queue);
        let readback = buffer(
            &self.device,
            8 * queue.len() as u64,
            D3D12_HEAP_TYPE_READBACK,
            D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_COPY_DEST,
        )?;
        for (i, (_, info)) in queue.iter().enumerate() {
            transition(
                &self.list,
                info,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
            );
            self.list
                .CopyBufferRegion(&readback, 8 * i as u64, info, 0, 8);
        }
        self.submit()?;
        let mut p = null_mut();
        readback.Map(0, None, Some(&mut p))?;
        let sizes = std::slice::from_raw_parts(p as *const u64, queue.len()).to_vec();
        readback.Unmap(0, None);
        self.begin()?;
        let mut swaps = vec![];
        for ((old, _), size) in queue.into_iter().zip(sizes) {
            if size == 0 || size >= old.GetDesc().Width {
                continue;
            }
            let new = buffer(
                &self.device,
                size,
                D3D12_HEAP_TYPE_DEFAULT,
                D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
                D3D12_RESOURCE_STATE_RAYTRACING_ACCELERATION_STRUCTURE,
            )?;
            self.list.CopyRaytracingAccelerationStructure(
                new.GetGPUVirtualAddress(),
                old.GetGPUVirtualAddress(),
                D3D12_RAYTRACING_ACCELERATION_STRUCTURE_COPY_MODE_COMPACT,
            );
            swaps.push((old, new));
        }
        uav(&self.list, None);
        for (old, new) in swaps {
            for m in self.meshes.values_mut() {
                if m.blas == old {
                    m.blas = new.clone();
                }
            }
            for g in &mut self.static_groups {
                if g.blas.as_ref() == Some(&old) {
                    g.blas = Some(new.clone());
                }
            }
            // The source must outlive the copy, which runs with the TLAS build.
            keep.push(old);
        }
        Ok(())
    }
    /// Materials, the emitter table and the instance table. Emitters stay world-space
    /// triangles: the static ones, then each emissive object's (its lookup range is
    /// `emitter_base` in the instance table; other objects have none).
    pub unsafe fn update_materials(&mut self, s: &Settings) -> Result<()> {
        let mut materials = crate::lighting::materials(s);
        // Maps first: an emissive map sets the emitted colour the table is built from.
        self.attach_maps(s, &mut materials)?;
        let mut world = self.static_cache.clone();
        // Emissive-mapped objects' UVs, by their first triangle in `world`.
        let mut mapped_uvs = vec![];
        for record in self.instances.iter_mut().filter(|r| r.material != u32::MAX) {
            let m = &materials[record.material as usize];
            let e = crate::lighting::emitted(m);
            let emission = e.x.max(e.y).max(e.z);
            // Procedural objects are not light-sampled: their emission is found by BSDF hits.
            let o = &s.objects[record.object as usize];
            record.emitter_base = if emission > 0. && o.sdf.is_none() {
                let base = (world.len() / 3) as u32;
                world.extend(crate::geometry::object_geometry(o, record.material));
                if m.lobes[3] as u32 & 64 != 0
                    && let Some(uv) = crate::mesh::uvs(o)
                {
                    mapped_uvs.push((base as usize, uv));
                }
                base
            } else {
                u32::MAX
            };
        }
        let analytic = crate::lighting::analytic(s);
        let (emitters, lookup) = crate::lighting::emitter_table(&world, &materials, &analytic);
        self.sun_count = crate::lighting::sun_count(&emitters);
        // Only real material records: the portal matrices after them hold a translation in optical.w.
        self.textured = materials
            .iter()
            .take(5 + s.objects.len() + s.lights.len())
            .any(|m| m.lobes[3] as u32 & 16 != 0 || m.optical[3] < 0.);
        // The LEAN variants' material gates (pathtrace.hlsl lean_strip), over every record: a
        // portal matrix can only turn them off.
        self.lean_scene = !s.objects.iter().any(|o| o.visible && !o.sdf.is_none())
            && materials.iter().all(|m| {
                m.layers[0] <= 0.
                    && m.layers[3] <= 0.
                    && m.sheen[..3].iter().all(|v| *v <= 0.)
                    && m.surface_scale[2] <= 0.
                    && m.medium[3] <= 0.
                    // IES lights keep their profile in fields LEAN strips.
                    && (m.lobes[3] as u32 & 16384) == 0
            });
        self.emitter_count = emitters.len();
        self.material_buffer = upload(&self.device, bytemuck::cast_slice(&materials))?;
        // The alias table for ReSTIR follows the emitters (`emitter_count` stays the table),
        // then the CDF's cutpoints for `select_index`.
        let mut table = emitters.clone();
        table.extend(crate::lighting::alias_records(&emitters));
        table.extend(crate::lighting::guide_records(&emitters));
        // Emissive procedural objects for photon mapping (`sdf_emitter_records`), last.
        let sdf = sdf_emitter_records(s, &self.instances, &materials);
        self.sdf_emitters = [(sdf.len() / 2) as u32, table.len() as u32];
        table.extend(sdf);
        // With an emissive map anywhere, one UV record per emitter after them (pathtrace.hlsl
        // `emitted_color`): a, b in position_area, c in edge1_cdf.xy, zero for the rest.
        if !mapped_uvs.is_empty() {
            let mut uvs: Vec<crate::lighting::Emitter> =
                vec![bytemuck::Zeroable::zeroed(); emitters.len()];
            for (base, uv) in &mapped_uvs {
                for (k, t) in uv.as_chunks::<3>().0.iter().enumerate() {
                    if let Some(&e) = lookup.get(base + k)
                        && e != 0
                        && let Some(r) = uvs.get_mut(e as usize - 1)
                    {
                        r.position_area = [t[0][0], t[0][1], t[1][0], t[1][1]];
                        r.edge1_cdf = [t[2][0], t[2][1], 0., 0.];
                    }
                }
            }
            table.extend(uvs);
        }
        self.emitter_buffer = upload(&self.device, bytemuck::cast_slice(&table))?;
        self.emitter_lookup = upload(&self.device, bytemuck::cast_slice(&lookup))?;
        self.instance_table = upload(&self.device, bytemuck::cast_slice(&self.instances))?;
        // Light painting keeps the exposure, but reservoirs made for the old lights would be
        // reused with stale weights.
        self.restir_history = false;
        if !self.painting {
            self.reset(true);
        }
        Ok(())
    }
}
