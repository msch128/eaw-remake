//! The title screen scene: one of the game's own attract-mode space maps (`Demo_Attract_Maps` in
//! `GameConstants`) rendered live behind the main menu with a slow camera drift. It is spawned when the
//! main menu opens and removed when the menu is left.

use alamo_render::SunLight;
use battle_view::SpaceScene;
use bevy::light::GlobalAmbientLight;
use bevy::prelude::*;

use crate::camera::GalaxyCamera;
use crate::flow::AppScreen;
use crate::game::Game;
use crate::menu::MenuBackdrop;

/// The maps the title screen picks from.
const MAPS: [&str; 3] = ["MenuSpaceSunsetMountains.ted", "MenuSpaceHapes.ted", "MenuSpaceVulcanicPlanet.ted"];

/// Marks everything of the title scene.
#[derive(Component)]
pub struct MenuSceneEntity;

/// What the camera drifts around while the scene is shown.
#[derive(Resource, Debug)]
pub struct MenuSceneInfo {
    center: Vec3,
    radius: f32,
    /// Horizontal direction towards the sun: the camera looks from this side so ships are lit.
    toward_sun: Vec3,
    /// Galaxy lights that were switched off while the scene is shown.
    ambient: Option<GlobalAmbientLight>,
    suns: Vec<(Entity, f32)>,
}

/// Title scene plugin.
pub struct MenuScenePlugin;

impl Plugin for MenuScenePlugin {
    fn build(&self, app: &mut App) {
        // The scene stays behind the campaign selection too (like the original) and goes when the game starts.
        app.add_systems(OnEnter(AppScreen::MainMenu), load_scene.after(crate::menu::spawn_main_menu))
            .add_systems(OnEnter(AppScreen::CampaignSelect), load_scene)
            .add_systems(
                Update,
                spawn_scene.run_if(resource_exists::<PendingScene>).run_if(in_state(AppScreen::MainMenu).or_else(in_state(AppScreen::CampaignSelect))),
            )
            .add_systems(OnEnter(AppScreen::Galaxy), remove_scene)
            .add_systems(Update, (tag_scene_entities, drift_camera).run_if(resource_exists::<MenuSceneInfo>));
    }
}

/// A title scene whose models are being loaded by the preloader.
#[derive(Resource)]
struct PendingScene {
    scene: SpaceScene,
    name: String,
}

/// Reads the map and asks the preloader for its models, so they are ready when the scene spawns (no freeze).
fn load_scene(
    mut commands: Commands,
    game: Res<Game>,
    existing: Option<Res<MenuSceneInfo>>,
    preloader: Option<Res<alamo_render::ModelPreloader>>,
    cache: Option<Res<alamo_render::ModelCache>>,
) {
    let (None, Some(assets)) = (existing, game.0.as_ref()) else { return };
    let pick = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as usize) % MAPS.len();
    let pick = std::env::var("SP_MENU_MAP").ok().and_then(|v| v.parse::<usize>().ok()).map_or(pick, |v| v % MAPS.len());
    let path = format!("Art/Maps/{}", MAPS[pick]);
    let scene = match SpaceScene::load(&*assets.fs, &assets.data, &path) {
        Ok(s) => s,
        Err(e) => {
            warn!("title scene {path}: {e}");
            return;
        }
    };
    if let (Some(pre), Some(cache)) = (preloader, cache) {
        let mut paths: Vec<String> = scene.props.iter().map(|p| p.model.clone()).collect();
        paths.extend(scene.skydome.clone());
        paths.extend(scene.nebula.clone());
        pre.request(&assets.fs, &cache, paths);
    }
    commands.insert_resource(PendingScene { scene, name: MAPS[pick].to_owned() });
}

#[allow(clippy::type_complexity)]
fn spawn_scene(
    mut commands: Commands,
    pending: Res<PendingScene>,
    preloader: Option<Res<alamo_render::ModelPreloader>>,
    ambient: Option<Res<GlobalAmbientLight>>,
    mut suns: Query<(Entity, &mut DirectionalLight), (With<SunLight>, Without<battle_view::BattleEntity>)>,
    backdrop: Query<Entity, With<MenuBackdrop>>,
) {
    if preloader.is_some_and(|p| !p.is_idle()) {
        return;
    }
    let scene = &pending.scene;
    commands.remove_resource::<PendingScene>();
    let props: Vec<Vec3> = scene.props.iter().filter(|p| !p.backdrop).map(|p| p.position).collect();
    let center = if props.is_empty() { Vec3::ZERO } else { props.iter().copied().sum::<Vec3>() / props.len() as f32 };
    // Frame the bulk of the objects (the 60th percentile), so the camera sits outside the cluster but still sees it.
    let mut dists: Vec<f32> = props.iter().map(|p| p.distance(center)).collect();
    dists.sort_by(f32::total_cmp);
    let radius = dists.get(dists.len() * 6 / 10).copied().unwrap_or(800.0).clamp(800.0, 12_000.0);
    info!("title scene {} with {} objects", pending.name, props.len());
    // The galaxy backdrop and the galaxy lights make way for the scene.
    for e in &backdrop {
        commands.entity(e).despawn();
    }
    let mut dimmed = Vec::new();
    for (e, mut light) in &mut suns {
        dimmed.push((e, light.illuminance));
        light.illuminance = 0.0;
    }
    battle_view::scene::spawn_scene(&mut commands, scene, 9_000.0, false);
    commands.insert_resource(MenuSceneInfo { center, radius, toward_sun: Vec3::new(scene.lights[0].direction.x, 0.0, scene.lights[0].direction.z).normalize_or(Vec3::Z), ambient: ambient.map(|a| a.clone()), suns: dimmed });
}

/// The scene's entities are spawned as battle entities; this claims them for the menu.
fn tag_scene_entities(mut commands: Commands, fresh: Query<Entity, (Added<battle_view::BattleEntity>, Without<MenuSceneEntity>)>, session: Option<Res<battle_view::BattleSession>>) {
    if session.is_some() {
        return;
    }
    for e in &fresh {
        commands.entity(e).insert(MenuSceneEntity);
    }
}

#[allow(clippy::type_complexity)]
fn drift_camera(
    time: Res<Time>,
    info: Res<MenuSceneInfo>,
    state: Res<State<AppScreen>>,
    mut camera: Single<&mut Transform, With<GalaxyCamera>>,
    mut sky: Query<(&battle_view::scene::SkyFollow, &alamo_render::AlamoModelLoaded, &mut Transform), Without<GalaxyCamera>>,
) {
    if *state.get() != AppScreen::MainMenu {
        return;
    }
    let t = time.elapsed_secs();
    let angle = (t * 0.1).sin() * 0.6;
    let dist = info.radius * 1.6 + 1200.0;
    let eye = info.center + Quat::from_rotation_y(angle) * (info.toward_sun * dist + Vec3::Y * (dist * 0.22 + 100.0 * (t * 0.3).sin()));
    **camera = Transform::from_translation(eye).looking_at(info.center, Vec3::Y);
    // Sky models stay centred on the camera and scaled to the far plane, like in a battle.
    for (follow, loaded, mut tf) in &mut sky {
        let scale = follow.radius / loaded.radius.max(1.0);
        tf.scale = Vec3::splat(scale);
        tf.translation = eye - tf.rotation * (loaded.center * scale);
    }
}

#[allow(clippy::type_complexity)]
fn remove_scene(
    mut commands: Commands,
    info: Option<Res<MenuSceneInfo>>,
    scene: Query<Entity, With<MenuSceneEntity>>,
    mut galaxy_suns: Query<(Entity, &mut DirectionalLight), (With<SunLight>, Without<MenuSceneEntity>)>,
) {
    commands.remove_resource::<PendingScene>();
    let Some(info) = info else { return };
    for e in &scene {
        commands.entity(e).despawn();
    }
    for (e, illuminance) in &info.suns {
        if let Ok((_, mut light)) = galaxy_suns.get_mut(*e) {
            light.illuminance = *illuminance;
        }
    }
    if let Some(a) = &info.ambient {
        commands.insert_resource(a.clone());
    }
    commands.remove_resource::<MenuSceneInfo>();
}

