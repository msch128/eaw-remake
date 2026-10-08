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
    /// Frame of the attack order of `--battle-orders` (the capture follows a few frames later).
    orders_at: Option<u32>,
}

/// Wires the battle screen to `battle-view`.
pub struct BattlePlayPlugin;

impl Plugin for BattlePlayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SavedLook>()
            .init_resource::<BattleShot>()
            .add_systems(
                OnEnter(AppScreen::SpaceBattle),
                (suspend_galaxy_look, spawn_loading_art).chain().run_if(resource_exists::<PendingTactical>),
            )
            .add_systems(Update, attach_loading_art.run_if(resource_exists::<PendingTactical>))
            .add_systems(Update, start_after_loading_art.run_if(in_state(AppScreen::SpaceBattle).and_then(resource_exists::<PendingTactical>)).before(finish_battle_view))
            .add_systems(Update, poll_session_build.run_if(in_state(AppScreen::SpaceBattle).and_then(resource_exists::<SessionBuild>)))
            .add_systems(OnExit(AppScreen::SpaceBattle), end_session)
            .add_systems(
                Update,
                (finish_battle_view, abort_key, battle_ui_camera, battle_shot, battle_header, battle_orders)
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

/// Root of the battle loading screen and the frames it has been shown.
#[derive(Component)]
struct BattleLoadingArt(u32);

/// Shows a faction loading screen (the session is built a few frames later, because that stalls for a moment).
fn spawn_loading_art(mut commands: Commands, game: Res<Game>, pending: Res<PendingTactical>, player: Res<PlayerFaction>, campaign: Res<crate::flow::CurrentCampaign>) {
    // The player's faction, or the defender's when watching.
    let _ = (&pending, &player);
    // The PNG decode takes tens of milliseconds: do it on a worker thread, the art appears a few frames later
    // (`attach_loading_art`) while the black screen with the text is already up.
    let (tx, rx) = std::sync::mpsc::channel();
    if let Some(g) = game.0.as_ref() {
        let fs = g.fs.clone();
        let faction = campaign.faction.clone();
        std::thread::spawn(move || {
            let _ = tx.send(crate::loading::decode_art(&fs, faction.as_deref()));
        });
    }
    let art: Option<Handle<Image>> = None;
    let mut e = commands.spawn((
        BattleLoadingArt(0),
        LoadingArtJob(std::sync::Mutex::new(Some(rx))),
        Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), justify_content: JustifyContent::Center, align_items: AlignItems::FlexEnd, padding: UiRect::bottom(Val::Px(70.0)), ..default() },
        BackgroundColor(Color::BLACK),
        GlobalZIndex(50),
    ));
    e.with_children(|root| {
        if let Some(h) = art {
            root.spawn((Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() }, ImageNode::new(h).with_mode(NodeImageMode::Stretch)));
        }
        root.spawn((
            Text::new("Preparing the battle..."),
            TextFont { font: FontSource::Family("Bahnschrift".into()), font_size: FontSize::Px(26.0), ..default() },
            TextColor(Color::srgb(0.9, 0.95, 1.0)),
            TextShadow { offset: Vec2::splat(1.5), color: Color::srgba(0.0, 0.0, 0.0, 0.9) },
        ));
    });
}

/// The loading art being decoded on a worker thread.
#[derive(Component)]
struct LoadingArtJob(std::sync::Mutex<Option<std::sync::mpsc::Receiver<Option<Image>>>>);

/// Puts the decoded loading art behind the text as soon as the worker is done.
fn attach_loading_art(mut commands: Commands, mut images: ResMut<Assets<Image>>, jobs: Query<(Entity, &LoadingArtJob)>) {
    for (e, job) in &jobs {
        let mut slot = job.0.lock().unwrap();
        let Some(rx) = slot.as_ref() else { continue };
        match rx.try_recv() {
            Ok(Some(image)) => {
                let h = images.add(image);
                commands.entity(e).with_children(|root| {
                    root.spawn((Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() }, ImageNode::new(h).with_mode(NodeImageMode::Stretch), ZIndex(-1)));
                });
                *slot = None;
            }
            Ok(None) | Err(std::sync::mpsc::TryRecvError::Disconnected) => *slot = None,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }
}

/// Builds the session after the loading screen had a few frames on screen, and removes the screen once the battle shows.
fn start_after_loading_art(world: &mut World) {
    let mut q = world.query::<(Entity, &mut BattleLoadingArt)>();
    let Some((e, art)) = q.iter_mut(world).next().map(|(e, a)| (e, a.0)) else { return };
    // Longest frame while the battle loads: the freeze the player sees.
    let dt = world.resource::<Time<Real>>().delta_secs();
    let mut worst = world.get_resource_or_insert_with(|| WorstFrame(0.0));
    worst.0 = worst.0.max(dt);
    let started = world.contains_resource::<BattleSession>() || world.contains_resource::<battle_view::BattleRuntime>() || world.contains_resource::<SessionBuild>();
    if !started {
        if let Some(mut a) = world.get_mut::<BattleLoadingArt>(e) {
            a.0 += 1;
        }
        if art == 3 {
            begin_session(world);
        }
    } else if world.contains_resource::<battle_view::BattleRuntime>() {
        if let Some(mut a) = world.get_mut::<BattleLoadingArt>(e) {
            a.0 += 1;
        }
        // The ships spawn over several frames (`spawn_budget`): keep the loading screen until all models are there.
        let pending = world
            .query_filtered::<(), (With<alamo_render::AlamoModel>, Without<alamo_render::AlamoModelLoaded>, Without<alamo_render::model::AlamoLoadFailed>)>()
            .iter(world)
            .count();
        if art > 20 && pending == 0 {
            info!("battle: longest frame while loading {:.0} ms", world.resource::<WorstFrame>().0 * 1000.0);
            world.entity_mut(e).despawn();
        }
    }
}

/// Builds the battle from the prepared request.
fn begin_session(world: &mut World) {
    let Some(mut req) = world.get_resource::<PendingTactical>().map(|p| (*p.0).clone()) else { return };
    let args = world.resource::<Args>().clone();
    let player = world.get_resource::<PlayerFaction>().and_then(|p| p.0);
    let target = world.get_resource::<OffscreenTarget>().map(|t| (t.image.clone(), t.scale_factor));
    let (msaa, bloom, shadows, density) = world.get_resource::<crate::settings::Settings>().map_or((4, true, true, 1.0), |s| (s.msaa, s.bloom, s.shadows, s.particle_density));
    let Some(assets) = world.resource::<Game>().0.clone() else { return };
    // Test scenario: pretend the planet cap is reached so the rest of the fleet waits in the reserve.
    if let Some(keep) = args.battle_reserve
        && let Some(side) = req.sides.iter_mut().find(|s| Some(s.faction) == player)
    {
        let rest = side.groups.split_off(keep.min(side.groups.len()));
        side.reserve.extend(rest.into_iter().filter(|g| g.kind == galaxy_sim::GroupKind::Unit));
        side.unit_cap = side.groups.iter().map(|g| g.count as i32).sum::<i32>().max(1);
        if args.battle_low_cap {
            let cap = side.unit_cap;
            for s in &mut req.sides {
                s.unit_cap = cap;
            }
        }
    }
    // `--battle-auto` lets the AI play both sides (headless scenarios).
    let side_player = if args.battle_auto { None } else { player };
    // The catalog, the map and the visuals take a moment: build them off the main thread while the loading screen runs.
    let (tx, rx) = std::sync::mpsc::channel();
    let started = std::time::Instant::now();
    std::thread::Builder::new()
        .name("battle-setup".into())
        .spawn(move || {
            let result = BattleSession::from_request(&assets.data, &*assets.fs, &req, side_player);
            let _ = tx.send((result, req.battle_id, req.planet_name.clone(), started.elapsed()));
        })
        .expect("spawn battle setup thread");
    world.insert_resource(SessionBuild { rx: std::sync::Mutex::new(rx), msaa, bloom, shadows, density, target, speed: args.battle_speed });
}

/// Longest frame time seen while a battle loads (seconds).
#[derive(Resource)]
struct WorstFrame(f32);

/// A battle session being built on a worker thread.
#[derive(Resource)]
#[allow(clippy::type_complexity)]
struct SessionBuild {
    rx: std::sync::Mutex<std::sync::mpsc::Receiver<(Result<BattleSession, String>, u32, String, std::time::Duration)>>,
    msaa: u32,
    bloom: bool,
    shadows: bool,
    density: f32,
    target: Option<(Handle<Image>, f32)>,
    speed: Option<f32>,
}

/// Starts the battle when the worker is done.
fn poll_session_build(world: &mut World) {
    let Some(build) = world.get_resource::<SessionBuild>() else { return };
    let got = build.rx.lock().ok().and_then(|rx| rx.try_recv().ok());
    let Some((result, battle_id, planet, took)) = got else { return };
    let build = world.remove_resource::<SessionBuild>().expect("present");
    let (msaa, bloom, shadows, density, target) = (build.msaa, build.bloom, build.shadows, build.density, build.target);
    let args_speed = build.speed;
    match result {
        Ok(session) => {
            info!("battle: session for {planet} built in {took:.2?} (off the main thread)");
            let mut settings = world.resource_mut::<BattleViewSettings>();
            settings.speed = args_speed.unwrap_or(1.0);
            settings.msaa_samples = msaa;
            settings.bloom = bloom;
            settings.shadows = shadows;
            settings.particle_density = density;
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
            world.write_message(BattleFinished { id: battle_id, outcome: BattleOutcome::Aborted });
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
#[allow(clippy::too_many_arguments)]
fn finish_battle_view(
    mut finished: MessageReader<battle_view::BattleFinished>,
    mut actions: MessageWriter<PlayAction>,
    mut done: MessageWriter<BattleFinished>,
    music: Option<ResMut<MusicDirector>>,
    mut commands: Commands,
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
        // Result screen: ships at the start and survivors per side.
        let mut lines = Vec::new();
        for (si, side) in pending.0.sides.iter().enumerate() {
            let total: u32 = side.groups.iter().map(|g| g.count).sum();
            let alive: u32 = side.groups.iter().enumerate().map(|(gi, _)| f.result.survived.get(si).and_then(|r| r.get(gi)).copied().unwrap_or(0)).sum();
            lines.push(format!("{}: {} of {} units survived (lost {})", side.faction_name, alive, total, total.saturating_sub(alive)));
        }
        let planet = pending.0.planet_name.trim_end_matches("_WW").replace('_', " ");
        let winner = f.result.winner.and_then(|w| pending.0.sides.get(w)).map(|s| s.faction_name.clone());
        let title = match (&winner, f.result.winner.and_then(|w| pending.0.sides.get(w)).map(|s| Some(s.faction) == player.0)) {
            (Some(_), Some(true)) => format!("Victory at {}", planet),
            (Some(w), _) => format!("Defeat at {} - {w} wins", planet),
            _ => format!("Battle at {} ends in a draw", planet),
        };
        commands.insert_resource(BattleSummary { title, text: format!("{}
Battle time: {:.0} s", lines.join("
"), f.result.duration) });
        let outcome = f.result.winner.map_or(BattleOutcome::Draw, BattleOutcome::Victory);
        actions.write(PlayAction::ApplyTactical { id: f.battle_id, result: Box::new(f.result.clone()) });
        done.write(BattleFinished { id: f.battle_id, outcome });
    }
}

/// Result of the last battle, shown as a dialog once the map is back.
#[derive(Resource)]
pub struct BattleSummary {
    pub title: String,
    pub text: String,
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
    let ready = if args.battle_orders { shot.orders_at.is_some_and(|f| shot.frames >= f + 12) } else { status.time >= at && shot.frames >= 200 };
    if shot.taken || status.tick == 0 || !ready {
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
    loading_art: Query<Entity, With<BattleLoadingArt>>,
    settings: Option<ResMut<EawUiSettings>>,
    mut hold: ResMut<ScriptHold>,
    mut shot: ResMut<BattleShot>,
) {
    commands.remove_resource::<BattleSession>();
    commands.remove_resource::<PendingTactical>();
    for e in &hints {
        commands.entity(e).despawn();
    }
    for e in &loading_art {
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

/// Offscreen `--battle-orders`: selects the player ships, orders a move and then an attack through the
/// synthetic pointer, so selection rings, bars and order markers can be checked in a capture.
#[allow(clippy::too_many_arguments)]
fn battle_orders(
    args: Res<Args>,
    status: Res<BattleStatus>,
    ships: Res<battle_view::RenderedShips>,
    settings: Res<BattleViewSettings>,
    mut sel: ResMut<battle_view::Selection>,
    mut pointer: ResMut<battle_view::Pointer>,
    camera: Single<(&Camera, &GlobalTransform), With<BattleCamera>>,
    mut frames: Local<u32>,
    mut shot: ResMut<BattleShot>,
) {
    if !args.battle_orders || status.tick == 0 || ships.0.is_empty() {
        return;
    }
    *frames += 1;
    let (cam, gt) = *camera;
    let player = settings.player_side;
    pointer.right_pressed = false;
    match *frames {
        120 => {
            sel.ships = ships.0.iter().filter(|(_, s)| s.side == player).map(|(id, _)| *id).collect();
            sel.ships.sort_unstable();
        }
        140 => {
            // Move to the middle of the screen.
            pointer.cursor = cam.logical_viewport_size().map(|s| s * 0.5);
            pointer.right_pressed = true;
        }
        175 => {
            if shot.orders_at.is_none() {
                shot.orders_at = Some(shot.frames);
            }
        }
        160 => {
            // Attack the enemy ship that is nearest to the middle of the screen.
            let middle = cam.logical_viewport_size().map_or(Vec2::ZERO, |s| s * 0.5);
            let target = ships
                .0
                .values()
                .filter(|s| s.side != player)
                .filter_map(|s| cam.world_to_viewport(gt, s.pos).ok().map(|p| (p, p.distance(middle))))
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((p, _)) = target {
                pointer.cursor = Some(p);
                pointer.right_pressed = true;
                shot.orders_at = Some(shot.frames);
            }
        }
        _ => {}
    }
}
