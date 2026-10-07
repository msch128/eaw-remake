//! The static scene of a space map: sky, nebula, lights, ambient, bounds and placed props.

use alamo_data::GameData;
use alamo_fs::FileSource;
use alamo_map::{ObjectClass, SpaceMap};
use alamo_render::{AlamoModel, spawn_sun};
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
        }
    }

    /// Parses a `.ted` space map and resolves its object names and models.
    pub fn load(fs: &dyn FileSource, data: &GameData, map_path: &str) -> Result<Self, String> {
        let bytes = fs
            .read(map_path)
            .map_err(|e| format!("cannot read {map_path}: {e}"))?;
        let mut map = SpaceMap::parse(&bytes).map_err(|e| format!("cannot parse {map_path}: {e}"))?;
        map.resolve_names(data.objects.iter().map(|o| o.name.as_str()));
        Ok(Self::from_map(&map, data))
    }

    /// Builds the scene from an already parsed map (object names must be resolved).
    pub fn from_map(map: &SpaceMap, data: &GameData) -> Self {
        let env = &map.environment;
        let light = |i: usize| {
            let l = &env.lights[i];
            let to_light = map_to_bevy(l.direction_to_light());
            SceneLight {
                direction: -to_light,
                color: Vec3::from(l.color),
            }
        };
        let name_or = |s: &str| (!s.trim().is_empty()).then(|| format!("{}.ALO", s.trim()));
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
            ambient: Vec3::from(env.ambient),
            skydome: name_or(&env.skydome),
            skydome_rotation: env.skydome_rotation_deg,
            nebula: name_or(&env.nebula),
            nebula_rotation: env.nebula_rotation_deg,
            props,
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
pub fn spawn_scene(commands: &mut Commands, scene: &SpaceScene, sky_radius: f32) {
    let exposure = Exposure::default().exposure();
    commands.insert_resource(GlobalAmbientLight {
        // The Alamo shaders read the raw ambient value times the view exposure.
        color: Color::LinearRgba(LinearRgba::rgb(scene.ambient.x, scene.ambient.y, scene.ambient.z)),
        brightness: 1.0 / exposure,
        ..default()
    });
    let sun = &scene.lights[0];
    let sun_entity = spawn_sun(
        commands,
        sun.direction,
        1.0,
        Color::srgb(sun.color.x, sun.color.y, sun.color.z),
    );
    commands.entity(sun_entity).insert(BattleEntity);
    for l in &scene.lights[1..] {
        commands.spawn((
            Name::new("fill light"),
            BattleEntity,
            DirectionalLight {
                color: Color::srgb(l.color.x, l.color.y, l.color.z),
                illuminance: 0.0,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::IDENTITY.looking_to(l.direction, Vec3::Y),
        ));
    }
    let sky_rot = |deg: [f32; 3]| map_object_rotation(deg);
    if let Some(sky) = &scene.skydome {
        commands.spawn((
            Name::new("skydome"),
            BattleEntity,
            SkyFollow { radius: sky_radius },
            AlamoModel::new(sky.clone()),
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
            AlamoModel::new(neb.clone()),
            Transform::from_rotation(sky_rot([
                scene.nebula_rotation[0],
                scene.nebula_rotation[1],
                0.0,
            ])),
        ));
    }
    for p in &scene.props {
        commands.spawn((
            Name::new(p.name.clone()),
            BattleEntity,
            AlamoModel::new(p.model.clone()),
            Transform {
                translation: p.position,
                rotation: p.rotation,
                scale: Vec3::splat(p.scale),
            },
        ));
    }
}
