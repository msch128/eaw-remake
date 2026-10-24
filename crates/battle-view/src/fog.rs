//! Tactical Fog of War visual shroud overlay in space battles (on by default, `SP_FOG_SHROUD=0` turns it off, see `setup_fog_shroud`).
//!
//! Spans a semi-transparent cosmic fog plane over the arena on the battle plane (`SpaceFOWHeight` -10, cells of about `DesiredSpaceFOWCellSize` 100).
//! Areas outside the player's ship sensor discs are covered in a faint white haze (`SpaceFOWColor` 255 255 255 in Gameconstants.xml; the data alpha is 0,
//! the Remake shows no haze, a faint one is a design choice, see [`FOG_MAX_ALPHA`]); areas within sensor range are clear with soft feathered borders, and the
//! plane fades out toward its own edge so no hard seam shows over a backdrop planet.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use crate::{BattleEntity, BattleRuntime, BattleViewSettings, RenderedShips};

pub const FOG_GRID_SIZE: usize = 128;
/// Opacity of the full haze: 10 of 255, a dark veil that marks the sensor edge and keeps the stars visible. User pick 2026-10-17 from
/// offscreen comparison renders at 0 / 10 / 30 / 70 / 140 (140 read as a nearly white sheet, 30 already as a grey one; the data alpha is 0).
pub const FOG_MAX_ALPHA: u8 = 10;
pub const FOG_FEATHER_DISTANCE: f32 = 800.0;
/// The plane fades to nothing over this distance at its edge (no hard seam).
pub const FOG_EDGE_FADE: f32 = 2500.0;
pub const FOG_COLOR_RGB: [u8; 3] = [255, 255, 255];

/// Opacity of the full haze: [`FOG_MAX_ALPHA`], or `SP_FOG_ALPHA=<0-255>` to try another value (comparison renders).
fn max_alpha() -> u8 {
    static ALPHA: std::sync::OnceLock<u8> = std::sync::OnceLock::new();
    *ALPHA.get_or_init(|| std::env::var("SP_FOG_ALPHA").ok().and_then(|v| v.parse().ok()).unwrap_or(FOG_MAX_ALPHA))
}

/// Resource holding the tactical fog shroud texture handle and arena size.
#[derive(Resource)]
pub struct TacticalFogShroud {
    pub texture: Handle<Image>,
    pub size: f32,
    pub timer: f32,
    /// The plane's material: its opacity follows the camera angle (see [`fade_fog_shroud`]).
    pub material: Handle<StandardMaterial>,
    /// Fade factor last written to the material.
    pub shown: f32,
}

/// Spawns the 3D fog of war shroud plane when a battle with Fog of War active begins.
pub(crate) fn setup_fog_shroud(
    mut commands: Commands,
    rt: Option<Res<BattleRuntime>>,
    existing: Option<Res<TacticalFogShroud>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(rt) = rt else { return };
    // By design the shroud stays visible over areas none of the player's ships sees (on by default; `SP_FOG_SHROUD=0` turns the plane off).
    if !rt.fog_of_war || existing.is_some() || std::env::var_os("SP_FOG_SHROUD").is_some_and(|v| v == "0") {
        return;
    }
    let n = FOG_GRID_SIZE;
    let arena_size = (rt.arena_half * 2.2).max(12000.0);
    let mut px = Vec::with_capacity(n * n * 4);
    for _ in 0..(n * n) {
        px.extend_from_slice(&[FOG_COLOR_RGB[0], FOG_COLOR_RGB[1], FOG_COLOR_RGB[2], max_alpha()]);
    }
    let mut img = Image::new(
        Extent3d { width: n as u32, height: n as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        px,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    img.sampler = bevy::image::ImageSampler::linear();
    let tex = images.add(img);
    let material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(tex.clone()),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        ..default()
    });
    commands.spawn((
        BattleEntity,
        Name::new("tactical fog of war shroud"),
        Mesh3d(meshes.add(Plane3d::default().mesh().size(arena_size, arena_size))),
        MeshMaterial3d(material.clone()),
        Transform::from_xyz(0.0, -10.0, 0.0),
    ));
    commands.insert_resource(TacticalFogShroud {
        texture: tex,
        size: arena_size,
        timer: 0.0,
        material,
        shown: 1.0,
    });
}
/// Opacity share for a view that looks `elevation` radians down onto the plane (smoothstep from 26 to 49 degrees; the default camera is at 60).
fn view_factor(elevation: f32) -> f32 {
    let t = ((elevation - 0.45) / (0.85 - 0.45)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}


/// Fades the shroud out as the view flattens: a plane seen nearly edge-on reads as a solid bright band across the picture (low pitch,
/// the movie and cinematic cameras). Full strength from about 49 degrees down onto the plane, gone below about 26 degrees; hidden while
/// a battle camera mode runs.
pub(crate) fn fade_fog_shroud(
    shroud: Option<ResMut<TacticalFogShroud>>,
    camera: Option<Single<&Transform, With<crate::camera::BattleCamera>>>,
    action: Option<Res<crate::camera::ActionCam>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let (Some(mut shroud), Some(camera)) = (shroud, camera) else { return };
    let elevation = camera.forward().y.abs().clamp(0.0, 1.0).asin();
    let mut factor = view_factor(elevation);
    if action.is_some_and(|a| a.active) {
        factor = 0.0;
    }
    if (factor - shroud.shown).abs() < 0.01 {
        return;
    }
    shroud.shown = factor;
    if let Some(mut m) = materials.get_mut(&shroud.material) {
        m.base_color = Color::srgba(1.0, 1.0, 1.0, factor);
    }
}

/// Calculates alpha values for the fog grid based on player sensor circles.
pub fn calculate_fog_alphas(
    grid_size: usize,
    arena_size: f32,
    sensors: &[(Vec2, f32)],
) -> Vec<u8> {
    let mut alphas = Vec::with_capacity(grid_size * grid_size);
    let half = arena_size * 0.5;
    let step = arena_size / (grid_size.saturating_sub(1).max(1) as f32);
    let full = max_alpha();

    for y in 0..grid_size {
        let wz = (y as f32) * step - half;
        for x in 0..grid_size {
            let wx = (x as f32) * step - half;
            let mut min_d = f32::INFINITY;
            for &(center, radius) in sensors {
                let dx = wx - center.x;
                let dz = wz - center.y;
                let dist = (dx * dx + dz * dz).sqrt();
                let d = dist - radius;
                if d < min_d {
                    min_d = d;
                }
            }
            let alpha = if sensors.is_empty() {
                full
            } else if min_d <= 0.0 {
                0
            } else if min_d >= FOG_FEATHER_DISTANCE {
                full
            } else {
                let t = min_d / FOG_FEATHER_DISTANCE;
                let smooth = t * t * (3.0 - 2.0 * t);
                (smooth * full as f32).round() as u8
            };
            // Fade out toward the plane's own edge.
            let edge = (half - wx.abs()).min(half - wz.abs()).max(0.0) / FOG_EDGE_FADE;
            let edge = edge.clamp(0.0, 1.0);
            let edge = edge * edge * (3.0 - 2.0 * edge);
            alphas.push((alpha as f32 * edge).round() as u8);
        }
    }
    alphas
}

/// Updates the dynamic fog shroud image periodically with the player fleet's sensor discs.
pub(crate) fn update_fog_shroud(
    time: Res<Time>,
    rt: Option<Res<BattleRuntime>>,
    settings: Option<Res<BattleViewSettings>>,
    ships: Option<Res<RenderedShips>>,
    shroud: Option<ResMut<TacticalFogShroud>>,
    mut images: ResMut<Assets<Image>>,
) {
    let (Some(rt), Some(settings), Some(ships), Some(mut shroud)) = (rt, settings, ships, shroud) else {
        return;
    };
    shroud.timer += time.delta_secs();
    if shroud.timer < 0.12 {
        return;
    }
    shroud.timer = 0.0;

    let Some(mut image) = images.get_mut(&shroud.texture) else { return };
    let player = settings.player_side;
    let sensors: Vec<(Vec2, f32)> = ships
        .0
        .values()
        .filter(|s| s.side == player && s.visible)
        .map(|s| {
            let r = rt.catalog.ship(s.def).sensor_range.max(1000.0);
            (Vec2::new(s.pos.x, s.pos.z), r)
        })
        .collect();

    let alphas = calculate_fog_alphas(FOG_GRID_SIZE, shroud.size, &sensors);
    let Some(data) = image.data.as_mut() else { return };
    for (i, &a) in alphas.iter().enumerate() {
        let base = i * 4;
        if base + 3 < data.len() {
            data[base] = FOG_COLOR_RGB[0];
            data[base + 1] = FOG_COLOR_RGB[1];
            data[base + 2] = FOG_COLOR_RGB[2];
            data[base + 3] = a;
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shroud_fades_out_as_the_view_flattens() {
        assert_eq!(view_factor(60f32.to_radians()), 1.0, "the default 60 degree view keeps the full haze");
        assert_eq!(view_factor(22f32.to_radians()), 0.0, "the movie camera view (about 22 degrees) shows no shroud");
        assert!(view_factor(38f32.to_radians()) > 0.0 && view_factor(38f32.to_radians()) < 1.0);
    }

    #[test]
    fn fog_alpha_calculation_reveals_sensors_and_shrouds_distant_space() {
        let grid = 64;
        let arena = 20000.0;
        let sensors = vec![(Vec2::ZERO, 2000.0)];

        let alphas = calculate_fog_alphas(grid, arena, &sensors);
        assert_eq!(alphas.len(), grid * grid);

        // Centre (around (0,0)) is inside the sensor radius: clear.
        let center_idx = (grid / 2) * grid + (grid / 2);
        assert_eq!(alphas[center_idx], 0, "center of sensor disc should be clear (alpha 0)");

        // Far from every sensor and well inside the plane (about (-5900, -5900)): full haze.
        let inner = 13 * grid + 13;
        assert_eq!(alphas[inner], FOG_MAX_ALPHA, "far space inside the plane is fully covered");

        // The plane's own corner fades out: no hard seam at the edge.
        assert_eq!(alphas[0], 0, "the edge fades to nothing");

        // A sensor far away leaves everything else covered.
        let sensors_off = vec![(Vec2::new(50000.0, 50000.0), 100.0)];
        let alphas_off = calculate_fog_alphas(grid, arena, &sensors_off);
        assert_eq!(alphas_off[inner], FOG_MAX_ALPHA);
    }
}
