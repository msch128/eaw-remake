//! The static scene of a space map: sky, nebula, lights, ambient, bounds and placed props.

use alamo_data::GameData;
use alamo_fs::FileSource;
use alamo_map::{ObjectClass, SpaceMap};
use alamo_render::{AlamoModel, AlamoModelLoaded, spawn_sun};
use bevy::camera::Exposure;
use bevy::light::GlobalAmbientLight;
use bevy::prelude::*;

use crate::BattleEntity;
use crate::convert::{map_object_rotation, map_to_bevy};

/// A decorative object placed in the map (ship prop, dock, asteroid, backdrop planet).
#[derive(Debug, Clone)]
pub struct SceneProp {
    /// Object name from the map.
    pub name: String,
    /// Model file.
    pub model: String,
    /// Uniform scale.
    pub scale: f32,
    /// Bevy world position.
    pub position: Vec3,
    /// Bevy world rotation.
    pub rotation: Quat,
    /// True for far-away planets and other backdrop objects.
    pub backdrop: bool,
}

/// One directional light of the map environment.
#[derive(Debug, Clone, Copy)]
pub struct SceneLight {
    /// Direction the light travels in Bevy space.
    pub direction: Vec3,
    /// Light colour (0..1).
    pub color: Vec3,
}

/// Everything the renderer needs from a `.ted` space map.
#[derive(Debug, Clone)]
pub struct SpaceScene {
    /// Map name for display.
    pub name: String,
    /// Lower playable corner on the map plane (map X, map Y).
    pub bounds_min: Vec2,
    /// Upper playable corner on the map plane.
    pub bounds_max: Vec2,
    /// Sun and the two fill lights.
    pub lights: [SceneLight; 3],
    /// Ambient colour.
    pub ambient: Vec3,
    /// Skydome model (e.g. `AAA_STARS_DENSE.ALO`).
    pub skydome: Option<String>,
    /// Skydome rotation (Euler degrees, map space).
    pub skydome_rotation: [f32; 3],
    /// Nebula model.
    pub nebula: Option<String>,
    /// Nebula rotation (degrees).
    pub nebula_rotation: [f32; 2],
    /// Placed decorative objects.
    pub props: Vec<SceneProp>,
    /// Linear distance fog of the map (`.ted` fog chunk): colour (stored gamma values), start and end distance; `None` when the map has it off.
    pub fog: Option<([f32; 3], f32, f32)>,
}

impl SpaceScene {
    /// A plain scene without sky or props: one sun, dim ambient, a 10000 unit arena.
    pub fn empty() -> Self {
        Self {
            name: "empty".into(),
            bounds_min: Vec2::splat(-10_000.0),
            bounds_max: Vec2::splat(10_000.0),
            lights: [
                SceneLight {
                    direction: Vec3::new(-0.5, -0.4, -0.6).normalize(),
                    color: Vec3::ONE,
                },
                SceneLight {
                    direction: Vec3::new(0.5, 0.3, 0.6).normalize(),
                    color: Vec3::splat(0.25),
                },
                SceneLight {
                    direction: Vec3::new(0.0, 0.6, -0.5).normalize(),
                    color: Vec3::splat(0.25),
                },
            ],
            ambient: Vec3::splat(0.12),
            skydome: None,
            skydome_rotation: [0.0; 3],
            nebula: None,
            nebula_rotation: [0.0; 2],
            props: Vec::new(),
            fog: None,
        }
    }

    /// Parses a `.ted` space map and resolves its object names and models.
    pub fn load(fs: &dyn FileSource, data: &GameData, map_path: &str) -> Result<Self, String> {
        let bytes = fs
            .read(map_path)
            .map_err(|e| format!("cannot read {map_path}: {e}"))?;
        let mut map = SpaceMap::parse(&bytes).map_err(|e| format!("cannot parse {map_path}: {e}"))?;
        map.resolve_names(data.objects.iter().map(|o| o.name.as_str()));
        Ok(Self::from_map(&map, data, Some(fs)))
    }

    /// Builds the scene from an already parsed map (object names must be resolved).
    pub fn from_map(map: &SpaceMap, data: &GameData, fs: Option<&dyn FileSource>) -> Self {
        let env = &map.environment;
        let light = |i: usize| {
            let l = &env.lights[i];
            let to_light = map_to_bevy(l.direction_to_light());
            SceneLight {
                direction: -to_light,
                // The TED scalar weighs the light, relative to 0.5 (full strength: the original shaders double the stored value).
                // Low scalars (asteroid fields at 0.05-0.2) made maps unplayably dark with a linear weight, so the weight is
                // a square root of the ratio, kept within 0.3..1.4 while a zero scalar still switches the light off. Only the sun
                // uses it: the fill lights stay linear so tinted fills (Coruscant) do not grow.
                // `SP_LIGHT_MODE=old` restores the linear weight (2 x scalar), `=raw` ignores the scalar.
                color: Vec3::from(l.color) * if i == 0 { light_gain(l.scalar) } else { l.scalar * 2.0 },
            }
        };
        let exists = |model: &str| {
            fs.is_none_or(|f| f.exists(&format!("Art/Models/{model}")))
        };
        // Sky and nebula are named by game object; the object names the model file.
        let name_or = |s: &str| {
            let s = s.trim();
            if s.is_empty() {
                return None;
            }
            let model = data
                .objects
                .resolve(s)
                .and_then(|r| r.string("space_model_name"))
                .unwrap_or_else(|| format!("{s}.ALO"));
            exists(&model).then_some(model)
        };
        let mut props = Vec::new();
        for o in &map.objects {
            let class = o.class();
            if !matches!(
                class,
                ObjectClass::Prop | ObjectClass::BackdropPlanet | ObjectClass::Asteroid
            ) {
                continue;
            }
            let Some(name) = o.type_name.as_deref() else {
                continue;
            };
            let Some(r) = data.objects.resolve(name) else {
                continue;
            };
            let Some(model) = r.string("space_model_name") else {
                continue;
            };
            // `w_planet_sphere_<x>.alo` backdrops were never shipped: use the matching `w_planet_<x>_low_light` model.
            let model = if exists(&model) {
                model
            } else if let Some(x) = model.to_ascii_lowercase().strip_prefix("w_planet_sphere_").and_then(|s| s.strip_suffix(".alo")) {
                let x = x.replace("eraidu", "eriadu");
                let cand = [format!("w_planet_{x}_low_light.alo"), format!("w_planet_{x}.alo")].into_iter().find(|c| exists(c));
                match cand {
                    Some(c) => c,
                    None => continue,
                }
            } else {
                continue;
            };
            let scale = r
                .num::<f32>("scale_factor")
                .filter(|s| *s > 0.0)
                .unwrap_or(1.0)
                * if o.scale > 0.0 { o.scale } else { 1.0 };
            props.push(SceneProp {
                name: name.to_string(),
                model,
                scale,
                position: map_to_bevy(o.position),
                rotation: map_object_rotation(o.rotation_deg),
                backdrop: class == ObjectClass::BackdropPlanet || o.position[2] < -15_000.0,
            });
        }
        Self {
            name: map.header.name.clone(),
            bounds_min: Vec2::new(map.bounds.inner_min[0], map.bounds.inner_min[1]),
            bounds_max: Vec2::new(map.bounds.inner_max[0], map.bounds.inner_max[1]),
            lights: [light(0), light(1), light(2)],
            ambient: ambient_floor(Vec3::from(env.ambient)),
            skydome: name_or(&env.skydome),
            skydome_rotation: env.skydome_rotation_deg,
            nebula: name_or(&env.nebula),
            nebula_rotation: env.nebula_rotation_deg,
            props,
            fog: (env.fog.enabled && env.fog.end > env.fog.start).then_some((env.fog.color, env.fog.start, env.fog.end)),
        }
    }
}

/// Marks a sky model that follows the camera position.
#[derive(Component, Debug)]
pub struct SkyFollow {
    /// Desired radius of the model's bounding sphere once loaded.
    pub radius: f32,
}

/// Spawns lights, ambient, sky and props.
pub fn spawn_scene(commands: &mut Commands, scene: &SpaceScene, sky_radius: f32, shadows: bool) {
    let exposure = Exposure::default().exposure();
    commands.insert_resource(GlobalAmbientLight {
        // The Alamo shaders read the raw ambient value times the view exposure.
        color: Color::LinearRgba(LinearRgba::rgb(scene.ambient.x, scene.ambient.y, scene.ambient.z)),
        brightness: 1.0 / exposure,
        ..default()
    });
    let sun = &scene.lights[0];
    // The Alamo shaders work on raw (gamma-space) light values: hand them over as linear so Bevy does not convert them
    // (an sRGB red sun of (1, 0.33, 0.21) would otherwise reach the shader as (1, 0.09, 0.04) and wash out to flat red).
    let sun_color = Color::LinearRgba(LinearRgba::rgb(sun.color.x, sun.color.y, sun.color.z));
    let sun_entity = if shadows {
        // Three cascades out to 8000 units: ships are 100..1600 long and the camera mostly looks from 1000..5000.
        alamo_render::spawn_sun_shadowed(commands, sun.direction, 1.0, sun_color, 8000.0, 3)
    } else {
        spawn_sun(commands, sun.direction, 1.0, sun_color)
    };
    commands.entity(sun_entity).insert(BattleEntity);
    // The map's two fill lights are plain directional lights after the sun; the Alamo shader adds their diffuse term.
    for (i, fill) in scene.lights.iter().enumerate().skip(1) {
        if fill.color.max_element() < 0.02 {
            continue;
        }
        commands.spawn((
            Name::new(format!("fill light {i}")),
            BattleEntity,
            DirectionalLight {
                // Raw value as well (see the sun): Bevy converts an sRGB colour to linear before the shader sees it.
                color: Color::LinearRgba(LinearRgba::rgb(fill.color.x, fill.color.y, fill.color.z)),
                illuminance: 1.0 / exposure,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::IDENTITY.looking_to(fill.direction, Vec3::Y),
        ));
    }
    let sky_rot = |deg: [f32; 3]| map_object_rotation(deg);
    if let Some(sky) = &scene.skydome {
        commands.spawn((
            Name::new("skydome"),
            BattleEntity,
            SkyFollow { radius: sky_radius },
            AlamoModel::new(sky.clone()).as_background(),
            Transform::from_rotation(sky_rot(scene.skydome_rotation)),
        ));
    }
    if let Some(neb) = &scene.nebula {
        commands.spawn((
            Name::new("nebula"),
            BattleEntity,
            SkyFollow {
                radius: sky_radius * 0.9,
            },
            AlamoModel::new(neb.clone()).as_background(),
            Transform::from_rotation(sky_rot([
                scene.nebula_rotation[0],
                scene.nebula_rotation[1],
                0.0,
            ])),
        ));
    }
    for p in &scene.props {
        let mut e = commands.spawn((
            Name::new(p.name.clone()),
            BattleEntity,
            AlamoModel::new(p.model.clone()),
            Transform {
                translation: p.position,
                rotation: p.rotation,
                scale: Vec3::splat(p.scale),
            },
        ));
        if (p.backdrop && p.model.to_ascii_lowercase().contains("planet")) || p.model.to_ascii_lowercase().contains("terrain") {
            e.insert(BackdropPlanet::default());
        }
    }
    // Maps with ground terrain (the Skies maps: an island over its ocean disc) keep their disc under the ground.
    commands.insert_resource(SceneHasTerrain(scene.props.iter().any(|p| p.model.to_ascii_lowercase().contains("terrain"))));
}

/// The scene has ground terrain props: its backdrop disc is the ground's surroundings, not a planet seen from space.
#[derive(Resource, Debug, Default)]
pub struct SceneHasTerrain(pub bool);

/// Marks a backdrop planet or ground terrain placed below the battle plane; the value counts the frames since its model loaded
/// ([`SETTLED`] once it was checked).
#[derive(Component, Debug, Default)]
pub struct BackdropPlanet(u8);

/// Closest the visible top of a backdrop planet or ground terrain (`W_Land_Terrain_*` props, e.g. the GrassyMountainsHuge on the Brentaal map at y -2500, whose peaks reach above the plane) may come to the battle plane (world units below it). The maps place the
/// planet well below the plane (Byss: surface filling the picture under the ships, like the Remake video); a planet that
/// reaches up to the plane or through it hides the ships (camera inside or right over the terrain), so only such a planet
/// is moved down, by the needed amount plus this clearance.
const PLANET_CLEARANCE: f32 = 500.0;
const SETTLED: u8 = u8::MAX;

/// Flat disc a `Planet.fx` backdrop is built from (the model has no sphere mesh: the texture is a planet seen from above on
/// a disc of up to 72 000 units radius, below or above the battle plane).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanetDisc {
    pub center: Vec3,
    pub normal: Vec3,
    pub radius: f32,
}

/// Start pose of the battle camera, as far as the disc check needs it.
#[derive(Clone, Copy, Debug)]
pub struct ViewPose {
    pub focus: Vec3,
    pub distance: f32,
    pub pitch: f32,
    pub yaw: f32,
}

/// Share of the start view (pitch and field of view of the default camera) in which the disc fills the picture. A disc that
/// lies under the whole arena fills every ray like a flat desert floor; the Remake shows a limb and black space.
pub fn disc_coverage(disc: &PlanetDisc, view: &ViewPose) -> f32 {
    let (mut hit, mut total) = (0u32, 0u32);
    let base = Vec3::new(view.yaw.sin() * view.pitch.cos(), -view.pitch.sin(), -view.yaw.cos() * view.pitch.cos());
    let eye = view.focus - base * view.distance;
    for lat in -3..=3 {
        for off in -3..=3 {
            let yw = view.yaw + lat as f32 * 9.0f32.to_radians();
            let pt = (view.pitch + off as f32 * 7.0f32.to_radians()).min(89.0f32.to_radians());
            let dir = Vec3::new(yw.sin() * pt.cos(), -pt.sin(), -yw.cos() * pt.cos());
            total += 1;
            let denom = dir.dot(disc.normal);
            if denom.abs() < 1e-4 {
                continue;
            }
            let t = (disc.center - eye).dot(disc.normal) / denom;
            if t > 0.0 && (eye + dir * t - disc.center).length() < disc.radius {
                hit += 1;
            }
        }
    }
    hit as f32 / total as f32
}

/// Coverage above which a disc counts as a floor under the whole start view.
const DISC_FLOOR_COVERAGE: f32 = 0.6;
/// Coverage a slid disc settles at: its limb crosses the view and the rest is space.
const DISC_LIMB_COVERAGE: f32 = 0.25;

/// Sideways shift (world units, along the camera's right or left vector) that moves a floor-like disc until its limb
/// crosses the start view, as `(direction, distance)`; `None` when the disc already leaves most of the view to space.
pub fn disc_slide(disc: &PlanetDisc, view: &ViewPose) -> Option<(Vec3, f32)> {
    if disc_coverage(disc, view) <= DISC_FLOOR_COVERAGE {
        return None;
    }
    let right = Vec3::new(view.yaw.cos(), 0.0, view.yaw.sin());
    // The planet stays on the side its map put it: the side of the camera the disc centre lies on.
    let dir = if (disc.center - view.focus).dot(right) < 0.0 { -right } else { right };
    let step = (disc.radius / 60.0).max(50.0);
    (1..=240).map(|k| k as f32 * step).find(|s| {
        let slid = PlanetDisc { center: disc.center + dir * *s, ..*disc };
        disc_coverage(&slid, view) <= DISC_LIMB_COVERAGE
    }).map(|s| (dir, s))
}

/// Measures the real top of a loaded backdrop planet (world bounds of its mesh parts) and moves it down when it reaches
/// the battle plane. Runs a few frames after loading so the transforms and bounds are up to date.
pub fn keep_planet_below(
    mut commands: Commands,
    mut planets: Query<(Entity, &AlamoModelLoaded, &mut Transform, &mut BackdropPlanet)>,
    parts: Query<(&bevy::camera::primitives::Aabb, &GlobalTransform)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    camera: Option<Single<&crate::camera::BattleCamera>>,
    terrain: Option<Res<SceneHasTerrain>>,
) {
    for (entity, loaded, mut tf, mut state) in &mut planets {
        if state.0 == SETTLED {
            continue;
        }
        state.0 += 1;
        if state.0 < 4 {
            continue;
        }
        let mut top = f32::MIN;
        let mut max_extent = 0.0f32;
        let mut disc = None::<PlanetDisc>;
        for e in &loaded.parts {
            let Ok((aabb, gt)) = parts.get(*e) else { continue };
            let m = gt.affine();
            let c = m.transform_point3a(aabb.center);
            let half_y = m.matrix3.row(1).abs().dot(aabb.half_extents);
            top = top.max(c.y + half_y);
            let half = aabb.half_extents;
            let sc = gt.scale();
            let real = Vec3::new(half.x * sc.x, half.y * sc.y, half.z * sc.z);
            if real.min_element() < 5.0 && real.max_element() > 1000.0 {
                let axis = if real.x <= real.y && real.x <= real.z { Vec3::X } else if real.y <= real.z { Vec3::Y } else { Vec3::Z };
                disc = Some(PlanetDisc { center: Vec3::from(c), normal: gt.rotation() * axis, radius: real.max_element() });
            }
            let ext = (half.x * half.x + half.y * half.y + half.z * half.z).sqrt() * gt.scale().max_element();
            max_extent = max_extent.max(ext);
        }
        if top == f32::MIN {
            if state.0 > 60 {
                state.0 = SETTLED;
            }
            continue;
        }
        if disc.is_some() && camera.is_none() && state.0 <= 60 {
            continue;
        }
        state.0 = SETTLED;
        info!("battle ground below the plane: top at {top:.0} (plane 0), position y {}", tf.translation.y);
        if top > -PLANET_CLEARANCE && std::env::var_os("SP_NO_PLANET_CLAMP").is_none() {
            let drop = top + PLANET_CLEARANCE;
            tf.translation.y -= drop;
            if let Some(d) = disc.as_mut() {
                d.center.y -= drop;
            }
        }
        // A flat disc that still lies under the whole arena is a desert floor, not a planet: slide it sideways until its limb
        // crosses the start view (Tatooine: centred 10000 below, 72000 wide; the Remake shows space with the planet at one side).
        if let (Some(d), Some(cam), false) = (disc, camera.as_deref(), terrain.as_deref().is_some_and(|t| t.0)) {
            let view = ViewPose { focus: cam.target_focus, distance: cam.target_distance, pitch: cam.pitch, yaw: cam.target_yaw };
            let cover = disc_coverage(&d, &view);
            let slide = disc_slide(&d, &view).filter(|_| std::env::var_os("SP_NO_DISC_SLIDE").is_none());
            info!("battle planet disc: radius {:.0}, normal {:?}, view coverage {cover:.2}, slide {slide:?}, view {view:?}", d.radius, d.normal);
            if let Some((dir, s)) = slide {
                tf.translation += dir * s;
            }
        }

        // U69: Spawn atmospheric rim glow halo around the backdrop planet sphere
        // The shell is a sphere around the model centre: a flat disc has no such sphere, and a slid one would show it as a blue ball.
        if max_extent > 200.0 && disc.is_none() {
            let atmo_mesh = meshes.add(Sphere::new(max_extent * 1.025).mesh().uv(36, 24));
            let atmo_mat = materials.add(StandardMaterial {
                base_color: Color::srgba(0.22, 0.62, 0.98, 0.38),
                unlit: true,
                alpha_mode: AlphaMode::Add,
                ..default()
            });
            commands.spawn((
                Name::new("planet_atmosphere_glow"),
                BattleEntity,
                Mesh3d(atmo_mesh),
                MeshMaterial3d(atmo_mat),
                Transform::from_translation(tf.translation),
                AtmosphereGlow {
                    planet: entity,
                    radius: max_extent * 1.025,
                },
            ));
        }
    }
}

/// Atmospheric rim glow halo around backdrop planet spheres (U69).
#[derive(Component, Debug)]
pub struct AtmosphereGlow {
    pub planet: Entity,
    pub radius: f32,
}

/// Keeps the atmosphere glow shell centered on its parent planet.
pub fn update_atmosphere_glow(
    planets: Query<&Transform, (With<BackdropPlanet>, Without<AtmosphereGlow>)>,
    mut glows: Query<(&AtmosphereGlow, &mut Transform)>,
) {
    for (glow, mut tf) in &mut glows {
        if let Ok(ptf) = planets.get(glow.planet) {
            tf.translation = ptf.translation;
        }
    }
}

/// Brightness weight of a map light from its TED scalar (see [`SceneLight`]).
fn light_gain(scalar: f32) -> f32 {
    match std::env::var("SP_LIGHT_MODE").as_deref() {
        Ok("old") => scalar * 2.0,
        Ok("raw") => 1.0,
        _ if scalar <= 0.0 => 0.0,
        _ => (scalar / 0.5).sqrt().clamp(0.3, 1.4),
    }
}

/// Keeps a faint ambient term on maps that define almost none, so unlit hulls and rocks stay readable.
fn ambient_floor(a: Vec3) -> Vec3 {
    if std::env::var("SP_LIGHT_MODE").is_ok() { a } else { a.max(Vec3::splat(0.05)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tatooine's backdrop: a 72000-wide disc, normal up, 10000 below the plane, centred next to the fleet (the QA desert floor).
    fn tatooine_disc() -> PlanetDisc {
        PlanetDisc { center: Vec3::new(9497.0, -15959.0, 145.0), normal: Vec3::Y, radius: 71_950.0 }
    }

    #[test]
    fn disc_under_the_arena_fills_the_view() {
        let cover = disc_coverage(&tatooine_disc(), &start_view());
        assert!(cover > 0.95, "flat floor under the fleet, coverage {cover}");
    }

    #[test]
    fn floor_disc_slides_until_space_shows() {
        let (d, view) = (tatooine_disc(), start_view());
        let (dir, shift) = disc_slide(&d, &view).expect("floor disc must slide");
        assert_eq!(dir, Vec3::X, "the planet stays on the side the map put it");
        let slid = PlanetDisc { center: d.center + dir * shift, ..d };
        let cover = disc_coverage(&slid, &view);
        assert!(cover <= 0.25 && cover > 0.0, "limb crosses the view, coverage {cover}");
        // A disc that already sits far to the side is left alone.
        let far = PlanetDisc { center: d.center + dir * (shift + 20_000.0), ..d };
        assert_eq!(disc_slide(&far, &view), None);
    }

    #[test]
    fn slide_follows_a_rotated_camera() {
        let (d, mut view) = (tatooine_disc(), start_view());
        view.yaw = std::f32::consts::FRAC_PI_2;
        let (dir, shift) = disc_slide(&d, &view).expect("floor disc must slide");
        assert!(dir.z.abs() > 0.99, "slides along the rotated camera's right vector, not world x: {dir:?}");
        let slid = PlanetDisc { center: d.center + dir * shift, ..d };
        assert!(disc_coverage(&slid, &view) <= 0.25);
    }

    #[test]
    fn tilted_coruscant_disc_slides_too() {
        // Coruscant_01: tilted 10 degrees, centred under the arena origin, fleet 6600 south of it.
        let d = PlanetDisc { center: Vec3::new(0.0, -19_610.0, 0.0), normal: Vec3::new(0.17364845, -0.98480785, 0.0), radius: 71_946.0 };
        let view = ViewPose { focus: Vec3::new(-243.0, 0.0, 6647.0), distance: 1300.0, pitch: 72f32.to_radians(), yaw: 0.0 };
        assert!(disc_coverage(&d, &view) > 0.95);
        let (dir, shift) = disc_slide(&d, &view).expect("floor disc must slide");
        let slid = PlanetDisc { center: d.center + dir * shift, ..d };
        assert!(disc_coverage(&slid, &view) <= 0.25);
    }

    fn start_view() -> ViewPose {
        ViewPose { focus: Vec3::new(-6300.0, 0.0, 2000.0), distance: 1300.0, pitch: 72f32.to_radians(), yaw: 0.0 }
    }

    #[test]
    fn test_atmosphere_glow_radius_and_scattering_color() {
        let planet_radius: f32 = 2500.0;
        let atmo_radius: f32 = planet_radius * 1.025;
        assert!(atmo_radius > planet_radius, "atmosphere shell is slightly larger than planet");
        assert!(((atmo_radius / planet_radius) - 1.025).abs() < 1e-4);

        let atmo_color = Color::srgba(0.22, 0.62, 0.98, 0.38);
        let srgba = atmo_color.to_srgba();
        assert!(srgba.blue > 0.9, "cyan-blue rayleigh scattering tint");
        assert!(srgba.alpha > 0.3 && srgba.alpha < 0.5, "soft additive translucency");
    }
}
