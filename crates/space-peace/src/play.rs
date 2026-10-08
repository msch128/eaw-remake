//! Glue between the app and `galaxy-play`: starts the simulation for the campaign that is shown,
//! keeps map selection, owners, credits and planet radii in sync, forwards battles to the
//! `battle_seam` and runs the scripted offscreen scenarios (`--weeks`, `--build`, `--move`).

use std::time::Instant;

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use eaw_ui::{EawCommandBar, EawUiAssets};
use galaxy_play::{
    BuildMenu, BuildTarget, GalaxyLayout, GameClock, PlayAction, PlayCatalog, PlayConfig, PlayCursorBlocked, PlayEvent, PlayInit, PlayerFaction,
    PlayerSelection, SimState,
};
use galaxy_sim::{FleetLocation, PlanetId};

use crate::battle_seam::{BattleFinished, BattleOutcome, BattleRequest, OpenBattle};
use crate::cli::Args;
use crate::flow::{AppScreen, CurrentCampaign, Paused, StartCampaign};
use crate::galaxy_map::Selection;
use crate::galaxy_view::GalaxyView;
use crate::game::{DEMO_CAMPAIGN, Game};
use crate::planet_visual::PlanetRadii;

/// Maps between the app's galaxy view and the simulation; present while a game is running.
#[derive(Resource, Debug)]
pub struct PlayBridge {
    /// Campaign the simulation was built for.
    pub campaign: String,
    /// View planet index to simulation planet.
    pub planet_to_sim: Vec<Option<PlanetId>>,
    /// Simulation planet to view planet index.
    pub sim_to_planet: Vec<Option<usize>>,
    /// Simulation faction to view faction index.
    pub faction_to_view: Vec<Option<usize>>,
}

/// While set, the offscreen capture waits (a scripted scenario is still running).
#[derive(Resource, Default, Debug)]
pub struct ScriptHold {
    /// Script running.
    pub active: bool,
    /// When it finished.
    pub done_at: Option<Instant>,
    /// A tactical battle will open and the capture has to wait for it and the return.
    pub waiting_battle: bool,
}

impl ScriptHold {
    /// True once a finished script had time to show its result.
    pub fn settled(&self) -> bool {
        !self.active && !self.waiting_battle && self.done_at.is_none_or(|t| t.elapsed().as_secs_f32() > 1.5)
    }
}

/// Starts and syncs the playable galaxy.
pub struct PlayGluePlugin;

impl Plugin for PlayGluePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScriptHold>()
            .init_resource::<ScriptState>()
            .add_plugins(galaxy_play::GalaxyPlayPlugin::deferred())
            .add_systems(OnEnter(AppScreen::Galaxy), begin_play)
            .add_systems(OnTransition { exited: AppScreen::Galaxy, entered: AppScreen::MainMenu }, end_play)
            .add_systems(OnTransition { exited: AppScreen::Galaxy, entered: AppScreen::CampaignSelect }, end_play)
            .add_systems(
                Update,
                (
                    hold_clock,
                    select_granted.run_if(resource_exists::<PlayBridge>),
                    sync_destroyed.run_if(resource_exists::<PlayBridge>),
                    galaxy_audio.run_if(resource_exists::<alamo_audio::plugin::AlamoAudio>.and_then(resource_exists::<PlayerFaction>)),
                    via_menu,
                    scripted_slots,
                    (sync_selection, sync_view, sync_radii, cursor_blocked, forward_battles, finish_battles, run_script)
                        .chain()
                        .run_if(resource_exists::<PlayBridge>),
                )
                    .chain(),
            );
    }
}

/// Builds the simulation for the shown campaign (once per campaign) and starts it.
#[allow(clippy::too_many_arguments)]
fn begin_play(
    mut commands: Commands,
    game: Res<Game>,
    current: Res<CurrentCampaign>,
    view: Res<GalaxyView>,
    bridge: Option<Res<PlayBridge>>,
    args: Res<Args>,
    options: Res<crate::newgame::NewGameOptions>,
    pending_load: Option<Res<crate::newgame::PendingLoad>>,
) {
    let (Some(name), Some(assets)) = (current.name.clone(), game.0.as_ref()) else { return };
    if name == DEMO_CAMPAIGN || bridge.is_some_and(|b| b.campaign == name) {
        return;
    }
    let Some(campaign) = assets.data.campaign(&name) else { return };
    let config = PlayConfig {
        pick_planets: false,
        tactical_hook: true,
        sim: galaxy_sim::SimConfig { debug_commands: args.grant_unit.is_some(), ..default() },
        // Scripted scenarios start paused and drive time themselves.
        initial_speed_level: if args.weeks.is_some() { 0 } else { 1 },
        difficulty: Some(options.difficulty),
        classic_ai: options.classic_ai,
        ..default()
    };
    let data = assets.data.clone();
    let fs: std::sync::Arc<dyn alamo_fs::FileSource> = assets.fs.clone();
    // A save chosen in the load menu (or `--load-from`) is replayed; otherwise a new scripted game starts.
    let load = pending_load.map(|p| p.0.clone()).or_else(|| args.load_from.clone());
    let started = Instant::now();
    let built = match &load {
        Some(path) => PlayInit::from_save(path.clone(), data.clone(), fs, config),
        None => PlayInit::scripted(data.clone(), fs, campaign, config).map_err(|e| e.to_string()),
    };
    let init = match built {
        Ok(init) => init,
        Err(e) => {
            error!("cannot start the galactic simulation for `{name}`: {e}");
            return;
        }
    };
    info!("play: simulation for {name} built in {:.2?}", started.elapsed());
    if let Some(h) = init.sim.world().victory.active_player {
        let mine: Vec<&str> = view.planets.iter().filter(|p| p.owner.is_some() && p.owner == view.player_faction).map(|p| p.display_name.as_str()).collect();
        info!("play: human faction {} owns {}", init.sim.world().factions[h as usize].name, mine.join(", "));
    }
    let world = init.sim.world().clone();
    let planet_to_sim: Vec<Option<PlanetId>> = view.planets.iter().map(|p| world.planet_index.get(&p.id.to_ascii_lowercase()).copied()).collect();
    let mut sim_to_planet = vec![None; world.planets.len()];
    for (i, s) in planet_to_sim.iter().enumerate() {
        if let Some(s) = s {
            sim_to_planet[*s as usize] = Some(i);
        }
    }
    let faction_to_view: Vec<Option<usize>> = world
        .factions
        .iter()
        .map(|f| view.factions.iter().position(|v| v.name.eq_ignore_ascii_case(&f.name)).or_else(|| faction_by_object(&assets.data, &f.name, &view)))
        .collect();
    let missing = planet_to_sim.iter().filter(|p| p.is_none()).count();
    if missing > 0 {
        warn!("play: {missing} map planets have no simulation planet");
    }
    let bridge = PlayBridge { campaign: name, planet_to_sim, sim_to_planet, faction_to_view };
    let data = assets.data.clone();
    commands.remove_resource::<crate::newgame::PendingLoad>();
    commands.queue(move |world: &mut World| {
        galaxy_play::start_play(world, init);
        if let Some(mut ui) = world.get_resource_mut::<EawUiAssets>() {
            ui.game = Some(data);
        }
        world.insert_resource(bridge);
    });
}

/// Display name of a faction object (`EMPIRE`) as shown in the view (`Galactic Empire`).
fn faction_by_object(data: &alamo_data::GameData, object: &str, view: &GalaxyView) -> Option<usize> {
    let text = data.faction(object).and_then(|f| f.text_id.clone()).and_then(|k| data.text(&k).map(str::to_owned))?;
    view.factions.iter().position(|v| v.name.eq_ignore_ascii_case(&text))
}

/// Stops the simulation when the map is left for the menus.
#[allow(clippy::type_complexity)]
fn end_play(mut commands: Commands, bars: Query<Entity, Or<(With<EawCommandBar>, With<crate::play_ui::BarBacking>)>>) {
    for e in &bars {
        commands.entity(e).despawn();
    }
    commands.queue(|world: &mut World| {
        galaxy_play::stop_play(world);
        world.remove_resource::<PlayBridge>();
    });
}

/// Time stands still behind the pause menu and during tactical battles.
fn hold_clock(
    paused: Res<Paused>,
    screen: Res<State<AppScreen>>,
    clock: Option<ResMut<GameClock>>,
    mut hidden: ResMut<galaxy_play::PlayMapHidden>,
) {
    let off_map = *screen.get() != AppScreen::Galaxy;
    if hidden.0 != off_map {
        hidden.0 = off_map;
    }
    if let Some(mut clock) = clock {
        let hold = paused.0 || *screen.get() != AppScreen::Galaxy;
        if clock.hold != hold {
            clock.hold = hold;
        }
    }
}

/// The map's planet selection becomes the simulation's.
fn sync_selection(selection: Res<Selection>, bridge: Res<PlayBridge>, mut actions: MessageWriter<PlayAction>) {
    if selection.is_changed() {
        actions.write(PlayAction::SelectPlanet(selection.0.and_then(|i| bridge.planet_to_sim.get(i).copied().flatten())));
    }
}

/// Owners and credits from the latest snapshot into the view the map is drawn from.
fn sync_view(state: Option<Res<SimState>>, bridge: Res<PlayBridge>, mut view: ResMut<GalaxyView>, mut seen: Local<u64>) {
    let Some(state) = state else { return };
    if *seen == state.current.seq && *seen != 0 {
        return;
    }
    *seen = state.current.seq;
    let g = state.galaxy();
    let owner_of = |s: PlanetId| g.planets[s as usize].owner.and_then(|o| bridge.faction_to_view.get(o as usize).copied().flatten());
    let changed_owner = view.planets.iter().enumerate().any(|(i, p)| bridge.planet_to_sim[i].is_some_and(|s| p.owner != owner_of(s)));
    let credits: Vec<(usize, i64)> = bridge
        .faction_to_view
        .iter()
        .enumerate()
        .filter_map(|(s, v)| Some((v.as_ref().copied()?, g.factions.get(s)?.credits as i64)))
        .collect();
    let changed_credits = credits.iter().any(|&(v, c)| view.factions[v].credits != c);
    if !changed_owner && !changed_credits {
        return;
    }
    let view = &mut *view;
    for (i, p) in view.planets.iter_mut().enumerate() {
        if let Some(s) = bridge.planet_to_sim[i] {
            p.owner = owner_of(s);
        }
    }
    for (v, c) in credits {
        view.factions[v].credits = c;
    }
}

/// Fleet icons are sized and placed by the real planet radii once the models loaded.
fn sync_radii(radii: Res<PlanetRadii>, bridge: Res<PlayBridge>, layout: Option<ResMut<GalaxyLayout>>) {
    let Some(mut layout) = layout else { return };
    if !radii.is_changed() && !layout.is_added() {
        return;
    }
    for (s, v) in bridge.sim_to_planet.iter().enumerate() {
        if let Some(r) = v.and_then(|i| radii.0.get(i)) {
            layout.radii[s] = *r;
        }
    }
}

/// Mouse input of `galaxy-play` is off behind menus, over the command bar and outside the map.
#[allow(clippy::too_many_arguments)]
fn cursor_blocked(
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    bar: Query<(&ComputedNode, &UiGlobalTransform), With<EawCommandBar>>,
    screen: Res<State<AppScreen>>,
    paused: Res<Paused>,
    state: Option<Res<SimState>>,
    mut blocked: ResMut<PlayCursorBlocked>,
) {
    let over_bar = window
        .and_then(|w| w.physical_cursor_position())
        .is_some_and(|c| bar.iter().any(|(node, tf)| node.contains_point(*tf, c)));
    let loading = state.is_some_and(|s| s.current.loading);
    let value = over_bar || loading || paused.0 || *screen.get() != AppScreen::Galaxy;
    if blocked.0 != value {
        blocked.0 = value;
    }
}

/// "Fight (tactical)" opens the battle screen through the seam.
fn forward_battles(mut commands: Commands, mut events: MessageReader<PlayEvent>, state: Option<Res<SimState>>, mut open: MessageWriter<OpenBattle>) {
    let Some(state) = state else { return };
    for e in events.read() {
        if let PlayEvent::TacticalReady(req) = e {
            let planet_name = state.world.planets[req.planet as usize].display_name.clone();
            let sides = req.sides.iter().map(|s| s.faction_name.clone()).collect();
            commands.insert_resource(crate::battle_play::PendingTactical(req.clone()));
            open.write(OpenBattle(BattleRequest { id: req.battle_id, planet_name, sides }));
        }
    }
}

/// A battle left without a result is abandoned: the sim auto-resolves it. Real results are
/// applied by `battle_play`.
fn finish_battles(mut finished: MessageReader<BattleFinished>, mut actions: MessageWriter<PlayAction>) {
    for f in finished.read() {
        if f.outcome == BattleOutcome::Aborted {
            actions.write(PlayAction::CancelTactical(f.id));
        }
    }
}

/// Progress of the scripted offscreen scenario.
#[derive(Resource, Default, Debug)]
pub struct ScriptState {
    phase: u8,
    /// Galactic seconds sent so far.
    sent: f32,
    /// Seconds the scenario wants in total.
    goal: f32,
    frames: u32,
}

/// Runs `--select`, `--build`, `--move` and `--weeks` in order, then lets the capture proceed.
#[allow(clippy::too_many_arguments)]
fn run_script(
    args: Res<Args>,
    mut script: ResMut<ScriptState>,
    mut hold: ResMut<ScriptHold>,
    state: Option<ResMut<SimState>>,
    bridge: Res<PlayBridge>,
    view: Res<GalaxyView>,
    catalog: Option<Res<PlayCatalog>>,
    player: Option<Res<PlayerFaction>>,
    selection: Res<PlayerSelection>,
    mut map_selection: ResMut<Selection>,
    mut actions: MessageWriter<PlayAction>,
    screen: Res<State<AppScreen>>,
) {
    if args.offscreen.is_none() || script.phase >= 3 || *screen.get() != AppScreen::Galaxy {
        return;
    }
    let (Some(mut state), Some(catalog), Some(player)) = (state, catalog, player) else { return };
    // The worker still prepares the galaxy or replays a save: nothing to order yet.
    if state.current.loading || state.driver.load_progress().is_some() {
        hold.active = true;
        return;
    }
    let scripted = args.weeks.is_some() || args.build.is_some() || args.move_to.is_some() || args.grant_unit.is_some();
    if !scripted {
        if hold.active {
            hold.active = false;
            hold.done_at = Some(Instant::now());
        }
        if let Some(path) = &args.save_to {
            actions.write(PlayAction::Save(path.clone()));
        }
        script.phase = 3;
        return;
    }
    hold.active = true;
    script.frames += 1;
    let world = state.world.clone();
    let find = |name: &str| view.planets.iter().position(|p| p.id.eq_ignore_ascii_case(name) || p.display_name.eq_ignore_ascii_case(name));
    match script.phase {
        0 => {
            // Let the map settle (planet models, camera) for a few frames, then give the orders.
            if script.frames < 5 {
                return;
            }
            if let Some(i) = args.select.as_deref().and_then(find) {
                map_selection.0 = Some(i);
            }
            script.phase = 1;
            actions.write(PlayAction::SetSpeedLevel(0));
        }
        1 => {
            // The selection reaches the simulation through `sync_selection` one frame later.
            let Some(h) = player.0 else {
                script.goal = args.weeks.unwrap_or(0) as f32 * world.config.week_seconds;
                script.phase = 2;
                return;
            };
            let Some(planet) = selection.planet else {
                if script.frames > 30 {
                    warn!("script: no planet selected, skipping orders");
                    // `--weeks` alone must still fast-forward.
                    script.goal = args.weeks.unwrap_or(0) as f32 * world.config.week_seconds;
                    script.phase = 2;
                }
                return;
            };
            if let (Some(unit), Some(at)) = (args.grant_unit.as_deref(), args.at.as_deref().and_then(find).and_then(|i| bridge.planet_to_sim[i])) {
                info!("script: granting {unit} at {}", world.planets[at as usize].display_name);
                actions.write(PlayAction::DebugGrant { unit_name: unit.to_owned(), planet: at, count: 1 });
            }
            if let Some(text) = &args.build {
                let g = state.galaxy();
                let menu = BuildMenu::for_planet(&world, g, &catalog, h, planet);
                let pick = menu.and_then(|m| {
                    m.options
                        .into_iter()
                        .find(|o| o.availability.is_available() && o.name.to_ascii_lowercase().contains(&text.to_ascii_lowercase()))
                });
                match pick {
                    Some(o) => {
                        info!("script: building {} at {}", o.name, world.planets[planet as usize].display_name);
                        actions.write(PlayAction::Build { planet, target: o.target });
                    }
                    None => warn!("script: nothing buildable matching `{text}` at the selected planet"),
                }
            }
            if let Some(dest) = args.move_to.as_deref().and_then(find).and_then(|i| bridge.planet_to_sim[i]) {
                let g = state.galaxy();
                let fleets: Vec<_> = g
                    .fleets
                    .values()
                    .filter(|f| f.owner == h && matches!(f.location, FleetLocation::AtPlanet(p) if p == planet))
                    .map(|f| f.id)
                    .collect();
                info!("script: sending {} fleets to {}", fleets.len(), world.planets[dest as usize].display_name);
                actions.write(PlayAction::SelectFleets(fleets));
                actions.write(PlayAction::MoveSelected(dest));
            }
            script.goal = args.weeks.unwrap_or(0) as f32 * world.config.week_seconds;
            script.phase = 2;
            script.frames = 0;
        }
        _ => {
            // Orders were handled in the frame before; now advance time in week-sized steps.
            let battle = !state.current.human_battles.is_empty();
            if script.sent >= script.goal || battle {
                hold.waiting_battle = battle && args.battle_choice.as_deref() == Some("fight");
                let idle = state.driver.advances_in_flight(&state.current) == 0;
                if idle || script.frames > 6000 {
                    script.phase = 3;
                    hold.active = false;
                    hold.done_at = Some(Instant::now());
                    if let Some(path) = &args.save_to {
                        actions.write(PlayAction::Save(path.clone()));
                    }
                    info!("script: done at week {} (credits {:.0})", state.current.week, player.0.map_or(0.0, |h| state.galaxy().factions[h as usize].credits));
                }
                return;
            }
            if script.frames < 3 {
                return;
            }
            if state.driver.advances_in_flight(&state.current) < 3 {
                let step = world.config.week_seconds.min(script.goal - script.sent);
                state.driver.advance(step);
                script.sent += step;
            }
        }
    }
}

/// Which build entries a pad list is made of: unit-like entries (ships, star base, tech,
/// research) or structures.
pub fn is_structure(menu_target: BuildTarget) -> bool {
    matches!(menu_target, BuildTarget::Structure(_))
}

/// Offscreen `--via-menu`: walks main menu -> campaign list -> campaign like a player (the clicks
/// themselves are the `StartCampaign` message the campaign buttons send).
fn via_menu(args: Res<Args>, screen: Res<State<AppScreen>>, mut next: ResMut<NextState<AppScreen>>, mut start: MessageWriter<StartCampaign>, mut sent: Local<u32>) {
    if !args.via_menu || args.offscreen.is_none() {
        return;
    }
    *sent += 1;
    match screen.get() {
        AppScreen::MainMenu if *sent > 20 => next.set(AppScreen::CampaignSelect),
        AppScreen::CampaignSelect if *sent > 40 => {
            start.write(StartCampaign(args.campaign.clone().unwrap_or_else(|| "GC_Full_Web_Way_Rebel".into())));
            *sent = 0;
        }
        _ => {}
    }
}

/// Advisor and unit-report speech for the player's events (silent without the audio plugin).
fn galaxy_audio(
    mut events: MessageReader<PlayEvent>,
    state: Option<Res<SimState>>,
    player: Res<PlayerFaction>,
    current: Res<CurrentCampaign>,
    mut out: MessageWriter<alamo_audio::plugin::GalaxyAudio>,
) {
    use alamo_audio::GalaxyEvent as G;
    use galaxy_sim::{Event, ProductionKind};
    let (Some(state), Some(me)) = (state, player.0) else { return };
    let faction = current.faction.clone().unwrap_or_else(|| "Rebel".to_owned());
    let mut say = |event: G| {
        out.write(alamo_audio::plugin::GalaxyAudio { faction: faction.clone(), event });
    };
    for e in events.read() {
        let PlayEvent::Sim(ev) = e else { continue };
        match ev {
            Event::ConstructionComplete { faction, kind, .. } if *faction == me => say(match kind {
                ProductionKind::Tech(_) => G::TechLevelGained,
                ProductionKind::Unit(u) if matches!(state.world.units[*u as usize].role, galaxy_sim::UnitRole::Research(_)) => G::ResearchComplete,
                ProductionKind::Unit(_) => G::UnitBuilt,
                _ => G::StructureBuilt,
            }),
            Event::PlanetCaptured { old_owner, new_owner, .. } if *new_owner == me => {
                let _ = old_owner;
                say(G::PlanetCaptured);
            }
            Event::PlanetCaptured { old_owner, .. } if *old_owner == Some(me) => say(G::PlanetLost),
            Event::FleetArrived { fleet, .. } => {
                let mine_parked = state.galaxy().fleets.get(fleet).is_some_and(|f| f.owner == me && f.planet().is_some());
                if mine_parked {
                    say(G::FleetArrived);
                }
            }
            Event::BattlePending(b) if b.factions.contains(&me) => say(G::BattlePending),
            Event::BattleResolved { losses, .. } if losses.iter().any(|(f, _)| *f == me) => say(G::BattleEnded),
            Event::TechAdvanced { faction, .. } if *faction == me => say(G::TechLevelGained),
            Event::FactionEliminated { faction } if *faction == me => say(G::Defeated),
            _ => {}
        }
    }
}

/// Offscreen `--slots save|load [--slot-click N]`: opens the slot list on the map and presses a slot.
fn scripted_slots(
    args: Res<Args>,
    screen: Res<State<AppScreen>>,
    state: Option<Res<SimState>>,
    mut slots: ResMut<crate::newgame::SlotScreen>,
    mut chosen: MessageWriter<crate::newgame::SlotChosen>,
    mut frames: Local<u32>,
) {
    use crate::newgame::{SlotChosen, SlotMode};
    let (Some(mode), true) = (args.slots.as_deref(), args.offscreen.is_some()) else { return };
    if *screen.get() != AppScreen::Galaxy || state.is_none_or(|s| s.current.loading) {
        return;
    }
    *frames += 1;
    let mode = if mode == "load" { SlotMode::Load } else { SlotMode::Save };
    if *frames == 30 {
        slots.0 = Some(mode);
    }
    if *frames == 120
        && let Some(n) = args.slot_click
    {
        chosen.write(SlotChosen(mode, n));
    }
    let _ = SlotChosen(mode, 0);
}

/// Destroyed planets of the simulation (and `--force-wreck` in offscreen tests) become wrecks on the map.
fn sync_destroyed(
    state: Option<Res<SimState>>,
    bridge: Option<Res<PlayBridge>>,
    args: Res<Args>,
    view: Res<GalaxyView>,
    mut destroyed: ResMut<crate::galaxy_map::DestroyedPlanets>,
    mut out: MessageWriter<PlayEvent>,
    mut frames: Local<u32>,
) {
    let mut list: Vec<usize> = Vec::new();
    if let (Some(state), Some(bridge)) = (state, bridge.as_ref()) {
        for (s, v) in bridge.sim_to_planet.iter().enumerate() {
            if let Some(v) = v
                && state.galaxy().planets[s].destroyed
            {
                list.push(*v);
            }
        }
    }
    if let Some(name) = args.force_wreck.as_deref()
        && let Some(i) = view.planets.iter().position(|p| p.id.eq_ignore_ascii_case(name) || p.display_name.eq_ignore_ascii_case(name))
    {
        list.push(i);
    }
    // `--force-wreck` also plays the explosion once, as the simulation event would.
    if args.force_wreck.is_some() && args.offscreen.is_some() {
        *frames += 1;
        if *frames == 150
            && let (Some(b), Some(i)) = (bridge.as_ref(), list.last())
            && let Some(Some(s)) = b.planet_to_sim.get(*i)
        {
            out.write(PlayEvent::Sim(galaxy_sim::Event::PlanetDestroyed { planet: *s, by: 0, owner: None, ships_lost: 0 }));
        }
    }
    if destroyed.0 != list {
        destroyed.0 = list;
    }
}

/// Offscreen `--grant-unit`: once the granted fleet exists it is selected together with its planet.
fn select_granted(
    args: Res<Args>,
    state: Option<Res<SimState>>,
    player: Res<PlayerFaction>,
    selection: Res<PlayerSelection>,
    mut actions: MessageWriter<PlayAction>,
) {
    let (Some(unit), Some(state), Some(me)) = (args.grant_unit.as_deref(), state, player.0) else { return };
    if !selection.fleets.is_empty() || state.current.loading {
        return;
    }
    let Some(uid) = state.world.unit_id(unit) else { return };
    let found = state.galaxy().fleets.values().find(|f| f.owner == me && f.units.get(&uid).is_some_and(|n| *n > 0) && f.planet().is_some());
    if let Some(f) = found {
        actions.write(PlayAction::SelectPlanet(f.planet()));
        actions.write(PlayAction::SelectFleets(vec![f.id]));
    }
}
