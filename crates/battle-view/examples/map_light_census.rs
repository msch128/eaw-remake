//! Census of the space maps' lighting: `map_light_census [albedo]`.
//!
//! For every `Space_Tactical_Map` (and destroyed variant) of the Remake's planets: the lights the renderer takes from the `.ted`
//! (effective colours, see `alamo_render::map_light_color`) and the shade of a grey hull (`albedo`, default 0.5) under the map
//! lighting model (`alamo_render::shade_map_lit`) for four faces: the top (what the play camera sees), the side turned to the sun,
//! the side turned away and the underside. `lum` is the Rec. 709 luma of the display value, `sat` the HSV saturation (the tint
//! the light gives a grey hull). The `old` columns are the model before the map lighting model (square-root sun weight, ambient
//! floor 0.05, half-wrapped fills, gain 1.4). `strong` marks maps whose data gives a strong look: a top darker than 0.08 (near black under
//! the play camera), or a lit side tinted 0.5 or more. These are report thresholds, nothing is clamped.
use std::collections::BTreeSet;

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use alamo_render::{EffectiveLight, shade_map_lit};
use battle_view::SpaceScene;
use bevy::math::Vec3;

fn luma(c: Vec3) -> f32 {
    0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z
}

fn saturation(c: Vec3) -> f32 {
    let (max, min) = (c.max_element(), c.min_element());
    if max <= 1.0e-4 { 0.0 } else { (max - min) / max }
}

/// The lighting before the map lighting model, for the before column.
fn old_shade(albedo: f32, n: Vec3, scene: &SpaceScene, raw: &alamo_map::Environment) -> Vec3 {
    let sun_s = raw.lights[0].scalar;
    let gain = if sun_s <= 0.0 { 0.0 } else { (sun_s / 0.5).sqrt().clamp(0.3, 1.4) };
    let sun = Vec3::from(raw.lights[0].color) * gain;
    let amb = Vec3::from(raw.ambient).max(Vec3::splat(0.05));
    let mut light = amb + sun * n.dot(-scene.lights[0].direction).max(0.0);
    for f in &scene.lights[1..] {
        light += f.color * ((n.dot(-f.direction) + 0.5) / 1.5).clamp(0.0, 1.0);
    }
    (Vec3::splat(albedo) * light.clamp(Vec3::ZERO, Vec3::ONE) * 1.4).min(Vec3::ONE)
}

fn main() {
    let albedo: f32 = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(0.5);
    let install = GameInstall::detect().expect("Empire at War install");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("game files");
    let (data, _) = load_galaxy(&fs);
    let maps: BTreeSet<String> = data
        .planets
        .iter()
        .flat_map(|p| [p.space_tactical_map.clone(), p.destroyed_space_tactical_map.clone()])
        .flatten()
        .map(|m| m.trim().to_ascii_uppercase())
        .filter(|m| !m.is_empty())
        .collect();
    println!("{} space maps, grey albedo {albedo}", maps.len());
    println!(
        "{:38} {:>17} {:>17} {:>15} | {:>5} {:>5} {:>5} {:>5} {:>5} | {:>5} {:>5} {:>5} | strong",
        "map", "sun (eff)", "ambient", "sun spec", "top", "lit", "litS", "away", "under", "oTop", "oLit", "oLitS"
    );
    let mut strong = Vec::new();
    for m in &maps {
        let path = format!("Art/Maps/{m}");
        let Ok(bytes) = alamo_fs::FileSource::read(&fs, &path) else {
            println!("{m:38} not found");
            continue;
        };
        let Ok(mut map) = alamo_map::SpaceMap::parse(&bytes) else {
            println!("{m:38} does not parse");
            continue;
        };
        map.resolve_names(data.objects.iter().map(|o| o.name.as_str()));
        let scene = SpaceScene::from_map(&map, &data, None);
        let light = |i: usize| EffectiveLight { to_light: -scene.lights[i].direction, color: scene.lights[i].color };
        let (sun, fills) = (light(0), [light(1), light(2)]);
        let horizontal = Vec3::new(sun.to_light.x, 0.0, sun.to_light.z);
        let toward = if horizontal.length() > 1.0e-3 { horizontal.normalize() } else { Vec3::X };
        let shade = |n: Vec3| shade_map_lit(Vec3::splat(albedo), Vec3::ONE, n, sun, &fills, scene.ambient);
        let (top, lit, away, under) = (shade(Vec3::Y), shade(toward), shade(-toward), shade(Vec3::NEG_Y));
        let (o_top, o_lit) = (old_shade(albedo, Vec3::Y, &scene, &map.environment), old_shade(albedo, toward, &scene, &map.environment));
        let is_strong = luma(top) < 0.08 || saturation(lit) >= 0.5;
        let f3 = |v: Vec3| format!("{:.2},{:.2},{:.2}", v.x, v.y, v.z);
        println!(
            "{m:38} {:>17} {:>17} {:>15} | {:5.2} {:5.2} {:5.2} {:5.2} {:5.2} | {:5.2} {:5.2} {:5.2} | {}",
            f3(sun.color),
            f3(scene.ambient),
            f3(scene.sun_specular),
            luma(top),
            luma(lit),
            saturation(lit),
            luma(away),
            luma(under),
            luma(o_top),
            luma(o_lit),
            saturation(o_lit),
            if is_strong { "strong" } else { "" }
        );
        if is_strong {
            strong.push(m.clone());
        }
    }
    println!("{} maps with a strong look from their data: {}", strong.len(), strong.join(", "));
}
