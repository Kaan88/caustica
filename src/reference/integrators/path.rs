//! The path tracer.
use crate::lighting::PackedMaterial;
use crate::reference::bsdf::{
    GRATING_ORDER, Wavelength, aniso_alpha, aniso_frame, coat_probability, emitted, eval_bsdf,
    flags, fresnel_dielectric, grating, grating_move, grating_wavelength, material_at,
    order_shares, prepare_film, rgb, spec_probability,
};
use crate::reference::media::{flight, hg_pdf, hg_sample, push, remove, top};
use crate::reference::sampling::{
    Rng, cosine_sample, ggx_sample, ggx_sample_aniso, reflect, refract,
};
use crate::reference::scene::{Ray, Scene};
use crate::reference::spectral::Waves;
use glam::{Vec3, Vec4};

/// Where a ray entering a portal's front face at `p` leaves its pair: the map record (`lighting`
/// portal records) holds the affine map's first three rows. The origin is nudged out along the
/// new direction.
pub(in crate::reference) fn portal_map(map: &PackedMaterial, p: Vec3, d: Vec3) -> (Vec3, Vec3) {
    let rows = [map.color_rough, map.lobes, map.optical].map(Vec4::from);
    let apply = |v: Vec4| Vec3::new(rows[0].dot(v), rows[1].dot(v), rows[2].dot(v));
    let direction = apply(d.extend(0.)).normalize();
    (apply(p.extend(1.)) + direction * 0.001, direction)
}

impl Scene<'_> {
    /// One path's radiance as linear RGB. `w` holds its wavelengths (`Scene::waves`): spectral,
    /// the channels are those wavelengths until the end, where `to_rgb` brings them back.
    pub(in crate::reference) fn radiance(
        &self,
        mut origin: Vec3,
        mut direction: Vec3,
        weight: Vec3,
        w: &mut Waves,
        r: &mut Rng,
    ) -> Vec3 {
        let s = self.s;
        // Which strategies count: 0 both, with MIS; 1 light sampling only; 2 BSDF sampling only.
        let mis = s.mis_mode.min(2);
        let mut throughput = weight;
        let mut radiance = Vec3::ZERO;
        let mut previous_pdf = 0f32;
        let mut previous_delta = true;
        let mut previous_position = origin;
        // The last vertex sampled the light table as ReSTIR does (`restir_vertex`).
        let mut previous_restir = false;
        // The dielectrics the path is inside: (priority, material), innermost last.
        let mut media: Vec<(u32, u32)> = self.camera_media.clone();
        let mut false_hits = 0;
        // Portal and beam-splitter crossings on this path (`trace` caps portals at 16, splitters
        // at 64).
        let mut crossings = 0;
        // Only delta bounces since the camera: the background it sees is ground-projected.
        let mut camera_chain = true;
        let mut bounce = 0;
        while bounce < s.bounces {
            camera_chain &= previous_delta;
            let kind = if bounce == 0 {
                Ray::Camera
            } else {
                Ray::Bounce
            };
            // False hits on nested dielectrics (`medium_top`): the ray goes on unbent.
            let hit = loop {
                let Some(h) = self.intersect(
                    origin,
                    direction,
                    1e4,
                    &self.accept(kind, origin, direction),
                    false,
                ) else {
                    break None;
                };
                let m = &self.materials[h.material as usize];
                if false_hits >= 32 || m.lobes[2] <= 0. {
                    break Some(h);
                }
                let sf = self.surface(&h, origin, direction);
                let entry = ((flags(m) >> 8) & 7, h.material);
                let skip = if sf.front {
                    top(&media).is_some_and(|top| entry.0 < top.0)
                } else {
                    media.contains(&entry) && top(&media) != Some(&entry)
                };
                if !skip {
                    break Some(h);
                }
                // The segment up to the false hit lies in the medium before it.
                throughput *= self.transmittance(&media, h.t, w);
                if sf.front {
                    push(&mut media, entry);
                } else {
                    remove(&mut media, entry);
                }
                false_hits += 1;
                origin = sf.p - sf.ng * 0.0003;
            };
            // A free flight through the medium the path is in may scatter before the surface (or
            // the background), with light sampling by the phase. Inside a scattering object it
            // fills the segment; the fog box fills the space outside every object. The segment
            // after false hits only (those before them were absorbed alone, as on the GPU).
            let tmax = hit.as_ref().map_or(1e4, |h| h.t);
            let medium = if media.is_empty() {
                self.fog_interval(origin, direction, tmax).map(|(t0, t1)| {
                    let f = &s.fog;
                    let (ts, fw) = self.fog_flight(origin, direction, t0, t1, w, r);
                    (ts, t1, fw, w.up(Vec3::from(f.color)), f.phase)
                })
            } else {
                self.object_medium(&media, w).map(|(sigma, albedo, g)| {
                    let (ts, fw) = flight(sigma, 0., tmax, r);
                    (ts, tmax, fw, albedo, g)
                })
            };
            if let Some((ts, t1, fw, albedo, g)) = medium {
                throughput *= fw;
                if ts < t1 {
                    // The dielectric's colour absorbs up to the scatter.
                    throughput *= self.transmittance(&media, ts, w) * albedo;
                    let x = origin + direction * ts;
                    if mis != 2
                        && let Some((ld, le, pl, limit)) = self.sample_light(x, w, r)
                        && self
                            .intersect(x, ld, limit, &self.accept(Ray::Shadow, x, ld), true)
                            .is_none()
                    {
                        let pp = hg_pdf(direction.dot(ld), g);
                        let density = if mis == 1 { pl } else { pl + pp };
                        radiance += throughput
                            * self.shadow_transmittance(&media, x, ld, limit, w, r)
                            * pp
                            * le
                            / density.max(1e-8);
                    }
                    let (d, pp) = hg_sample(direction, g, r);
                    previous_pdf = pp;
                    previous_delta = false;
                    previous_restir = false;
                    previous_position = x;
                    origin = x;
                    direction = d;
                    if bounce >= 3 {
                        let survival = throughput.max_element().clamp(0.05, 0.95);
                        if r.f() > survival {
                            break;
                        }
                        throughput /= survival;
                    }
                    bounce += 1;
                    continue;
                }
            }
            let Some(h) = hit else {
                // Suns: seen inside their disc, weighted against light sampling.
                radiance += throughput
                    * self.suns(direction, bounce, w, |pl| {
                        if bounce == 0 || previous_delta {
                            1.
                        } else {
                            match mis {
                                _ if previous_restir => 0.,
                                1 => 0.,
                                2 => 1.,
                                _ => previous_pdf / (previous_pdf + pl),
                            }
                        }
                    });
                let Some(env) = &self.env else {
                    // The sky gradient over the background colour (no HDRI).
                    radiance += throughput * w.up(self.plain_sky(direction));
                    break;
                };
                let seen = if bounce == 0 && !env.visible {
                    Vec3::from(s.background)
                } else if camera_chain {
                    env.radiance(env.background_dir(origin, direction))
                } else {
                    env.radiance(direction)
                };
                let weight = if bounce == 0 || previous_delta {
                    1.
                } else {
                    match mis {
                        1 => 0.,
                        2 => 1.,
                        _ => previous_pdf / (previous_pdf + self.p_env * env.pdf(direction)),
                    }
                };
                radiance += throughput * w.up(seen) * weight;
                break;
            };
            throughput *= self.transmittance(&media, h.t, w);
            let mut sf = self.surface(&h, origin, direction);
            // Shadow catcher: the floor is the background behind it, and the path ends there.
            if s.shadow_catcher && sf.material == 3 {
                radiance += throughput * self.catcher(&sf, direction, bounce, camera_chain, w, r);
                break;
            }
            // Portals: a front face maps the ray into the paired portal (the map record's first
            // three rows); a back face lets it through. Neither is a bounce, and light sampling
            // never looks through one, so what is found beyond counts in full.
            let record = &self.materials[sf.material as usize];
            if flags(record) & 4 != 0 && crossings < 16 {
                crossings += 1;
                if !sf.front {
                    origin = sf.p + direction * 0.001;
                    continue;
                }
                (origin, direction) = portal_map(
                    &self.materials[record.secondary[3] as usize],
                    sf.p,
                    direction,
                );
                previous_delta = true;
                continue;
            }
            let mut m = material_at(&self.materials[sf.material as usize], sf.p);
            // Beam splitters: a thin partial mirror, a delta event either way, sharing the
            // portals' crossing budget (past it the sheet shades as an ordinary material). A back
            // face (leaving a closed splitter) passes untinted, so a thin box splits once, not
            // twice. Only a reflection is a bounce; the sheet's own emission is never seen.
            if flags(&m) & 4096 != 0 && crossings < 64 {
                crossings += 1;
                previous_delta = true;
                if sf.front {
                    throughput *= w.up_material(&m, rgb(m.color_rough));
                    if r.f() < m.lobes[1] {
                        direction = reflect(direction, sf.n);
                        bounce += 1;
                    }
                }
                origin = sf.p + direction * 0.0005;
                continue;
            }
            let colour = self.emission_colour(&m, h.tri, h.b);
            let le = emitted(
                &m,
                w.up_emission(&m, colour),
                sf.front,
                sf.emission_cos(direction),
            );
            // Maps after emission, as the GPU: an emitter keeps its unmapped colour.
            self.apply_maps(&mut m, &h, &mut sf.n);
            if le.max_element() > 0. {
                let weight = if bounce == 0 || previous_delta {
                    1.
                } else {
                    match mis {
                        // Light sampling found this emitter already. Every emissive triangle and
                        // procedural object is in a light table (`Scene::new`).
                        1 => 0.,
                        2 => 1.,
                        // After a ReSTIR vertex: the table's light sample counted in full, so
                        // what it covers counts 0 (all but a sphere light seen from inside);
                        // procedural emitters had no light sample there and count in full.
                        _ if previous_restir => match h.sdf {
                            Some(_) => 1.,
                            None if self.light_pdf(h.tri, previous_position, sf.p) > 0. => 0.,
                            None => 1.,
                        },
                        _ => {
                            let pl = match h.sdf {
                                Some(i) => self.sdf_light_pdf(i, previous_position),
                                None => self.light_pdf(h.tri, previous_position, sf.p),
                            };
                            previous_pdf / (previous_pdf + pl)
                        }
                    }
                };
                radiance += throughput * le * weight;
            }
            // A thin film, between the media on both sides of a dielectric, else over an opaque
            // base.
            let film = (m.surface_scale[2] > 0.).then(|| {
                let (outside, base) = if m.lobes[2] > 0. {
                    let (n1, n2, _) = self.interface(&media, &m, sf.material, sf.front, w);
                    (n1, n2)
                } else {
                    (1., 0.)
                };
                let c = (-direction).dot(sf.n).clamp(0., 1.);
                prepare_film(&mut m, c, outside, base, None);
                (c, outside, base)
            });
            if w.on {
                // A grating's orders leave at the hero wavelength's angles only, so the path
                // keeps that one from here, before light sampling.
                if grating(&m).is_some() {
                    w.hero(&mut throughput);
                }
                // The colours become spectra, and the film is exact at the path's wavelengths
                // over the uplifted f0 (its RGB pass above is what the RGB guides would read).
                let c = w.up_material(&m, rgb(m.color_rough));
                m.color_rough[..3].copy_from_slice(&c.to_array());
                let sheen = w.up(rgb(m.sheen));
                m.sheen[..3].copy_from_slice(&sheen.to_array());
                if let Some((c, outside, base)) = film {
                    prepare_film(&mut m, c, outside, base, Some(w.l));
                }
            }
            // The brushing direction, from the raw record as the GPU's hit reads it; a procedural
            // hit's is fixed by its normal, as `intersect` gives it.
            let mut tangent = self.tangent(sf.material, sf.p);
            if h.sdf.is_some() && tangent != Vec3::ZERO {
                let up = if sf.n.y.abs() < 0.99 {
                    Vec3::Y
                } else {
                    Vec3::X
                };
                tangent = sf.n.cross(up).normalize();
            }
            // A grating hit's wavelength, shared by light sampling and the BSDF sample here, from
            // its own stream so the path's draws stay as they were.
            let lambda = match grating(&m) {
                None => Wavelength::NONE,
                Some(_) if w.on => Wavelength {
                    nm: w.l.x,
                    weight: Vec3::ONE,
                },
                Some(_) => {
                    Wavelength::rgb(grating_wavelength(&mut r.fork(0x6772_6174 ^ bounce as u64)))
                }
            };
            // Unlit: emission only.
            if flags(&m) & 1 != 0 {
                break;
            }
            let v = -direction;
            // ReSTIR DI at the first hit, for the GPU's first `restir_layers` samples of each
            // frame (drawn here with that share): the environment keeps its own strategy and
            // MIS, the light table is sampled alone with weight 1 (the reservoirs only lower its
            // noise), and procedural emitters are left to the BSDF sample. At a bounce limit this
            // differs from MIS, which keeps only its share of what the untraced BSDF sample
            // would find.
            let restir = bounce == 0
                && s.restir
                && mis == 0
                && !self.lights.is_empty()
                && r.f() < self.restir_share();
            // The table's ReSTIR sample: its density alone, no BSDF share.
            let mut alone = false;
            // Light sampling.
            let sample = if mis == 2 {
                None
            } else if restir {
                match &self.env {
                    Some(env) if r.f() < self.p_env => {
                        let (ld, pe) = env.sample(r);
                        (pe > 0.).then(|| (ld, w.up(env.radiance(ld)), self.p_env * pe, 1e4))
                    }
                    _ => {
                        alone = true;
                        self.sample_table(sf.p, w, r)
                            .map(|(ld, le, pl, limit)| (ld, le, pl / (1. - self.sdf_share), limit))
                    }
                }
            } else {
                self.sample_light(sf.p, w, r)
            };
            if let Some((ld, le, pl, limit)) = sample {
                let nl = sf.n.dot(ld);
                if nl > 0. {
                    let (f, pdf) = eval_bsdf(&m, sf.n, tangent, v, ld, lambda);
                    if f.max_element() > 0.
                        && self
                            .intersect(
                                sf.p + sf.ng * 0.0003,
                                ld,
                                limit,
                                &self.accept(Ray::Shadow, sf.p, ld),
                                true,
                            )
                            .is_none()
                    {
                        let density = if mis == 1 || alone { pl } else { pl + pdf };
                        let o = sf.p + sf.ng * 0.0003;
                        let tr = self.shadow_transmittance(&media, o, ld, limit, w, r);
                        radiance += throughput * f * nl * le * tr / density.max(1e-8);
                    }
                }
            }
            // The next direction.
            let next;
            let mut delta = false;
            let mut bsdf_pdf = 0.;
            if r.f() < m.lobes[2] {
                // Dispersion: from here the path follows the hero wavelength's index (light
                // sampling above still used all three).
                if w.dispersive(&m) {
                    w.hero(&mut throughput);
                }
                let (n1, n2, after) = self.interface(&media, &m, sf.material, sf.front, w);
                let eta = n1 / n2;
                let ci = v.dot(sf.n).clamp(0., 1.);
                let mut fr = fresnel_dielectric(ci, eta);
                // A film colours the reflection and, by what it leaves, the transmission: the
                // choice goes by the mean, the weight per channel.
                let film = m.surface_scale[2];
                let mut frc = Vec3::splat(fr);
                if film > 0. && fr < 1. {
                    frc = frc.lerp(
                        Vec3::new(m.base_scale[0], m.base_scale[1], m.base_scale[2]),
                        film,
                    );
                    fr = (frc.element_sum() / 3.).clamp(1e-4, 1. - 1e-4);
                }
                let reflected = r.f() < fr;
                if film > 0. {
                    throughput *= if reflected {
                        frc / fr
                    } else {
                        (Vec3::ONE - frc) / (1. - fr)
                    };
                }
                let mut d = if reflected {
                    reflect(direction, sf.n)
                } else {
                    refract(direction, sf.n, eta)
                };
                if !reflected {
                    media = after;
                    throughput *= eta * eta;
                }
                let rough = m.color_rough[3];
                if rough > 0.001 {
                    let pert = cosine_sample(if reflected { sf.n } else { -sf.n }, r);
                    d = d.lerp(pert, rough * rough).normalize();
                }
                next = d;
                delta = true;
            } else {
                let ps = spec_probability(&m);
                let pc = coat_probability(&m);
                let d = if pc > 0. && r.f() < pc {
                    let cr = m.layers[1];
                    ggx_sample(sf.n, direction, (cr * cr).max(0.001), r)
                } else if r.f() < ps {
                    let rough = m.color_rough[3];
                    let a = (rough * rough).max(0.001);
                    // A grating's order (`order_shares`): the isotropic sample moved across the
                    // grooves; one moved past the horizon ends the path.
                    let mut order = None;
                    if let Some((share, spacing)) = grating(&m) {
                        let (_, b) = aniso_frame(sf.n, tangent, m.sheen[3]);
                        let k = lambda.nm / spacing;
                        let w = order_shares(share, k, sf.n, b, v);
                        let u = r.f();
                        let mut c = 0.;
                        order = (0..4)
                            .find(|&o| {
                                c += w[o];
                                u < c
                            })
                            .map(|o| (GRATING_ORDER[o] * k, b));
                    }
                    if let Some((shift, b)) = order {
                        let d = ggx_sample(sf.n, direction, a, r);
                        match grating_move(sf.n, b, d, shift).filter(|_| sf.n.dot(d) > 0.) {
                            Some(d) => d,
                            None => break,
                        }
                    } else if m.layers[3] > 0. {
                        let (t, b) = aniso_frame(sf.n, tangent, m.sheen[3]);
                        ggx_sample_aniso(sf.n, t, b, direction, aniso_alpha(&m), r)
                    } else {
                        ggx_sample(sf.n, direction, a, r)
                    }
                } else {
                    cosine_sample(sf.n, r)
                };
                if sf.n.dot(d) <= 0. {
                    break;
                }
                let (f, pdf) = eval_bsdf(&m, sf.n, tangent, v, d, lambda);
                throughput *= f * sf.n.dot(d).abs() / pdf.max(1e-7);
                bsdf_pdf = pdf;
                next = d;
            }
            previous_pdf = bsdf_pdf;
            previous_delta = delta;
            previous_restir = restir;
            previous_position = sf.p;
            direction = next.normalize();
            origin = sf.p
                + sf.ng
                    * if direction.dot(sf.ng) > 0. {
                        0.0003
                    } else {
                        -0.0003
                    };
            if bounce >= 3 {
                let survival = throughput.max_element().clamp(0.05, 0.95);
                if r.f() > survival {
                    break;
                }
                throughput /= survival;
            }
            bounce += 1;
        }
        // Spectral: no clamp, as on the GPU. A wavelength triple's estimate can be negative in a
        // channel (the output weights are), and clamping each would bias the mean (+35% red on
        // Classic, engine-notes).
        if !radiance.is_finite() {
            Vec3::ZERO
        } else if w.on {
            w.to_rgb(radiance)
        } else {
            radiance.max(Vec3::ZERO)
        }
    }
    /// The IORs on the near and far side of dielectric `material` hit from `front`, and the
    /// media stack beyond it: entering, the current medium's and its own; leaving, its own and
    /// the one the stack falls back to (air when empty). Each at the path's hero wavelength once
    /// it keeps that alone (`Waves::ior`).
    fn interface(
        &self,
        media: &[(u32, u32)],
        m: &crate::lighting::PackedMaterial,
        material: u32,
        front: bool,
        w: &Waves,
    ) -> (f32, f32, Vec<(u32, u32)>) {
        let entry = ((flags(m) >> 8) & 7, material);
        let ior = |e: Option<&(u32, u32)>| e.map_or(1., |e| w.ior(&self.materials[e.1 as usize]));
        let mut after = media.to_vec();
        if front {
            push(&mut after, entry);
            (ior(top(media)), w.ior(m), after)
        } else {
            remove(&mut after, entry);
            let n2 = ior(top(&after));
            (w.ior(m), n2, after)
        }
    }
    /// The sky gradient over the background colour, without an environment map.
    fn plain_sky(&self, d: Vec3) -> Vec3 {
        let up = (d.y * 0.5 + 0.5).clamp(0., 1.);
        Vec3::from(self.s.background)
            + self.s.sky * Vec3::new(0.8, 0.87, 1.0).lerp(Vec3::new(0.16, 0.35, 0.72), up)
    }
    /// What the shadow catcher shows (`Settings::shadow_catcher`). Seen from the camera (through
    /// delta bounces only), the background behind it, ground-projected when that is on, times
    /// the share of four light samples' power the scene leaves unblocked; any other ray sees
    /// the projected ground, unshadowed.
    fn catcher(
        &self,
        sf: &crate::reference::scene::Surface,
        direction: Vec3,
        bounce: u32,
        camera_chain: bool,
        w: &Waves,
        r: &mut Rng,
    ) -> Vec3 {
        let env = self.env.as_ref();
        let seen = match env {
            Some(e) if e.ground > 0. => e.ground_dir(sf.p),
            _ => direction,
        };
        let background = |d: Vec3| env.map_or_else(|| self.plain_sky(d), |e| e.radiance(d));
        if !camera_chain {
            return w.up(background(seen));
        }
        let bg = match env {
            Some(e) if bounce == 0 && !e.visible => Vec3::from(self.s.background),
            _ => background(seen),
        };
        let (mut all, mut lit) = (0f32, 0f32);
        for _ in 0..4 {
            let Some((ld, le, pl, limit)) = self.sample_light(sf.p, w, r) else {
                continue;
            };
            let c = sf.n.dot(ld);
            if c <= 0. || pl <= 0. {
                continue;
            }
            let lw = crate::environment::luminance(le.to_array()) * c / pl;
            all += lw;
            let blocked = self
                .intersect(
                    sf.p + sf.ng * 0.0003,
                    ld,
                    limit,
                    &self.accept(Ray::Shadow, sf.p, ld),
                    true,
                )
                .is_some();
            if !blocked {
                lit += lw;
            }
        }
        w.up(bg) * if all > 0. { lit / all } else { 1. }
    }
}
