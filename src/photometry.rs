//! Photometric units shared by the sky, the exposure meter and the Eyes look (roadmap: auto
//! exposure). Scene radiance 1.0 is a white surface under a sun of intensity 1; this module says
//! how many cd/m² that is, so the two renderers and the shaders agree on one absolute scale.
//! `SCENE_CD_M2` is mirrored in `shaders/common.hlsli` (a test below keeps the two equal).

/// Scene radiance 1.0 in cd/m² (a noon sun lights a white surface to about 32,000 cd/m²; the
/// night sky's physical luminances are scaled by its inverse). Changeable: it moves every
/// absolute threshold below and nothing else.
pub const SCENE_CD_M2: f32 = 32_000.;
/// Below this scene luminance vision is scotopic (3e-7 scene units, about 0.01 cd/m²).
// Used by the Eyes look (E4).
#[allow(dead_code)]
pub const SCOTOPIC_BELOW: f32 = 3e-7;
/// Below this it is mesopic (1e-4 scene units, about 3 cd/m²); above, photopic.
#[allow(dead_code)]
pub const MESOPIC_BELOW: f32 = 1e-4;
/// Middle grey, the exposure meter's target (the false-colour view uses the same).
#[allow(dead_code)]
pub const MIDDLE_GREY: f32 = 0.18;

/// Rec. 709 relative luminance (the shaders' `luminance`).
pub fn luminance(c: [f32; 3]) -> f32 {
    (0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]).max(0.)
}
/// Scene units to cd/m².
pub fn scene_to_cd(l: f32) -> f32 {
    l * SCENE_CD_M2
}
/// cd/m² to scene units.
pub fn cd_to_scene(cd: f32) -> f32 {
    cd / SCENE_CD_M2
}
/// The linear multiplier for an exposure value in stops (+1 EV doubles it).
#[allow(dead_code)]
pub fn ev_to_scale(ev: f32) -> f32 {
    ev.exp2()
}
/// Stops for a linear multiplier; 0 and negatives clamp to the lowest manual exposure.
#[allow(dead_code)]
pub fn scale_to_ev(scale: f32) -> f32 {
    if scale > 0. { scale.log2() } else { -64. }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hlsl_mirrors_the_constant() {
        let hlsl = include_str!("../shaders/common.hlsli");
        let line = hlsl
            .lines()
            .find(|l| l.starts_with("static const float SCENE_CD_M2"))
            .expect("common.hlsli defines SCENE_CD_M2");
        let v: f32 = line
            .split('=')
            .nth(1)
            .unwrap()
            .trim()
            .trim_end_matches(';')
            .parse()
            .unwrap();
        assert_eq!(v, SCENE_CD_M2);
    }
    #[test]
    fn conversions_round_trip_and_luminance_is_rec709() {
        assert_eq!(luminance([1.; 3]), 1.);
        assert!((cd_to_scene(scene_to_cd(0.37)) - 0.37).abs() < 1e-6);
        assert_eq!(ev_to_scale(3.), 8.);
        assert_eq!(scale_to_ev(ev_to_scale(-5.)), -5.);
        assert_eq!(scale_to_ev(0.), -64.);
        assert!(SCOTOPIC_BELOW < MESOPIC_BELOW);
    }
}
