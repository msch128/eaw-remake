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

/// Dark plate behind the command bar (the original art is very transparent on the right).
#[derive(Component)]
pub(crate) struct BarBacking;

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
    /// Queue slots: lane, position in the lane and the credits a cancel gives back.
    queue: Vec<(galaxy_sim::Lane, u32, f32)>,
}

impl Default for BarState {
    fn default() -> Self {
        Self { spawned: false, space_tab: true, pads: Vec::new(), queue: Vec::new() }
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
            .init_resource::<MoveMarkers>()
            .init_resource::<Toast>()
            .init_resource::<EndState>()
            .add_systems(PreUpdate, drive_ui_scale.run_if(resource_exists::<EawUiSettings>))
            .add_systems(
                Update,
                (
                    spawn_bar.run_if(resource_exists::<EawUiAssets>),
                    spawn_hud,
                    feed_bar.run_if(in_state(AppScreen::Galaxy)),
                    feed_minimap.run_if(resource_exists::<MinimapData>.and_then(in_state(AppScreen::Galaxy))),
                    minimap_pan.run_if(in_state(AppScreen::Galaxy)),
                    bar_clicks,
                    loading_overlay,
                    skin_panels,
                    collect_move_markers,
                    draw_toast.run_if(in_state(AppScreen::Galaxy)),
                    scripted_clicks,
                    fleet_tooltip.run_if(in_state(AppScreen::Galaxy)),
                    draw_move_markers,
                    refresh_ticker,
                    refresh_texts.run_if(in_state(AppScreen::Galaxy)),
                    battle_dialog,
                    scripted_battle_click,
                    battle_dialog_result,
                    move_info_panel,
                )
                    .chain()
                    .run_if(resource_exists::<PlayBridge>.and_then(resource_exists::<SimState>)),
            )
            .add_systems(
                Update,
                (track_game_end, force_end_script, show_end_dialog, end_dialog_result)
                    .chain()
                    .run_if(resource_exists::<PlayBridge>.and_then(resource_exists::<SimState>)),
            )
            .add_systems(Update, fix_fleet_label_scale.after(galaxy_play::PlaySet::Visuals).run_if(resource_exists::<SimState>))
            .add_systems(OnEnter(AppScreen::SpaceBattle), hide_play_ui)
            .add_systems(OnExit(AppScreen::SpaceBattle), show_play_ui)
            .add_systems(Update, (reset_on_stop, reset_end_state).run_if(not(resource_exists::<SimState>)));
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
    backing: Query<(), With<BarBacking>>,
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
    if backing.is_empty() {
        commands.spawn((
            BarBacking,
            Node { position_type: PositionType::Absolute, left: Val::Px(0.0), bottom: Val::Px(0.0), width: Val::Px(1380.0), height: Val::Px(127.0), ..default() },
            BackgroundColor(Color::srgba(0.01, 0.015, 0.03, 0.9)),
            GlobalZIndex(-1),
        ));
    }
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
    bar.queue.clear();
    bar.pads.clear();
    if let Some(menu) = &model.planet {
        let mut per_lane: std::collections::BTreeMap<u8, u32> = Default::default();
        for q in &menu.queue {
            let index = per_lane.entry(q.lane as u8).or_insert(0);
            let refund = menu.options.iter().find(|o| o.target == q.target).map(|o| o.cost * (1.0 - q.progress) * state.world.config.cancel_refund);
            let cancel = refund.map_or("Click to cancel".to_owned(), |r| format!("Click to cancel - refund {r:.0} credits"));
            bar.queue.push((q.lane, *index, refund.unwrap_or(0.0)));
            *index += 1;
            next.queue.push(QueueEntry {
                icon: icon_for(&state, game, q.target),
                progress: q.progress,
                count: 1,
                tooltip: format!("{}\n{} left\n{cancel}", q.name, format_duration(q.remaining_seconds, week)),
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
    mut toast: ResMut<Toast>,
) {
    for c in clicks.read() {
        let name = c.name.as_str();
        if let Some(i) = name.strip_prefix("b_create_").and_then(|n| n.parse::<usize>().ok()) {
            if let (Some(planet), Some(&target)) = (selection.planet, bar.pads.get(i)) {
                actions.write(PlayAction::Build { planet, target });
            }
            continue;
        }
        if let Some(i) = name.strip_prefix("queue").and_then(|n| n.parse::<usize>().ok()) {
            if let (Some(planet), Some(&(lane, index, refund))) = (selection.planet, bar.queue.get(i)) {
                actions.write(PlayAction::CancelProduction { planet, lane, index });
                toast.0 = if refund > 0.0 { format!("Production cancelled - {refund:.0} credits refunded") } else { "Production cancelled".to_owned() };
                toast.1 = 3.0;
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
        TextFont { font: FontSource::Family("Bahnschrift".into()), font_size: FontSize::Px(size), ..default() },
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
        ToastText,
        text_bundle("", 18.0, Color::srgb(1.0, 0.9, 0.5)),
        Node { top: Val::Px(84.0), left: Val::Percent(40.0), ..abs.clone() },
        Visibility::Hidden,
        GalaxyEntity,
    ));
    commands.spawn((
        PlayHud,
        FleetTip,
        text_bundle("", 14.0, Color::srgb(0.9, 0.95, 1.0)),
        Node {
            position_type: PositionType::Absolute,
            padding: UiRect::axes(Val::Px(8.0), Val::Px(5.0)),
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        crate::ui::SkinPanel,
        BackgroundColor(Color::srgba(0.02, 0.05, 0.10, 0.92)),
        BorderColor::all(Color::srgba(0.35, 0.65, 1.0, 0.8)),
        GlobalZIndex(30),
        Visibility::Hidden,
        GalaxyEntity,
    ));
    commands.spawn((
        PlayHud,
        text_bundle("", 15.0, Color::srgb(0.75, 1.0, 0.85)),
        Node {
            bottom: Val::Px(296.0),
            left: Val::Px(40.0),
            max_width: Val::Px(620.0),
            padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
            border: UiRect::all(Val::Px(1.0)),
            ..abs
        },
        crate::ui::SkinPanel,
        BackgroundColor(Color::srgba(0.02, 0.05, 0.10, 0.8)),
        BorderColor::all(Color::srgba(0.35, 0.65, 1.0, 0.7)),
        Visibility::Hidden,
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
    mut fleets: Single<(&mut Text, &mut Visibility), (With<FleetText>, Without<SpeedText>)>,
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
    let (ftext, fvis) = &mut *fleets;
    let want = if s.is_empty() { Visibility::Hidden } else { Visibility::Inherited };
    if **fvis != want {
        **fvis = want;
    }
    if ftext.0 != s {
        ftext.0 = s;
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
/// galaxy-play places its fleet count labels in viewport pixels; UI nodes are multiplied by the UI scale.
fn fix_fleet_label_scale(scale: Res<UiScale>, mut labels: Query<&mut Node, With<galaxy_play::visuals::FleetLabel>>) {
    if (scale.0 - 1.0).abs() < 1e-3 {
        return;
    }
    for mut n in &mut labels {
        if let Val::Px(x) = n.left {
            n.left = Val::Px(x / scale.0);
        }
        if let Val::Px(y) = n.top {
            n.top = Val::Px(y / scale.0);
        }
    }
}

fn move_info_panel(mut panel: Single<&mut Node, With<InfoPanel>>, mut hint: Query<&mut Node, (With<crate::ui::HintText>, Without<InfoPanel>)>) {
    if panel.right == Val::Px(16.0) {
        return;
    }
    panel.left = Val::Auto;
    panel.bottom = Val::Auto;
    panel.right = Val::Px(16.0);
    panel.top = Val::Px(52.0);
    for mut h in &mut hint {
        h.bottom = Val::Px(130.0);
    }
}

fn hide_play_ui(mut bar: Query<&mut Visibility, Or<(With<EawCommandBar>, With<BarBacking>)>>) {
    for mut v in &mut bar {
        *v = Visibility::Hidden;
    }
}

fn show_play_ui(mut bar: Query<&mut Visibility, Or<(With<EawCommandBar>, With<BarBacking>)>>) {
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
    let next = MinimapData { bounds: Some((lo - margin, hi + margin)), planets, units, camera: Some((center - half, center + half)), background: None };
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

/// Full-screen "Preparing galaxy" / "Loading save" overlay with a progress bar.
#[derive(Component)]
struct LoadOverlay;
/// Text line of the overlay.
#[derive(Component)]
struct LoadText;
/// Fill of the overlay bar.
#[derive(Component)]
struct LoadFill;

fn loading_overlay(
    mut commands: Commands,
    state: Res<SimState>,
    overlay: Query<Entity, With<LoadOverlay>>,
    mut text: Query<&mut Text, With<LoadText>>,
    mut fill: Query<&mut Node, With<LoadFill>>,
) {
    let progress = state.driver.load_progress();
    let busy = state.current.loading || progress.is_some();
    if !busy {
        for e in &overlay {
            commands.entity(e).despawn();
        }
        return;
    }
    let (label, fraction) = match progress {
        Some(p) => (format!("Loading save - week {} ({:.0}%)", p.week + 1, p.fraction() * 100.0), p.fraction()),
        None => ("Preparing galaxy...".to_owned(), 0.05),
    };
    if overlay.is_empty() {
        commands
            .spawn((
                LoadOverlay,
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(14.0),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.03, 0.75)),
                GlobalZIndex(40),
                Interaction::default(),
                bevy::ui::FocusPolicy::Block,
            ))
            .with_children(|root| {
                root.spawn((text_bundle(&label, 26.0, Color::srgb(0.85, 0.93, 1.0)), LoadText));
                root.spawn((
                    Node { width: Val::Px(420.0), height: Val::Px(12.0), border: UiRect::all(Val::Px(1.0)), ..default() },
                    BorderColor::all(Color::srgba(0.35, 0.65, 1.0, 0.8)),
                    BackgroundColor(Color::srgba(0.02, 0.05, 0.10, 0.9)),
                ))
                .with_children(|bar| {
                    bar.spawn((
                        Node { width: Val::Percent(fraction * 100.0), height: Val::Percent(100.0), ..default() },
                        BackgroundColor(Color::srgb(0.35, 0.65, 1.0)),
                        LoadFill,
                    ));
                });
            });
        return;
    }
    for mut t in &mut text {
        if t.0 != label {
            t.0 = label.clone();
        }
    }
    for mut n in &mut fill {
        n.width = Val::Percent(fraction * 100.0);
    }
}

/// Destination rings of recent move orders (planet and age in seconds).
#[derive(Resource, Default)]
struct MoveMarkers(Vec<(galaxy_sim::PlanetId, f32)>);

const MARKER_SECONDS: f32 = 1.6;

/// A move order of the player leaves a pulsing ring on the destination planet.
fn collect_move_markers(mut actions: MessageReader<PlayAction>, selection: Res<PlayerSelection>, mut markers: ResMut<MoveMarkers>) {
    for a in actions.read() {
        if let PlayAction::MoveSelected(to) = a
            && !selection.fleets.is_empty()
        {
            markers.0.push((*to, 0.0));
        }
    }
}

fn draw_move_markers(time: Res<Time>, layout: Res<galaxy_play::GalaxyLayout>, mut markers: ResMut<MoveMarkers>, mut gizmos: Gizmos) {
    let dt = time.delta_secs();
    for m in &mut markers.0 {
        m.1 += dt;
    }
    markers.0.retain(|m| m.1 < MARKER_SECONDS);
    for &(planet, age) in &markers.0 {
        let t = age / MARKER_SECONDS;
        let c = layout.positions[planet as usize];
        let r = layout.radii[planet as usize] * (1.3 + 1.2 * t);
        let color = Color::srgba(0.4, 1.0, 0.6, 1.0 - t);
        gizmos.circle(Isometry3d::new(c, Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)), r, color);
        gizmos.circle(Isometry3d::new(c, Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)), r * 0.7, color);
    }
}

/// Mouse-follow tooltip of the fleet under the cursor.
#[derive(Component)]
struct FleetTip;

fn fleet_tooltip(
    hover: Res<galaxy_play::PlayHover>,
    state: Res<SimState>,
    catalog: Res<galaxy_play::PlayCatalog>,
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    scale: Res<UiScale>,
    mut tip: Single<(&mut Node, &mut Text, &mut Visibility), With<FleetTip>>,
) {
    let (node, text, vis) = &mut *tip;
    let cursor = window.and_then(|w| w.cursor_position());
    let fleet = hover.fleet.and_then(|id| state.galaxy().fleets.get(&id));
    let (Some(cursor), Some(f)) = (cursor, fleet) else {
        if **vis != Visibility::Hidden {
            **vis = Visibility::Hidden;
        }
        return;
    };
    let mut lines = vec![format!("{} fleet - {} ships", catalog.faction(f.owner), f.unit_count())];
    for (&u, &c) in f.units.iter().take(8) {
        lines.push(format!("  {c}x {}", catalog.unit(u)));
    }
    if f.units.len() > 8 {
        lines.push(format!("  +{} more types", f.units.len() - 8));
    }
    let place = match &f.location {
        galaxy_sim::FleetLocation::AtPlanet(p) => format!("in orbit of {}", state.world.planets[*p as usize].display_name),
        galaxy_sim::FleetLocation::Moving { to, .. } => format!("heading to {}", state.world.planets[*to as usize].display_name),
    };
    lines.push(place);
    let s = lines.join("\n");
    if text.0 != s {
        text.0 = s;
    }
    node.left = Val::Px((cursor.x + 16.0) / scale.0);
    node.top = Val::Px((cursor.y + 18.0) / scale.0);
    **vis = Visibility::Inherited;
}

/// HUD panels take the faction skin colours of the command bar.
fn skin_panels(
    state: Res<SimState>,
    player: Res<PlayerFaction>,
    mut panels: Query<(&mut BackgroundColor, &mut BorderColor), With<crate::ui::SkinPanel>>,
) {
    let (bg, border, _) = eaw_ui::panels::faction_colors(ui_faction(&state, &player));
    for (mut b, mut c) in &mut panels {
        if b.0 != bg {
            b.0 = bg;
        }
        let want = BorderColor::all(border);
        if *c != want {
            *c = want;
        }
    }
}

/// A short message under the speed read-out (order confirmations).
#[derive(Resource, Default)]
struct Toast(String, f32);
/// The toast text.
#[derive(Component)]
struct ToastText;

fn draw_toast(time: Res<Time>, mut toast: ResMut<Toast>, mut tip: Single<(&mut Text, &mut Visibility), With<ToastText>>) {
    let (text, vis) = &mut *tip;
    if toast.1 > 0.0 {
        toast.1 -= time.delta_secs();
        if text.0 != toast.0 {
            text.0 = toast.0.clone();
        }
        **vis = Visibility::Inherited;
    } else if **vis != Visibility::Hidden {
        **vis = Visibility::Hidden;
    }
}

/// Offscreen `--click` / `--battle-click`: presses command bar buttons by name so UI paths can be verified headless.
fn scripted_clicks(
    args: Res<crate::cli::Args>,
    screen: Res<State<AppScreen>>,
    state: Res<SimState>,
    hold: Res<crate::play::ScriptHold>,
    mut out: MessageWriter<EawButtonClicked>,
    mut galaxy_frames: Local<u32>,
    mut battle_frames: Local<u32>,
) {
    if args.offscreen.is_none() {
        return;
    }
    let (list, frames) = match screen.get() {
        AppScreen::Galaxy if !hold.active && !state.current.loading => (args.click.as_deref(), &mut *galaxy_frames),
        AppScreen::SpaceBattle => (args.battle_click.as_deref(), &mut *battle_frames),
        _ => return,
    };
    let Some(list) = list else { return };
    *frames += 1;
    // First click after 60 frames, then one every 40.
    if *frames >= 60 && (*frames - 60) % 40 == 0 {
        let n = ((*frames - 60) / 40) as usize;
        if let Some(name) = list.split(',').nth(n) {
            info!("script: clicking `{name}`");
            out.write(EawButtonClicked { name: name.trim().to_lowercase(), entity: Entity::PLACEHOLDER, shift: false });
        }
    }
}

/// What the player did in this game (counted from the simulation events; a loaded save starts again at zero).
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct GameStats {
    battles_won: u32,
    battles_lost: u32,
    ships_built: u32,
    ships_lost: u32,
}

impl GameStats {
    /// One line stored next to a save file (`slotN.stats`): the sim does not count these, a replay does not report them.
    pub(crate) fn to_text(&self) -> String {
        format!("v1 {} {} {} {}\n", self.battles_won, self.battles_lost, self.ships_built, self.ships_lost)
    }

    /// Reads [`GameStats::to_text`].
    pub(crate) fn from_text(text: &str) -> Option<Self> {
        let mut it = text.split_whitespace();
        if it.next()? != "v1" {
            return None;
        }
        let mut n = || it.next()?.parse::<u32>().ok();
        Some(Self { battles_won: n()?, battles_lost: n()?, ships_built: n()?, ships_lost: n()? })
    }

    /// Side file of a save.
    pub(crate) fn sidecar(save: &std::path::Path) -> std::path::PathBuf {
        save.with_extension("stats")
    }
}

/// The campaign result and its dialog.
#[derive(Resource, Default)]
pub(crate) struct EndState {
    pub(crate) stats: GameStats,
    /// `Some(true)` victory, `Some(false)` defeat, waiting for the dialog.
    pending: Option<bool>,
    /// The dialog entity once shown.
    dialog: Option<Entity>,
    /// The result was already announced for this game.
    announced: bool,
}

/// Counts the player's events and notices victory and defeat.
fn track_game_end(mut events: MessageReader<galaxy_play::PlayEvent>, player: Res<PlayerFaction>, mut end: ResMut<EndState>) {
    use galaxy_sim::Event;
    let Some(me) = player.0 else {
        events.clear();
        return;
    };
    for e in events.read() {
        let galaxy_play::PlayEvent::Sim(ev) = e else { continue };
        match ev {
            Event::BattleWon { faction, .. } if *faction == me => end.stats.battles_won += 1,
            Event::BattleLost { faction, .. } if *faction == me => end.stats.battles_lost += 1,
            Event::UnitsDestroyed { faction, count, .. } if *faction == me => end.stats.ships_lost += count,
            Event::ConstructionComplete { faction, kind: galaxy_sim::ProductionKind::Unit(_), .. } if *faction == me => end.stats.ships_built += 1,
            Event::Victory { faction } if !end.announced => {
                end.pending = Some(*faction == me);
                end.announced = true;
            }
            Event::FactionEliminated { faction } if *faction == me && !end.announced => {
                end.pending = Some(false);
                end.announced = true;
            }
            _ => {}
        }
    }
}

/// Offscreen `--force-end win|lose`: injects the event that ends the campaign.
fn force_end_script(
    args: Res<crate::cli::Args>,
    player: Res<PlayerFaction>,
    state: Res<SimState>,
    hold: Res<crate::play::ScriptHold>,
    mut out: MessageWriter<galaxy_play::PlayEvent>,
    end: Res<EndState>,
    mut dialog_out: MessageWriter<EawDialogResult>,
    mut commands: Commands,
    mut frames: Local<u32>,
) {
    let (Some(mode), Some(me), true) = (args.force_end.as_deref(), player.0, args.offscreen.is_some()) else { return };
    let win = mode.starts_with("win");
    if hold.active || state.current.loading {
        return;
    }
    *frames += 1;
    if *frames == 90 {
        let ev = if win { galaxy_sim::Event::Victory { faction: me } } else { galaxy_sim::Event::FactionEliminated { faction: me } };
        out.write(galaxy_play::PlayEvent::Sim(ev));
    }
    // `win-menu` / `lose-menu`: press "Main Menu" in the dialog.
    if *frames == 160 && mode.ends_with("menu") && let Some(d) = end.dialog {
        info!("script: clicking `Main Menu` in the end dialog");
        dialog_out.write(EawDialogResult { dialog: d, button: 0 });
        commands.entity(d).despawn();
    }
}

/// Victory / defeat dialog with the game statistics, music stinger and advisor line.
fn show_end_dialog(world: &mut World) {
    let Some(win) = world.resource::<EndState>().pending else { return };
    if world.get_resource::<EawUiAssets>().is_none() {
        world.resource_mut::<EndState>().pending = None;
        return;
    }
    let (faction, weeks, planets, credits, total) = {
        let state = world.resource::<SimState>();
        let player = world.resource::<PlayerFaction>();
        let model = world.resource::<PlayUiModel>();
        (
            ui_faction(state, player),
            state.current.week + 1,
            model.top.planets,
            model.top.credits,
            world.resource::<crate::galaxy_view::GalaxyView>().planets.len(),
        )
    };
    let stats = world.resource::<EndState>().stats.clone();
    let name = world.resource::<crate::flow::CurrentCampaign>().faction.clone().unwrap_or_else(|| "Rebel".into());
    let text = format!(
        "Weeks played: {weeks}\nPlanets held: {planets} of {total}\nBattles won: {}   lost: {}\nShips built: {}   lost: {}\nCredits: {}",
        stats.battles_won,
        stats.battles_lost,
        stats.ships_built,
        stats.ships_lost,
        crate::ui::thousands(credits as i64)
    );
    let spec = DialogSpec {
        title: if win { "Victory".into() } else { "Defeat".into() },
        text,
        portrait: None,
        faction,
        buttons: vec!["Main Menu".into(), "Stay on the map".into()],
    };
    if let Some(e) = spawn_dialog(world, &spec) {
        let mut end = world.resource_mut::<EndState>();
        end.pending = None;
        end.dialog = Some(e);
        if let Some(mut music) = world.get_resource_mut::<alamo_audio::MusicDirector>() {
            music.set_scene(alamo_audio::MusicScene::Result { faction: name.clone(), win, tactical: false });
        }
        if !win && world.contains_resource::<alamo_audio::plugin::AlamoAudio>() {
            world.write_message(alamo_audio::plugin::GalaxyAudio { faction: name, event: alamo_audio::GalaxyEvent::Defeated });
        }
    }
}

/// "Main Menu" leaves the game, the other button keeps watching the map.
fn end_dialog_result(
    mut results: MessageReader<EawDialogResult>,
    mut end: ResMut<EndState>,
    mut next: ResMut<NextState<AppScreen>>,
    mut paused: ResMut<crate::flow::Paused>,
) {
    for r in results.read() {
        if end.dialog != Some(r.dialog) {
            continue;
        }
        end.dialog = None;
        if r.button == 0 {
            paused.0 = false;
            next.set(AppScreen::MainMenu);
        }
    }
}

/// A new game starts with fresh statistics.
fn reset_end_state(mut end: ResMut<EndState>) {
    if end.announced || end.dialog.is_some() || end.pending.is_some() || end.stats != GameStats::default() {
        *end = EndState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::GameStats;

    #[test]
    fn stats_sidecar_round_trips() {
        let s = GameStats { battles_won: 3, battles_lost: 1, ships_built: 12, ships_lost: 7 };
        assert_eq!(GameStats::from_text(&s.to_text()), Some(s));
        assert_eq!(GameStats::from_text("v2 1 2 3 4"), None);
        assert_eq!(GameStats::from_text("v1 1 2"), None);
        assert_eq!(GameStats::sidecar(std::path::Path::new("saves/slot1.json")), std::path::PathBuf::from("saves/slot1.stats"));
    }
}
