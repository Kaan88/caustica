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
    /// Use the mesh's texture maps (core track 5). The colour, roughness and metallic values
    /// are then the map averages, so the maps are scaled by value / average: editing a value
    /// tints the map, and switching maps off shows the averages.
    pub textures: bool,
    /// A texture set (core track 5, `texture.rs`): an image, or a folder of maps such as an
    /// ambientCG download. Box-mapped, so it works on shapes, meshes, the floor and the walls.
    /// Empty = none.
    pub texture: String,
    /// Repeats of the texture set per scene unit (the Cornell box is 2 wide).
    pub texture_scale: f32,
    /// Nested dielectrics (track 10): where transmissive objects overlap, the higher priority
    /// owns the overlap (glass 2 over the water 1 that reaches into its wall). 0–7 (3 bits
    /// of the shader's medium stack).
    #[serde(skip_serializing_if = "is_zero")]
    pub priority: u32,
    /// Layers (track 11, as glTF's KHR_materials_clearcoat and _sheen): a clear coat (IOR 1.5)
    /// over everything, strength 0–1, with its own roughness.
    #[serde(skip_serializing_if = "is_zero_f32")]
    pub clearcoat: f32,
    #[serde(skip_serializing_if = "is_default_coat_roughness")]
    pub clearcoat_roughness: f32,
    /// A sheen (velvet, cloth, dust) over the base: strength 0–1 times a colour, with its own
    /// roughness (a wide rim at 1, a thin grazing one near 0).
    #[serde(skip_serializing_if = "is_zero_f32")]
    pub sheen: f32,
    #[serde(skip_serializing_if = "is_white")]
    pub sheen_color: [f32; 3],
    #[serde(skip_serializing_if = "is_default_sheen_roughness")]
    pub sheen_roughness: f32,
    /// Anisotropy (track 11, brushed metal): 0–1 raises the roughness across the brushing
    /// direction (glTF's KHR_materials_anisotropy strength), so highlights streak across it.
    #[serde(skip_serializing_if = "is_zero_f32")]
    pub anisotropy: f32,
    /// The brushing direction in degrees about the normal (−180..180), from the object's x axis.
    #[serde(skip_serializing_if = "is_zero_f32")]
    pub anisotropy_rotation: f32,
    /// Brushed in circles around the object's y axis (a pan, a watch dial, a record) instead.
    #[serde(skip_serializing_if = "is_false")]
    pub anisotropy_spun: bool,
    /// Beam splitter (track 17): a thin partial mirror. A ray reflects with probability
    /// `reflectivity` and otherwise passes straight through; both are tinted by `color`.
    #[serde(skip_serializing_if = "is_false")]
    pub splitter: bool,
    /// Thin-film interference (track 11, glTF's KHR_materials_iridescence): a film 0–1 strong
    /// on the surface (soap, oil, anodized titanium) whose reflections take colours from its
    /// thickness (nanometres, 0–2000) and index of refraction (1–2.5).
    #[serde(skip_serializing_if = "is_zero_f32")]
    pub iridescence: f32,
    #[serde(skip_serializing_if = "is_default_film_thickness")]
    pub iridescence_thickness: f32,
    #[serde(skip_serializing_if = "is_default_film_ior")]
    pub iridescence_ior: f32,
    /// Diffraction grating (track 11, a CD or DVD): the share 0–1 of the specular reflection
    /// spread into the first and second spectral orders across the grooves, which run along the
    /// brush direction (`anisotropy_rotation`, `anisotropy_spun`); spacing in µm (0.3–10).
    #[serde(skip_serializing_if = "is_zero_f32")]
    pub grating: f32,
    #[serde(skip_serializing_if = "is_default_grating_spacing")]
    pub grating_spacing: f32,
    /// Volume (track 12): a transmissive object filled with a scattering medium (smoke, milk,
    /// jade). `density` is the extinction per unit length (0 = clear), `scatter_color` the
    /// single-scattering albedo, `phase` the Henyey-Greenstein g (−0.9 back, 0.9 forward).
    #[serde(skip_serializing_if = "is_zero_f32")]
    pub density: f32,
    #[serde(skip_serializing_if = "is_white")]
    pub scatter_color: [f32; 3],
    #[serde(skip_serializing_if = "is_zero_f32")]
    pub phase: f32,
}
/// Global fog (track 12): a homogeneous medium filling a box from the floor (y = −1) up to
/// `height`, `extent` units out from the centre on x and z.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Fog {
    pub density: f32,
    pub color: [f32; 3],
    pub phase: f32,
    pub height: f32,
    pub extent: f32,
}
impl Default for Fog {
    fn default() -> Self {
        Self {
            density: 0.,
            color: [1.; 3],
            phase: 0.,
            height: 3.,
            extent: 6.,
        }
    }
}
impl Fog {
    pub fn is_default(&self) -> bool {
        self.density == 0.
    }
}
fn is_false(v: &bool) -> bool {
    !*v
}
fn is_zero(v: &u32) -> bool {
    *v == 0
}
fn one() -> f32 {
    1.
}
fn is_one(v: &f32) -> bool {
    *v == 1.
}
fn is_zero_f32(v: &f32) -> bool {
    *v == 0.
}
fn is_white(v: &[f32; 3]) -> bool {
    *v == [1.; 3]
}
fn is_default_coat_roughness(v: &f32) -> bool {
    *v == Material::default().clearcoat_roughness
}
fn is_default_sheen_roughness(v: &f32) -> bool {
    *v == Material::default().sheen_roughness
}
fn is_default_film_thickness(v: &f32) -> bool {
    *v == Material::default().iridescence_thickness
}
fn is_default_film_ior(v: &f32) -> bool {
    *v == Material::default().iridescence_ior
}
fn is_default_grating_spacing(v: &f32) -> bool {
    *v == Material::default().grating_spacing
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
            textures: true,
            texture: String::new(),
            texture_scale: 0.5,
            priority: 0,
            clearcoat: 0.,
            clearcoat_roughness: 0.03,
            sheen: 0.,
            sheen_color: [1.; 3],
            sheen_roughness: 0.5,
            anisotropy: 0.,
            anisotropy_rotation: 0.,
            anisotropy_spun: false,
            splitter: false,
            iridescence: 0.,
            iridescence_thickness: 400.,
            iridescence_ior: 1.3,
            grating: 0.,
            grating_spacing: 1.6,
            density: 0.,
            scatter_color: [1.; 3],
            phase: 0.,
        }
    }
}
/// Objects are TLAS instances of shared mesh BLASes (core track 4), so the limit is about UI
/// and settings size, not the BLAS.
pub const MAX_OBJECTS: usize = 4096;
pub const MAX_LIGHTS: usize = 32;
/// DLSS Super Resolution presets offered by SDK 310.9.16 (`NVSDK_NGX_DLSS_Hint_Render_Preset`).
/// A–D are removed, E/F deprecated, and G–I, N, O revert to the default.
/// Pixel reconstruction filters of the progressive path (`Settings::pixel_filter`).
pub const PIXEL_FILTERS: [&str; 3] = ["Box", "Gaussian", "Blackman-Harris"];
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
    Tetrahedron,
    /// Optics bench (track 17): a biconvex lens, its axis along local z, standing on its rim.
    ConvexLens,
    /// A biconcave lens, axis along local z.
    ConcaveLens,
    /// An equilateral triangular prism, extruded along local z, apex up.
    Prism,
}
impl Shape {
    pub const ALL: [Self; 15] = [
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
        Self::Tetrahedron,
        Self::ConvexLens,
        Self::ConcaveLens,
        Self::Prism,
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
            Self::Tetrahedron => "Tetrahedron",
            Self::ConvexLens => "Convex lens",
            Self::ConcaveLens => "Concave lens",
            Self::Prism => "Prism",
        }
    }
}
/// What an extra light is (track 7). The flat kinds face their local −Y, as the rectangle does.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LightKind {
    /// A rectangle of `size`, sampled by area.
    #[default]
    Rect,
    /// An ellipse inscribed in `size`.
    Disk,
    /// A sphere of diameter `size[0]`, sampled by the solid angle it covers.
    Sphere,
    /// A disk whose emission falls off outside a cone (`spread`, `blend`).
    Spot,
    /// A distant disk of angular diameter `angle`, sampled by direction; position only places
    /// its handle.
    Sun,
}
impl LightKind {
    pub const ALL: [Self; 5] = [Self::Rect, Self::Disk, Self::Sphere, Self::Spot, Self::Sun];
    pub fn label(self) -> &'static str {
        match self {
            Self::Rect => "Rectangle",
            Self::Disk => "Disk",
            Self::Sphere => "Sphere",
            Self::Spot => "Spot",
            Self::Sun => "Sun",
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
    /// Draw the light's rectangle over the viewport (for grabbing it); the selected light
    /// always shows it. Display only.
    pub outline: bool,
    pub kind: LightKind,
    /// Spot: full cone angle in degrees.
    pub spread: f32,
    /// Spot: share of the cone over which the edge fades (0 hard, 1 from the axis).
    pub blend: f32,
    /// Sun: angular diameter in degrees (the real sun is 0.53).
    pub angle: f32,
}
impl AreaLight {
    /// The direction the light faces (a sun's light travels along it).
    pub fn direction(&self) -> glam::Vec3 {
        let r = glam::Mat3::from_euler(
            glam::EulerRot::YXZ,
            self.rotation[1].to_radians(),
            self.rotation[0].to_radians(),
            self.rotation[2].to_radians(),
        );
        r * -glam::Vec3::Y
    }
    /// Emitted radiance per unit colour. A rectangle's intensity is its radiance; disks, spots
    /// and spheres shine as a 1 × 1 rectangle of that intensity seen face on, whatever their
    /// size; a sun of intensity 1 lights a white surface facing it to 1.
    pub fn radiance(&self) -> f32 {
        let pi = std::f32::consts::PI;
        match self.kind {
            LightKind::Rect => self.intensity,
            LightKind::Disk | LightKind::Spot => {
                self.intensity / (pi * 0.25 * self.size[0] * self.size[1])
            }
            LightKind::Sphere => self.intensity / (pi * 0.25 * self.size[0] * self.size[0]),
            LightKind::Sun => {
                let s = (self.angle * 0.5).to_radians().sin();
                self.intensity / (s * s)
            }
        }
    }
    /// Spot: cosines of the cone's outer and inner half angles.
    pub fn cone(&self) -> (f32, f32) {
        let outer = (self.spread * 0.5).to_radians();
        // A hard edge still needs inner > outer for smoothstep.
        let inner = (outer * (1. - self.blend)).cos().max(outer.cos() + 1e-5);
        (outer.cos(), inner)
    }
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
            outline: false,
            kind: LightKind::Rect,
            spread: 60.,
            blend: 0.25,
            angle: 0.53,
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
    /// Index of the paired portal object (roadmap 5d), -1 for none. Rays that hit this object
    /// continue from the matching point of the pair.
    pub portal: i32,
    /// An imported mesh file (OBJ, glTF, GLB) used instead of `shape`; relative paths are
    /// relative to the scenes folder. Empty for built-in shapes.
    pub mesh: String,
    /// Track 13: rendered as an exact procedural surface instead of a mesh (`shape` is then
    /// ignored; `kind()` gives the proxy mesh used for picking, framing and grounding).
    #[serde(skip_serializing_if = "Sdf::is_none")]
    pub sdf: Sdf,
    /// Animation phase of a procedural surface (metaballs: blob positions); keyframe it.
    /// Always serialized: keyframes and the I key find fields in the scene's JSON.
    #[serde(default)]
    pub phase: f32,
    /// Track 17 physics: how the sandbox simulates this object.
    #[serde(skip_serializing_if = "Body::is_dynamic")]
    pub body: Body,
    /// Track 17: density multiplier of the body (1: water-like default).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub mass: f32,
    /// Track 17 laser puzzle: the gizmo can't move it (walls, the target, fixed parts).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
    /// Track 17 physics: this object is a string (a cylinder) from the fixed point `from` to
    /// the centre of object `object`. The simulation redraws it every step, and the strings
    /// pointing at a body become its joint (one: a ball joint, two or more: a hinge through
    /// the first two points). Strings have no collider.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tether: Option<Tether>,
    /// Track 17 physics: ball joints from this body to other bodies, each at a world point
    /// (taken as body-local anchors when the simulation starts). Chain links, rope segments
    /// and bridge planks hang from each other this way.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    /// Track 17 physics: this object is a cloth (a particle grid over the object's local x-z
    /// square, simulated by `physics::Cloth`, drawn as its mesh). Never a rigid body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloth: Option<Cloth>,
    /// The texture maps of another mesh part (`file.gltf#part`) for an object without its own
    /// UV-mapped file: a cloth showing a model's fabric (sponza-cloth's curtains). Used with
    /// `material.textures`, like a mesh's own maps. Empty = none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub maps_from: String,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Link {
    pub object: u32,
    pub at: [f32; 3],
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClothPin {
    /// Falls freely (drapes over what is below).
    #[default]
    None,
    /// The two corners of the far edge (-z) hang from where they start.
    Corners,
    /// The whole far edge (a curtain or flag).
    Edge,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Cloth {
    /// Particles per side (4–48).
    pub resolution: u32,
    pub pin: ClothPin,
    /// Bending stiffness 0–1 (0 silk, 1 stiff canvas).
    pub stiffness: f32,
    /// Texture coordinates of the grid: u0, v0 at the first corner of the pinned (−z) edge,
    /// u1, v1 at the opposite corner (u along the edge, v away from it), for `maps_from`.
    pub uv: [f32; 4],
}
impl Default for Cloth {
    fn default() -> Self {
        Self {
            resolution: 24,
            pin: ClothPin::None,
            stiffness: 0.2,
            uv: [0., 0., 1., 1.],
        }
    }
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Body {
    /// Falls, collides and can be thrown.
    #[default]
    Dynamic,
    /// Collides but never moves (a frame, a table, a wall).
    Fixed,
    /// Not simulated at all (no collider).
    None,
}
impl Body {
    pub fn is_dynamic(&self) -> bool {
        *self == Self::Dynamic
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Tether {
    pub object: u32,
    pub from: [f32; 3],
}
/// Track 17 physics sandbox. `enabled` runs the simulation; a scene file that sets it opens
/// with the bodies live (stacks settle, then sleep until something touches them).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Physics {
    pub enabled: bool,
    /// Downward acceleration in scene units/s² (negative pulls up).
    pub gravity: f32,
    /// Simulation speed (1 = real time, below 1 is slow motion, 0 pauses).
    pub speed: f32,
    /// Restitution of every body.
    pub bounce: f32,
    pub friction: f32,
    /// The direction gravity pulls (normalized when used; default straight down).
    pub down: [f32; 3],
    /// Wind as an acceleration, scene units/s² (every dynamic body, by its mass).
    pub wind: [f32; 3],
    /// Gust strength, 0 (steady) to 1 (the wind drops to nothing and doubles).
    pub gusts: f32,
}
impl Default for Physics {
    fn default() -> Self {
        Self {
            enabled: false,
            gravity: 9.81,
            speed: 1.,
            bounce: 0.15,
            friction: 0.7,
            down: [0., -1., 0.],
            wind: [0.; 3],
            gusts: 0.,
        }
    }
}
impl Physics {
    pub fn is_off(&self) -> bool {
        *self == Self::default()
    }
}
/// Track 19 audio-reactive lights: what the PC plays scales the lights and emitters of each
/// rendered frame (see audio.rs). The scene's own intensities are never changed.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AudioReact {
    pub enabled: bool,
    /// Added intensity at full level (×, on top of `floor`).
    pub gain: f32,
    /// Intensity at silence (×).
    pub floor: f32,
    /// How much of the last level each frame keeps (at 60 fps), 0 = none.
    pub smoothing: f32,
}
impl Default for AudioReact {
    fn default() -> Self {
        Self {
            enabled: false,
            gain: 1.5,
            floor: 0.3,
            smoothing: 0.6,
        }
    }
}
impl AudioReact {
    pub fn is_off(&self) -> bool {
        *self == Self::default()
    }
}
/// Track 17 laser puzzle: a laser from `origin` along `direction`, routed by mirrors, splitters
/// and glass to the object `target` (−1 = none) in at most `par` moves. A zero direction is
/// treated as +x.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Puzzle {
    pub enabled: bool,
    pub origin: [f32; 3],
    pub direction: [f32; 3],
    pub target: i32,
    pub par: u32,
    pub color: [f32; 3],
    /// More objects the beam must reach as well (all of them, and `target`, to solve).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<i32>,
    /// The beam also renders as thin glowing rods in the image (not only the overlay).
    pub beam: bool,
}
impl Default for Puzzle {
    fn default() -> Self {
        Self {
            enabled: false,
            origin: [-2., -0.6, 0.],
            direction: [1., 0., 0.],
            target: -1,
            par: 3,
            color: [1., 0.1, 0.1],
            targets: vec![],
            beam: false,
        }
    }
}
impl Puzzle {
    pub fn is_off(&self) -> bool {
        *self == Self::default()
    }
}
/// Procedural surfaces (track 13), intersected in the shader inside the unit cube [−1, 1]³.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Sdf {
    #[default]
    None,
    /// An exact sphere (analytic), radius 1.
    Sphere,
    /// An exact torus (sphere traced), the mesh torus's radii 0.72 and 0.28.
    Torus,
    /// The power-8 Mandelbulb, scaled into the unit sphere.
    Mandelbulb,
    /// A Menger sponge, five levels, filling the unit cube.
    Menger,
    /// Six blobs blended with a smooth minimum, moving with `Object::phase` (a lava lamp).
    Metaballs,
}
impl Sdf {
    pub const ALL: [Sdf; 6] = [
        Sdf::None,
        Sdf::Sphere,
        Sdf::Torus,
        Sdf::Mandelbulb,
        Sdf::Menger,
        Sdf::Metaballs,
    ];
    pub fn is_none(&self) -> bool {
        *self == Sdf::None
    }
    pub fn label(self) -> &'static str {
        match self {
            Sdf::None => "Mesh",
            Sdf::Sphere => "Exact sphere",
            Sdf::Torus => "Exact torus",
            Sdf::Mandelbulb => "Mandelbulb",
            Sdf::Menger => "Menger sponge",
            Sdf::Metaballs => "Metaballs",
        }
    }
    /// The shader's kind id (low 8 bits of `Instance.vertex_base` of a procedural instance);
    /// the high 24 bits carry the phase in thousandths (metaballs).
    pub fn id(self, phase: f32) -> u32 {
        self as u32 | ((((phase.rem_euclid(10000.) * 1000.) as u32) & 0xff_ffff) << 8)
    }
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
            portal: -1,
            mesh: String::new(),
            sdf: Sdf::None,
            phase: 0.,
            body: Body::Dynamic,
            mass: 1.,
            locked: false,
            tether: None,
            links: vec![],
            cloth: None,
            maps_from: String::new(),
        }
    }
}
impl Object {
    pub fn kind(&self) -> Shape {
        match self.sdf {
            Sdf::Sphere | Sdf::Mandelbulb => return Shape::Sphere,
            Sdf::Torus => return Shape::Torus,
            Sdf::Menger | Sdf::Metaballs => return Shape::Box,
            Sdf::None => {}
        }
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
    /// Draw the main light's rectangle over the viewport (see `AreaLight::outline`).
    pub light_outline: bool,
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
    /// Pixel reconstruction filter of the progressive path (RR and the temporal upscalers set
    /// their own jitter): 0 box, 1 Gaussian, 2 Blackman-Harris; `filter_width` in pixels.
    pub pixel_filter: u32,
    pub filter_width: f32,
    /// Most texture taps along a grazing footprint (1 = isotropic, up to 16).
    pub anisotropy: u32,
    /// Display output: 0 follows Windows HDR, 1 always SDR, 2 forces the scRGB HDR path.
    pub hdr_mode: u32,
    /// HDR peak and paper white in nits; 0 takes the values Windows reports.
    pub hdr_peak: f32,
    pub hdr_white: f32,
    /// HDR swapchain encoding: false scRGB (RGBA16F, Windows converts), true HDR10 (10-bit PQ / BT.2020).
    pub hdr10: bool,
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
    /// Thin lens (roadmap 5c): aperture radius in scene units (0 = pinhole), focus distance
    /// along the view axis, blade count (0 = round, 3-12), blade rotation, cat-eye strength.
    pub aperture: f32,
    pub focus_distance: f32,
    pub aperture_blades: u32,
    pub blade_rotation: f32,
    pub cat_eye: f32,
    /// Camera model: 0 perspective, 1 orthographic, 2 360° equirectangular, 3 little planet,
    /// 4 tilt-shift (perspective with the focal plane tilted by `tilt` degrees).
    pub projection: u32,
    pub tilt: f32,
    /// Track 16: a real lens system (0 = the thin lens above; 1.. = `lens::LENSES`), used with
    /// the perspective model. `f_number` stops it down (0 = wide open), `lens_scale` is metres
    /// per scene unit (sets how much depth of field the scene gets), `dispersion` scales the
    /// glass's dispersion (0 = none, 1 = real glass).
    pub lens_system: u32,
    pub f_number: f32,
    pub lens_scale: f32,
    pub dispersion: f32,
    /// Film look (display only): halation, grain and its size (0 fine, 1 medium, 2 coarse),
    /// vignette, lift / gamma / gain per channel, and a .cube LUT.
    pub halation: f32,
    pub grain: f32,
    /// Track 16 glare (display only): the aperture's diffraction pattern around highlights.
    /// Strength (0 = off), size (0 small to 1 large) and the luminance it starts above.
    pub glare: f32,
    pub glare_size: f32,
    pub glare_threshold: f32,
    pub grain_size: u32,
    pub vignette: f32,
    pub lift: [f32; 3],
    pub grade_gamma: [f32; 3],
    pub gain: [f32; 3],
    pub lut_path: String,
    pub lut_strength: f32,
    /// Light sampling (roadmap 5e): 0 MIS (both strategies), 1 light sampling (NEE) only,
    /// 2 BSDF sampling only. 1 and 2 show what MIS combines, as in Veach's plates.
    pub mis_mode: u32,
    /// ReSTIR DI (track 15): first-bounce light sampling resamples the emitter table through
    /// per-pixel reservoirs reused across frames and neighbours (see engine-notes "ReSTIR DI").
    pub restir: bool,
    /// Fresh candidates per pixel and frame (RIS from the power CDF).
    pub restir_candidates: u32,
    /// Neighbours merged by the spatial pass (0 = none).
    pub restir_neighbors: u32,
    /// Cap on the history's confidence M in the temporal merge (0 = no temporal reuse).
    pub restir_history: u32,
    /// Integrator (track 15): 0 path tracing, 1 bidirectional (BDPT), 2 progressive photon
    /// mapping (see engine-notes "BDPT and photon mapping").
    pub integrator: u32,
    /// Photon mapping: photons emitted per frame.
    pub photons: u32,
    /// Photon mapping: the starting gather radius as a share of the scene's bounding radius.
    pub photon_radius: f32,
    /// Battle mode (track 15): the image split into four quadrants rendered at equal GPU time
    /// by path tracing, path tracing with ReSTIR DI, BDPT and photon mapping.
    pub battle: bool,
    /// Track 17 physics sandbox: rigid-body simulation of the objects (see physics.rs).
    #[serde(skip_serializing_if = "Physics::is_off")]
    pub physics: Physics,
    /// Track 17 laser puzzle (see laser.rs).
    #[serde(skip_serializing_if = "Puzzle::is_off")]
    pub puzzle: Puzzle,
    /// Track 19 audio-reactive lights (see audio.rs).
    #[serde(skip_serializing_if = "AudioReact::is_off")]
    pub audio: AudioReact,
    /// Light path expressions (track 14): up to `lpe::SLOTS` regular expressions over a path's
    /// events, each accumulated into its own AOV (see lpe.rs). Empty strings are unused slots.
    pub lpe: Vec<String>,
    /// The LPE slot the "Light path (LPE)" view shows.
    pub lpe_slot: u32,
    /// An equirectangular HDRI (.hdr or .exr) lighting the scene instead of the sky gradient
    /// (roadmap core track 3). Relative paths are relative to the scenes folder; empty = none.
    pub hdri: String,
    /// Multiplier on the HDRI's radiance.
    pub hdri_strength: f32,
    /// Turn about the vertical axis, degrees.
    pub hdri_rotation: f32,
    /// Camera rays see the HDRI; off, they see the background colour (it still lights).
    pub hdri_visible: bool,
    /// Physical sky and sun (track 9); replaces the HDRI and the sky light while on.
    #[serde(skip_serializing_if = "crate::sky::PhysicalSky::is_default")]
    pub physical_sky: crate::sky::PhysicalSky,
    /// Global fog (track 12).
    #[serde(skip_serializing_if = "Fog::is_default")]
    pub fog: Fog,
    /// Keyframes (track 8); the playhead is `anim::Timeline` in the app.
    #[serde(skip_serializing_if = "crate::anim::Animation::is_default")]
    pub animation: crate::anim::Animation,
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
            light_outline: false,
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
            pixel_filter: 2,
            filter_width: 1.5,
            anisotropy: 16,
            hdr_mode: 0,
            hdr_peak: 0.,
            hdr10: false,
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
            aperture: 0.,
            focus_distance: 4.7,
            aperture_blades: 0,
            blade_rotation: 0.,
            cat_eye: 0.,
            projection: 0,
            tilt: 0.,
            lens_system: 0,
            f_number: 0.,
            lens_scale: 1.,
            dispersion: 0.,
            halation: 0.,
            glare: 0.,
            glare_size: 0.5,
            glare_threshold: 1.,
            grain: 0.,
            grain_size: 1,
            vignette: 0.,
            lift: [0.; 3],
            grade_gamma: [1.; 3],
            gain: [1.; 3],
            lut_path: String::new(),
            lut_strength: 1.,
            mis_mode: 0,
            restir: false,
            restir_candidates: 8,
            restir_neighbors: 4,
            restir_history: 20,
            integrator: 0,
            photons: 1 << 18,
            photon_radius: 0.01,
            battle: false,
            physics: Physics::default(),
            puzzle: Puzzle::default(),
            audio: AudioReact::default(),
            lpe: vec![],
            lpe_slot: 0,
            hdri: String::new(),
            hdri_strength: 1.,
            hdri_rotation: 0.,
            hdri_visible: true,
            physical_sky: Default::default(),
            fog: Default::default(),
            animation: Default::default(),
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
        self.denoise = from.denoise;
        self.vsync = from.vsync;
        self.rr_preset = from.rr_preset;
        self.upscaler = from.upscaler;
        self.dlss_preset = from.dlss_preset;
        self.native_aa = from.native_aa;
        self.pixel_filter = from.pixel_filter;
        self.filter_width = from.filter_width;
        self.anisotropy = from.anisotropy;
        self.hdr_mode = from.hdr_mode;
        self.hdr_peak = from.hdr_peak;
        self.hdr_white = from.hdr_white;
        self.hdr10 = from.hdr10;
        self.lpe = from.lpe.clone();
        self.lpe_slot = from.lpe_slot;
        self.battle = from.battle;
    }
    pub fn preset(&mut self, id: usize) {
        // Presets 0–8 share the Cornell box, so moving between them keeps the
        // user's camera. Any other switch takes the preset's own camera.
        let keep_camera = id < 9 && PRESETS[..9].contains(&self.scene_name.as_str());
        let camera = (self.yaw, self.pitch, self.distance, self.target, self.fov);
        let output = (
            self.scale,
            (self.rr, self.denoise),
            self.vsync,
            self.rr_preset,
            self.upscaler,
            (self.dlss_preset, self.native_aa),
            (self.hdr_mode, self.hdr_peak, self.hdr_white, self.hdr10),
            (self.pixel_filter, self.filter_width, self.anisotropy),
            (std::mem::take(&mut self.lpe), self.lpe_slot, self.battle),
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
            (self.rr, self.denoise),
            self.vsync,
            self.rr_preset,
            self.upscaler,
            (self.dlss_preset, self.native_aa),
            (self.hdr_mode, self.hdr_peak, self.hdr_white, self.hdr10),
            (self.pixel_filter, self.filter_width, self.anisotropy),
            (self.lpe, self.lpe_slot, self.battle),
        ) = output;
    }
    pub fn validate(&mut self) -> anyhow::Result<()> {
        let f = &self.fog;
        anyhow::ensure!(
            (0. ..=10.).contains(&f.density)
                && f.color.iter().all(|v| (0. ..=1.).contains(v))
                && (-0.95..=0.95).contains(&f.phase)
                && (-1. ..=50.).contains(&f.height)
                && (0.1..=100.).contains(&f.extent),
            "Invalid fog"
        );
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
            (self.pixel_filter as usize) < PIXEL_FILTERS.len()
                && (0.5..=4.).contains(&self.filter_width),
            "Invalid pixel filter"
        );
        anyhow::ensure!(
            (1..=16).contains(&self.anisotropy),
            "Invalid texture anisotropy"
        );
        self.animation.validate()?;
        self.physical_sky.validate()?;
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
                && (1..=64).contains(&self.spp)
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
                && self.tonemap < 8
                && self.view < 13,
            "Invalid display settings"
        );
        anyhow::ensure!(
            self.lpe.len() <= crate::lpe::SLOTS && (self.lpe_slot as usize) < crate::lpe::SLOTS,
            "At most {} light path expressions",
            crate::lpe::SLOTS
        );
        for e in self.lpe.iter().filter(|e| !e.trim().is_empty()) {
            crate::lpe::compile(e)
                .map_err(|err| anyhow::anyhow!("Light path expression {e:?}: {err}"))?;
        }
        anyhow::ensure!(
            (0. ..=4.).contains(&self.direct)
                && (0. ..=4.).contains(&self.indirect)
                && (0. ..=1.).contains(&self.denoise_strength)
                && (0. ..=100.).contains(&self.firefly)
                && (0. ..=1.).contains(&self.split),
            "Invalid filter settings"
        );
        anyhow::ensure!(
            (0. ..=2.).contains(&self.aperture)
                && (0.05..=500.).contains(&self.focus_distance)
                && (self.aperture_blades == 0 || (3..=12).contains(&self.aperture_blades))
                && self.blade_rotation.abs() <= 360.
                && (0. ..=1.).contains(&self.cat_eye)
                && self.projection < 5
                && self.mis_mode < 3
                && (1..=32).contains(&self.restir_candidates)
                && self.restir_neighbors <= 8
                && self.restir_history <= 64
                && self.tilt.abs() <= 60.
                && self.lens_system as usize <= crate::lens::LENSES.len()
                && (self.f_number == 0. || (0.5..=64.).contains(&self.f_number))
                && (0.001..=1000.).contains(&self.lens_scale)
                && (0. ..=20.).contains(&self.dispersion),
            "Invalid lens or camera model"
        );
        // At most 1M photons: 16384 dispatch groups, and the photon buffer stays under 400 MB.
        anyhow::ensure!(
            self.integrator < 3
                && ((1 << 12)..=(1 << 20)).contains(&self.photons)
                && (0.0005..=0.2).contains(&self.photon_radius),
            "Invalid integrator settings"
        );
        anyhow::ensure!(
            (0. ..=1.).contains(&self.halation)
                && (0. ..=4.).contains(&self.glare)
                && (0. ..=1.).contains(&self.glare_size)
                && (0. ..=100.).contains(&self.glare_threshold)
                && (0. ..=1.).contains(&self.grain)
                && self.grain_size < 3
                && (0. ..=1.).contains(&self.vignette)
                && self.lift.iter().all(|v| (-0.5..=0.5).contains(v))
                && self.grade_gamma.iter().all(|v| (0.2..=5.).contains(v))
                && self.gain.iter().all(|v| (0. ..=4.).contains(v))
                && (0. ..=1.).contains(&self.lut_strength)
                && self.lut_path.len() <= 1024,
            "Invalid film look"
        );
        anyhow::ensure!(
            self.hdri.len() <= 1024
                && (0. ..=1000.).contains(&self.hdri_strength)
                && (-360. ..=360.).contains(&self.hdri_rotation),
            "Invalid HDRI"
        );
        anyhow::ensure!(
            (-30. ..=30.).contains(&self.physics.gravity)
                && (0. ..=4.).contains(&self.physics.speed)
                && (0. ..=1.).contains(&self.physics.bounce)
                && (0. ..=2.).contains(&self.physics.friction)
                && self.physics.down.iter().all(|v| v.abs() <= 1.)
                && self.physics.down.iter().any(|v| v.abs() > 0.01)
                && self.physics.wind.iter().all(|v| v.abs() <= 50.)
                && (0. ..=1.).contains(&self.physics.gusts),
            "Invalid physics"
        );
        anyhow::ensure!(
            (0. ..=8.).contains(&self.audio.gain)
                && (0. ..=1.).contains(&self.audio.floor)
                && (0. ..=0.99).contains(&self.audio.smoothing),
            "Invalid audio settings"
        );
        let p = &self.puzzle;
        anyhow::ensure!(
            p.origin.iter().all(|v| v.abs() <= 40.)
                && p.direction.iter().all(|v| v.is_finite() && v.abs() <= 1e3)
                && (-1..self.objects.len() as i32).contains(&p.target)
                && p.par <= 99
                && p.targets.len() <= 16
                && p.targets
                    .iter()
                    .all(|t| (0..self.objects.len() as i32).contains(t))
                && p.color.iter().all(|v| (0. ..=1.).contains(v)),
            "Invalid laser puzzle"
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
                    && (0. ..=20.).contains(&m.emission)
                    && m.priority <= 7
                    && [
                        m.clearcoat,
                        m.clearcoat_roughness,
                        m.sheen,
                        m.sheen_roughness,
                        m.anisotropy,
                        m.iridescence,
                        m.grating
                    ]
                    .iter()
                    .chain(&m.sheen_color)
                    .all(|v| (0. ..=1.).contains(v))
                    && (-180. ..=180.).contains(&m.anisotropy_rotation)
                    && (0. ..=2000.).contains(&m.iridescence_thickness)
                    && (1. ..=2.5).contains(&m.iridescence_ior)
                    && (0.3..=10.).contains(&m.grating_spacing)
                    && (0. ..=100.).contains(&m.density)
                    && m.scatter_color.iter().all(|v| (0. ..=1.).contains(v))
                    && (-0.95..=0.95).contains(&m.phase),
                "Invalid material"
            );
            anyhow::ensure!(
                m.secondary_color.iter().all(|v| (0. ..=1.).contains(v))
                    && (0.1..=100.).contains(&m.pattern_scale),
                "Invalid material pattern"
            );
            anyhow::ensure!(
                (0.01..=100.).contains(&m.texture_scale) && m.texture.len() <= 1024,
                "Invalid material texture"
            );
        }
        for (i, o) in self.objects.iter().enumerate() {
            anyhow::ensure!(
                o.portal == -1
                    || (o.portal >= 0
                        && (o.portal as usize) < self.objects.len()
                        && o.portal as usize != i),
                "Invalid portal pair"
            );
        }
        for (i, o) in self.objects.iter().enumerate() {
            anyhow::ensure!(
                o.tether
                    .is_none_or(|t| (t.object as usize) < self.objects.len()
                        && t.object as usize != i
                        && t.from.iter().all(|v| v.abs() <= 40.)),
                "Invalid string (tether)"
            );
            anyhow::ensure!(
                o.links.len() <= 8
                    && o.links
                        .iter()
                        .all(|l| (l.object as usize) < self.objects.len()
                            && l.object as usize != i
                            && l.at.iter().all(|v| v.is_finite() && v.abs() <= 40.)),
                "Invalid joint (links)"
            );
            anyhow::ensure!(
                o.cloth.is_none_or(|c| (4..=48).contains(&c.resolution)
                    && (0. ..=1.).contains(&c.stiffness)
                    && c.uv.iter().all(|u| u.is_finite() && u.abs() <= 64.)),
                "Invalid cloth"
            );
            anyhow::ensure!(
                (0.05..=30.).contains(&o.size)
                    && (0.05..=10.).contains(&o.stretch)
                    && o.rotation.abs() <= 360.
                    && o.position.iter().all(|v| v.abs() <= 40.)
                    && o.elevation.abs() <= 40.
                    && (0. ..=10000.).contains(&o.phase)
                    && (0.01..=100.).contains(&o.mass)
                    && o.tilt.iter().all(|v| v.abs() <= 360.)
                    && o.scale_xz.iter().all(|v| (0.01..=20.).contains(v))
                    && o.name.len() <= 128
                    && o.mesh.len() <= 1024,
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
                    && (0. ..=2000.).contains(&l.intensity)
                    && (1. ..=179.).contains(&l.spread)
                    && (0. ..=1.).contains(&l.blend)
                    && (0.05..=30.).contains(&l.angle),
                "Invalid area light"
            );
        }
        Ok(())
    }
    /// RR, the temporal upscalers and the gizmo assume a perspective camera.
    pub fn perspective(&self) -> bool {
        (self.projection == 0 && !self.real_lens()) || self.projection == 4
    }
    /// A lens system is in use (track 16): perspective model only. Its distortion breaks the
    /// pinhole math of RR's motion vectors, the upscalers, picking and the gizmo.
    pub fn real_lens(&self) -> bool {
        self.projection == 0 && self.lens_system > 0
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
    /// Texture coordinates (meshes only; zero elsewhere).
    pub uv: [f32; 2],
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
