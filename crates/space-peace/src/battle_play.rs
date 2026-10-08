//! The real tactical battle: turns a prepared `TacticalRequest` into a `battle-view` session when
//! the `SpaceBattle` screen opens, and hands the result back to the galaxy simulation.
//!
//! Flow: "Fight" in the pending-battle dialog -> `galaxy-play` asks the sim worker for the tactical
//! setup -> `PlayEvent::TacticalReady` (see `play.rs`) stores [`PendingTactical`] and opens the screen
//! -> [`begin_session`] inserts the `BattleSession` -> `battle_view::BattleFinished` ->
//! `PlayAction::ApplyTactical` and back to the map. F10 abandons the fight (the sim auto-resolves it).

use alamo_render::SunLight;
use battle_view::{BattleCamera, BattleEntity, BattleSession, BattleStatus, BattleViewSettings};
use bevy::camera::{ImageRenderTarget, RenderTarget};
use alamo_audio::{MusicDirector, MusicScene};
use bevy::light::GlobalAmbientLight;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use eaw_ui::EawUiSettings;
use galaxy_play::{PlayAction, PlayerFaction};
use galaxy_sim::TacticalRequest;

use crate::battle_seam::{BattleFinished, BattleOutcome};
use crate::camera::GalaxyCamera;
use crate::cli::Args;
use crate::flow::AppScreen;
use crate::galaxy_map::Skybox;
use crate::game::Game;
use crate::offscreen::OffscreenTarget;
use crate::play::ScriptHold;
use crate::ui::HudEntity;

/// The tactical fight the sim prepared for the battle being opened.
#[derive(Resource, Debug, Clone)]
pub struct PendingTactical(pub Box<TacticalRequest>);

/// What was changed for the battle and must be put back afterwards.
#[derive(Resource, Default)]
struct SavedLook {
    ambient: Option<GlobalAmbientLight>,
    hud: Vec<(Entity, Visibility)>,
    /// The galaxy sun and its illuminance: it must not compete with the battle's sun.
    suns: Vec<(Entity, f32)>,
}

/// Hint text of the battle screen.
#[derive(Component)]
struct BattleHint;

/// Offscreen scenario state: the mid-battle capture.
#[derive(Resource, Default)]
struct BattleShot {
    frames: u32,
    taken: bool,
}

/// Wires the battle screen to `battle-view`.
pub struct BattlePlayPlugin;

impl Plugin for BattlePlayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SavedLook>()
            .init_resource::<BattleShot>()
            .add_systems(
                OnEnter(AppScreen::SpaceBattle),
                (suspend_galaxy_look, begin_session).chain().run_if(resource_exists::<PendingTactical>),
            )
            .add_systems(OnExit(AppScreen::SpaceBattle), end_session)
            .add_systems(
                Update,
                (finish_battle_view, abort_key, battle_ui_camera, battle_shot, battle_header)
                    .run_if(in_state(AppScreen::SpaceBattle).and_then(resource_exists::<PendingTactical>)),
            );
    }
}

/// Dims the map around the battle: galaxy camera, sun, ambient light and HUD make way for the
/// battle's own.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn suspend_galaxy_look(
    mut commands: Commands,
    mut saved: ResMut<SavedLook>,
    ambient: Option<Res<GlobalAmbientLight>>,
    mut cameras: Query<(Entity, &mut Camera), With<GalaxyCamera>>,
    mut suns: Query<(Entity, &mut DirectionalLight), (With<SunLight>, Without<BattleEntity>)>,
    mut hud: Query<(Entity, &mut Visibility), (Or<(With<HudEntity>, With<Skybox>)>, Without<SunLight>)>,
    music: Option<ResMut<MusicDirector>>,
) {
    if let Some(mut m) = music {
        m.set_scene(MusicScene::BattleLoading);
    }
    saved.ambient = ambient.map(|a| a.clone());
    for (e, mut cam) in &mut cameras {
        cam.is_active = false;
        commands.entity(e).remove::<IsDefaultUiCamera>();
    }
    saved.suns.clear();
    for (e, mut light) in &mut suns {
        saved.suns.push((e, light.illuminance));
        light.illuminance = 0.0;
    }
    saved.hud.clear();
    for (e, mut v) in &mut hud {
        saved.hud.push((e, *v));
        *v = Visibility::Hidden;
    }
    commands.spawn((
        BattleHint,
        Text::new("F10: abandon the battle (auto-resolve)"),
        TextFont { font_size: FontSize::Px(13.0), ..default() },
        TextColor(Color::srgba(0.75, 0.85, 1.0, 0.6)),
        Node { position_type: PositionType::Absolute, top: Val::Px(6.0), left: Val::Px(8.0), ..default() },
    ));
}

/// Builds the battle from the prepared request.
fn begin_session(world: &mut World) {
    let Some(req) = world.get_resource::<PendingTactical>().map(|p| (*p.0).clone()) else { return };
    let args = world.resource::<Args>().clone();
    let player = world.get_resource::<PlayerFaction>().and_then(|p| p.0);
    let target = world.get_resource::<OffscreenTarget>().map(|t| (t.image.clone(), t.scale_factor));
    let Some(assets) = world.resource::<Game>().0.clone() else { return };
    let started = std::time::Instant::now();
    // `--battle-auto` lets the AI play both sides (headless scenarios).
    let side_player = if args.battle_auto { None } else { player };
    match BattleSession::from_request(&assets.data, &*assets.fs, &req, side_player) {
        Ok(session) => {
            info!("battle: session for {} built in {:.2?}", req.planet_name, started.elapsed());
            let mut settings = world.resource_mut::<BattleViewSettings>();
            settings.speed = args.battle_speed.unwrap_or(1.0);
            match target {
                Some((image, scale)) => {
                    settings.render_target = Some(image);
                    settings.target_scale = scale;
                    settings.blocking = true;
                    settings.synthetic_pointer = true;
                    settings.audio = false;
                }
                None => settings.audio = true,
            }
            world.insert_resource(session);
        }
        Err(e) => {
            error!("battle: cannot build the tactical session: {e}");
            world.write_message(BattleFinished { id: req.battle_id, outcome: BattleOutcome::Aborted });
        }
    }
}

/// The battle's own camera takes over the UI.
fn battle_ui_camera(mut commands: Commands, cams: Query<Entity, Added<BattleCamera>>, settings: Option<ResMut<EawUiSettings>>) {
    let Some(e) = cams.iter().next() else { return };
    commands.entity(e).insert(IsDefaultUiCamera);
    if let Some(mut s) = settings {
        s.camera = Some(e);
    }
}

/// The battle reached its end: the result goes into the simulation and the map comes back.
fn finish_battle_view(
    mut finished: MessageReader<battle_view::BattleFinished>,
    mut actions: MessageWriter<PlayAction>,
    mut done: MessageWriter<BattleFinished>,
    music: Option<ResMut<MusicDirector>>,
    pending: Res<PendingTactical>,
    player: Res<PlayerFaction>,
    campaign: Res<crate::flow::CurrentCampaign>,
) {
    let mut music = music;
    for f in finished.read() {
        if let Some(m) = music.as_mut() {
            let win = f.result.winner.and_then(|w| pending.0.sides.get(w)).is_some_and(|s| Some(s.faction) == player.0);
            let faction = campaign.faction.clone().unwrap_or_else(|| "Rebel".to_owned());
            m.set_scene(MusicScene::Result { faction, win, tactical: true });
        }
        info!("battle: over after {:.0} s, winner side {:?}, survivors {:?}", f.result.duration, f.result.winner, f.result.survived);
        let outcome = f.result.winner.map_or(BattleOutcome::Draw, BattleOutcome::Victory);
        actions.write(PlayAction::ApplyTactical { id: f.battle_id, result: Box::new(f.result.clone()) });
        done.write(BattleFinished { id: f.battle_id, outcome });
    }
}

/// F10 gives the fight up; the sim settles the battle with its auto-resolve.
fn abort_key(keys: Res<ButtonInput<KeyCode>>, pending: Res<PendingTactical>, mut done: MessageWriter<BattleFinished>) {
    if keys.just_pressed(KeyCode::F10) {
        done.write(BattleFinished { id: pending.0.battle_id, outcome: BattleOutcome::Aborted });
    }
}

/// Offscreen scenario: one capture in the middle of the battle (`<out>_battle.png`).
fn battle_shot(
    mut commands: Commands,
    args: Res<Args>,
    status: Res<BattleStatus>,
    target: Option<Res<OffscreenTarget>>,
    ships: Res<battle_view::RenderedShips>,
    mut shot: ResMut<BattleShot>,
) {
    let (Some(out), Some(target)) = (args.offscreen.as_ref(), target) else { return };
    shot.frames += 1;
    let at = args.battle_seconds.unwrap_or(12.0);
    // Models and textures load asynchronously: wait a couple of hundred frames.
    if shot.taken || status.tick == 0 || status.time < at || shot.frames < 200 {
        return;
    }
    shot.taken = true;
    let mut path = out.clone();
    let stem = path.file_stem().map_or_else(|| "out".into(), |s| s.to_string_lossy().into_owned());
    path.set_file_name(format!("{stem}_battle.png"));
    info!("battle: capture at t={:.1}s alive {:?} rendered {} -> {}", status.time, status.alive, ships.0.len(), path.display());
    commands
        .spawn(Screenshot(RenderTarget::Image(ImageRenderTarget { handle: target.image.clone(), scale_factor: target.scale_factor })))
        .observe(save_to_disk(path));
}

/// Back on the map: everything the battle replaced is restored.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn end_session(
    mut commands: Commands,
    mut saved: ResMut<SavedLook>,
    mut cameras: Query<(Entity, &mut Camera), With<GalaxyCamera>>,
    mut suns: Query<&mut DirectionalLight, (With<SunLight>, Without<BattleEntity>)>,
    mut hud: Query<&mut Visibility, (Or<(With<HudEntity>, With<Skybox>)>, Without<SunLight>)>,
    hints: Query<Entity, With<BattleHint>>,
    settings: Option<ResMut<EawUiSettings>>,
    mut hold: ResMut<ScriptHold>,
    mut shot: ResMut<BattleShot>,
) {
    commands.remove_resource::<BattleSession>();
    commands.remove_resource::<PendingTactical>();
    for e in &hints {
        commands.entity(e).despawn();
    }
    if let Some(a) = saved.ambient.take() {
        commands.insert_resource(a);
    }
    for (e, mut cam) in &mut cameras {
        cam.is_active = true;
        commands.entity(e).insert(IsDefaultUiCamera);
    }
    for (e, illuminance) in saved.suns.drain(..) {
        if let Ok(mut light) = suns.get_mut(e) {
            light.illuminance = illuminance;
        }
    }
    for (e, v) in saved.hud.drain(..) {
        if let Ok(mut vis) = hud.get_mut(e) {
            *vis = v;
        }
    }
    if let Some(mut s) = settings {
        s.camera = None;
    }
    *shot = BattleShot::default();
    hold.waiting_battle = false;
    hold.done_at = Some(std::time::Instant::now());
}

/// Header of the tactical bar: the planet the battle is at (the bar shares `CommandBarData` with the
/// galactic bar).
fn battle_header(pending: Res<PendingTactical>, state: Option<Res<galaxy_play::SimState>>, data: Option<ResMut<eaw_ui::CommandBarData>>) {
    let (Some(state), Some(mut data)) = (state, data) else { return };
    let name = state.world.planets[pending.0.planet as usize].display_name.clone();
    if data.planet_name != name {
        data.planet_name = name;
        data.planet_owner = None;
        data.planet_income = 0;
    }
}
