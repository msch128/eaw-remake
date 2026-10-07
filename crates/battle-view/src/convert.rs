//! Coordinate conversion between battle-sim space, map space and Bevy world space.
//!
//! * battle-sim: X forward at yaw 0, Y left, Z up (right handed).
//! * map (`.ted`): Z up; attackers sit at -Y facing +Y, defenders at +Y. The simulation places
//!   attackers at -X facing +X, so sim space is rotated by [`SIM_TO_MAP_YAW`] about Z.
//! * Bevy: Y up, -Z forward. Map space maps to Bevy with `(x, y, z) -> (x, z, -y)`, which is the
//!   same axis change the model loader applies to `.alo` geometry.

use bevy::prelude::*;

/// Rotation about Z that turns sim space into map space (sim -X attackers -> map -Y attackers).
pub const SIM_TO_MAP_YAW: f32 = std::f32::consts::FRAC_PI_2;

/// Map space (Z up) to Bevy world space.
pub fn map_to_bevy(p: [f32; 3]) -> Vec3 {
    Vec3::new(p[0], p[2], -p[1])
}

/// Bevy world space to map space (Z up).
pub fn bevy_to_map(p: Vec3) -> [f32; 3] {
    [p.x, -p.z, p.y]
}

/// The rotation that carries map-space directions into Bevy space (`Rx(-90 deg)`).
pub fn map_basis() -> Quat {
    Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)
}

/// Sim-space position to Bevy world space.
pub fn sim_to_bevy(p: battle_sim::Vec3) -> Vec3 {
    let (s, c) = SIM_TO_MAP_YAW.sin_cos();
    map_to_bevy([p.x * c - p.y * s, p.x * s + p.y * c, p.z])
}

/// Bevy world space to sim space.
pub fn bevy_to_sim(p: Vec3) -> battle_sim::Vec3 {
    let m = bevy_to_map(p);
    let (s, c) = (-SIM_TO_MAP_YAW).sin_cos();
    battle_sim::Vec3::new(m[0] * c - m[1] * s, m[0] * s + m[1] * c, m[2])
}

/// Sim-space velocity / direction to Bevy world space (same as positions, no translation).
pub fn sim_dir_to_bevy(v: battle_sim::Vec3) -> Vec3 {
    sim_to_bevy(v)
}

/// Rotation of a ship entity from its sim heading (`bow_forward` is false: the `.alo` bow points
/// along its -Y axis, which the model loader turns into Bevy +Z, so a quarter turn aligns it with
/// the heading).
pub fn ship_rotation(yaw: f32) -> Quat {
    Quat::from_rotation_y(yaw + SIM_TO_MAP_YAW + std::f32::consts::FRAC_PI_2)
}

/// Rotation of a placed map object from its Euler angles in degrees (Z yaw, Y pitch, X roll
/// applied in map space), expressed in Bevy space for an [`alamo_render::AlamoModel`].
pub fn map_object_rotation(deg: [f32; 3]) -> Quat {
    let r = Quat::from_rotation_z(deg[2].to_radians())
        * Quat::from_rotation_y(deg[1].to_radians())
        * Quat::from_rotation_x(deg[0].to_radians());
    map_basis() * r * map_basis().inverse()
}

/// Shortest-arc angle interpolation.
pub fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    let mut d = (b - a) % std::f32::consts::TAU;
    if d > std::f32::consts::PI {
        d -= std::f32::consts::TAU;
    } else if d < -std::f32::consts::PI {
        d += std::f32::consts::TAU;
    }
    a + d * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sim_round_trip() {
        let p = battle_sim::Vec3::new(1234.0, -567.0, 89.0);
        let q = bevy_to_sim(sim_to_bevy(p));
        assert!((p.x - q.x).abs() < 1e-2 && (p.y - q.y).abs() < 1e-2 && (p.z - q.z).abs() < 1e-3);
    }

    #[test]
    fn attackers_end_up_at_map_minus_y() {
        // Sim -X (attackers) must land at map -Y, i.e. Bevy +Z (toward the default camera).
        let v = sim_to_bevy(battle_sim::Vec3::new(-1000.0, 0.0, 0.0));
        assert!(v.z > 900.0 && v.x.abs() < 1.0);
    }

    #[test]
    fn heading_zero_faces_map_y() {
        // Yaw 0 is sim +X, which is map +Y, i.e. Bevy -Z; the model bow (Bevy +Z before the
        // rotation) must follow.
        let bow = ship_rotation(0.0) * Vec3::Z;
        assert!(bow.z < -0.99, "{bow:?}");
    }
}
