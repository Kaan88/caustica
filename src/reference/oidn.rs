//! The CPU renderer's mean through Intel OIDN (`crate::oidn`): the finished image of the
//! software view and `--reference`'s PNG or JPEG, never an EXR (which keeps the plain mean).
//! The guides are what the GPU writes (`write_guides`): per pixel one ray through the centre, the
//! hit's shading normal and its albedo, the diffuse part plus the specular Fresnel term, so a
//! metal keeps its colour. They are noise-free (the thin lens looks through the pinhole), and gathered here on demand, so a render that
//! never asks for OIDN pays nothing for them.
use super::aov::{diffuse_albedo, specular_albedo};
use super::bsdf::material_at;
use super::progressive::{Progressive, for_rows};
use super::sampling::Rng;
use super::scene::{Ray, Scene};
use glam::{Vec2, Vec3};

impl Scene<'_> {
    /// The albedo and normal of the first hit of the ray through pixel (x, y)'s centre; black
    /// and a zero normal for the background.
    fn oidn_guide(&self, x: usize, y: usize) -> (Vec3, Vec3) {
        let pixel = (y * self.size[0] as usize + x) as u64;
        let centre = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
        // The thin lens picks a random lens point, which would make the albedo and the normal
        // disagree at defocused edges: the guide looks through the pinhole instead.
        let thin = self.lens.is_none()
            && matches!(self.frame.projection, 0 | 4)
            && self.frame.aperture > 0.;
        let ray = if thin {
            Some((self.eye, self.camera_direction(centre), Vec3::ONE))
        } else {
            self.camera_ray(
                centre,
                &mut super::spectral::Waves::rgb(),
                &mut Rng::new(pixel, u64::MAX),
            )
        };
        let Some((o, d, _)) = ray else {
            return (Vec3::ZERO, Vec3::ZERO);
        };
        let Some(h) = self.intersect(o, d, 1e4, &self.accept(Ray::Camera, o, d), false) else {
            return (Vec3::ZERO, Vec3::ZERO);
        };
        let mut sf = self.surface(&h, o, d);
        let mut m = material_at(&self.materials[sf.material as usize], sf.p);
        self.apply_maps(&mut m, &h, &mut sf.n);
        let albedo = diffuse_albedo(&m) + specular_albedo(&m, sf.n.dot(-d).abs());
        (albedo.min(Vec3::ONE), sf.n)
    }
}

impl Progressive<'_> {
    /// The mean (RGBA, rows top first) through OIDN with the albedo and normal guides, on
    /// `threads` threads for the guides; the filter itself uses every core.
    pub fn oidn_image(&self, threads: usize) -> anyhow::Result<Vec<f32>> {
        let [w, h] = self.scene().size.map(|v| v as usize);
        let mut guides = vec![(Vec3::ZERO, Vec3::ZERO); w * h];
        let scene = self.scene();
        for_rows(
            &mut guides,
            w,
            threads,
            false,
            &Default::default(),
            &|y, row| {
                for (x, g) in row.iter_mut().enumerate() {
                    *g = scene.oidn_guide(x, y);
                }
            },
        );
        let albedo: Vec<f32> = guides.iter().flat_map(|g| g.0.to_array()).collect();
        let normal: Vec<f32> = guides.iter().flat_map(|g| g.1.to_array()).collect();
        let mut image = self.image();
        crate::oidn::denoise_linear_rgba(w, h, &mut image, Some(&(albedo, normal)))?;
        Ok(image)
    }
}
