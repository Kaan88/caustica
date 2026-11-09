//! The path tracer.
use crate::reference::bsdf::{
    coat_probability, emitted, eval_bsdf, flags, fresnel_dielectric, material_at, spec_probability,
};
use crate::reference::media::{hg_pdf, hg_sample, push, remove, top};
use crate::reference::sampling::{Rng, cosine_sample, ggx_sample, reflect, refract};
use crate::reference::scene::{Ray, Scene};
use glam::Vec3;

impl Scene<'_> {
    pub(in crate::reference) fn radiance(
        &self,
        mut origin: Vec3,
        mut direction: Vec3,
        r: &mut Rng,
    ) -> Vec3 {
        let s = self.s;
        // Which strategies count: 0 both, with MIS; 1 light sampling only; 2 BSDF sampling only.
        let mis = s.mis_mode.min(2);
        let mut throughput = Vec3::ONE;
        let mut radiance = Vec3::ZERO;
        let mut previous_pdf = 0f32;
        let mut previous_delta = true;
        let mut previous_position = origin;
        // The dielectrics the path is inside: (priority, material), innermost last.
        let mut media: Vec<(u32, u32)> = self.camera_media.clone();
        let mut false_hits = 0;
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
                let Some(h) = self
                    .bvh
                    .intersect(origin, direction, 1e4, &self.accept(kind), false)
                else {
                    break None;
                };
                let t = &self.bvh.tris[h.tri];
                let m = &self.materials[t.material as usize];
                if false_hits >= 32 || m.lobes[2] <= 0. {
                    break Some(h);
                }
                let sf = self.surface(&h, direction);
                let entry = ((flags(m) >> 8) & 7, t.material);
                let skip = if sf.front {
                    top(&media).is_some_and(|top| entry.0 < top.0)
                } else {
                    media.contains(&entry) && top(&media) != Some(&entry)
                };
                if !skip {
                    break Some(h);
                }
                // The segment up to the false hit lies in the medium before it.
                throughput *= self.transmittance(&media, h.t);
                if sf.front {
                    push(&mut media, entry);
                } else {
                    remove(&mut media, entry);
                }
                false_hits += 1;
                origin = sf.p - sf.ng * 0.0003;
            };
            // The fog box fills the space outside every object: a free flight through it may
            // scatter before the surface (or the background), with light sampling by the phase.
            if media.is_empty()
                && let Some((t0, t1)) =
                    self.fog_interval(origin, direction, hit.as_ref().map_or(1e4, |h| h.t))
            {
                let f = &s.fog;
                let ts = t0 - (1. - r.f()).ln() / f.density;
                if ts < t1 {
                    throughput *= Vec3::from(f.color);
                    let x = origin + direction * ts;
                    if mis != 2
                        && let Some((ld, le, pl, limit)) = self.sample_light(x, r)
                        && self
                            .bvh
                            .intersect(x, ld, limit, &self.accept(Ray::Shadow), true)
                            .is_none()
                    {
                        let pp = hg_pdf(direction.dot(ld), f.phase);
                        let density = if mis == 1 { pl } else { pl + pp };
                        radiance += throughput * self.fog_transmittance(x, ld, limit) * pp * le
                            / density.max(1e-8);
                    }
                    let (d, pp) = hg_sample(direction, f.phase, r);
                    previous_pdf = pp;
                    previous_delta = false;
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
                let Some(env) = &self.env else {
                    // The sky gradient over the background colour (no HDRI).
                    radiance += throughput * self.plain_sky(direction);
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
                radiance += throughput * seen * weight;
                break;
            };
            throughput *= self.transmittance(&media, h.t);
            let sf = self.surface(&h, direction);
            // Shadow catcher: the floor is the background behind it, and the path ends there.
            if s.shadow_catcher && sf.material == 3 {
                radiance += throughput * self.catcher(&sf, direction, bounce, camera_chain, r);
                break;
            }
            let m = material_at(&self.materials[sf.material as usize], sf.p);
            let le = emitted(&m, sf.front, sf.emission_cos(direction));
            if le.max_element() > 0. {
                let weight = if bounce == 0 || previous_delta {
                    1.
                } else {
                    match mis {
                        // Light sampling found this emitter already. Every emissive triangle is
                        // in the light table (`Scene::new`).
                        1 => 0.,
                        2 => 1.,
                        _ => {
                            let pl = self.light_pdf(h.tri, previous_position, sf.p);
                            previous_pdf / (previous_pdf + pl)
                        }
                    }
                };
                radiance += throughput * le * weight;
            }
            // Unlit: emission only.
            if flags(&m) & 1 != 0 {
                break;
            }
            let v = -direction;
            // Light sampling.
            let sample = if mis == 2 {
                None
            } else {
                self.sample_light(sf.p, r)
            };
            if let Some((ld, le, pl, limit)) = sample {
                let nl = sf.n.dot(ld);
                if nl > 0. {
                    let (f, pdf) = eval_bsdf(&m, sf.n, v, ld);
                    if f.max_element() > 0.
                        && self
                            .bvh
                            .intersect(
                                sf.p + sf.ng * 0.0003,
                                ld,
                                limit,
                                &self.accept(Ray::Shadow),
                                true,
                            )
                            .is_none()
                    {
                        let density = if mis == 1 { pl } else { pl + pdf };
                        let fog = if media.is_empty() {
                            self.fog_transmittance(sf.p, ld, limit)
                        } else {
                            1.
                        };
                        radiance += throughput * f * nl * le * fog / density.max(1e-8);
                    }
                }
            }
            // The next direction.
            let next;
            let mut delta = false;
            let mut bsdf_pdf = 0.;
            if r.f() < m.lobes[2] {
                let entry = ((flags(&m) >> 8) & 7, sf.material);
                let ior = |e: Option<&(u32, u32)>| {
                    e.map_or(1., |e| self.materials[e.1 as usize].optical[0])
                };
                let mut after = media.clone();
                if sf.front {
                    push(&mut after, entry);
                } else {
                    remove(&mut after, entry);
                }
                let (n1, n2) = if sf.front {
                    (ior(top(&media)), m.optical[0])
                } else {
                    (m.optical[0], ior(top(&after)))
                };
                let eta = n1 / n2;
                let ci = v.dot(sf.n).clamp(0., 1.);
                let fr = fresnel_dielectric(ci, eta);
                let reflected = r.f() < fr;
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
                    ggx_sample(sf.n, direction, (rough * rough).max(0.001), r)
                } else {
                    cosine_sample(sf.n, r)
                };
                if sf.n.dot(d) <= 0. {
                    break;
                }
                let (f, pdf) = eval_bsdf(&m, sf.n, v, d);
                throughput *= f * sf.n.dot(d).abs() / pdf.max(1e-7);
                bsdf_pdf = pdf;
                next = d;
            }
            previous_pdf = bsdf_pdf;
            previous_delta = delta;
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
        if radiance.is_finite() {
            radiance.max(Vec3::ZERO)
        } else {
            Vec3::ZERO
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
        r: &mut Rng,
    ) -> Vec3 {
        let env = self.env.as_ref();
        let seen = match env {
            Some(e) if e.ground > 0. => e.ground_dir(sf.p),
            _ => direction,
        };
        let background = |d: Vec3| env.map_or_else(|| self.plain_sky(d), |e| e.radiance(d));
        if !camera_chain {
            return background(seen);
        }
        let bg = match env {
            Some(e) if bounce == 0 && !e.visible => Vec3::from(self.s.background),
            _ => background(seen),
        };
        let (mut all, mut lit) = (0f32, 0f32);
        for _ in 0..4 {
            let Some((ld, le, pl, limit)) = self.sample_light(sf.p, r) else {
                continue;
            };
            let c = sf.n.dot(ld);
            if c <= 0. || pl <= 0. {
                continue;
            }
            let w = crate::environment::luminance(le.to_array()) * c / pl;
            all += w;
            let blocked = self
                .bvh
                .intersect(
                    sf.p + sf.ng * 0.0003,
                    ld,
                    limit,
                    &self.accept(Ray::Shadow),
                    true,
                )
                .is_some();
            if !blocked {
                lit += w;
            }
        }
        bg * if all > 0. { lit / all } else { 1. }
    }
}
