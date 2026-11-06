//! The nested-dielectric stack and what it absorbs.
use super::bsdf::rgb;
use super::scene::Scene;
use glam::Vec3;

/// The nested-dielectric stack (`medium_top`, `medium_push`, `medium_remove`): entries are
/// (priority, material), at most 4; the current medium is the highest priority, the later
/// entry among equals.
pub(super) fn top(media: &[(u32, u32)]) -> Option<&(u32, u32)> {
    // max_by_key keeps the last of equal maxima.
    media.iter().max_by_key(|e| e.0)
}
pub(super) fn push(media: &mut Vec<(u32, u32)>, e: (u32, u32)) {
    // A full stack drops the push, as on the GPU.
    if media.len() < 4 {
        media.push(e);
    }
}
pub(super) fn remove(media: &mut Vec<(u32, u32)>, e: (u32, u32)) {
    if let Some(i) = media.iter().rposition(|x| *x == e) {
        media.remove(i);
    }
}

impl Scene<'_> {
    /// Beer-Lambert through `t` of the innermost dielectric: its colour is the transmittance
    /// per unit (`medium_absorption`).
    pub(super) fn transmittance(&self, media: &[(u32, u32)], t: f32) -> Vec3 {
        let Some(top) = top(media) else {
            return Vec3::ONE;
        };
        let c = rgb(self.materials[top.1 as usize].color_rough).max(Vec3::splat(0.001));
        c.powf(t)
    }
}
