use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Material {
    pub color: [f32; 3],
    pub roughness: f32,
    pub metallic: f32,
    pub reflectivity: f32,
    pub transmission: f32,
    pub ior: f32,
    pub emission: f32,
    pub pattern: Pattern,
    pub secondary_color: [f32; 3],
    pub pattern_scale: f32,
    pub unlit: bool,
}
impl Default for Material {
    fn default() -> Self {
        Self {
            color: [0.72, 0.70, 0.66],
            roughness: 0.55,
            metallic: 0.,
            reflectivity: 0.05,
            transmission: 0.,
            ior: 1.5,
            emission: 0.,
            pattern: Pattern::Solid,
            secondary_color: [0.04; 3],
            pattern_scale: 3.,
            unlit: false,
        }
    }
}
pub const MAX_OBJECTS: usize = 256;
pub const MAX_LIGHTS: usize = 32;
/// DLSS Super Resolution presets offered by SDK 310.9.16 (`NVSDK_NGX_DLSS_Hint_Render_Preset`).
/// A–D are removed, E/F deprecated, and G–I, N, O revert to the default.
pub const DLSS_PRESETS: [(u32, &str); 5] = [
    (0, "NVIDIA default"),
    (10, "J · transformer, less ghosting"),
    (11, "K · transformer, best quality"),
    (12, "L · ultra performance"),
    (13, "M · performance"),
];
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    #[default]
    Box,
    Sphere,
    Cylinder,
    Cone,
    Torus,
    Knot,
    Teapot,
    Bunny,
    Octahedron,
    Pawn,
    Ring,
}
impl Shape {
    pub const ALL: [Self; 11] = [
        Self::Box,
        Self::Sphere,
        Self::Cylinder,
        Self::Cone,
        Self::Torus,
        Self::Knot,
        Self::Teapot,
        Self::Bunny,
        Self::Octahedron,
        Self::Pawn,
        Self::Ring,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Box => "Box",
            Self::Sphere => "Sphere",
            Self::Cylinder => "Cylinder",
            Self::Cone => "Cone",
            Self::Torus => "Torus",
            Self::Knot => "Trefoil knot",
            Self::Teapot => "Utah teapot",
            Self::Bunny => "Stanford bunny",
            Self::Octahedron => "Octahedron",
            Self::Pawn => "Chess pawn",
            Self::Ring => "Thin ring",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Pattern {
    #[default]
    Solid,
    Checker,
    Stripes,
    Marble,
}
/// How the native renderer reaches output resolution when Ray Reconstruction is off and the
/// render scale is below 1. The temporal upscalers fall back to bicubic when unavailable.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Upscaler {
    Bilinear,
    Bicubic,
    #[default]
    Dlss,
    Fsr,
    Xess,
}
impl Upscaler {
    pub const ALL: [Self; 5] = [
        Self::Dlss,
        Self::Fsr,
        Self::Xess,
        Self::Bicubic,
        Self::Bilinear,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Bilinear => "Bilinear",
            Self::Bicubic => "Bicubic (Catmull-Rom)",
            Self::Dlss => "NVIDIA DLSS Super Resolution",
            Self::Fsr => "AMD FSR",
            Self::Xess => "Intel XeSS",
        }
    }
    pub fn temporal(self) -> bool {
        matches!(self, Self::Dlss | Self::Fsr | Self::Xess)
    }
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AreaLight {
    pub name: String,
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub size: [f32; 2],
    pub color: [f32; 3],
    pub intensity: f32,
    pub enabled: bool,
    pub visible: bool,
}
impl Default for AreaLight {
    fn default() -> Self {
        Self {
            name: "Softbox".into(),
            position: [0., 3., 0.],
            rotation: [0.; 3],
            size: [2., 2.],
            color: [1.; 3],
            intensity: 12.,
            enabled: true,
            visible: true,
        }
    }
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Object {
    pub name: String,
    pub shape: Shape,
    pub sphere: bool,
    pub visible: bool,
    pub position: [f32; 2],
    pub size: f32,
    pub stretch: f32,
    pub rotation: f32,
    pub tilt: [f32; 2],
    pub scale_xz: [f32; 2],
    pub elevation: f32,
    pub grounded: bool,
    pub material: Material,
}
impl Default for Object {
    fn default() -> Self {
        Self {
            name: "Object".into(),
            shape: Shape::Box,
            visible: true,
            tilt: [0.; 2],
            scale_xz: [1.; 2],
            elevation: 0.,
            grounded: true,
            sphere: false,
            position: [-0.42, 0.05],
            size: 1.,
            stretch: 1.7,
            rotation: 18.,
            material: Material::default(),
        }
    }
}
impl Object {
    pub fn kind(&self) -> Shape {
        if self.sphere {
            Shape::Sphere
        } else {
            self.shape
        }
    }
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub objects: Vec<Object>,
    pub lights: Vec<AreaLight>,
    pub scene_name: String,
    pub cornell_room: bool,
    pub floor_extent: f32,
    pub sky: f32,
    pub left: [f32; 3],
    pub right: [f32; 3],
    pub base: [f32; 3],
    pub background: [f32; 3],
    pub walls: Material,
    pub floor: Material,
    pub light_on: bool,
    pub light_visible: bool,
    pub light_intensity: f32,
    pub light_size: f32,
    pub light_aspect: f32,
    pub light_position: [f32; 3],
    pub light_temperature: f32,
    pub light_tint: [f32; 3],
    pub glass_shadow: bool,
    pub bounces: u32,
    pub spp: u32,
    pub max_samples: u32,
    pub scale: f32,
    pub firefly: f32,
    pub sobol: bool,
    pub exposure: f32,
    pub gamma: f32,
    pub tonemap: u32,
    pub view: u32,
    pub direct: f32,
    pub indirect: f32,
    pub denoise: bool,
    pub denoise_strength: f32,
    pub rr: bool,
    pub rr_preset: u32,
    pub upscaler: Upscaler,
    /// DLSS Super Resolution preset: 0 = NVIDIA's choice per quality mode, 10–13 = J, K, L, M.
    pub dlss_preset: u32,
    /// At 100% resolution, run the temporal upscaler as anti-aliasing (DLAA / Native AA)
    /// instead of the progressive native path.
    pub native_aa: bool,
    /// Display output: 0 follows Windows HDR, 1 always SDR, 2 forces the scRGB HDR path.
    pub hdr_mode: u32,
    /// HDR peak and paper white in nits; 0 takes the values Windows reports.
    pub hdr_peak: f32,
    pub hdr_white: f32,
    pub temporal: bool,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub target: [f32; 3],
    pub fov: f32,
    pub rotate: bool,
    pub rotate_speed: f32,
    pub compare: bool,
    pub split: f32,
    pub vsync: bool,
}
impl Default for Settings {
    fn default() -> Self {
        let b = Object {
            position: [0.38, -0.38],
            size: 0.62,
            stretch: 0.62,
            rotation: -14.,
            ..Default::default()
        };
        let wall = Material {
            roughness: 0.85,
            reflectivity: 0.,
            ..Default::default()
        };
        Self {
            objects: vec![
                Object {
                    name: "Tall block".into(),
                    ..Object::default()
                },
                Object {
                    name: "Short block".into(),
                    ..b
                },
            ],
            lights: vec![],
            scene_name: "Classic Cornell box".into(),
            cornell_room: true,
            floor_extent: 1.,
            sky: 0.,
            left: [0.527, 0.051, 0.042],
            right: [0.050, 0.275, 0.060],
            base: [0.73; 3],
            background: [0.003, 0.004, 0.006],
            walls: wall.clone(),
            floor: wall,
            light_on: true,
            light_visible: true,
            light_intensity: 12.,
            light_size: 0.75,
            light_aspect: 1.,
            light_position: [0., 0.965, 0.15],
            light_temperature: 5200.,
            light_tint: [1.; 3],
            glass_shadow: true,
            bounces: 6,
            spp: 4,
            max_samples: 4096,
            scale: 1.,
            firefly: 0.,
            sobol: true,
            exposure: 1.,
            gamma: 2.2,
            tonemap: 3,
            view: 0,
            direct: 1.,
            indirect: 1.,
            denoise: true,
            denoise_strength: 0.5,
            rr: true,
            rr_preset: 6,
            upscaler: Upscaler::Dlss,
            dlss_preset: 0,
            native_aa: false,
            hdr_mode: 0,
            hdr_peak: 0.,
            hdr_white: 0.,
            temporal: true,
            yaw: 0.,
            pitch: 4.,
            distance: 4.7,
            target: [0., 0., -0.15],
            fov: 36.,
            rotate: false,
            rotate_speed: 6.,
            compare: false,
            split: 0.5,
            vsync: true,
        }
    }
}
pub const PRESETS: [&str; 24] = [
    "Classic Cornell box",
    "Reflective spheres",
    "Sphere and box",
    "Mirror + glass",
    "Mirror",
    "Glass",
    "Warm lighting",
    "Dramatic lighting",
    "Emissive spheres",
    "Material matrix",
    "Weekend sphere field",
    "Whitted checkerboard",
    "Utah teapot studio",
    "Stanford bunny studio",
    "Glass laboratory",
    "RGB shadow study",
    "Sphere pyramid",
    "Mirror corridor",
    "Knot gallery",
    "Column courtyard",
    "Chess still life",
    "Neon sculpture",
    "Orbital sculpture",
    "Build your own studio",
];
impl Settings {
    pub fn randomize(&mut self, mut seed: u32) {
        let mut rand = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed >> 8) as f32 / 16777216.
        };
        // Any study except the empty stage. Cornell blocks are reshuffled inside
        // the box; larger studies keep their layout and only change colors.
        let id = ((rand() * (PRESETS.len() - 1) as f32) as usize).min(PRESETS.len() - 2);
        self.preset(id);
        for (i, o) in self.objects.iter_mut().enumerate() {
            let m = &mut o.material;
            if id < 9 {
                o.size = 0.45 + rand() * 0.65;
                o.stretch = 0.6 + rand() * 1.2;
                o.rotation = rand() * 90. - 45.;
                o.position = [
                    if i == 0 {
                        -0.55 + rand() * 0.25
                    } else {
                        0.25 + rand() * 0.3
                    },
                    rand() * 0.7 - 0.4,
                ];
                m.color = std::array::from_fn(|_| 0.2 + rand() * 0.75);
            } else if m.metallic < 0.5 && m.pattern == Pattern::Solid && m.emission == 0. {
                // Diffuse and glass objects; metals, emitters and patterns keep their look.
                let floor = if m.transmission > 0.5 { 0.55 } else { 0.03 };
                m.color = std::array::from_fn(|_| floor + rand() * (0.95 - floor));
            }
        }
        self.light_temperature = 2800. + rand() * 5200.;
    }
    /// Takes the viewer's render and display choices from `from` (used when a scene file
    /// replaces the scene): resolution, RR and its model, upscaler, DLSS preset, DLAA, VSync, HDR.
    pub fn keep_output(&mut self, from: &Settings) {
        self.scale = from.scale;
        self.rr = from.rr;
        self.vsync = from.vsync;
        self.rr_preset = from.rr_preset;
        self.upscaler = from.upscaler;
        self.dlss_preset = from.dlss_preset;
        self.native_aa = from.native_aa;
        self.hdr_mode = from.hdr_mode;
        self.hdr_peak = from.hdr_peak;
        self.hdr_white = from.hdr_white;
    }
    pub fn preset(&mut self, id: usize) {
        // Presets 0–8 share the Cornell box, so moving between them keeps the
        // user's camera. Any other switch takes the preset's own camera.
        let keep_camera = id < 9 && PRESETS[..9].contains(&self.scene_name.as_str());
        let camera = (self.yaw, self.pitch, self.distance, self.target, self.fov);
        let output = (
            self.scale,
            self.rr,
            self.vsync,
            self.rr_preset,
            self.upscaler,
            (self.dlss_preset, self.native_aa),
            (self.hdr_mode, self.hdr_peak, self.hdr_white),
        );
        *self = Self::default();
        match id {
            1 => {
                self.objects[0].sphere = true;
                self.objects[0].size = 0.95;
                self.objects[1].sphere = true;
                self.objects[1].size = 0.55;
                for o in &mut self.objects {
                    o.material.metallic = 1.;
                    o.material.reflectivity = 1.;
                    o.material.roughness = 0.03;
                }
                self.objects[1].material.color = [0.79, 0.58, 0.36];
                self.bounces = 8;
            }
            2 => {
                self.objects[0].sphere = true;
                self.objects[0].size = 0.95;
                self.objects[1].stretch = 1.65;
            }
            3 => {
                self.objects[0].sphere = true;
                self.objects[0].size = 0.9;
                self.objects[0].material.metallic = 1.;
                self.objects[0].material.roughness = 0.02;
                self.objects[1].stretch = 1.;
                self.objects[1].size = 0.75;
                self.objects[1].material.transmission = 1.;
                self.objects[1].material.roughness = 0.05;
                self.objects[1].material.color = [0.82, 0.9, 1.];
                self.bounces = 10;
            }
            4 => {
                for o in &mut self.objects {
                    o.material.metallic = 1.;
                    o.material.roughness = 0.02;
                    o.material.color = [0.9; 3];
                }
                self.objects[0].rotation = 40.;
                self.objects[1].rotation = -25.;
                self.bounces = 10;
            }
            5 => {
                self.objects[0].sphere = true;
                for o in &mut self.objects {
                    o.material.transmission = 1.;
                    o.material.roughness = 0.;
                    o.material.color = [0.88, 0.96, 0.93];
                }
                self.bounces = 12;
            }
            6 => {
                self.light_temperature = 3200.;
                self.light_intensity = 18.;
                self.light_size = 0.9;
                self.exposure = 1.15;
                self.tonemap = 4;
            }
            7 => {
                self.light_intensity = 26.;
                self.light_size = 0.35;
                self.light_temperature = 4300.;
                self.objects[1].material.metallic = 1.;
                self.objects[1].material.roughness = 0.15;
                self.bounces = 8;
                self.tonemap = 5;
            }
            8 => {
                self.objects[0].sphere = true;
                self.objects[1].sphere = true;
                self.objects[0].size = 0.55;
                self.objects[1].size = 0.85;
                self.objects[0].material.color = [1., 0.62, 0.2];
                self.objects[0].material.emission = 6.;
                self.objects[1].material.color = [0.21, 0.69, 1.];
                self.objects[1].material.transmission = 0.55;
                self.light_intensity = 3.;
                self.bounces = 7;
            }
            _ => {
                if id >= 9 {
                    crate::presets::apply(self, id);
                }
            }
        }
        if keep_camera {
            (self.yaw, self.pitch, self.distance, self.target, self.fov) = camera;
        }
        self.scene_name = PRESETS.get(id).unwrap_or(&PRESETS[0]).to_string();
        for o in &mut self.objects {
            if o.sphere {
                o.sphere = false;
                o.shape = Shape::Sphere;
                o.stretch = 1.;
            }
        }
        (
            self.scale,
            self.rr,
            self.vsync,
            self.rr_preset,
            self.upscaler,
            (self.dlss_preset, self.native_aa),
            (self.hdr_mode, self.hdr_peak, self.hdr_white),
        ) = output;
    }
    pub fn validate(&mut self) -> anyhow::Result<()> {
        for o in &mut self.objects {
            if o.sphere {
                o.sphere = false;
                o.shape = Shape::Sphere;
                o.stretch = 1.;
            }
        }
        anyhow::ensure!(
            self.objects.len() <= MAX_OBJECTS && self.lights.len() <= MAX_LIGHTS,
            "Scene exceeds object/light limit"
        );
        anyhow::ensure!(
            (0.5..=40.).contains(&self.floor_extent) && (0. ..=4.).contains(&self.sky),
            "Invalid environment"
        );
        anyhow::ensure!(self.scene_name.len() <= 256, "Scene name is too long");
        anyhow::ensure!(
            [0, 4, 5, 6].contains(&self.rr_preset),
            "Invalid DLSS RR model preset"
        );
        anyhow::ensure!(
            DLSS_PRESETS.iter().any(|(id, _)| *id == self.dlss_preset),
            "Invalid DLSS Super Resolution preset"
        );
        anyhow::ensure!(
            self.hdr_mode < 3
                && (self.hdr_peak == 0. || (80. ..=10000.).contains(&self.hdr_peak))
                && (self.hdr_white == 0. || (40. ..=1000.).contains(&self.hdr_white)),
            "Invalid HDR output settings"
        );
        // serde_json rejects nonfinite values. Bound every value used for resource sizes or transport.
        anyhow::ensure!(
            (0.25..=1.).contains(&self.scale),
            "Resolution scale must be 0.25–1"
        );
        anyhow::ensure!(
            (1..=32).contains(&self.bounces)
                && (1..=16).contains(&self.spp)
                && (1..=1_000_000).contains(&self.max_samples),
            "Invalid sample/bounce limits"
        );
        anyhow::ensure!(
            (10. ..=100.).contains(&self.fov)
                && (0.5..=200.).contains(&self.distance)
                && self.pitch.abs() <= 89.,
            "Invalid camera"
        );
        anyhow::ensure!(
            (0.05..=40.).contains(&self.light_size)
                && (0.1..=10.).contains(&self.light_aspect)
                && (1000. ..=12000.).contains(&self.light_temperature)
                && (0. ..=2000.).contains(&self.light_intensity),
            "Invalid light"
        );
        anyhow::ensure!(
            (0.1..=4.).contains(&self.gamma)
                && (0. ..=10.).contains(&self.exposure)
                && self.tonemap < 6
                && self.view < 8,
            "Invalid display settings"
        );
        anyhow::ensure!(
            (0. ..=4.).contains(&self.direct)
                && (0. ..=4.).contains(&self.indirect)
                && (0. ..=1.).contains(&self.denoise_strength)
                && (0. ..=100.).contains(&self.firefly)
                && (0. ..=1.).contains(&self.split),
            "Invalid filter settings"
        );
        for c in [
            &self.left,
            &self.right,
            &self.base,
            &self.background,
            &self.light_tint,
        ] {
            anyhow::ensure!(c.iter().all(|v| (0. ..=1.).contains(v)), "Invalid color");
        }
        for m in self
            .objects
            .iter()
            .map(|o| &o.material)
            .chain([&self.walls, &self.floor])
        {
            anyhow::ensure!(
                m.color.iter().all(|v| (0. ..=1.).contains(v))
                    && [m.roughness, m.metallic, m.reflectivity, m.transmission]
                        .iter()
                        .all(|v| (0. ..=1.).contains(v))
                    && (1. ..=2.5).contains(&m.ior)
                    && (0. ..=20.).contains(&m.emission),
                "Invalid material"
            );
            anyhow::ensure!(
                m.secondary_color.iter().all(|v| (0. ..=1.).contains(v))
                    && (0.1..=100.).contains(&m.pattern_scale),
                "Invalid material pattern"
            );
        }
        for o in &self.objects {
            anyhow::ensure!(
                (0.05..=30.).contains(&o.size)
                    && (0.05..=10.).contains(&o.stretch)
                    && o.rotation.abs() <= 360.
                    && o.position.iter().all(|v| v.abs() <= 40.)
                    && o.elevation.abs() <= 40.
                    && o.tilt.iter().all(|v| v.abs() <= 360.)
                    && o.scale_xz.iter().all(|v| (0.01..=20.).contains(v))
                    && o.name.len() <= 128,
                "Invalid object"
            );
        }
        anyhow::ensure!(
            self.target.iter().all(|v| v.abs() <= 20.)
                && self.yaw.abs() <= 1e6
                && self.light_position.iter().all(|v| v.abs() <= 50.)
                && self.rotate_speed.abs() <= 60.,
            "Invalid scene coordinates"
        );
        for l in &self.lights {
            anyhow::ensure!(
                l.name.len() <= 128
                    && l.position.iter().all(|v| v.abs() <= 50.)
                    && l.rotation.iter().all(|v| v.abs() <= 360.)
                    && l.size.iter().all(|v| (0.05..=40.).contains(v))
                    && l.color.iter().all(|v| (0. ..=1.).contains(v))
                    && (0. ..=2000.).contains(&l.intensity),
                "Invalid area light"
            );
        }
        Ok(())
    }
    pub fn camera(&self, aspect: f32) -> (Mat4, Mat4, Vec3) {
        let (sy, cy) = self.yaw.to_radians().sin_cos();
        let (sp, cp) = self.pitch.to_radians().sin_cos();
        let target = Vec3::from(self.target);
        let eye = target + Vec3::new(sy * cp, sp, cy * cp) * self.distance;
        let view = Mat4::look_at_rh(eye, target, Vec3::Y);
        let proj = Mat4::perspective_rh(self.fov.to_radians(), aspect, 0.01, 500.);
        (view, proj, eye)
    }
    pub fn light_color(&self) -> [f32; 3] {
        let t = self.light_temperature / 100.;
        let c = [
            if t <= 66. {
                255.
            } else {
                329.69873 * (t - 60.).powf(-0.13320476)
            },
            if t <= 66. {
                99.4708 * t.ln() - 161.11957
            } else {
                288.12216 * (t - 60.).powf(-0.07551485)
            },
            if t >= 66. {
                255.
            } else if t <= 19. {
                0.
            } else {
                138.51773 * (t - 10.).ln() - 305.0448
            },
        ]
        .map(|v| {
            let v = v.clamp(0., 255.) / 255.;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        });
        let lum = c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722;
        std::array::from_fn(|i| {
            c[i] / lum
                * self.light_tint[i]
                * if self.light_on {
                    self.light_intensity
                } else {
                    0.
                }
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub material: u32,
    pub normal: [f32; 3],
    pub pad: u32,
}
pub fn geometry(s: &Settings) -> Vec<Vertex> {
    crate::geometry::geometry(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn presets_are_valid() {
        for i in 0..PRESETS.len() {
            let mut s = Settings::default();
            s.preset(i);
            s.validate().unwrap_or_else(|e| panic!("Preset {i}: {e}"));
            let g = geometry(&s);
            assert_eq!(g.len() % 3, 0);
            assert!(g.iter().all(
                |v| Vec3::from(v.position).is_finite() && Vec3::from(v.normal).length() > 0.99
            ));
        }
    }
    #[test]
    fn randomized_scenes_are_valid() {
        let mut names = std::collections::HashSet::new();
        for seed in 1..400u32 {
            let mut s = Settings::default();
            s.randomize(seed.wrapping_mul(2654435761) | 1);
            s.validate()
                .unwrap_or_else(|e| panic!("Seed {seed} ({}): {e}", s.scene_name));
            names.insert(s.scene_name);
        }
        assert!(names.len() > 15, "Randomize only reached {names:?}");
        assert!(!names.contains(PRESETS[23]));
    }
    #[test]
    fn camera_carries_over_only_inside_the_cornell_box() {
        let mut s = Settings::default();
        s.distance = 3.;
        s.preset(4);
        assert_eq!(s.distance, 3.);
        s.preset(12);
        assert_eq!(s.distance, 9.);
        s.preset(0);
        assert_eq!(s.distance, Settings::default().distance);
    }
    #[test]
    fn reject_bad_resources() {
        let mut s = Settings::default();
        s.scale = 0.;
        assert!(s.validate().is_err());
        s = Settings::default();
        s.spp = 0;
        assert!(s.validate().is_err());
    }
}
