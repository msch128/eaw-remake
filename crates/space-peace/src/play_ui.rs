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
use crate::battle_play::PendingTactical;
use crate::galaxy_map::GalaxyEntity;
use crate::offscreen::OffscreenTarget;
use crate::play::{PlayBridge, on_space_tab};
use crate::settings::Settings;
use crate::ui::InfoPanel;

/// Dark plate behind the command bar (the original art is very transparent on the right).
#[derive(Component)]
pub(crate) struct BarBacking;

/// Pads the galactic bar can show (`b_create_00..39`).
const MAX_PADS: usize = 40;
/// Queue slots per lane (left: space, right: ground).
const QUEUE_LANE_SLOTS: usize = 5;

/// Ship class filter of the left grid (the four class buttons of the galactic bar). A second click on the active button clears it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BarClass {
    /// `b_filters_corvette`: the small ships, fighter and bomber squadrons included (the grid has no extra fighter button).
    Corvette,
    /// `b_filters_frigate`.
    Frigate,
    /// `b_filters_capital`: capital and super capital ships.
    Capital,
    /// `b_filters_station`: stations and freighters, with the star base, orbital structures, upgrades, tech and heroes.
    Station,
}

impl BarClass {
    fn from_button(name: &str) -> Option<Self> {
        Some(match name {
            "b_filters_corvette" => Self::Corvette,
            "b_filters_frigate" => Self::Frigate,
            "b_filters_capital" => Self::Capital,
            "b_filters_station" => Self::Station,
            _ => return None,
        })
    }

    /// The class filter a build entry belongs to. Every entry belongs to exactly one: heroes, orbital structures, the star
    /// base, tech and research items and transports go with the stations / freighters filter.
    pub(crate) fn of(category: galaxy_play::BuildCategory, hero: bool) -> Self {
        use galaxy_play::BuildCategory as C;
        match category {
            _ if hero => Self::Station,
            C::Squadrons | C::Corvettes => Self::Corvette,
            C::Frigates => Self::Frigate,
            C::Capitals => Self::Capital,
            C::Starbase | C::Tech | C::Research | C::Structures | C::Other => Self::Station,
        }
    }

    /// Whether a build option belongs to this class.
    /// In EaW Remake, heroes appear under their ship class (e.g. Capitals or Frigates)
    /// AND always under the Stations / Freighters filter (tab 4).
    fn matches(self, category: galaxy_play::BuildCategory, hero: bool) -> bool {
        if hero && self == Self::Station {
            return true;
        }
        Self::of(category, false) == self
    }
}

/// Command bar state of the running game.
#[derive(Resource, Debug)]
pub(crate) struct BarState {
    /// Active class filter of the space tab (`None` = everything).
    pub(crate) class: Option<BarClass>,
    spawned: bool,
    /// Space production tab (ships, star base, tech) instead of the planet tab (structures).
    space_tab: bool,
    /// What each visible pad orders.
    pub(crate) pads: Vec<Option<BuildTarget>>,
    /// Queue slots (empty ones are `None`): lane, position in the lane and the credits a cancel gives back.
    pub(crate) queue: Vec<Option<(galaxy_sim::Lane, u32, f32)>>,
    /// Page of the build pads (see `MAX_PADS`).
    page: usize,
}

impl Default for BarState {
    fn default() -> Self {
        Self { class: None, spawned: false, space_tab: true, pads: Vec::new(), queue: Vec::new(), page: 0 }
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
/// Centered "PAUSED" banner shown at the top of the screen when galaxy game is paused.
#[derive(Component)]
pub(crate) struct PauseBanner;
/// Selected fleets read-out.
#[derive(Component)]
struct FleetText;

/// The open "battle pending" dialog.
#[derive(Resource, Default)]
pub(crate) struct BattleDialogState {
    pub(crate) open: Option<(u32, Entity)>,
    /// Battle the player just decided; the model still lists it until the next snapshot.
    pub(crate) decided: Option<u32>,
}

/// Wires the HUD.
pub struct PlayUiPlugin;

impl Plugin for PlayUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BarState>()
            .init_resource::<BattleDialogState>()
            .init_resource::<ObjectivesWindow>()
            .init_resource::<MoveMarkers>()
            .init_resource::<SuperweaponAsk>()
            .init_resource::<Toast>()
            .init_resource::<EndState>()
            .add_systems(PreUpdate, drive_ui_scale.run_if(resource_exists::<EawUiSettings>).run_if(not(in_state(AppScreen::CampaignSelect).or_else(in_state(AppScreen::MainMenu)))))
            .add_systems(
                Update,
                (
                    spawn_bar.run_if(resource_exists::<EawUiAssets>),
                    spawn_hud,
                    feed_bar.run_if(in_state(AppScreen::Galaxy)),
                    feed_minimap.run_if(resource_exists::<MinimapData>.and_then(in_state(AppScreen::Galaxy))),
                    minimap_pan.run_if(in_state(AppScreen::Galaxy)),
                    bar_clicks,
                    sync_filter_buttons,
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
            .add_systems(Update, (declutter_top, hero_bar, spawn_pad_arrows, pad_arrows, locked_clicks, spawn_planet_panel, update_planet_panel, place_planet_panel, planet_view_ui, planet_view_bar_tab, objectives_toggle, objectives_window, objectives_scroll).chain().run_if(resource_exists::<PlayBridge>.and_then(resource_exists::<SimState>)))
            .add_systems(Update, scripted_tooltip.run_if(resource_exists::<eaw_ui::ForcedTooltip>))
            .add_systems(Update, save_toast)
            .add_systems(Update, auto_resolve_summary.after(battle_dialog_result).run_if(resource_exists::<PlayBridge>.and_then(resource_exists::<SimState>)))
            .add_systems(Update, show_battle_summary.run_if(in_state(AppScreen::Galaxy)).run_if(resource_exists::<crate::battle_play::BattleSummary>))
            .add_systems(Update, (result_return, result_scroll))
            .add_systems(
                Update,
                (track_game_end, force_end_script, show_end_dialog, end_dialog_result, superweapon_button, superweapon_dialog, explode_on_destroy, expire_explosions)
                    .chain()
                    .run_if(resource_exists::<PlayBridge>.and_then(resource_exists::<SimState>)),
            )
            .add_systems(Update, (live_info_panel.after(crate::ui::update_info_panel), publish_panel_rect, focus_start.run_if(in_state(AppScreen::Galaxy))).run_if(resource_exists::<SimState>))
            .add_systems(Update, fix_fleet_label_scale.after(galaxy_play::PlaySet::Visuals).run_if(resource_exists::<SimState>))
            .add_systems(OnExit(AppScreen::Galaxy), |mut tip: ResMut<eaw_ui::WorldTooltip>| tip.text = None)
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
    let design = design_scale(window.as_deref().map(|w| w.size()), offscreen.as_deref());
    let v = if bar.is_empty() { settings.ui_scale } else { design * settings.ui_scale };
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

/// The object a build target creates.
pub(crate) fn target_object<'a>(world: &'a galaxy_sim::World, target: BuildTarget) -> Option<&'a str> {
    match target {
        BuildTarget::Unit(u) => Some(&world.units[u as usize].name),
        BuildTarget::Structure(s) => Some(&world.structures[s as usize].name),
        BuildTarget::Starbase(s) => Some(&world.starbases[s as usize].name),
        BuildTarget::Tech(_) => None,
    }
}

/// The lore / description of an object: `Encyclopedia_Text` (text ids joined, `TEXT_LINE` is a line break), else its `Tooltip_Text`.
pub(crate) fn lore(game: Option<&alamo_data::GameData>, object: &str) -> Option<String> {
    let game = game?;
    let r = game.objects.resolve(object)?;
    let join = |ids: &str| {
        let mut out = String::new();
        for id in ids.split_whitespace() {
            if id.eq_ignore_ascii_case("TEXT_LINE") {
                out.push('\n');
                continue;
            }
            if let Some(t) = game.text(id).map(str::trim).filter(|t| !t.is_empty()) {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push(' ');
                }
                out.push_str(t);
            }
        }
        let out = out.trim().to_owned();
        (!out.is_empty()).then_some(out)
    };
    r.get("encyclopedia_text").and_then(&join).or_else(|| r.get("tooltip_text").and_then(&join))
}

/// What reaching tech level `level` unlocks for `faction`, from the ships, structures and star bases that require it.
fn tech_unlocks(state: &SimState, catalog: &galaxy_play::PlayCatalog, faction: Option<u32>, level: i32) -> String {
    let world = &state.world;
    let mut s = format!("Raises your tech level to {level}. Higher tech levels unlock stronger ships, buildings and star base upgrades.");
    let Some(f) = faction.filter(|&f| (f as usize) < world.buildable_units.len()) else { return s };
    let list = |names: Vec<&str>| {
        let mut names: Vec<String> = names.into_iter().map(str::to_owned).collect();
        names.sort();
        names.dedup();
        const SHOWN: usize = 14;
        let more = names.len().saturating_sub(SHOWN);
        names.truncate(SHOWN);
        let mut out = names.join(", ");
        if more > 0 {
            out.push_str(&format!(" and {more} more"));
        }
        out
    };
    let units: Vec<&str> = world.buildable_units[f as usize].iter().filter(|&&u| world.units[u as usize].tech == level && !world.units[u as usize].hero).map(|&u| catalog.unit(u)).collect();
    let structures: Vec<&str> = world.buildable_structures[f as usize].iter().filter(|&&b| world.structures[b as usize].tech == level && world.structures[b as usize].galactic).map(|&b| catalog.structure(b)).collect();
    let faction_name = world.factions[f as usize].name.to_ascii_lowercase();
    let bases: Vec<&str> = world.starbases.iter().enumerate().filter(|(_, b)| b.tech == level && b.affiliation.iter().any(|a| *a == faction_name)).map(|(i, _)| catalog.starbase(i as u32)).collect();
    let mut any = false;
    for (label, names) in [("Ships", units), ("Structures", structures), ("Star bases", bases)] {
        if !names.is_empty() {
            any = true;
            s.push_str(&format!("\n\nUnlocks {label}: {}", list(names)));
        }
    }
    if !any {
        s.push_str("\n\nNo new ships or buildings are listed for this level.");
    }
    s
}

/// Tooltip of the small upkeep / income marker on a pad.
fn marker_tooltip(upkeep: f32) -> String {
    if upkeep < -0.5 {
        format!("Upkeep\nThis unit costs {:.0} credits per week for as long as it exists. The upkeep is deducted from your income every week, so a big fleet lowers your income.\n", -upkeep)
    } else if upkeep > 0.5 {
        format!("Income bonus\nThis unit adds {upkeep:.0} credits per week to your income for as long as it exists.\n")
    } else {
        String::new()
    }
}

/// Rich tooltip of a build pad (format: see `eaw_ui::TooltipLayout`): name, lore, cost, time, population, upkeep and the reason when greyed out.
fn pad_tooltip(o: &galaxy_play::BuildOption, week: f32, state: &SimState, game: Option<&alamo_data::GameData>, catalog: &galaxy_play::PlayCatalog, faction: Option<u32>) -> String {
    let description = match o.target {
        BuildTarget::Tech(_) => {
            let level = faction.and_then(|f| state.galaxy().factions.get(f as usize)).map_or(2, |f| f.tech + 1);
            Some(tech_unlocks(state, catalog, faction, level))
        }
        target => target_object(&state.world, target).and_then(|n| lore(game, n)),
    };
    let mut s = o.name.to_uppercase();
    if let Some(d) = &description {
        s.push('\n');
        s.push_str(d.trim());
    }
    s.push_str(&format!("\n@Cost\t{:.0} credits\n@Build time\t{}", o.cost, format_duration(o.build_seconds, week)));
    if o.population > 0 {
        s.push_str(&format!("\n@Population\t{}", o.population));
    }
    let upkeep = pad_upkeep(state, o.target);
    let mentioned = description.as_deref().is_some_and(|d| d.to_ascii_lowercase().contains("upkeep"));
    if upkeep < -0.5 && !mentioned {
        s.push_str(&format!("\n@Upkeep\t{:.0} credits per week", -upkeep));
    } else if upkeep > 0.5 {
        s.push_str(&format!("\n@Income\t+{upkeep:.0} credits per week"));
    }
    // A structure that is not built yet: what it changes once it stands, worded so it is not mistaken for current income.
    if let BuildTarget::Structure(id) = o.target {
        let change = state.world.structures[id as usize].income_abs;
        if change > 0.5 {
            s.push_str(&format!("\n@Income\t+{change:.0} credits/week when built"));
        } else if change < -0.5 && !mentioned {
            s.push_str(&format!("\n@Upkeep\t{:.0} credits/week when built", -change));
        }
    }
    if !o.availability.is_available() {
        s.push_str(&format!("\n!{}", capitalize(&o.availability.reason())));
    }
    s
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().collect::<String>() + c.as_str())
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
    mut panel: ResMut<eaw_ui::PanelData>,
    player: Res<PlayerFaction>,
    catalog: Res<galaxy_play::PlayCatalog>,
    mut last_focus: Local<Option<u32>>,
    mut pad_context: Local<(Option<u32>, bool, Option<BarClass>)>,
) {
    if !model.is_changed() && !bar.is_changed() {
        return;
    }
    let game = assets.as_ref().and_then(|a| a.game.clone());
    let game = game.as_deref();
    let mut next = CommandBarData {
        credits: model.top.credits as i64,
        day: (state.current.time / f64::from(state.world.config.week_seconds) / 4.0) as u32 + 1,
        month_income: (model.top.income_per_week * 4.0) as i64,
        month_progress: ((state.current.time / f64::from(state.world.config.week_seconds) / 4.0).fract()) as f32,
        tech_level: state.galaxy().factions.get(state.world.victory.active_player.unwrap_or(0) as usize).map_or(0, |f| f.tech.max(0) as u32),
        pop_current: model.top.population_used.max(0) as u32,
        pop_cap: model.top.population_cap.max(0) as u32,
        space_tab: bar.space_tab,
        ..default()
    };
    // With only a fleet selected the panel shows the planet it orbits.
    let mut focus = selection.planet.or_else(|| selection.fleets.first().and_then(|id| state.galaxy().fleets.get(id)).and_then(|f| f.planet()));
    // Nothing selected: keep showing the last planet, else the player's richest one (the capital), like the original.
    if focus.is_some() {
        *last_focus = focus;
    } else {
        focus = (*last_focus).or_else(|| {
            let me = player.0?;
            let income = |p: usize| bridge.sim_to_planet[p].and_then(|i| view.planets.get(i)).map_or(0, |pv| pv.income);
            let capital = (0..state.galaxy().planets.len()).filter(|&p| state.galaxy().planets[p].owner == Some(me)).max_by_key(|&p| income(p))?;
            u32::try_from(capital).ok()
        });
    }
    if let Some(p) = focus {
        next.planet_name = state.world.planets[p as usize].display_name.clone();
        next.planet_owner = state.galaxy().planets[p as usize].owner.map(|o| UiFaction::from_name(&state.world.factions[o as usize].name));
        let owner = state.galaxy().planets[p as usize].owner;
        next.planet_color = Some(match (owner, player.0) {
            (Some(o), Some(me)) if o == me => Color::srgb(0.3, 1.0, 0.4),
            (Some(_), _) => Color::srgb(1.0, 0.3, 0.3),
            _ => Color::WHITE,
        });
        next.planet_income = match &model.planet {
            Some(m) => m.income as i64,
            None => bridge.sim_to_planet[p as usize].and_then(|i| view.planets.get(i)).map_or(0, |pv| i64::from(pv.income)),
        };
    }
    let week = state.world.config.week_seconds;
    let space_tab = bar.space_tab;
    let class = bar.class;
    // Another planet, tab or class filter starts at the first page.
    if *pad_context != (focus, space_tab, class) {
        *pad_context = (focus, space_tab, class);
        bar.page = 0;
    }
    bar.queue.clear();
    bar.pads.clear();
    if let Some(menu) = &model.planet {
        // Two lanes of five slots: space items (ships, star base, tech) left, ground structures right.
        let mut per_lane: std::collections::BTreeMap<u8, u32> = Default::default();
        let mut shown = [0usize; 2];
        next.queue = vec![QueueEntry::default(); QUEUE_LANE_SLOTS * 2];
        bar.queue = vec![None; QUEUE_LANE_SLOTS * 2];
        for q in &menu.queue {
            let index = per_lane.entry(q.lane as u8).or_insert(0);
            // The lanes follow the tabs: space items and orbital structures left (blue), ground structures right (orange).
            let side = usize::from(!on_space_tab(&state.world, q.target));
            let (lane_index, cancel_index) = (shown[side], *index);
            *index += 1;
            if lane_index >= QUEUE_LANE_SLOTS {
                continue;
            }
            shown[side] += 1;
            let slot = side * QUEUE_LANE_SLOTS + lane_index;
            let refund = menu.options.iter().find(|o| o.target == q.target).map(|o| o.cost * (1.0 - q.progress) * state.world.config.cancel_refund);
            let cancel = refund.map_or("click".to_owned(), |r| format!("click, refunds {r:.0} credits"));
            bar.queue[slot] = Some((q.lane, cancel_index, refund.unwrap_or(0.0)));
            next.queue[slot] = QueueEntry {
                icon: icon_for(&state, game, q.target),
                progress: q.progress,
                count: 1,
                lane: side as u8,
                tooltip: {
                    let mut t = q.name.to_uppercase();
                    if let Some(d) = target_object(&state.world, q.target).and_then(|n| lore(game, n)) {
                        t.push('\n');
                        t.push_str(d.trim());
                    }
                    t.push_str(&format!("\n@Progress\t{:.0}%\n@Time left\t{}\n@Cancel\t{}", q.progress * 100.0, format_duration(q.remaining_seconds, week), cancel));
                    t
                },
            };
        }
        // The ground/structure tab lists only what can be ordered (or is short of credits, a full queue ...),
        // like the Remake's single row: structures behind a missing requirement stay hidden (v0260-v0450).
        let listed: Vec<_> = menu
            .options
            .iter()
            .filter(|o| on_space_tab(&state.world, o.target) == space_tab && (!space_tab || class.is_none_or(|c| c.matches(o.category, is_hero(&state.world, o.target)))))
            .filter(|o| space_tab || !matches!(o.availability, galaxy_play::Availability::Requires(_)))
            .collect();
        // Pages of MAX_PADS entries (the shell has that many pad slots); the page arrows above the pads step through them.
        let size = pad_page_size();
        let pages = listed.len().div_ceil(size).max(1);
        let page = bar.page.min(pages - 1);
        bar.page = page;
        next.pad_page = page;
        next.pad_pages = pages;
        let listed = listed.into_iter().skip(page * size).take(size);
        for o in listed {
            next.pads.push(BuildPadEntry {
                icon: icon_for(&state, game, o.target),
                price: Some(o.cost.max(0.0) as u32),
                disabled: !o.availability.is_available(),
                building: menu.queue.iter().any(|q| q.target == o.target),
                tooltip: pad_tooltip(o, week, &state, game, &catalog, player.0),
                marker_tooltip: marker_tooltip(pad_upkeep(&state, o.target)),
                upkeep: pad_upkeep(&state, o.target),
                level: pad_level(&state, o.target).max(required_level(&o.availability)),
                locked: matches!(o.availability, galaxy_play::Availability::Requires(_)),
                label: String::new(),
            });
            bar.pads.push(Some(o.target));
        }
    }
    if model.planet.is_none() && !selection.fleets.is_empty() {
        // A selected fleet shows its units in the bar (icon and count), like the original's fleet view.
        let mut units: std::collections::BTreeMap<u32, u32> = Default::default();
        for f in selection.fleets.iter().filter_map(|id| state.galaxy().fleets.get(id)) {
            for (&u, &c) in &f.units {
                *units.entry(u).or_default() += c;
            }
        }
        for (u, c) in units.into_iter().take(MAX_PADS) {
            next.pads.push(BuildPadEntry {
                icon: icon_for(&state, game, BuildTarget::Unit(u)),
                price: None,
                label: if c > 1 { format!("x{c}") } else { String::new() },
                tooltip: {
                    let name = catalog.unit(u);
                    let mut t = format!("{c}x {}", name.to_uppercase());
                    if let Some(d) = lore(game, &state.world.units[u as usize].name) {
                        t.push('\n');
                        t.push_str(d.trim());
                    }
                    t.push_str(&format!("\n@In this fleet\t{c}"));
                    t
                },
                ..default()
            });
        }
    }
    next.pads_bottom_row = !space_tab && model.planet.is_some();
    let level = (next.month_progress * 10.0).round() / 10.0;
    if panel.bars.get("fight_bar_g") != Some(&level) {
        panel.bars.insert("fight_bar_g".into(), level);
    }
    if *data != next {
        *data = next;
    }
}

/// Bar buttons: build pads, tabs, pause and fast-forward.
fn bar_clicks(
    remap: Res<eaw_ui::PadRemap>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut clicks: MessageReader<EawButtonClicked>,
    mut bar: ResMut<BarState>,
    selection: Res<PlayerSelection>,
    clock: Res<GameClock>,
    mut actions: MessageWriter<PlayAction>,
    mut toast: ResMut<Toast>,
    mut filters: ResMut<crate::galaxy_map::MapFilters>,
    mut paused: ResMut<crate::flow::Paused>,
    queue_buttons: Query<(&eaw_ui::EawButton, &Interaction)>,
) {
    if mouse.just_pressed(MouseButton::Right) {
        for (b, inter) in &queue_buttons {
            if *inter == Interaction::Hovered || *inter == Interaction::Pressed {
                if let Some(i) = b.name.strip_prefix("queue").and_then(|n| n.parse::<usize>().ok()) {
                    if let (Some(planet), Some(&(lane, index, refund))) = (selection.planet, bar.queue.get(i).and_then(Option::as_ref)) {
                        actions.write(PlayAction::CancelProduction { planet, lane, index });
                        toast.0 = if refund > 0.0 { format!("Production cancelled - {refund:.0} credits refunded") } else { "Production cancelled".to_owned() };
                        toast.1 = 3.0;
                    }
                }
            }
        }
    }
    for c in clicks.read() {
        let name = c.name.as_str();
        if let Some(i) = name.strip_prefix("b_create_").and_then(|n| n.parse::<usize>().ok()) {
            if let (Some(planet), Some(target)) = (selection.planet, bar.pads.get(remap.entry(i)).copied().flatten()) {
                actions.write(PlayAction::Build { planet, target });
            }
            continue;
        }
        if let Some(i) = name.strip_prefix("queue").and_then(|n| n.parse::<usize>().ok()) {
            if let (Some(planet), Some(&(lane, index, refund))) = (selection.planet, bar.queue.get(i).and_then(Option::as_ref)) {
                actions.write(PlayAction::CancelProduction { planet, lane, index });
                toast.0 = if refund > 0.0 { format!("Production cancelled - {refund:.0} credits refunded") } else { "Production cancelled".to_owned() };
                toast.1 = 3.0;
            }
            continue;
        }
        if let Some(class) = BarClass::from_button(name) {
            bar.class = if bar.class == Some(class) { None } else { Some(class) };
            bar.space_tab = true;
            continue;
        }
        match name {
            "b_space_tab" => bar.space_tab = true,
            "b_planet_tab" => bar.space_tab = false,
            "b_scroll_left" => bar.page = bar.page.saturating_sub(1),
            "b_scroll_right" => bar.page += 1,
            "b_filters0" => filters.credits = !filters.credits,
            "b_filters1" => filters.base_level = !filters.base_level,
            "b_filters2" => filters.weather = !filters.weather,
            "b_filters3" => filters.heroes = !filters.heroes,
            // The "Main Menu" button (`TEXT_BUTTON_MAIN_MENU`) opens the same pause menu as Esc.
            "b_option_g" => paused.0 = true,
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

/// Keeps the pressed look of the four filter toggles in line with [`crate::galaxy_map::MapFilters`].
fn sync_filter_buttons(filters: Res<crate::galaxy_map::MapFilters>, bar: Res<BarState>, mut buttons: Query<(&eaw_ui::EawButton, &mut eaw_ui::EawButtonState)>) {
    for (b, mut st) in &mut buttons {
        let on = match b.name.as_str() {
            "b_filters0" => filters.credits,
            "b_filters1" => filters.base_level,
            "b_filters2" => filters.weather,
            "b_filters3" => filters.heroes,
            n => match BarClass::from_button(n) {
                Some(c) => bar.class == Some(c),
                None => continue,
            },

        };
        if st.selected != on {
            st.selected = on;
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
        Node { top: Val::Px(52.0), left: Val::Px(16.0), width: Val::Px(440.0), padding: UiRect::all(Val::Px(6.0)), flex_direction: FlexDirection::Column, row_gap: Val::Px(2.0), ..abs.clone() },
        BackgroundColor(Color::NONE),
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
        PauseBanner,
        text_bundle("PAUSED", 26.0, Color::srgb(1.0, 0.85, 0.35)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(46.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..abs.clone()
        },
        TextLayout::justify(Justify::Center),
        GlobalZIndex(80),
        Visibility::Hidden,
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
        (text_bundle("", 11.5, Color::srgb(0.9, 0.95, 1.0)), TextLayout::no_wrap()),
        Node {
            bottom: Val::Px(296.0),
            left: Val::Px(40.0),
            max_width: Val::Px(620.0),
            padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
            border: UiRect::all(Val::Px(1.0)),
            ..abs
        },
        BackgroundColor(Color::srgba(0.13, 0.27, 0.31, 0.9)),
        BorderColor::all(Color::srgb(0.0, 0.76, 0.99)),
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
    let backing = if model.notifications.is_empty() { Color::NONE } else { Color::srgba(0.02, 0.04, 0.09, 0.55) };
    commands.entity(*ticker).insert(BackgroundColor(backing));
    commands.entity(*ticker).despawn_children();
    for n in model.notifications.iter().rev().take(6) {
        commands.spawn((text_bundle(&n.text, 13.0, kind_color(n.kind)), ChildOf(*ticker)));
    }
}

#[allow(clippy::type_complexity)]
fn refresh_texts(
    model: Res<PlayUiModel>,
    clock: Res<GameClock>,
    state: Res<SimState>,
    mut speed: Single<(&mut Text, &mut TextColor), (With<SpeedText>, Without<FleetText>, Without<PauseBanner>)>,
    mut fleets: Single<(&mut Text, &mut Visibility), (With<FleetText>, Without<SpeedText>, Without<PauseBanner>)>,
    mut pause_banner: Query<&mut Visibility, (With<PauseBanner>, Without<FleetText>, Without<SpeedText>)>,
) {
    if !model.is_changed() && !clock.is_changed() {
        return;
    }
    let paused = clock.paused() || model.speed.rate <= 0.0;
    for mut v in &mut pause_banner {
        *v = if paused { Visibility::Inherited } else { Visibility::Hidden };
    }
    let (text, color) = &mut *speed;
    let (s, c) = if let Some(w) = &model.top.winner {
        (format!("{w} wins the galaxy"), Color::srgb(1.0, 0.9, 0.4))
    } else if model.speed.held_by_battle {
        ("Battle pending".to_owned(), Color::srgb(1.0, 0.8, 0.35))
    } else if paused {
        (String::new(), Color::NONE)
    } else {
        (format!("Week {}   x{}", model.top.week + 1, model.speed.rate), Color::srgb(0.85, 0.92, 1.0))
    };
    if text.0 != s {
        text.0 = s;
    }
    color.0 = c;
    let week = state.world.config.week_seconds;
    // The selected fleet's units are in the bar; this box only carries the route and arrival time of moving fleets.
    let lines: Vec<String> = model
        .fleets
        .iter()
        .filter_map(|f| f.heading.as_ref().map(|(d, eta)| format!("To {d}: arrival in {}", format_duration(*eta, week))))
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
fn battle_dialog(world: &mut World, mut gone_frames: Local<u32>) {
    let Some(model) = world.get_resource::<PlayUiModel>() else { return };
    let wanted = model.battle.clone();
    let open = world.resource::<BattleDialogState>().open;
    if let (Some((id, e)), Some(b)) = (open, &wanted)
        && b.id == id
        && world.get_entity(e).is_ok()
    {
        *gone_frames = 0;
        return;
    }
    // The dialog closes itself when a button is pressed; its result arrives a frame later. Do not
    // open a second dialog in between (it would be left behind with nobody listening).
    if let (Some((id, _)), Some(b)) = (open, &wanted)
        && b.id == id
        && *gone_frames < 10
    {
        *gone_frames += 1;
        return;
    }
    *gone_frames = 0;
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
    let game = world.get_resource::<EawUiAssets>().and_then(|a| a.game.clone());
    let side = |s: &galaxy_play::BattleSide| eaw_ui::BannerSide {
        stacks: s.ships.clamp(1, 5),
        icon: game.as_ref().and_then(|g| eaw_ui::unit_icon(g, &s.flagship)).filter(|_| !s.flagship.is_empty()),
        tint: if s.mine { Color::srgb(0.35, 1.0, 0.45) } else { Color::srgb(1.0, 0.3, 0.3) },
    };
    let mine = b.sides.iter().find(|s| s.mine).or(b.sides.first());
    let other = b.sides.iter().find(|s| !s.mine && Some(*s) != mine).or(b.sides.get(1));
    let (Some(mine), Some(other)) = (mine, other) else { return };
    let spec = eaw_ui::BattleBannerSpec {
        title: format!("{}{}", b.planet_name.to_uppercase(), game.as_ref().and_then(|g| g.text("TEXT_TACTICAL_SPACE_BATTLE_IS_PENDING")).unwrap_or(": SPACE BATTLE IMMINENT!")),
        faction,
        begin: "BEGIN".into(),
        auto: "AUTO-RESOLVE".into(),
        left: side(mine),
        right: side(other),
    };
    info!("play: battle banner flagships {:?} / {:?}, icons {:?} / {:?}", mine.flagship, other.flagship, spec.left.icon, spec.right.icon);
    if let Some(e) = eaw_ui::spawn_battle_banner(world, &spec) {
        info!("play: battle dialog shown for battle {} at {}", b.id, b.planet_name);
        world.resource_mut::<BattleDialogState>().open = Some((b.id, e));
    }
}

fn battle_dialog_result(
    mut results: MessageReader<EawDialogResult>,
    mut clicks: MessageReader<EawButtonClicked>,
    mut dialog: ResMut<BattleDialogState>,
    mut actions: MessageWriter<PlayAction>,
    mut commands: Commands,
) {
    let mut pressed: Vec<usize> = Vec::new();
    for r in results.read() {
        if dialog.open.is_some_and(|(_, e)| e == r.dialog) {
            pressed.push(r.button);
        }
    }
    for c in clicks.read() {
        match c.name.as_str() {
            eaw_ui::BANNER_BEGIN => pressed.push(0),
            eaw_ui::BANNER_AUTO => pressed.push(1),
            _ => {}
        }
    }
    let (Some(&button), Some((id, e))) = (pressed.first(), dialog.open) else { return };
    info!("play: battle dialog button {button} pressed for battle {id}");
    commands.entity(e).try_despawn();
    dialog.open = None;
    dialog.decided = Some(id);
    let choice = if button == 0 { BattleChoice::Fight } else { BattleChoice::AutoResolve };
    actions.write(PlayAction::ResolveBattle { id, choice });
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

/// Tells the label layout where the planet panel is.
#[allow(clippy::type_complexity)]
fn publish_panel_rect(
    panels: Query<(&ComputedNode, &UiGlobalTransform, &InheritedVisibility), (Or<(With<crate::ui::SkinPanel>, With<Ticker>, With<eaw_ui::EawPanel>, With<PlanetPanel>)>, Without<crate::ui::Tooltip>)>,
    ui_scale: Res<UiScale>,
    mut rect: ResMut<crate::galaxy_map::PanelRect>,
) {
    let rects: Vec<(Vec2, Vec2)> = panels
        .iter()
        .filter(|(_, _, vis)| vis.get())
        .map(|(node, tf, _)| {
            let sf = node.inverse_scale_factor() * ui_scale.0.max(0.1);
            let c = tf.translation * sf;
            let half = node.size() * sf * 0.5;
            (c - half, c + half)
        })
        .collect();
    if rect.0 != rects {
        rect.0 = rects;
    }
}

fn move_info_panel(mut panel: Single<&mut Node, With<InfoPanel>>, mut hint: Query<&mut Node, (With<crate::ui::HintText>, Without<InfoPanel>)>) {
    if panel.right == Val::Px(16.0) {
        return;
    }
    panel.left = Val::Auto;
    panel.bottom = Val::Auto;
    panel.right = Val::Px(16.0);
    panel.top = Val::Px(72.0);
    for mut h in &mut hint {
        h.bottom = Val::Px(130.0);
    }
}

#[allow(clippy::type_complexity)]
fn hide_play_ui(mut bar: Query<&mut Visibility, Or<(With<EawCommandBar>, With<BarBacking>, With<HeroBar>, With<SpeedText>, With<PauseBanner>)>>) {
    for mut v in &mut bar {
        *v = Visibility::Hidden;
    }
}

#[allow(clippy::type_complexity)]
fn show_play_ui(mut bar: Query<&mut Visibility, Or<(With<EawCommandBar>, With<BarBacking>, With<HeroBar>)>>) {
    for mut v in &mut bar {
        *v = Visibility::Inherited;
    }
}


/// Galactic minimap: planets by owner, fleets, the camera's view; refreshed a few times a second.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn feed_minimap(
    time: Res<Time>,
    bridge: Res<PlayBridge>,
    player: Res<PlayerFaction>,
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
            let owner = bridge.planet_to_sim.get(i).copied().flatten().and_then(|p| state.galaxy().planets[p as usize].owner);
            MinimapDot { pos, color: relation_color(&state, player.0, owner), selected: selection.0 == Some(i) }
        })
        .collect();
    if planets.is_empty() {
        return;
    }
    let margin = (hi - lo) * 0.04;
    let units = fleets
        .iter()
        .map(|(f, t)| {
            MinimapDot { pos: galaxy(t.translation), color: relation_color(&state, player.0, Some(f.owner)), selected: false }
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
    let next = MinimapData { bounds: Some((lo - margin, hi + margin)), planets, units, pings: Vec::new(), camera: Some((center - half, center + half)), background: None };
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
    if !args.scripted() || hold.active {
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

#[allow(clippy::too_many_arguments)]
fn loading_overlay(
    mut commands: Commands,
    state: Res<SimState>,
    overlay: Query<Entity, With<LoadOverlay>>,
    mut text: Query<&mut Text, With<LoadText>>,
    mut fill: Query<&mut Node, With<LoadFill>>,
    game: Res<crate::game::Game>,
    mut images: ResMut<Assets<Image>>,
    campaign: Res<crate::flow::CurrentCampaign>,
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
        let art = game.0.as_ref().and_then(|g| crate::loading::load_art(&g.fs, campaign.faction.as_deref(), &mut images));
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
                BackgroundColor(if art.is_some() { Color::BLACK } else { Color::srgba(0.0, 0.0, 0.03, 0.75) }),
                GlobalZIndex(40),
                Interaction::default(),
                bevy::ui::FocusPolicy::Block,
            ))
            .with_children(|root| {
                if let Some(h) = art.clone() {
                    root.spawn((
                        Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
                        ImageNode::new(h).with_mode(NodeImageMode::Stretch),
                    ));
                }
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
fn collect_move_markers(
    mut actions: MessageReader<PlayAction>,
    selection: Res<PlayerSelection>,
    state: Res<SimState>,
    player: Res<PlayerFaction>,
    mut markers: ResMut<MoveMarkers>,
    mut toast: ResMut<Toast>,
) {
    for a in actions.read() {
        let to = match a {
            PlayAction::MoveSelected(to) => Some(*to),
            PlayAction::MoveAndMerge { to, .. } => Some(*to),
            _ => None,
        };
        if let Some(to) = to
            && !selection.fleets.is_empty()
        {
            let g = state.galaxy();
            if selection.fleets.iter().any(|id| g.fleets.get(id).is_some_and(|f| player.0 == Some(f.owner))) {
                markers.0.push((to, 0.0));
            } else {
                // Foreign fleets can be inspected but not commanded.
                toast.0 = "Not your fleet - you cannot give it orders".to_owned();
                toast.1 = 2.5;
            }
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

/// Hover text of the fleet or planet under the cursor, shown by `eaw-ui`'s tooltip next to the advisor droid.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fleet_tooltip(
    hover: Res<galaxy_play::PlayHover>,
    state: Res<SimState>,
    catalog: Res<galaxy_play::PlayCatalog>,
    bridge: Res<PlayBridge>,
    view: Res<crate::galaxy_view::GalaxyView>,
    fog: Res<galaxy_play::Fog>,
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    mut tip: ResMut<eaw_ui::WorldTooltip>,
) {
    let cursor = window.as_ref().and_then(|w| w.cursor_position());
    let fleet = hover.fleet.and_then(|id| state.galaxy().fleets.get(&id).map(|f| (f, fog.fleet(id)))).filter(|(_, s)| *s != galaxy_sim::Sight::Hidden);
    let planet = hover.planet.and_then(|p| bridge.sim_to_planet.get(p as usize).copied().flatten().map(|i| (i, fog.planet(p)))).and_then(|(i, s)| view.planets.get(i).map(|pv| (i, pv, s)));
    let text = cursor.and_then(|_| fleet.map(|(f, s)| fleet_text(f, s, &state, &catalog)).or_else(|| planet.map(|(i, pv, s)| planet_text(i, pv, s, &view))));
    if tip.text != text {
        tip.text = text;
    }
}

fn fleet_text(f: &galaxy_sim::Fleet, sight: galaxy_sim::Sight, state: &SimState, catalog: &galaxy_play::PlayCatalog) -> String {
    let detailed = sight == galaxy_sim::Sight::Detailed;
    let mut lines = vec![if detailed { format!("{} fleet - {} ships", catalog.faction(f.owner), f.unit_count()) } else { format!("{} fleet - ? ships", catalog.faction(f.owner)) }];
    if detailed {
        for (&u, &c) in f.units.iter().take(8) {
            lines.push(format!("{c}x {}", catalog.unit(u)));
        }
        if f.units.len() > 8 {
            lines.push(format!("+{} more types", f.units.len() - 8));
        }
    } else {
        lines.push("Strength unknown".to_owned());
    }
    lines.push(match &f.location {
        galaxy_sim::FleetLocation::AtPlanet(p) => format!("@Location\tIn orbit of {}", state.world.planets[*p as usize].display_name),
        galaxy_sim::FleetLocation::Moving { to, .. } => format!("@Location\tHeading to {}", state.world.planets[*to as usize].display_name),
    });
    lines.join("\n")
}

fn planet_text(i: usize, p: &crate::galaxy_view::PlanetView, sight: galaxy_sim::Sight, view: &crate::galaxy_view::GalaxyView) -> String {
    let mut lines = vec![p.display_name.clone()];
    if sight != galaxy_sim::Sight::Hidden
        && let Some((name, desc)) = &p.ability
    {
        lines.push(format!("Bonus: {name}"));
        if !desc.is_empty() {
            lines.push(desc.clone());
        }
    }
    lines.push(format!("@Owner\t{}", view.owner_of(i).map_or("Neutral", |f| f.name.as_str())));
    lines.push(format!("@Income\t{} credits", crate::ui::thousands(i64::from(p.income))));
    // Out of sight: the video shows name, owner colour and income of distant planets, nothing about their defences.
    if sight != galaxy_sim::Sight::Hidden {
        lines.push(format!("@Space slots\t{}", p.max_space_base));
        lines.push(format!("@Ground slots\t{}", p.land_slots));
    }
    lines.join("\n")
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
pub(crate) struct Toast(pub(crate) String, pub(crate) f32);
/// The toast text.
#[derive(Component)]
struct ToastText;

/// "Game saved" confirmation after a save finished (or why it failed).
fn save_toast(mut events: MessageReader<galaxy_play::PlayEvent>, mut toast: ResMut<Toast>) {
    for e in events.read() {
        if let galaxy_play::PlayEvent::SaveFinished { result, .. } = e {
            toast.0 = match result {
                Ok(()) => "Game saved".to_owned(),
                Err(_) => "Could not save the game".to_owned(),
            };
            toast.1 = 3.0;
        }
    }
}

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
#[allow(clippy::too_many_arguments)]
fn scripted_clicks(
    args: Res<crate::cli::Args>,
    screen: Res<State<AppScreen>>,
    state: Res<SimState>,
    hold: Res<crate::play::ScriptHold>,
    mut out: MessageWriter<EawButtonClicked>,
    mut galaxy_frames: Local<u32>,
    mut battle_frames: Local<u32>,
    runtime: Option<Res<battle_view::BattleRuntime>>,
) {
    if !args.scripted() {
        return;
    }
    let (list, frames) = match screen.get() {
        AppScreen::Galaxy if !hold.active && !state.current.loading => (args.click.as_deref(), &mut *galaxy_frames),
        // Clicks wait for the battle itself: its bar and reserve are set up after the loading screen.
        AppScreen::SpaceBattle if runtime.is_some() => (args.battle_click.as_deref(), &mut *battle_frames),
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

/// The campaign result and its dialog.
#[derive(Resource, Default)]
pub(crate) struct EndState {
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
#[allow(clippy::too_many_arguments)]
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
    let (Some(mode), Some(me), true) = (args.force_end.as_deref(), player.0, args.scripted()) else { return };
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
    let stats = {
        let state = world.resource::<SimState>();
        world.resource::<PlayerFaction>().0.map(|h| state.current.stats(h)).unwrap_or_default()
    };
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
        if win && world.contains_resource::<alamo_audio::plugin::AlamoAudio>() {
            // The announcer line of the faction (`<FACTION>_VICTORY_NN.WAV`), picked from the game state so a replay says the same.
            let (upper, lines) = match name.to_ascii_lowercase().as_str() {
                "rebel" => ("REBEL", 3),
                "underworld" => ("UNDERWORLD", 3),
                _ => ("EMPIRE", 8),
            };
            let n = (weeks + u64::from(planets)) % lines;
            world.write_message(alamo_audio::PlayFile { name: format!("{upper}_VICTORY_{n:02}.WAV"), bus: alamo_audio::Bus::Speech });
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
    if end.announced || end.dialog.is_some() || end.pending.is_some() {
        *end = EndState::default();
    }
}

/// While a game runs the planet info shows the live state (star base, structures, fleets in orbit), not the starting forces.
fn live_info_panel(
    selection: Res<PlayerSelection>,
    model: Res<PlayUiModel>,
    state: Res<SimState>,
    catalog: Res<galaxy_play::PlayCatalog>,
    fog: Res<galaxy_play::Fog>,
    mut lines: Query<(&crate::ui::InfoLine, &mut Text)>,
) {
    let Some(p) = selection.planet else { return };
    let mut rows: Vec<String> = Vec::new();
    let mut slots = None;
    if let Some(menu) = model.planet.as_ref().filter(|m| m.planet == p) {
        slots = Some(format!(
            "Space slots {}/{}   Ground slots {}/{}",
            menu.space_slots.0, menu.space_slots.1, menu.ground_slots.0, menu.ground_slots.1
        ));
        rows.push(match &menu.starbase_name {
            Some(n) => format!("Star base: {n} (level {})", menu.starbase_level),
            None => "Star base: none".to_owned(),
        });
        if !menu.structures.is_empty() {
            rows.push("Structures:".to_owned());
            rows.extend(menu.structures.iter().take(8).map(|s| format!("  {s}")));
            if menu.structures.len() > 8 {
                rows.push(format!("  +{} more", menu.structures.len() - 8));
            }
        }
    }
    // Fog of war: fleets out of sight are not listed, fleets that are only `Seen` have an unknown size.
    let mut orbit: std::collections::BTreeMap<u32, (u32, bool)> = Default::default();
    for f in state.galaxy().fleets.values() {
        let sight = fog.fleet(f.id);
        if f.planet() == Some(p) && sight != galaxy_sim::Sight::Hidden {
            let e = orbit.entry(f.owner).or_default();
            e.0 += f.unit_count();
            e.1 |= sight != galaxy_sim::Sight::Detailed;
        }
    }
    if !orbit.is_empty() {
        rows.push("In orbit:".to_owned());
        rows.extend(orbit.iter().map(|(&o, &(n, unknown))| if unknown { format!("  {}: ? ships", catalog.faction(o)) } else { format!("  {}: {n} ships", catalog.faction(o)) }));
    }
    let forces = rows.join("\n");
    for (line, mut text) in &mut lines {
        match line {
            crate::ui::InfoLine::Forces if text.0 != forces => text.0 = forces.clone(),
            crate::ui::InfoLine::Slots => {
                if let Some(s) = &slots
                    && text.0 != *s
                {
                    text.0 = s.clone();
                }
            }
            _ => {}
        }
    }
}

/// A new game starts looking at the player's richest planet (the capital) instead of the whole galaxy.
#[allow(clippy::too_many_arguments)]
fn focus_start(
    args: Res<crate::cli::Args>,
    view: Res<crate::galaxy_view::GalaxyView>,
    bounds: Res<crate::camera::CameraBounds>,
    state: Res<SimState>,
    mut cam: Single<&mut crate::camera::GalaxyCamera>,
    mut selection: ResMut<crate::galaxy_map::Selection>,
    mut done: Local<usize>,
    mut frames: Local<u32>,
) {
    let game = std::sync::Arc::as_ptr(&state.world) as usize;
    if *done == game || state.current.loading {
        return;
    }
    if args.scripted() && (args.select.is_some() || args.zoom.is_some()) {
        *done = game;
        return;
    }
    *frames += 1;
    if *frames < 10 || bounds.dirty {
        return;
    }
    *done = game;
    *frames = 0;
    let capital = view.planets.iter().enumerate().filter(|(_, p)| p.owner.is_some() && p.owner == view.player_faction).max_by_key(|(_, p)| p.income);
    if let Some((i, p)) = capital {
        // The build pads and the planet panel are the first thing a new player needs.
        selection.0 = Some(i);
        let w = crate::galaxy_map::galaxy_to_world(p.position);
        cam.target_focus = Vec3::new(w.x, 0.0, w.z);
        cam.focus = cam.target_focus;
        let d = bounds.min_distance + (bounds.max_distance - bounds.min_distance) * 0.25;
        cam.target_distance = d;
        cam.distance = d;
    }
}

/// Ships on each side of an auto-resolved battle, noted when the player decides.
struct AutoBattle {
    id: u32,
    planet: galaxy_sim::PlanetId,
    /// Per faction: id, ships before, ids of its fleets in the battle.
    sides: Vec<(galaxy_sim::FactionId, u32, Vec<galaxy_sim::FleetId>)>,
}

/// Auto-resolve has no tactical result: the sides are counted from the galaxy fleets before the decision, the losses come from
/// `BattleResolved`, and ships that fled are the units of the battle's fleets that now leave the planet.
fn auto_resolve_summary(
    mut actions: MessageReader<PlayAction>,
    mut events: MessageReader<galaxy_play::PlayEvent>,
    state: Res<SimState>,
    player: Res<PlayerFaction>,
    mut noted: Local<Vec<AutoBattle>>,
    mut commands: Commands,
) {
    for a in actions.read() {
        let PlayAction::ResolveBattle { id, choice: BattleChoice::AutoResolve } = a else { continue };
        let Some(b) = state.current.human_battles.iter().find(|b| b.id == *id) else { continue };
        let galaxy = state.galaxy();
        let sides = b
            .factions
            .iter()
            .map(|&f| {
                let fleets: Vec<_> = b.fleets.iter().copied().filter(|fid| galaxy.fleets.get(fid).is_some_and(|fl| fl.owner == f)).collect();
                let ships = fleets.iter().filter_map(|fid| galaxy.fleets.get(fid)).map(|fl| fl.unit_count()).sum();
                (f, ships, fleets)
            })
            .collect();
        noted.retain(|n| n.id != *id);
        noted.push(AutoBattle { id: *id, planet: b.planet, sides });
    }
    for e in events.read() {
        let galaxy_play::PlayEvent::Sim(galaxy_sim::Event::BattleResolved { id, winner, losses, .. }) = e else { continue };
        let Some(pos) = noted.iter().position(|n| n.id == *id) else { continue };
        let n = noted.remove(pos);
        let galaxy = state.galaxy();
        let planet = state.world.planets[n.planet as usize].display_name.trim_end_matches("_WW").replace('_', " ");
        let mut lines = Vec::new();
        for (f, total, fleets) in &n.sides {
            let name = &state.world.factions[*f as usize].name;
            let lost = losses.iter().find(|(lf, _)| lf == f).map_or(0, |&(_, l)| l).min(*total);
            let fled: u32 = fleets
                .iter()
                .filter_map(|fid| galaxy.fleets.get(fid))
                .filter(|fl| fl.location != galaxy_sim::FleetLocation::AtPlanet(n.planet))
                .map(|fl| fl.unit_count())
                .sum();
            let mut line = if *total == 0 {
                format!("{name}: planetary defenses")
            } else {
                format!("{name}: {} of {} units survived (lost {})", total - lost, total, lost)
            };
            if fled > 0 {
                line.push_str(&format!(", {fled} escaped"));
            }
            lines.push(line);
        }
        let title = match winner {
            Some(w) if Some(*w) == player.0 => format!("Victory at {planet}"),
            Some(w) => format!("Defeat at {planet} - {} wins", state.world.factions[*w as usize].name),
            None => format!("Battle at {planet} ends in a draw"),
        };
        commands.insert_resource(crate::battle_play::BattleSummary { title, text: format!("{}\nAuto-resolved", lines.join("\n")), screen: None });
    }
}

/// The battle result dialog (EaW shows the losses of the fight when it ends).
fn show_battle_summary(world: &mut World) {
    if world.get_resource::<EawUiAssets>().is_none() || world.contains_resource::<PendingTactical>() {
        world.remove_resource::<crate::battle_play::BattleSummary>();
        return;
    }
    let Some(s) = world.remove_resource::<crate::battle_play::BattleSummary>() else { return };
    if s.screen.is_some() {
        if !spawn_result_screen(world, &s) {
            world.insert_resource(s);
        }
        return;
    }
    let faction = {
        let state = world.resource::<SimState>();
        ui_faction(state, world.resource::<PlayerFaction>())
    };
    let spec = DialogSpec { title: s.title.clone(), text: s.text.clone(), portrait: None, faction, buttons: vec!["OK".into()] };
    if spawn_dialog(world, &spec).is_none() {
        world.insert_resource(s);
    } else {
        info!("play: battle result dialog shown: {} | {}", s.title, s.text.replace('\n', " | "));
    }
}

/// Button of the full-screen battle result: closes it.
#[derive(Component)]
struct ResultReturn(Entity);

/// The full-screen battle result of a played battle (EaW: planet name, "YOU HAVE WON THE BATTLE!", battle time, two columns of losses with unit
/// icon, name and count, RETURN TO GALACTIC). Returns false when the interface assets are not ready yet.
fn spawn_result_screen(world: &mut World, s: &crate::battle_play::BattleSummary) -> bool {
    let Some(screen) = s.screen.as_ref() else { return false };
    let Some(assets) = world.get_resource::<EawUiAssets>() else { return false };
    let (game, texts) = (assets.game.clone(), assets.text.clone());
    let camera = world.get_resource::<EawUiSettings>().and_then(|s| s.camera);
    let display = |object: &str| -> String {
        game.as_deref()
            .and_then(|g| g.objects.resolve(object))
            .and_then(|r| r.string("text_id"))
            .and_then(|k| texts.get(k.split_whitespace().next().unwrap_or(&k)).filter(|t| !t.is_empty()).map(str::to_string))
            .unwrap_or_else(|| object.replace('_', " "))
    };
    let headline = match screen.won {
        Some(true) => "YOU HAVE WON THE BATTLE!",
        Some(false) => "YOU HAVE LOST THE BATTLE!",
        None => "THE BATTLE ENDED IN A DRAW.",
    };
    let secs = screen.seconds.max(0.0) as u32;
    let time = format!("BATTLE TIME: {:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60);
    let mut rows: [Vec<(Option<Handle<Image>>, String, u32)>; 2] = [Vec::new(), Vec::new()];
    if let Some(mut textures) = world.remove_resource::<eaw_ui::textures::UiTextures>() {
        if let Some(mut images) = world.get_resource_mut::<Assets<Image>>() {
            for (i, list) in [&screen.yours, &screen.enemy].into_iter().enumerate() {
                for (name, n) in list.iter() {
                    let icon = game.as_deref().and_then(|g| eaw_ui::unit_icon(g, name)).and_then(|i| textures.get(&mut images, &i)).map(|t| t.handle);
                    rows[i].push((icon, display(name), *n));
                }
            }
        }
        world.insert_resource(textures);
    }
    for r in &mut rows {
        r.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.1.cmp(&b.1)));
    }
    let gold = Color::srgb(1.0, 0.85, 0.35);
    let root = world
        .spawn((
            Name::new("Battle result"),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                padding: UiRect::all(Val::Px(30.0)),
                row_gap: Val::Px(8.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.01, 0.03, 0.09, 0.97)),
            GlobalZIndex(600),
        ))
        .id();
    if let Some(c) = camera {
        world.entity_mut(root).insert(UiTargetCamera(c));
    }
    world.spawn((text_bundle(&screen.planet, 44.0, Color::WHITE), ChildOf(root)));
    world.spawn((text_bundle(headline, 30.0, gold), ChildOf(root)));
    world.spawn((text_bundle(&time, 20.0, Color::srgb(0.8, 0.9, 1.0)), ChildOf(root)));
    let columns = world
        .spawn((
            Node {
                width: Val::Percent(85.0),
                flex_grow: 1.0,
                flex_shrink: 1.0,
                min_height: Val::Px(0.0),
                column_gap: Val::Px(40.0),
                margin: UiRect::vertical(Val::Px(12.0)),
                overflow: Overflow::clip(),
                ..default()
            },
            ChildOf(root),
        ))
        .id();
    for (i, title) in ["YOUR LOSSES", "ENEMY LOSSES"].into_iter().enumerate() {
        let col = world
            .spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    flex_basis: Val::Percent(50.0),
                    flex_grow: 1.0,
                    flex_shrink: 1.0,
                    min_height: Val::Px(0.0),
                    row_gap: Val::Px(4.0),
                    padding: UiRect::all(Val::Px(12.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    overflow: Overflow::clip(),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.05, 0.1, 0.22, 0.8)),
                BorderColor::all(Color::srgb(0.42, 0.58, 0.88)),
                ChildOf(columns),
            ))
            .id();
        world.spawn((text_bundle(title, 24.0, gold), Node { flex_shrink: 0.0, margin: UiRect::bottom(Val::Px(6.0)), ..default() }, ChildOf(col)));
        if rows[i].is_empty() {
            world.spawn((text_bundle("NONE", 18.0, Color::srgb(0.7, 0.8, 0.9)), ChildOf(col)));
        }
        let scroll_box = world
            .spawn((
                ResultScroll(i),
                ScrollPosition::default(),
                Node {
                    flex_direction: FlexDirection::Column,
                    flex_grow: 1.0,
                    flex_shrink: 1.0,
                    min_height: Val::Px(0.0),
                    overflow: Overflow::scroll_y(),
                    ..default()
                },
                ChildOf(col),
            ))
            .id();
        // Rows sit in a wrapping grid so that a long list still fits.
        let grid = world.spawn((Node { flex_wrap: FlexWrap::Wrap, column_gap: Val::Px(24.0), row_gap: Val::Px(6.0), ..default() }, ChildOf(scroll_box))).id();
        for (icon, name, n) in rows[i].iter() {
            let row = world.spawn((Node { width: Val::Px(250.0), align_items: AlignItems::Center, column_gap: Val::Px(8.0), ..default() }, ChildOf(grid))).id();
            match icon {
                Some(h) => {
                    world.spawn((ImageNode::new(h.clone()), Node { width: Val::Px(36.0), height: Val::Px(28.0), flex_shrink: 0.0, ..default() }, ChildOf(row)));
                }
                None => {
                    world.spawn((Node { width: Val::Px(36.0), height: Val::Px(28.0), flex_shrink: 0.0, ..default() }, ChildOf(row)));
                }
            }
            world.spawn((text_bundle(&format!("{name}"), 16.0, Color::WHITE), Node { flex_grow: 1.0, ..default() }, ChildOf(row)));
            world.spawn((text_bundle(&format!("x{n}"), 16.0, gold), ChildOf(row)));
        }
    }
    let button = world
        .spawn((
            Button,
            ResultReturn(root),
            Node {
                padding: UiRect::axes(Val::Px(30.0), Val::Px(10.0)),
                border: UiRect::all(Val::Px(2.0)),
                margin: UiRect::vertical(Val::Px(12.0)),
                flex_shrink: 0.0,
                ..default()
            },
            BackgroundColor(Color::srgb(0.01, 0.01, 0.03)),
            BorderColor::all(Color::srgb(0.42, 0.58, 0.88)),
            ChildOf(root),
        ))
        .id();
    world.spawn((text_bundle("RETURN TO GALACTIC", 22.0, Color::WHITE), ChildOf(button)));
    info!("play: battle result screen shown: {} | {headline} | {time} | lost {} / {} unit types", screen.planet, screen.yours.len(), screen.enemy.len());
    true
}

#[derive(Component)]
struct ResultScroll(usize);

fn result_scroll(
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    mut boxes: Query<(&ResultScroll, &mut ScrollPosition, &ComputedNode, &UiGlobalTransform)>,
) {
    let delta: f32 = wheel.read().map(|w| if matches!(w.unit, bevy::input::mouse::MouseScrollUnit::Line) { w.y * 36.0 } else { w.y }).sum();
    if delta == 0.0 {
        return;
    }
    let cursor = window.and_then(|w| w.physical_cursor_position());
    let hovered = cursor.and_then(|c| boxes.iter().find(|(_, _, n, g)| n.contains_point(**g, c)).map(|(id, ..)| id.0));
    for (id, mut pos, node, _) in &mut boxes {
        if hovered.is_none() || hovered == Some(id.0) {
            let inv = node.inverse_scale_factor();
            let max = ((node.content_size().y - node.size().y) * inv).max(0.0);
            pos.y = (pos.y - delta).clamp(0.0, max);
        }
    }
}

fn result_return(mut q: Query<(&ResultReturn, &Interaction, &mut BackgroundColor), Changed<Interaction>>, mut commands: Commands) {
    for (r, i, mut bg) in &mut q {
        match i {
            Interaction::Pressed => commands.entity(r.0).despawn(),
            Interaction::Hovered => bg.0 = Color::srgb(0.1, 0.13, 0.24),
            Interaction::None => bg.0 = Color::srgb(0.01, 0.01, 0.03),
        }
    }
}

/// The "fire superweapon" button and its confirmation dialog.
#[derive(Component)]
struct SuperweaponButton;
/// Text of the button.
#[derive(Component)]
struct SuperweaponLabel;

/// Pending confirmation: the dialog entity and the planet it is about.
#[derive(Resource, Default)]
struct SuperweaponAsk(Option<(Entity, galaxy_sim::PlanetId)>);

/// Planet the selected fleets can fire at right now (armed fleet in orbit, planet destroyable, weapon ready).
fn superweapon_target(state: &SimState, player: &PlayerFaction, selection: &PlayerSelection) -> Option<(galaxy_sim::PlanetId, Result<(), galaxy_sim::DestroyRefusal>)> {
    let me = player.0?;
    let g = state.galaxy();
    for id in &selection.fleets {
        let Some(p) = g.fleets.get(id).filter(|f| f.owner == me).and_then(|f| f.planet()) else { continue };
        match state.world.can_destroy_planet(g, me, *id, p) {
            Ok(()) => return Some((p, Ok(()))),
            Err(e @ galaxy_sim::DestroyRefusal::Recharging) => return Some((p, Err(e))),
            Err(_) => {}
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn superweapon_button(
    mut commands: Commands,
    state: Res<SimState>,
    player: Res<PlayerFaction>,
    selection: Res<PlayerSelection>,
    existing: Query<Entity, With<SuperweaponButton>>,
    mut label: Query<&mut Text, With<SuperweaponLabel>>,
    interaction: Query<(&Interaction, &SuperweaponButton), Changed<Interaction>>,
    ask: Res<SuperweaponAsk>,
    assets: Option<Res<EawUiAssets>>,
    mut dialog: Local<Option<(galaxy_sim::PlanetId, bool)>>,
    mut auto_frames: Local<u32>,
    args: Res<crate::cli::Args>,
) {
    let target = superweapon_target(&state, &player, &selection);
    *auto_frames += u32::from(target.is_some());
    match (target, existing.iter().next()) {
        (Some((p, res)), None) => {
            let name = state.world.planets[p as usize].display_name.clone();
            let text = if res.is_ok() { format!("Fire superweapon at {name}") } else { "Superweapon recharging".to_owned() };
            commands
                .spawn((
                    SuperweaponButton,
                    Button,
                    PlayHud,
                    GalaxyEntity,
                    Node {
                        position_type: PositionType::Absolute,
                        bottom: Val::Px(390.0),
                        left: Val::Px(40.0),
                        padding: UiRect::axes(Val::Px(14.0), Val::Px(8.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.35, 0.05, 0.04, 0.95)),
                    BorderColor::all(Color::srgb(1.0, 0.45, 0.3)),
                ))
                .with_children(|b| {
                    b.spawn((text_bundle(&text, 18.0, Color::WHITE), SuperweaponLabel));
                });
            *dialog = Some((p, res.is_ok()));
        }
        (Some((p, res)), Some(_)) => {
            let name = state.world.planets[p as usize].display_name.clone();
            let text = if res.is_ok() { format!("Fire superweapon at {name}") } else { "Superweapon recharging".to_owned() };
            for mut t in &mut label {
                if t.0 != text {
                    t.0 = text.clone();
                }
            }
            *dialog = Some((p, res.is_ok()));
        }
        (None, Some(e)) => {
            commands.entity(e).despawn();
            *dialog = None;
        }
        (None, None) => {}
    }
    let scripted_press = args.scripted() && args.fire.is_some() && *auto_frames == 60;
    if (interaction.iter().any(|(i, _)| *i == Interaction::Pressed) || scripted_press)
        && let Some((p, true)) = *dialog
        && ask.0.is_none()
        && assets.is_some()
    {
        let faction = ui_faction(&state, &player);
        let name = state.world.planets[p as usize].display_name.clone();
        commands.queue(move |world: &mut World| {
            let spec = DialogSpec {
                title: "Superweapon".into(),
                text: format!("Destroy {name}?\nThe planet and every ship of other factions in orbit will be lost. This cannot be undone."),
                portrait: None,
                faction,
                buttons: vec!["Fire".into(), "Cancel".into()],
            };
            if let Some(e) = spawn_dialog(world, &spec) {
                world.resource_mut::<SuperweaponAsk>().0 = Some((e, p));
            }
        });
    }
}

fn superweapon_dialog(
    mut results: MessageReader<EawDialogResult>,
    mut ask: ResMut<SuperweaponAsk>,
    mut actions: MessageWriter<PlayAction>,
    args: Res<crate::cli::Args>,
    mut waited: Local<u32>,
    mut commands: Commands,
) {
    // Offscreen `--fire confirm`: press "Fire" in the dialog after a short wait, like a click.
    if args.scripted() && args.fire.as_deref() == Some("confirm") && let Some((e, p)) = ask.0 {
        *waited += 1;
        if *waited == 90 {
            info!("script: clicking `Fire` in the superweapon dialog");
            actions.write(PlayAction::DestroyPlanet(p));
            // The simulation reports the destruction with its next step: let time run.
            actions.write(PlayAction::SetSpeedLevel(1));
            commands.entity(e).despawn();
            ask.0 = None;
        }
    }
    for r in results.read() {
        let Some((e, p)) = ask.0 else { continue };
        if r.dialog != e {
            continue;
        }
        ask.0 = None;
        if r.button == 0 {
            actions.write(PlayAction::DestroyPlanet(p));
        }
    }
}

/// The planet explosion effect of the galactic map plays at a planet that was just destroyed.
fn explode_on_destroy(
    mut commands: Commands,
    mut events: MessageReader<galaxy_play::PlayEvent>,
    layout: Res<galaxy_play::GalaxyLayout>,
    alamo: Option<Res<alamo_render::AlamoFs>>,
) {
    for e in events.read() {
        let galaxy_play::PlayEvent::Sim(galaxy_sim::Event::PlanetDestroyed { planet, .. }) = e else { continue };
        info!("play: planet {planet} destroyed");
        if alamo.is_some() {
            let pos = layout.positions[*planet as usize];
            let scale = (layout.radii[*planet as usize] * 0.12).max(0.2);
            commands.spawn((alamo_render::AlamoParticles::new("P_Planet_Explode_Galactic.alo"), Transform::from_translation(pos).with_scale(Vec3::splat(scale)), GalaxyEntity, ExplosionLife(8.0)));
        }
    }
}

/// Seconds a planet explosion stays before it is removed.
#[derive(Component)]
struct ExplosionLife(f32);

fn expire_explosions(mut commands: Commands, time: Res<Time>, mut q: Query<(Entity, &mut ExplosionLife)>) {
    for (e, mut life) in &mut q {
        life.0 -= time.delta_secs();
        if life.0 <= 0.0 {
            commands.entity(e).despawn();
        }
    }
}

/// The EaW command bar carries credits and planet counts; the generic top bar goes away, and the
/// speed read-out makes room for the battle banner.
fn declutter_top(
    bar: Query<(), With<EawCommandBar>>,
    mut hidden: Query<&mut Visibility, Or<(With<crate::ui::TopBar>, With<SpeedText>, With<crate::ui::HintText>)>>,
) {
    // The EaW bar carries credits, month, speed buttons and the planet; the generic read-outs and the key hint go away.
    if !bar.is_empty() {
        for mut v in &mut hidden {
            v.set_if_neq(Visibility::Hidden);
        }
    }
}

/// The player's heroes, top right (the original's hero frame with one ring per living hero).
#[derive(Component)]
struct HeroBar;

fn hero_bar(world: &mut World, mut shown: Local<Option<Vec<String>>>) {
    let screen = world.resource::<State<AppScreen>>().get().to_owned();
    let heroes = world.resource::<PlayUiModel>().heroes.clone();
    let faction = {
        let state = world.resource::<SimState>();
        ui_faction(state, world.resource::<PlayerFaction>())
    };
    let existing: Vec<Entity> = world.query_filtered::<Entity, With<HeroBar>>().iter(world).collect();
    if screen != AppScreen::Galaxy || world.get_resource::<EawUiAssets>().is_none() {
        return;
    }
    if shown.as_ref() == Some(&heroes) && !existing.is_empty() {
        return;
    }
    if shown.as_ref() == Some(&heroes) && heroes.is_empty() {
        return;
    }
    for e in existing {
        world.entity_mut(e).despawn();
    }
    let slots = heroes.len().min(11);
    debug!("play: hero bar {heroes:?}");
    let side = if faction == UiFaction::Rebel { "good" } else { "evil" };
    let game = world.get_resource::<EawUiAssets>().and_then(|a| a.game.clone());
    {
        let mut data = world.resource_mut::<eaw_ui::PanelData>();
        for k in 0..11 {
            data.slots.remove(&format!("hero_slot_{side}{k:02}"));
        }
        for (k, name) in heroes.iter().take(slots).enumerate() {
            let icon = game.as_ref().and_then(|g| eaw_ui::unit_icon(g, name)).unwrap_or_default();
            data.set_slot(&format!("hero_slot_{side}{k:02}"), eaw_ui::PanelSlot { icon, ..default() });
            data.bars.insert(format!("hero_hb_{side}{k:02}"), 1.0);
        }
    }
    if slots == 0 {
        *shown = Some(heroes);
        return;
    }
    let model = if faction == UiFaction::Rebel { "I_Main_Hero_Frame_Rebel.ALO" } else { "I_Main_Hero_Frame_Empire.ALO" };
    let mut spec = eaw_ui::ShellSpec::new(model, faction);
    spec.art_alt = Some(slots - 1);
    spec.anchor = eaw_ui::ShellAnchor::TopRight;
    spec.offset = Vec2::new(0.0, -2.0);
    if let Some(root) = eaw_ui::spawn_shell(world, &spec) {
        world.entity_mut(root).insert((HeroBar, GlobalZIndex(8)));
        *shown = Some(heroes);
    }
}

/// Income change per week of a unit while it exists (negative = upkeep).
fn pad_upkeep(state: &SimState, target: BuildTarget) -> f32 {
    match target {
        BuildTarget::Unit(u) => state.world.units[u as usize].upkeep,
        _ => 0.0,
    }
}

/// Starbase level of an upgrade pad.
fn pad_level(state: &SimState, target: BuildTarget) -> u8 {
    match target {
        BuildTarget::Starbase(s) => state.world.starbases[s as usize].level.clamp(0, 5) as u8,
        _ => 0,
    }
}


/// Star base level named by a missing requirement ("star base level 2"), 0 if it names something else.
fn required_level(a: &galaxy_play::Availability) -> u8 {
    match a {
        galaxy_play::Availability::Requires(r) => r
            .to_ascii_lowercase()
            .strip_prefix("star base level ")
            .and_then(|n| n.trim().parse::<u8>().ok())
            .unwrap_or(0),
        _ => 0,
    }
}

/// A press on a greyed-out build pad: say why in the toast (the refusal sound is played by `ui_sounds`).
fn locked_clicks(
    remap: Res<eaw_ui::PadRemap>,
    mut clicks: MessageReader<eaw_ui::EawLockedClicked>,
    model: Res<PlayUiModel>,
    bar: Res<BarState>,
    mut toast: ResMut<Toast>,
) {
    for c in clicks.read() {
        let Some(index) = c.name.strip_prefix("b_create_").and_then(|n| n.parse::<usize>().ok()) else { continue };
        let (Some(target), Some(menu)) = (bar.pads.get(remap.entry(index)).copied().flatten(), model.planet.as_ref()) else { continue };
        let Some(o) = menu.options.iter().find(|o| o.target == target) else { continue };
        toast.0 = format!("{}: {}", o.name, capitalize(&o.availability.reason()));
        toast.1 = 3.5;
    }
}

/// Minimap colours of the original: own green, the main enemy factions (Rebel, Empire, Underworld) at war red, everything else
/// (minor factions such as pirates or warlords, neutral, allied) orange. The minor factions are formally at war with everyone,
/// so "hostile" alone painted the whole map red.
fn relation_color(state: &SimState, me: Option<u32>, owner: Option<u32>) -> Color {
    let main = |o: u32| ["Rebel", "Empire", "Underworld"].iter().any(|n| state.world.faction_id(n) == Some(o));
    match (me, owner) {
        (Some(m), Some(o)) if m == o => Color::srgb(0.2, 0.9, 0.25),
        (Some(m), Some(o)) if main(o) && state.galaxy().hostile(&state.world, m, o) => Color::srgb(0.95, 0.15, 0.12),
        _ => Color::srgb(0.95, 0.55, 0.12),
    }
}


/// The UI is designed for 1366x768 and scales with the screen height, but never wider than the screen
/// allows: on 16:10 or 4:3 screens the width decides, so the bar and its build pads stay on screen.
pub(crate) fn design_scale(window: Option<Vec2>, offscreen: Option<&OffscreenTarget>) -> f32 {
    let size = match (window, offscreen) {
        (Some(s), _) => s,
        (None, Some(t)) => {
            let (w, h) = crate::offscreen::size();
            Vec2::new(w as f32, h as f32) / t.scale_factor
        }
        _ => Vec2::new(1366.0, 768.0),
    };
    // The bar's frame ends at design x=1360: scale by the width so it runs flush to the right edge (up to 1.2 % taller than the height fit).
    (size.x / 1360.0).min(size.y / 768.0 * 1.012)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planet_tooltip_lists_owner_income_and_slots() {
        let view = crate::galaxy_view::GalaxyView::demo();
        let text = planet_text(0, &view.planets[0], galaxy_sim::Sight::Detailed, &view);
        assert!(text.starts_with(&view.planets[0].display_name));
        assert!(text.contains("@Owner	"));
        assert!(text.contains("@Income	"));
        assert!(text.contains("@Space slots	"));
    }


    #[test]
    fn every_entry_of_a_real_space_menu_belongs_to_exactly_one_class_filter() {
        use alamo_fs::{GameFs, GameInstall};
        use galaxy_play::{BuildCategory, BuildMenu, BuildTarget, PlayCatalog};
        let Some(install) = GameInstall::detect() else { return };
        if !install.mod_data("1770851727").is_dir() {
            eprintln!("skipping: Remake workshop mod not found");
            return;
        }
        let data = alamo_data::load_galaxy(&GameFs::for_mod(&install, Some("1770851727")).unwrap()).0;
        let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
        let mut sim = galaxy_sim::Sim::new(&data, campaign, galaxy_sim::SimConfig { victory_enabled: false, ..Default::default() }).unwrap();
        let w = sim.world().clone();
        let names = PlayCatalog::from_data(&w, &data);
        let empire = w.faction_id("Empire").unwrap();
        let p = w.planet_id("Coruscant_WW").unwrap();
        sim.set_planet_owner(p, None);
        sim.set_planet_owner(p, Some(empire));
        let mut sb = w.first_starbase[empire as usize];
        for _ in 1..3 {
            sb = sb.and_then(|b| w.starbases[b as usize].next);
        }
        sim.set_starbase(p, sb);
        sim.add_credits(empire, 1.0e9);
        sim.set_tech_level(empire, 9);
        for n in ["E_Shipyard_1", "E_Shipyard_2", "E_Shipyard_3"] {
            assert!(sim.add_structure(p, w.structure_id(n).unwrap()));
        }
        let menu = BuildMenu::for_planet(&w, sim.galaxy(), &names, empire, p).expect("menu");
        let all = [BarClass::Corvette, BarClass::Frigate, BarClass::Capital, BarClass::Station];
        let mut per_class = [0usize; 4];
        let (mut heroes, mut starbase, mut tech) = (0, 0, 0);
        for o in menu.options.iter().filter(|o| crate::play::on_space_tab(&w, o.target)) {
            let hero = is_hero(&w, o.target);
            let hits: Vec<_> = all.iter().enumerate().filter(|(_, c)| c.matches(o.category, hero)).collect();
            if hero {
                heroes += 1;
                assert!(BarClass::Station.matches(o.category, hero), "hero {} must match Station filter", o.name);
                assert!(!hits.is_empty() && hits.len() <= 2, "hero {} should match Station and at most its ship class", o.name);
            } else {
                assert_eq!(hits.len(), 1, "{} ({:?}, hero {hero}) must belong to exactly one filter", o.name, o.category);
            }
            for (idx, _) in hits {
                per_class[idx] += 1;
            }
            starbase += usize::from(matches!(o.target, BuildTarget::Starbase(_)));
            tech += usize::from(o.category == BuildCategory::Tech);
        }
        assert!(starbase > 0 || tech > 0 || heroes > 0);
        assert!(heroes >= 5, "expected multiple heroes (including 30k Hoytt and 40k Zaarin): got {heroes}");
        assert!(per_class.iter().all(|&n| n > 0), "the test planet should list entries of every filter: {per_class:?}");
        assert_eq!(BarClass::of(BuildCategory::Starbase, false), BarClass::Station);
        assert_eq!(BarClass::of(BuildCategory::Tech, false), BarClass::Station);
        assert_eq!(BarClass::of(BuildCategory::Structures, false), BarClass::Station);
    }
    #[test]
    fn capitalize_first_letter() {
        assert_eq!(capitalize("requires tech"), "Requires tech");
        assert_eq!(capitalize(""), "");
    }

    #[test]
    fn pause_banner_tracks_pause_state() {
        let clock = GameClock { level: 0, resume_level: 1, speeds: vec![1.0], override_multiplier: None, hold: false, pending: 0.0 };
        let model = PlayUiModel { speed: galaxy_play::SpeedModel { rate: 0.0, ..Default::default() }, ..Default::default() };
        let is_paused = clock.paused() || model.speed.rate <= 0.0;
        assert!(is_paused, "banner should be active when paused");

        let clock = GameClock { level: 1, resume_level: 1, speeds: vec![1.0], override_multiplier: None, hold: false, pending: 0.0 };
        let model = PlayUiModel { speed: galaxy_play::SpeedModel { rate: 1.0, ..Default::default() }, ..Default::default() };
        let is_paused = clock.paused() || model.speed.rate <= 0.0;
        assert!(!is_paused, "banner should be hidden when running");
    }
}

/// The selected planet's description panel, upper left of the HUD, in the original's cyan glass. In the planet view it
/// sits on the right without a frame, text straight on the dark background.
#[derive(Component)]
struct PlanetPanel;
/// Its title / income / body texts.
#[derive(Component, Clone, Copy, PartialEq)]
enum PlanetPanelText {
    Title,
    Income,
    Body,
    Population,
}
/// Owner emblem of the panel header.
#[derive(Component)]
struct PanelEmblem;
/// A slot pip of the panel: `row` 0 ground (trooper), 1 space (tower), `index` within the row.
#[derive(Component)]
struct PanelPip {
    row: usize,
    index: usize,
}
fn spawn_planet_panel(mut commands: Commands, existing: Query<(), With<PlanetPanel>>, mut images: ResMut<Assets<Image>>) {
    if !existing.is_empty() {
        return;
    }
    let font = |size: f32| TextFont { font: FontSource::Family("Bahnschrift".into()), font_size: FontSize::Px(size), ..default() };
    let pips = [images.add(crate::galaxy_map::pip_image(false)), images.add(crate::galaxy_map::pip_image(true))];
    commands
        .spawn((
            PlanetPanel,
            PlayHud,
            GalaxyEntity,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(20.0),
                top: Val::Percent(13.5),
                width: Val::Px(420.0),
                min_height: Val::Px(150.0),
                max_height: Val::Percent(46.0),
                padding: UiRect::all(Val::Px(8.0)),
                border: UiRect::all(Val::Px(1.5)),
                flex_direction: FlexDirection::Column,
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(Color::srgba(0.13, 0.27, 0.31, 0.88)),
            BorderColor::all(Color::srgb(0.0, 0.76, 0.99)),
            GlobalZIndex(12),
            Visibility::Hidden,
        ))
        .with_children(|p| {
            p.spawn(Node { align_items: AlignItems::Center, column_gap: Val::Px(8.0), margin: UiRect::bottom(Val::Px(8.0)), ..default() }).with_children(|r| {
                r.spawn((Text::new(""), font(11.0), TextColor(Color::srgb(0.3, 1.0, 0.4)), PlanetPanelText::Population));
                r.spawn((ImageNode::default().with_color(Color::NONE), Node { width: Val::Px(18.0), height: Val::Px(18.0), ..default() }, PanelEmblem));
                r.spawn((Text::new(""), font(13.0), TextColor(Color::srgb(0.9, 0.95, 1.0)), bevy::text::LetterSpacing::Rem(0.12), Node { flex_grow: 1.0, justify_content: JustifyContent::Center, ..default() }, TextLayout::justify(Justify::Center), PlanetPanelText::Title));
                r.spawn((Text::new(""), font(12.0), TextColor(Color::srgb(0.3, 1.0, 0.4)), PlanetPanelText::Income));
            });
            p.spawn((Text::new(""), font(11.0), TextColor(Color::srgb(0.93, 0.96, 0.98)), PlanetPanelText::Body));
            for (row, pip) in pips.iter().enumerate().rev() {
                p.spawn(Node { column_gap: Val::Px(4.0), margin: UiRect::top(Val::Px(8.0)), ..default() }).with_children(|r| {
                    for index in 0..8 {
                        r.spawn((
                            ImageNode::new(pip.clone()).with_color(Color::srgb(0.75, 0.9, 1.0)),
                            Node { width: Val::Px(14.0), height: Val::Px(14.0), display: Display::None, ..default() },
                            PanelPip { row, index },
                        ));
                    }
                });
            }
        });
}

#[allow(clippy::too_many_arguments)]
fn update_planet_panel(
    hover: Res<crate::galaxy_map::Hover>,
    (fleet_hover, selection, sim, bridge): (Option<Res<galaxy_play::PlayHover>>, Option<Res<PlayerSelection>>, Option<Res<SimState>>, Option<Res<PlayBridge>>),
    screen: Res<State<AppScreen>>,
    pv: Res<crate::planet_view::PlanetViewState>,
    view: Res<crate::galaxy_view::GalaxyView>,
    textures: Option<ResMut<eaw_ui::textures::UiTextures>>,
    mut images: ResMut<Assets<Image>>,
    mut panel: Single<&mut Visibility, With<PlanetPanel>>,
    mut texts: Query<(&PlanetPanelText, &mut Text)>,
    mut emblem: Single<&mut ImageNode, With<PanelEmblem>>,
    mut pips: Query<(&PanelPip, &mut Node)>,
) {
    // In the planet view the viewed planet counts even when only a fleet is selected.
    // Outside the planet view: the planet under a hovered fleet, else the hovered planet, else the planet of the
    // selected fleet (the panel never keeps showing a stale hover).
    let fleet_planet = |id: galaxy_sim::FleetId| -> Option<usize> {
        let p = sim.as_ref()?.galaxy().fleets.get(&id)?.planet()?;
        bridge.as_ref()?.sim_to_planet.get(p as usize).copied().flatten()
    };
    let hovered_fleet_planet = fleet_hover.as_ref().and_then(|h| h.fleet).and_then(fleet_planet);
    let selected_fleet_planet = selection.as_ref().and_then(|s| s.fleets.iter().copied().find_map(fleet_planet));
    // The long description panel only belongs to the planet close-up (user); on the map the hover tooltip at R2 shows the short info.
    let _ = (hovered_fleet_planet, selected_fleet_planet, &hover);
    let planet = if pv.active { pv.planet } else { None }.and_then(|i| view.planets.get(i));
    if *screen.get() == AppScreen::SpaceBattle {
        panel.set_if_neq(Visibility::Hidden);
        return;
    }
    let Some(p) = planet.filter(|p| !p.info.is_empty()) else {
        panel.set_if_neq(Visibility::Hidden);
        return;
    };
    panel.set_if_neq(Visibility::Inherited);
    for (kind, mut t) in &mut texts {
        let s = match kind {
            PlanetPanelText::Title => p.display_name.to_uppercase(),
            PlanetPanelText::Income => format!("+{}", p.income),
            PlanetPanelText::Population => if p.population_bonus > 0 { p.population_bonus.to_string() } else { String::new() },
            PlanetPanelText::Body => {
                // Planet view: a gap under the header and a blank line before the long description, as in the Remake.
                let (facts, last) = p.info.split_at(p.info.len().saturating_sub(1));
                if pv.active { format!("\n{}\n\n{}", facts.join("\n"), last.join("")) } else { p.info.join("\n") }
            }
        };
        if t.0 != s {
            t.0 = s;
        }
    }
    let icon = p.owner.and_then(|o| view.factions.get(o)).and_then(|f| f.icon.clone());
    match (icon, textures) {
        (Some(name), Some(mut tex)) => {
            if let Some(t) = tex.get(&mut images, &name) {
                if emblem.image != t.handle {
                    emblem.image = t.handle;
                }
                emblem.color = Color::WHITE;
            }
        }
        _ => emblem.color = Color::NONE,
    }
    let counts = [p.land_slots.clamp(0, 8) as usize, p.max_space_base.clamp(0, 8) as usize];
    for (pip, mut node) in &mut pips {
        let want = if pip.index < counts[pip.row] { Display::Flex } else { Display::None };
        if node.display != want {
            node.display = want;
        }
    }
}

/// Description panel placement: left in the galaxy view; in the planet view right of the platform, frameless.
fn place_planet_panel(
    state: Res<crate::planet_view::PlanetViewState>,
    mut panel: Single<(&mut Node, &mut BackgroundColor, &mut BorderColor), With<PlanetPanel>>,
) {
    let in_view = state.active && state.blend > 0.5;
    let (node, bg, border) = &mut *panel;
    let want_left = if in_view { Val::Percent(63.75) } else { Val::Px(20.0) };
    if node.left != want_left {
        node.left = want_left;
        node.top = if in_view { Val::Percent(33.0) } else { Val::Percent(13.5) };
        node.width = if in_view { Val::Percent(23.25) } else { Val::Px(420.0) };
        node.max_height = if in_view { Val::Percent(52.0) } else { Val::Percent(46.0) };
        node.border = if in_view { UiRect::ZERO } else { UiRect::all(Val::Px(1.5)) };
        bg.0 = if in_view { Color::NONE } else { Color::srgba(0.13, 0.27, 0.31, 0.88) };
        **border = if in_view { BorderColor::all(Color::NONE) } else { BorderColor::all(Color::srgb(0.0, 0.76, 0.99)) };
    }
}

/// The planet view's orbit boxes: the original fleet organizer shell (`I_Galactic_Organize_New`: three framed boxes with
/// unit and hero icons and xN counts) across the top, filled with the forces in orbit of the planet. The description panel
/// ([`PlanetPanel`]) moves to the right side while the view is open.
#[derive(Component)]
struct OrbitBoxes;

#[allow(clippy::too_many_arguments)]
fn planet_view_ui(world: &mut World, mut last_key: Local<Option<(usize, u64)>>) {
    let (active, blend, planet) = {
        let s = world.resource::<crate::planet_view::PlanetViewState>();
        (s.active, s.blend, s.planet)
    };
    let existing: Vec<Entity> = world.query_filtered::<Entity, With<OrbitBoxes>>().iter(world).collect();
    if !active {
        for e in existing {
            world.entity_mut(e).despawn();
        }
        *last_key = None;
        return;
    }
    let (Some(view_index), true) = (planet, blend > 0.8) else { return };
    if world.get_resource::<EawUiAssets>().is_none() {
        return;
    }
    // The three boxes hold the player's own fleets at the planet (the fleet organizer, see `planet_fleets`); other forces are one line above them.
    let Some(sim_planet) = world.resource::<PlayBridge>().planet_to_sim.get(view_index).copied().flatten() else { return };
    let faction = {
        let state = world.resource::<SimState>();
        ui_faction(state, world.resource::<PlayerFaction>())
    };
    let me = world.resource::<PlayerFaction>().0;
    let org = {
        let state = world.resource::<SimState>();
        crate::planet_fleets::organizer(state, world.resource::<galaxy_play::Fog>(), me, sim_planet)
    };
    let key = (view_index, org.key);
    if *last_key == Some(key) && !existing.is_empty() {
        return;
    }
    for e in existing {
        world.entity_mut(e).despawn();
    }
    *last_key = Some(key);
    let game = world.resource::<EawUiAssets>().game.clone();
    let (slots, orbit_slots) = org.slots(&world.resource::<SimState>().world, game.as_deref());
    {
        let mut data = world.resource_mut::<eaw_ui::PanelData>();
        data.slots.retain(|k, _| !(k.starts_with("q1_") || k.starts_with("q2_") || k.starts_with("q3_")));
        for (name, slot) in slots {
            data.set_slot(&name, slot);
        }
    }
    world.insert_resource(crate::planet_view::OrbitSlots(orbit_slots));
    world.insert_resource(crate::planet_fleets::OtherForces(org.others));
    let mut spec = eaw_ui::ShellSpec::new("I_Galactic_Organize_New.ALO", faction);
    spec.anchor = eaw_ui::ShellAnchor::TopCenter;
    spec.offset = Vec2::new(0.0, -45.0);
    if let Some(root) = eaw_ui::spawn_shell(world, &spec) {
        world.entity_mut(root).insert((OrbitBoxes, GlobalZIndex(9)));
        // The frame texture (`i_galactic_organize.dds`) carries a faint blue wash (alpha 2 of 255) over its whole quad
        // outside and inside the rounded frame; through our compositing it shows as a dark band behind the three boxes
        // that the game does not show. Clear it, keeping the frame line and its glow.
        let handles: Vec<Handle<Image>> = world
            .get::<Children>(root)
            .map(|c| c.iter().collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|k| world.get::<Name>(k).is_some_and(|n| n.as_str().ends_with("_frame")).then(|| world.get::<ImageNode>(k).map(|i| i.image.clone())).flatten())
            .collect();
        let mut images = world.resource_mut::<Assets<Image>>();
        for h in handles {
            if let Some(mut img) = images.get_mut(&h)
                && let Some(data) = img.data.as_mut()
            {
                for px in data.chunks_exact_mut(4).filter(|p| p[3] <= 3) {
                    px[3] = 0;
                }
            }
        }
    }
}


/// The "Active Story Objectives" window (the holocron button of the bar): the campaign's story chapters read from the
/// data ([`crate::objectives`]) with the advisor transmission of the selected one underneath.
#[derive(Resource, Default)]
pub(crate) struct ObjectivesWindow {
    open: bool,
    completed_tab: bool,
    pub(crate) list: Option<Vec<crate::objectives::Objective>>,
    selected: usize,
    list_scroll: f32,
}
#[derive(Component)]
struct ObjectivesRoot;
#[derive(Component, Clone, Copy)]
enum ObjectivesClick {
    Active,
    Completed,
    Close,
    Row(usize),
}

fn objectives_toggle(mut clicks: MessageReader<EawButtonClicked>, mut win: ResMut<ObjectivesWindow>) {
    for c in clicks.read() {
        if c.name == "b_story_arc_g" {
            win.open = !win.open;
            win.completed_tab = false;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn objectives_window(
    mut commands: Commands,
    mut win: ResMut<ObjectivesWindow>,
    existing: Query<Entity, With<ObjectivesRoot>>,
    clicks: Query<(&Interaction, &ObjectivesClick), Changed<Interaction>>,
    keys: Res<ButtonInput<KeyCode>>,
    game: Res<crate::game::Game>,
    campaign: Res<crate::flow::CurrentCampaign>,
    mut last: Local<Option<(bool, bool, usize)>>,
) {
    for (i, c) in &clicks {
        if *i == Interaction::Pressed {
            match c {
                ObjectivesClick::Active => win.completed_tab = false,
                ObjectivesClick::Completed => win.completed_tab = true,
                ObjectivesClick::Close => win.open = false,
                ObjectivesClick::Row(n) => win.selected = *n,
            }
        }
    }
    if keys.just_pressed(KeyCode::Escape) && win.open {
        win.open = false;
    }
    if win.open && win.list.is_none() {
        let list = game
            .0
            .as_ref()
            .zip(campaign.name.as_deref().zip(campaign.faction.as_deref()))
            .map(|(g, (name, faction))| crate::objectives::load(&g.fs, &g.data, name, faction))
            .unwrap_or_default();
        win.selected = list.iter().position(|o| o.title == "Welcome Guide").unwrap_or(0);
        win.list = Some(list);
    }
    let state = (win.open, win.completed_tab, win.selected);
    if *last == Some(state) {
        return;
    }
    *last = Some(state);
    for e in &existing {
        commands.entity(e).despawn();
    }
    if !win.open {
        return;
    }
    let list: Vec<crate::objectives::Objective> = if win.completed_tab { Vec::new() } else { win.list.clone().unwrap_or_default() };
    let selected = win.selected.min(list.len().saturating_sub(1));
    let font = |size: f32| TextFont { font: FontSource::Family("Bahnschrift".into()), font_size: FontSize::Px(size), ..default() };
    let rgb = |c: [u8; 3]| Color::srgb_u8(c[0], c[1], c[2]);
    let blue = Color::srgb(0.32, 0.52, 0.93);
    let list_scroll = win.list_scroll;
    commands
        .spawn((
            ObjectivesRoot,
            Node { position_type: PositionType::Absolute, left: Val::Percent(17.5), top: Val::Percent(7.3), width: Val::Percent(65.5), height: Val::Percent(85.0), padding: UiRect::all(Val::Px(14.0)), border: UiRect::all(Val::Px(3.0)), flex_direction: FlexDirection::Column, row_gap: Val::Px(8.0), overflow: Overflow::clip(), ..default() },
            BackgroundColor(Color::srgba(0.02, 0.03, 0.08, 0.96)),
            BorderColor::all(Color::srgb(0.45, 0.6, 0.95)),
            BoxShadow::new(Color::srgba(0.3, 0.5, 1.0, 0.5), Val::Px(0.0), Val::Px(0.0), Val::Px(4.0), Val::Px(20.0)),
            GlobalZIndex(600),
        ))
        .with_children(|r| {
            r.spawn((Text::new("ACTIVE STORY OBJECTIVES"), font(22.0), TextColor(Color::WHITE), bevy::text::LetterSpacing::Rem(0.08)));
            r.spawn(Node { column_gap: Val::Px(8.0), margin: UiRect::horizontal(Val::Px(12.0)), ..default() }).with_children(|t| {
                for (label, click, on) in [("ACTIVE", ObjectivesClick::Active, !win.completed_tab), ("COMPLETED", ObjectivesClick::Completed, win.completed_tab)] {
                    t.spawn((Button, click, Node { width: Val::Percent(32.0), justify_content: JustifyContent::Center, padding: UiRect::vertical(Val::Px(3.0)), border: UiRect::top(Val::Px(2.0)), ..default() }, BackgroundColor(if on { Color::srgb(0.58, 0.72, 0.92) } else { Color::srgb(0.38, 0.41, 0.48) }), BorderColor::all(if on { Color::srgb(0.85, 0.93, 1.0) } else { Color::srgb(0.5, 0.53, 0.6) })))
                        .with_child((Text::new(label), font(14.0), TextColor(Color::srgb(0.02, 0.03, 0.06)), bevy::text::LetterSpacing::Rem(0.1)));
                }
            });
            // Objective list with its own scroll bar (wheel or thumb), like the transmission box below.
            r.spawn(Node { height: Val::Percent(22.0), flex_shrink: 0.0, margin: UiRect::horizontal(Val::Px(12.0)), column_gap: Val::Px(2.0), ..default() }).with_children(|row| {
                row.spawn((ObjectivesScroll(0), ScrollPosition(Vec2::new(0.0, list_scroll)), Node { flex_grow: 1.0, min_width: Val::Px(0.0), padding: UiRect::all(Val::Px(4.0)), flex_direction: FlexDirection::Column, overflow: Overflow::scroll_y(), ..default() }, BackgroundColor(Color::BLACK)))
                    .with_children(|l| {
                        for (i, o) in list.iter().enumerate() {
                            l.spawn((Button, ObjectivesClick::Row(i), Node { padding: UiRect::horizontal(Val::Px(6.0)), flex_shrink: 0.0, ..default() }, BackgroundColor(if i == selected { Color::srgba(0.6, 0.6, 0.65, 0.9) } else { Color::NONE })))
                                .with_child((Text::new(o.title.clone()), font(17.0), TextColor(rgb(o.color))));
                        }
                    });
                scroll_bar(row, 0, &font);
            });
            r.spawn((Text::new("INCOMING TRANSMISSION FROM ADVISOR"), Node { margin: UiRect::left(Val::Px(4.0)), ..default() }, font(13.0), TextColor(blue), bevy::text::LetterSpacing::Rem(0.08)));
            // Transmission box with its scroll bar (arrows, track and thumb) on the right.
            r.spawn(Node { flex_grow: 1.0, min_height: Val::Px(0.0), margin: UiRect::horizontal(Val::Px(12.0)), column_gap: Val::Px(2.0), ..default() }).with_children(|row| {
                row.spawn((ObjectivesScroll(1), ScrollPosition::default(), Node { flex_grow: 1.0, min_width: Val::Px(0.0), padding: UiRect::all(Val::Px(10.0)), overflow: Overflow::scroll_y(), ..default() }, BackgroundColor(Color::BLACK)))
                    .with_children(|b| {
                        if let Some(o) = list.get(selected) {
                            let mut spans = o.text.iter();
                            if let Some(first) = spans.next() {
                                b.spawn((Text::new(first.text.clone()), font(14.0), TextColor(rgb(first.color)))).with_children(|t| {
                                    for s in spans {
                                        t.spawn((TextSpan::new(s.text.clone()), font(14.0), TextColor(rgb(s.color))));
                                    }
                                });
                            }
                        }
                    });
                scroll_bar(row, 1, &font);
            });
            r.spawn(Node { justify_content: JustifyContent::SpaceBetween, align_items: AlignItems::Center, ..default() }).with_children(|f| {
                f.spawn((Text::new("HINT: THIS MENU RECALLS MISSION GOALS."), font(14.0), TextColor(blue), bevy::text::LetterSpacing::Rem(0.06)));
                f.spawn((Button, ObjectivesClick::Close, Node { width: Val::Px(220.0), padding: UiRect::vertical(Val::Px(7.0)), justify_content: JustifyContent::Center, border: UiRect::all(Val::Px(1.0)), ..default() }, BackgroundColor(Color::BLACK), BorderColor::all(Color::srgb(0.6, 0.65, 0.75))))
                    .with_child((Text::new("CLOSE"), font(16.0), TextColor(Color::WHITE), bevy::text::LetterSpacing::Rem(0.1)));
            });
        });
}

/// Opening the planet view switches the command bar to the ground/structure tab (v0260); leaving it goes back to space.
fn planet_view_bar_tab(pv: Res<crate::planet_view::PlanetViewState>, mut bar: ResMut<BarState>, mut was: Local<bool>) {
    if pv.active != *was {
        *was = pv.active;
        bar.space_tab = !pv.active;
    }
}

/// A scrollable box of the objectives window (0 = objective list, 1 = advisor transmission).
#[derive(Component)]
pub(crate) struct ObjectivesScroll(pub(crate) u8);
/// Thumb of the scroll bar of the box with the same id.
#[derive(Component)]
struct ObjectivesThumb(u8);

/// Scroll bar (arrows, track, thumb) of the box `id`; [`objectives_scroll`] moves the thumb.
fn scroll_bar(row: &mut ChildSpawnerCommands, id: u8, font: &dyn Fn(f32) -> TextFont) {
    row.spawn((Node { width: Val::Px(26.0), flex_direction: FlexDirection::Column, align_items: AlignItems::Center, ..default() }, BackgroundColor(Color::srgb(0.01, 0.01, 0.03)), BorderColor::all(Color::srgb(0.25, 0.32, 0.5)))).with_children(|bar| {
        bar.spawn((Text::new("\u{25B2}"), font(12.0), TextColor(Color::WHITE)));
        bar.spawn(Node { flex_grow: 1.0, width: Val::Percent(100.0), min_height: Val::Px(0.0), ..default() }).with_children(|track| {
            track.spawn((ObjectivesThumb(id), Node { position_type: PositionType::Absolute, left: Val::Px(3.0), right: Val::Px(3.0), top: Val::Px(0.0), height: Val::Px(26.0), border: UiRect::all(Val::Px(1.0)), ..default() }, BackgroundColor(Color::srgb(0.2, 0.34, 0.6)), BorderColor::all(Color::srgb(0.7, 0.82, 1.0))));
        });
        bar.spawn((Text::new("\u{25BC}"), font(12.0), TextColor(Color::WHITE)));
    });
}

/// Mouse wheel scrolls the box under the cursor (the transmission when the cursor is elsewhere); the thumbs follow.
fn objectives_scroll(
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    mut win: ResMut<ObjectivesWindow>,
    mut boxes: Query<(&ObjectivesScroll, &mut ScrollPosition, &ComputedNode, &UiGlobalTransform)>,
    mut thumbs: Query<(&ObjectivesThumb, &mut Node, &ChildOf)>,
    nodes: Query<&ComputedNode, Without<ObjectivesThumb>>,
) {
    let delta: f32 = wheel.read().map(|w| if matches!(w.unit, bevy::input::mouse::MouseScrollUnit::Line) { w.y * 36.0 } else { w.y }).sum();
    let cursor = window.and_then(|w| w.physical_cursor_position());
    let hovered = cursor.and_then(|c| boxes.iter().find(|(_, _, n, g)| n.contains_point(**g, c)).map(|(id, ..)| id.0));
    let target = hovered.unwrap_or(1);
    for (id, mut pos, node, _) in &mut boxes {
        let inv = node.inverse_scale_factor();
        let max = ((node.content_size().y - node.size().y) * inv).max(0.0);
        if delta != 0.0 && id.0 == target {
            pos.y = (pos.y - delta).clamp(0.0, max);
        }
        if id.0 == 0 {
            win.list_scroll = pos.y;
        }
        for (t_id, mut t, parent) in &mut thumbs {
            if t_id.0 != id.0 {
                continue;
            }
            let Ok(track) = nodes.get(parent.parent()) else { continue };
            let track_h = track.size().y * track.inverse_scale_factor();
            let view = node.size().y * inv;
            let total = (node.content_size().y * inv).max(1.0);
            let thumb = (track_h * (view / total)).clamp(22.0, track_h.max(22.0));
            let top = if max > 0.0 { (track_h - thumb) * (pos.y / max) } else { 0.0 };
            t.height = Val::Px(thumb);
            t.top = Val::Px(top);
        }
    }
}

/// `--tooltip <button>`: shows that button's tooltip as if hovered (offscreen verification).
fn scripted_tooltip(args: Res<crate::cli::Args>, mut forced: ResMut<eaw_ui::ForcedTooltip>) {
    if let Some(t) = args.tooltip.as_deref()
        && forced.0.as_deref() != Some(t)
    {
        forced.0 = Some(t.to_lowercase());
    }
}

/// Whether a build target is a named hero unit.
fn is_hero(world: &galaxy_sim::World, target: BuildTarget) -> bool {
    matches!(target, BuildTarget::Unit(u) if world.units[u as usize].hero)
}

/// Build pads per page: one per `b_create_NN` slot of the shell. `SP_PAD_PAGE=<n>` shrinks it (offscreen checks of the page arrows).
fn pad_page_size() -> usize {
    std::env::var("SP_PAD_PAGE").ok().and_then(|v| v.parse::<usize>().ok()).map_or(MAX_PADS, |n| n.clamp(1, MAX_PADS))
}

/// A page arrow above the right end of the build pads (`-1` back, `1` forward). The shell has `b_scroll_left` / `b_scroll_right`
/// bones, but the mod ships no art for them and they sit below the bar, so the arrows are drawn here.
#[derive(Component, Clone, Copy)]
struct PadArrow(i32);

fn spawn_pad_arrows(mut commands: Commands, existing: Query<(), With<PadArrow>>) {
    if !existing.is_empty() {
        return;
    }
    for (step, label, left) in [(-1, "<", 1262.0), (1, ">", 1302.0)] {
        commands
            .spawn((
                PlayHud,
                GalaxyEntity,
                PadArrow(step),
                Button,
                Node { position_type: PositionType::Absolute, left: Val::Px(left), bottom: Val::Px(123.0), width: Val::Px(32.0), height: Val::Px(20.0), justify_content: JustifyContent::Center, align_items: AlignItems::Center, border: UiRect::all(Val::Px(1.0)), display: Display::None, ..default() },
                BackgroundColor(Color::srgba(0.05, 0.12, 0.2, 0.9)),
                BorderColor::all(Color::srgb(0.25, 0.5, 0.9)),
                GlobalZIndex(20),
            ))
            .with_child((Text::new(label), TextFont { font: FontSource::Family("Bahnschrift".into()), font_size: FontSize::Px(15.0), ..default() }, TextColor(Color::srgb(0.85, 0.93, 1.0))));
    }
}

/// Shows the arrows when the pads fill more than one page, dims the end ones and turns the page on a click.
fn pad_arrows(
    data: Res<CommandBarData>,
    mut bar: ResMut<BarState>,
    pressed: Query<(&PadArrow, &Interaction), Changed<Interaction>>,
    mut arrows: Query<(&PadArrow, &mut Node, &mut BorderColor, &Children)>,
    mut texts: Query<&mut TextColor>,
    mut sound: MessageWriter<crate::flow::UiSound>,
) {
    for (arrow, i) in &pressed {
        let usable = if arrow.0 < 0 { data.pad_page > 0 } else { data.pad_page + 1 < data.pad_pages };
        if *i == Interaction::Pressed && usable {
            bar.page = if arrow.0 < 0 { bar.page.saturating_sub(1) } else { bar.page + 1 };
            sound.write(crate::flow::UiSound("GUI_Unit_Scroll_SFX"));
        }
    }
    for (arrow, mut node, mut border, kids) in &mut arrows {
        let show = if data.pad_pages > 1 { Display::Flex } else { Display::None };
        if node.display != show {
            node.display = show;
        }
        let usable = if arrow.0 < 0 { data.pad_page > 0 } else { data.pad_page + 1 < data.pad_pages };
        *border = BorderColor::all(if usable { Color::srgb(0.25, 0.5, 0.9) } else { Color::srgb(0.18, 0.22, 0.3) });
        for &k in kids {
            if let Ok(mut c) = texts.get_mut(k) {
                c.0 = if usable { Color::srgb(0.85, 0.93, 1.0) } else { Color::srgb(0.35, 0.4, 0.5) };
            }
        }
    }
}
