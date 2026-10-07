//! The galaxy HUD of a running game: the original galactic command bar (`eaw-ui`) fed from
//! `galaxy-play`, the notification ticker, speed and fleet read-outs and the "battle pending"
//! dialog.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use eaw_ui::{
    BuildPadEntry, CommandBarData, CommandBarMode, CommandBarSpec, DialogSpec, EawButtonClicked, EawCommandBar, EawDialogResult, EawUiAssets,
    EawMinimapClicked, EawUiCommandsExt, EawUiSettings, MinimapData, MinimapDot, QueueEntry, UiFaction, UiScaleMode, spawn_dialog,
};
use galaxy_play::{
    BattleChoice, BuildTarget, GameClock, NotificationKind, PlayAction, PlayUiModel, PlayerFaction, PlayerSelection, SimState,
    geometry::format_duration,
};

use crate::flow::AppScreen;
use crate::galaxy_map::GalaxyEntity;
use crate::offscreen::OffscreenTarget;
use crate::play::{PlayBridge, is_structure};
use crate::settings::Settings;
use crate::ui::InfoPanel;

/// Pads the galactic bar can show (`b_create_00..39`).
const MAX_PADS: usize = 40;

/// Command bar state of the running game.
#[derive(Resource, Debug)]
struct BarState {
    spawned: bool,
    /// Space production tab (ships, star base, tech) instead of the planet tab (structures).
    space_tab: bool,
    /// What each visible pad orders.
    pads: Vec<BuildTarget>,
}

impl Default for BarState {
    fn default() -> Self {
        Self { spawned: false, space_tab: true, pads: Vec::new() }
    }
}

/// Root of the HUD texts of the running game.
#[derive(Component)]
struct PlayHud;
/// Notification ticker (children are rebuilt when the list changes).
#[derive(Component)]
struct Ticker;
/// Speed and winner read-out.
#[derive(Component)]
struct SpeedText;
/// Selected fleets read-out.
#[derive(Component)]
struct FleetText;

/// The open "battle pending" dialog.
#[derive(Resource, Default)]
struct BattleDialogState {
    open: Option<(u32, Entity)>,
    /// Battle the player just decided; the model still lists it until the next snapshot.
    decided: Option<u32>,
}

/// Wires the HUD.
pub struct PlayUiPlugin;

impl Plugin for PlayUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BarState>()
            .init_resource::<BattleDialogState>()
            .add_systems(PreUpdate, drive_ui_scale.run_if(resource_exists::<EawUiSettings>))
            .add_systems(
                Update,
                (
                    spawn_bar.run_if(resource_exists::<EawUiAssets>),
                    spawn_hud,
                    feed_bar.run_if(in_state(AppScreen::Galaxy)),
                    feed_minimap.run_if(resource_exists::<MinimapData>.and_then(in_state(AppScreen::Galaxy))),
                    minimap_pan,
                    bar_clicks,
                    refresh_ticker,
                    refresh_texts,
                    battle_dialog,
                    scripted_battle_click,
                    battle_dialog_result,
                    move_info_panel,
                )
                    .chain()
                    .run_if(resource_exists::<PlayBridge>.and_then(resource_exists::<SimState>)),
            )
            .add_systems(OnEnter(AppScreen::SpaceBattle), hide_play_ui)
            .add_systems(OnExit(AppScreen::SpaceBattle), show_play_ui)
            .add_systems(Update, reset_on_stop.run_if(not(resource_exists::<SimState>)));
    }
}

/// The command bar designs for 1366x768: everything scales with the screen height (like the
/// original); the user's UI scale multiplies it.
fn drive_ui_scale(
    settings: Res<Settings>,
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    offscreen: Option<Res<OffscreenTarget>>,
    bar: Query<(), With<EawCommandBar>>,
    mut reserved: ResMut<crate::galaxy_map::ReservedBottom>,
    mut eaw: ResMut<EawUiSettings>,
) {
    let height = match (&window, &offscreen) {
        (Some(w), _) => w.height(),
        (None, Some(t)) => crate::offscreen::SIZE.1 as f32 / t.scale_factor,
        _ => 768.0,
    };
    let v = if bar.is_empty() { settings.ui_scale } else { height / 768.0 * settings.ui_scale };
    let strip = if bar.is_empty() { 0.0 } else { 290.0 * v };
    if reserved.0 != strip {
        reserved.0 = strip;
    }
    if eaw.scale != UiScaleMode::Fixed(v) {
        eaw.scale = UiScaleMode::Fixed(v);
    }
}

fn ui_faction(state: &SimState, player: &PlayerFaction) -> UiFaction {
    player.0.map_or(UiFaction::Empire, |h| UiFaction::from_name(&state.world.factions[h as usize].name))
}

/// Spawns the galactic bar once for the running game (waits for the UI textures by itself).
#[allow(clippy::too_many_arguments)]
fn spawn_bar(
    mut commands: Commands,
    mut bar: ResMut<BarState>,
    state: Res<SimState>,
    player: Res<PlayerFaction>,
    screen: Res<State<AppScreen>>,
    bars: Query<(), With<EawCommandBar>>,
    mut last_spawn: Local<u32>,
    mut frame: Local<u32>,
) {
    *frame += 1;
    // The tactical UI removes every command bar when a battle ends: bring the galactic one back.
    if bar.spawned && bars.is_empty() && *screen.get() == AppScreen::Galaxy && *frame > *last_spawn + 30 {
        bar.spawned = false;
    }
    if bar.spawned || player.0.is_none() || *screen.get() != AppScreen::Galaxy {
        return;
    }
    bar.spawned = true;
    *last_spawn = *frame;
    commands.spawn_eaw_command_bar(CommandBarSpec { mode: CommandBarMode::Galactic, faction: ui_faction(&state, &player) });
}

/// Forgets the bar when the game ended (the bar entity itself is despawned by `end_play`).
fn reset_on_stop(mut bar: ResMut<BarState>, data: Option<ResMut<CommandBarData>>, mut dialog: ResMut<BattleDialogState>) {
    if bar.spawned || dialog.open.is_some() {
        *bar = BarState::default();
        if let Some(mut data) = data {
            *data = CommandBarData::default();
        }
        dialog.open = None;
    }
}

/// Name, cost, build time and (when greyed out) the reason.
fn pad_tooltip(o: &galaxy_play::BuildOption, week: f32) -> String {
    let mut s = format!("{}
Cost: {:.0} credits   Build time: {}", o.name, o.cost, format_duration(o.build_seconds, week));
    if o.population > 0 {
        s.push_str(&format!("   Pop: {}", o.population));
    }
    if !o.availability.is_available() {
        s.push_str(&format!("
({})", o.availability.reason()));
    }
    s
}

fn icon_for(state: &SimState, game: Option<&alamo_data::GameData>, target: BuildTarget) -> String {
    let world = &state.world;
    let name = match target {
        BuildTarget::Unit(u) => &world.units[u as usize].name,
        BuildTarget::Structure(s) => &world.structures[s as usize].name,
        BuildTarget::Starbase(s) => &world.starbases[s as usize].name,
        BuildTarget::Tech(_) => return String::new(),
    };
    game.and_then(|g| eaw_ui::unit_icon(g, name)).unwrap_or_default()
}

/// `PlayUiModel` into the bar's data resource.
#[allow(clippy::too_many_arguments)]
fn feed_bar(
    model: Res<PlayUiModel>,
    state: Res<SimState>,
    selection: Res<PlayerSelection>,
    bridge: Res<PlayBridge>,
    view: Res<crate::galaxy_view::GalaxyView>,
    assets: Option<Res<EawUiAssets>>,
    mut bar: ResMut<BarState>,
    mut data: ResMut<CommandBarData>,
) {
    if !model.is_changed() && !bar.is_changed() {
        return;
    }
    let game = assets.as_ref().and_then(|a| a.game.clone());
    let game = game.as_deref();
    let mut next = CommandBarData {
        credits: model.top.credits as i64,
        day: (state.current.time / f64::from(state.world.config.week_seconds) * 7.0) as u32 + 1,
        tech_level: state.galaxy().factions.get(state.world.victory.active_player.unwrap_or(0) as usize).map_or(0, |f| f.tech.max(0) as u32),
        pop_current: model.top.population_used.max(0) as u32,
        pop_cap: model.top.population_cap.max(0) as u32,
        space_tab: bar.space_tab,
        ..default()
    };
    if let Some(p) = selection.planet {
        next.planet_name = state.world.planets[p as usize].display_name.clone();
        next.planet_owner = state.galaxy().planets[p as usize].owner.map(|o| UiFaction::from_name(&state.world.factions[o as usize].name));
        next.planet_income = match &model.planet {
            Some(m) => m.income as i64,
            None => bridge.sim_to_planet[p as usize].and_then(|i| view.planets.get(i)).map_or(0, |pv| i64::from(pv.income)),
        };
    }
    let week = state.world.config.week_seconds;
    let space_tab = bar.space_tab;
    bar.pads.clear();
    if let Some(menu) = &model.planet {
        for q in &menu.queue {
            next.queue.push(QueueEntry {
                icon: icon_for(&state, game, q.target),
                progress: q.progress,
                count: 1,
                tooltip: format!("{}
{} left", q.name, format_duration(q.remaining_seconds, week)),
            });
        }
        for o in menu.options.iter().filter(|o| is_structure(o.target) != space_tab).take(MAX_PADS) {
            next.pads.push(BuildPadEntry {
                icon: icon_for(&state, game, o.target),
                price: Some(o.cost.max(0.0) as u32),
                disabled: !o.availability.is_available(),
                building: menu.queue.iter().any(|q| q.target == o.target),
                tooltip: pad_tooltip(o, week),
            });
            bar.pads.push(o.target);
        }
    }
    if *data != next {
        *data = next;
    }
}

/// Bar buttons: build pads, tabs, pause and fast-forward.
fn bar_clicks(
    mut clicks: MessageReader<EawButtonClicked>,
    mut bar: ResMut<BarState>,
    selection: Res<PlayerSelection>,
    clock: Res<GameClock>,
    mut actions: MessageWriter<PlayAction>,
) {
    for c in clicks.read() {
        let name = c.name.as_str();
        if let Some(i) = name.strip_prefix("b_create_").and_then(|n| n.parse::<usize>().ok()) {
            if let (Some(planet), Some(&target)) = (selection.planet, bar.pads.get(i)) {
                actions.write(PlayAction::Build { planet, target });
            }
            continue;
        }
        match name {
            "b_space_tab" => bar.space_tab = true,
            "b_planet_tab" => bar.space_tab = false,
            "b_play_pause" => {
                actions.write(PlayAction::TogglePause);
            }
            "b_fast_forward" => {
                let next = if clock.paused() || clock.level >= clock.speeds.len() { 1 } else { clock.level + 1 };
                actions.write(PlayAction::SetSpeedLevel(next));
            }
            _ => {}
        }
    }
}

fn text_bundle(s: &str, size: f32, color: Color) -> impl Bundle {
    (
        Text::new(s),
        TextFont { font_size: FontSize::Px(size), ..default() },
        TextColor(color),
        TextShadow { offset: Vec2::splat(1.5), color: Color::srgba(0.0, 0.0, 0.0, 0.9) },
    )
}

fn spawn_hud(mut commands: Commands, existing: Query<(), With<PlayHud>>) {
    if !existing.is_empty() {
        return;
    }
    let abs = Node { position_type: PositionType::Absolute, ..default() };
    commands.spawn((
        PlayHud,
        Node { top: Val::Px(52.0), left: Val::Px(16.0), width: Val::Px(440.0), flex_direction: FlexDirection::Column, row_gap: Val::Px(2.0), ..abs.clone() },
        Ticker,
        GalaxyEntity,
    ));
    commands.spawn((
        PlayHud,
        text_bundle("", 20.0, Color::srgb(0.9, 0.95, 1.0)),
        Node { top: Val::Px(52.0), left: Val::Percent(45.0), ..abs.clone() },
        SpeedText,
        GalaxyEntity,
    ));
    commands.spawn((
        PlayHud,
        text_bundle("", 15.0, Color::srgb(0.75, 1.0, 0.85)),
        Node { bottom: Val::Px(296.0), left: Val::Px(40.0), max_width: Val::Px(620.0), ..abs },
        FleetText,
        GalaxyEntity,
    ));
}

fn kind_color(kind: NotificationKind) -> Color {
    match kind {
        NotificationKind::Error => Color::srgb(1.0, 0.55, 0.45),
        NotificationKind::BattlePending => Color::srgb(1.0, 0.8, 0.35),
        NotificationKind::PlanetCaptured | NotificationKind::Construction => Color::srgb(0.55, 1.0, 0.65),
        NotificationKind::PlanetLost => Color::srgb(1.0, 0.5, 0.5),
        _ => Color::srgb(0.8, 0.88, 1.0),
    }
}

fn refresh_ticker(mut commands: Commands, model: Res<PlayUiModel>, ticker: Single<Entity, With<Ticker>>, mut last: Local<Vec<u64>>) {
    let ids: Vec<u64> = model.notifications.iter().map(|n| n.id).collect();
    if ids == *last {
        return;
    }
    *last = ids;
    commands.entity(*ticker).despawn_children();
    for n in model.notifications.iter().rev().take(6) {
        commands.spawn((text_bundle(&n.text, 15.0, kind_color(n.kind)), ChildOf(*ticker)));
    }
}

#[allow(clippy::type_complexity)]
fn refresh_texts(
    model: Res<PlayUiModel>,
    state: Res<SimState>,
    mut speed: Single<(&mut Text, &mut TextColor), (With<SpeedText>, Without<FleetText>)>,
    mut fleets: Single<&mut Text, (With<FleetText>, Without<SpeedText>)>,
) {
    if !model.is_changed() {
        return;
    }
    let (text, color) = &mut *speed;
    let (s, c) = if let Some(w) = &model.top.winner {
        (format!("{w} wins the galaxy"), Color::srgb(1.0, 0.9, 0.4))
    } else if model.speed.held_by_battle {
        ("Battle pending".to_owned(), Color::srgb(1.0, 0.8, 0.35))
    } else if model.speed.rate <= 0.0 {
        ("PAUSED".to_owned(), Color::srgb(1.0, 0.75, 0.4))
    } else {
        (format!("Week {}   x{}", model.top.week + 1, model.speed.rate), Color::srgb(0.85, 0.92, 1.0))
    };
    if text.0 != s {
        text.0 = s;
    }
    color.0 = c;
    let week = state.world.config.week_seconds;
    let lines: Vec<String> = model
        .fleets
        .iter()
        .map(|f| {
            let ships = f.ships.iter().map(|(n, c)| format!("{c}x {n}")).collect::<Vec<_>>().join(", ");
            let place = match (&f.at, &f.heading) {
                (Some(p), _) => format!("at {p}"),
                (_, Some((d, eta))) => format!("to {d}, ETA {}", format_duration(*eta, week)),
                _ => String::new(),
            };
            format!("{}{} ships ({ships}) {place}", if f.mine { "" } else { "Enemy: " }, f.count)
        })
        .collect();
    let mut s = lines.join("\n");
    if let Some(h) = &model.route_hint {
        if !s.is_empty() {
            s.push('\n');
        }
        s.push_str(h);
    }
    if fleets.0 != s {
        fleets.0 = s;
    }
}

/// Opens the dialog for a battle that waits for the player.
fn battle_dialog(world: &mut World) {
    let Some(model) = world.get_resource::<PlayUiModel>() else { return };
    let wanted = model.battle.clone();
    let open = world.resource::<BattleDialogState>().open;
    if let (Some((id, e)), Some(b)) = (open, &wanted)
        && b.id == id
        && world.get_entity(e).is_ok()
    {
        return;
    }
    let Some(b) = wanted else {
        world.resource_mut::<BattleDialogState>().decided = None;
        return;
    };
    if world.resource::<BattleDialogState>().decided == Some(b.id) {
        return;
    }
    if world.get_resource::<EawUiAssets>().is_none() {
        // Without the original UI the battle is fought automatically.
        world.write_message(PlayAction::ResolveBattle { id: b.id, choice: BattleChoice::AutoResolve });
        return;
    }
    let faction = {
        let state = world.resource::<SimState>();
        ui_faction(state, world.resource::<PlayerFaction>())
    };
    let sides: Vec<String> = b
        .sides
        .iter()
        .map(|s| format!("{}: {} ships{}{}", s.faction, s.ships, if s.defender { " (defending)" } else { "" }, if s.mine { " - you" } else { "" }))
        .collect();
    let spec = DialogSpec {
        title: format!("Battle at {}", b.planet_name),
        text: sides.join("\n"),
        portrait: None,
        faction,
        buttons: vec!["Fight".into(), "Auto-resolve".into()],
    };
    if let Some(e) = spawn_dialog(world, &spec) {
        world.resource_mut::<BattleDialogState>().open = Some((b.id, e));
    }
}

fn battle_dialog_result(mut results: MessageReader<EawDialogResult>, mut dialog: ResMut<BattleDialogState>, mut actions: MessageWriter<PlayAction>) {
    for r in results.read() {
        let Some((id, e)) = dialog.open else { continue };
        if r.dialog != e {
            continue;
        }
        dialog.open = None;
        dialog.decided = Some(id);
        let choice = if r.button == 0 { BattleChoice::Fight } else { BattleChoice::AutoResolve };
        actions.write(PlayAction::ResolveBattle { id, choice });
    }
}

/// With a command bar in the bottom left corner the planet details move to the right edge.
fn move_info_panel(mut panel: Single<&mut Node, With<InfoPanel>>, mut done: Local<bool>) {
    if *done {
        return;
    }
    *done = true;
    panel.left = Val::Auto;
    panel.bottom = Val::Auto;
    panel.right = Val::Px(16.0);
    panel.top = Val::Px(52.0);
}

fn hide_play_ui(mut bar: Query<&mut Visibility, With<EawCommandBar>>) {
    for mut v in &mut bar {
        *v = Visibility::Hidden;
    }
}

fn show_play_ui(mut bar: Query<&mut Visibility, With<EawCommandBar>>) {
    for mut v in &mut bar {
        *v = Visibility::Inherited;
    }
}


/// Galactic minimap: planets by owner, fleets, the camera's view; refreshed a few times a second.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn feed_minimap(
    time: Res<Time>,
    view: Res<crate::galaxy_view::GalaxyView>,
    selection: Res<crate::galaxy_map::Selection>,
    state: Res<SimState>,
    fleets: Query<(&galaxy_play::FleetVisual, &Transform)>,
    camera: Single<(&Camera, &crate::camera::GalaxyCamera, &Projection)>,
    data: Option<ResMut<MinimapData>>,
    mut since: Local<f32>,
) {
    let Some(mut data) = data else { return };
    *since += time.delta_secs();
    if *since < 0.2 && data.bounds.is_some() {
        return;
    }
    *since = 0.0;
    let galaxy = |w: Vec3| Vec2::new(w.x, -w.z);
    let mut lo = Vec2::splat(f32::MAX);
    let mut hi = Vec2::splat(f32::MIN);
    let planets: Vec<MinimapDot> = view
        .planets
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let pos = galaxy(crate::galaxy_map::galaxy_to_world(p.position));
            lo = lo.min(pos);
            hi = hi.max(pos);
            let c = view.owner_of(i).map_or([0.55, 0.58, 0.62], |f| f.color);
            MinimapDot { pos, color: Color::srgb(c[0], c[1], c[2]), selected: selection.0 == Some(i) }
        })
        .collect();
    if planets.is_empty() {
        return;
    }
    let margin = (hi - lo) * 0.04;
    let units = fleets
        .iter()
        .map(|(f, t)| {
            let c = state.world.factions[f.owner as usize].color;
            MinimapDot { pos: galaxy(t.translation), color: Color::srgb_u8(c[0], c[1], c[2]), selected: false }
        })
        .collect();
    let (cam, orbit, projection) = *camera;
    let fov = match projection {
        Projection::Perspective(p) => p.fov,
        _ => 0.7,
    };
    let aspect = cam.logical_viewport_size().map_or(1.78, |s| s.x / s.y.max(1.0));
    let half_h = orbit.distance * (fov * 0.5).tan() / orbit.pitch.sin().max(0.3);
    let center = galaxy(orbit.focus);
    let half = Vec2::new(half_h * aspect, half_h);
    let next = MinimapData { bounds: Some((lo - margin, hi + margin)), planets, units, camera: Some((center - half, center + half)) };
    if *data != next {
        *data = next;
    }
}

/// Click or drag on the minimap pans the map camera.
fn minimap_pan(mut clicks: MessageReader<EawMinimapClicked>, mut camera: Single<&mut crate::camera::GalaxyCamera>) {
    if let Some(c) = clicks.read().last() {
        camera.target_focus = Vec3::new(c.world.x, 0.0, -c.world.y);
    }
}

/// Offscreen scenarios (`--battle-choice`): presses a button of the open battle dialog the way a
/// click would, so the seam can be verified headless.
fn scripted_battle_click(
    args: Res<crate::cli::Args>,
    hold: Res<crate::play::ScriptHold>,
    mut dialog: ResMut<BattleDialogState>,
    mut results: MessageWriter<EawDialogResult>,
    mut commands: Commands,
) {
    let (Some(choice), Some((_, e))) = (args.battle_choice.as_deref(), dialog.open) else { return };
    if args.offscreen.is_none() || hold.active {
        return;
    }
    let button = usize::from(choice != "fight");
    info!("script: clicking `{}` in the battle dialog", ["Fight", "Auto-resolve"][button]);
    results.write(EawDialogResult { dialog: e, button });
    commands.entity(e).despawn();
    // The next system (battle_dialog_result) reads the message and clears `open`.
    let _ = &mut dialog;
}
