//! Makes the galactic conquest map playable: runs `galaxy-sim` on a worker thread, shows fleets
//! on the map, handles selection and orders, planet build menus, notifications and battle
//! prompts.
//!
//! ```ignore
//! // Host app (see the crate README in `lib.rs` docs of `GalaxyPlayPlugin`):
//! let init = PlayInit::from_data(&data, campaign, PlayConfig::default())?;
//! app.add_plugins((GalaxyPlayPlugin::new(init), GalaxyPlayUiPlugin));
//! ```
//!
//! The crate is layered so each part can be replaced:
//!
//! * [`driver`]: threaded simulation driver (no Bevy, fully testable);
//! * [`geometry`], [`build_menu`], [`notify`], [`catalog`]: pure data models;
//! * [`state`]: resources, the [`PlayAction`] / [`PlayEvent`] messages and [`PlayUiModel`];
//! * [`GalaxyPlayPlugin`]: logic, fleets on the map, mouse input;
//! * [`GalaxyPlayUiPlugin`]: a simple UI that only reads [`PlayUiModel`] and writes [`PlayAction`].

pub mod build_menu;
pub mod catalog;
pub mod driver;
pub mod geometry;
pub mod input;
mod logic;
pub mod notify;
pub mod state;
pub mod ui;
pub mod visuals;

use std::sync::{Arc, Mutex};

use bevy::prelude::*;

pub use build_menu::{Availability, BuildCategory, BuildMenu, BuildOption, BuildTarget, QueueEntry, build_command};
pub use catalog::{FleetModelStyle, PlayCatalog};
pub use driver::{TacticalRuntime, CommandResult, LoadProgressView, LoadResult, ResolveMode, SaveResult, ScriptSetup, SightView, SimDriver, Snapshot, Ticket};
pub use geometry::{FleetPlace, GalaxyLayout, RoutePlan, galaxy_to_world};
pub use input::{PlayCursorBlocked, PlayHover, PlayUiBlock};

/// Set by the host while the map is not shown (tactical battle screen): fleet markers and labels
/// stop drawing.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct PlayMapHidden(pub bool);
pub use notify::{Notification, NotificationKind, Notifications};
pub use state::{
    BattleChoice, BattleDialog, BattleSide, FleetRow, GameClock, PlayAction, PlayConfig, PlayEvent, PlayInit, PlayUiModel, PlayerFaction,
    PlayerSelection, SimState, SpeedModel, TopBarModel,
};
pub use ui::GalaxyPlayUiPlugin;
pub use visuals::{FleetEntities, FleetVisual};

/// Systems of the plugin run in this order every frame.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlaySet {
    /// Receive snapshots, apply actions, feed the clock.
    Logic,
    /// Fleet entities and markers.
    Visuals,
    /// Mouse picking and orders.
    Input,
    /// Refresh [`PlayUiModel`].
    Ui,
}

/// The playable galactic game.
///
/// Integration into the `space-peace` app:
///
/// 1. Add `GalaxyPlayPlugin::deferred()` once (after `AlamoRenderPlugin`); nothing runs until a
///    game is started.
/// 2. When a campaign starts, build a [`PlayInit`] from the loaded `GameData` and call
///    [`start_play`] (or [`GalaxyPlayPlugin::new`] for a fixed game). [`stop_play`] ends it.
/// 3. Keep the app's own planet picking: set `PlayConfig::pick_planets = false` and forward its
///    selection with `PlayAction::SelectPlanet(Some(sim_planet_id))` (look the id up with
///    `GalaxyLayout::index` by planet object name).
/// 4. Copy the planet radii into `GalaxyLayout::radii` (`set_radius_by_name`) once the models loaded.
/// 5. Read [`PlayEvent`] messages, e.g. `OpenTacticalBattle` to open the space battle view.
pub struct GalaxyPlayPlugin {
    init: Mutex<Option<PlayInit>>,
}

impl GalaxyPlayPlugin {
    /// Plugin for a prepared simulation.
    pub fn new(init: PlayInit) -> Self {
        Self { init: Mutex::new(Some(init)) }
    }

    /// Plugin without a game; start one later with [`start_play`].
    pub fn deferred() -> Self {
        Self { init: Mutex::new(None) }
    }
}

/// Starts (or restarts) a game: spawns the worker thread and inserts the per-game resources.
pub fn start_play(world: &mut World, init: PlayInit) {
    stop_play(world);
    let PlayInit { sim, catalog, config, script } = init;
    let sim_world = sim.world().clone();
    let human = match &script {
        // A scripted game (or a save) names its human faction in the script config; "none" = spectator.
        Some(s) => match s.config.human.as_deref() {
            Some(n) if n.eq_ignore_ascii_case("none") => None,
            Some(n) => sim_world.faction_index.get(&n.to_ascii_lowercase()).copied(),
            None => sim_world.victory.active_player,
        },
        None if config.human_is_ai => None,
        None => sim_world.victory.active_player,
    };
    let (driver, current) = match script {
        Some(setup) => {
            // The worker thread sets up scripts, AI (and replays a save): show the starting state, flagged `loading`, until it is done.
            let placeholder = Arc::new(Snapshot::placeholder(sim.galaxy().clone()));
            (SimDriver::spawn_scripted(sim, setup, human), placeholder)
        }
        None => {
            let mut driver = SimDriver::spawn(sim, human);
            let current = driver.wait_initial();
            (driver, current)
        }
    };
    world.insert_resource(GalaxyLayout::from_world(&sim_world));
    world.insert_resource(GameClock::new(&config));
    world.insert_resource(PlayerFaction(human));
    world.insert_resource(SimState { driver, current, previous: None, received_at: 0.0, world: sim_world, awaiting_external: Default::default() });
    world.insert_resource(catalog);
    world.insert_resource(config);
    world.insert_resource(PlayerSelection::default());
    world.insert_resource(Notifications::default());
    world.insert_resource(PlayUiModel::default());
}

/// Ends the running game: stops the worker and removes the fleets from the map.
pub fn stop_play(world: &mut World) {
    if world.remove_resource::<SimState>().is_none() {
        return;
    }
    let fleets: Vec<Entity> = world.resource_mut::<FleetEntities>().0.drain().map(|(_, e)| e).collect();
    let labels: Vec<Entity> = world.query_filtered::<Entity, With<visuals::FleetLabel>>().iter(world).collect();
    for e in fleets.into_iter().chain(labels) {
        if let Ok(e) = world.get_entity_mut(e) {
            e.despawn();
        }
    }
}

impl Plugin for GalaxyPlayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlayConfig>()
            .init_resource::<PlayerSelection>()
            .init_resource::<Notifications>()
            .init_resource::<PlayUiModel>()
            .init_resource::<PlayHover>()
            .init_resource::<PlayCursorBlocked>()
            .init_resource::<PlayMapHidden>()
            .init_resource::<input::DragState>()
            .init_resource::<visuals::FleetEntities>()
            .add_message::<PlayAction>()
            .add_message::<PlayEvent>()
            .init_gizmo_group::<visuals::RouteGizmos>()
            .init_gizmo_group::<visuals::RouteGlowGizmos>()
            .add_systems(Startup, (visuals::setup_assets, input::spawn_box_rect, visuals::configure_route_gizmos))
            .configure_sets(Update, (PlaySet::Logic, PlaySet::Visuals, PlaySet::Input, PlaySet::Ui).chain())
            .configure_sets(Update, (PlaySet::Logic, PlaySet::Visuals, PlaySet::Input, PlaySet::Ui).run_if(resource_exists::<SimState>))
            .add_systems(
                Update,
                (
                    (logic::pump_snapshots, logic::hotkeys, logic::handle_actions, logic::drive_clock, logic::prune_selection).chain().in_set(PlaySet::Logic),
                    (
                        (visuals::sync_fleets, visuals::fit_fleet_models).chain(),
                        (visuals::draw_fleet_markers, visuals::draw_routes, visuals::update_fleet_labels).run_if(|hidden: Res<PlayMapHidden>| !hidden.0),
                    )
                        .chain()
                        .in_set(PlaySet::Visuals),
                    (input::update_hover, input::handle_mouse, input::draw_drag).chain().in_set(PlaySet::Input),
                    logic::update_ui_model.in_set(PlaySet::Ui),
                ),
            );
        if let Some(init) = self.init.lock().expect("init lock").take() {
            start_play(app.world_mut(), init);
        }
    }
}
