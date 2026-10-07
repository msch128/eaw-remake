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

use std::sync::Mutex;

use bevy::prelude::*;

pub use build_menu::{Availability, BuildCategory, BuildMenu, BuildOption, BuildTarget, QueueEntry, build_command};
pub use catalog::PlayCatalog;
pub use driver::{CommandResult, ResolveMode, SimDriver, Snapshot, Ticket};
pub use geometry::{FleetPlace, GalaxyLayout, RoutePlan, galaxy_to_world};
pub use input::{PlayHover, PlayUiBlock};
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
/// 1. Build a [`PlayInit`] from the already loaded `GameData` and campaign.
/// 2. `app.add_plugins((GalaxyPlayPlugin::new(init), GalaxyPlayUiPlugin))` after `AlamoRenderPlugin`.
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
}

impl Plugin for GalaxyPlayPlugin {
    fn build(&self, app: &mut App) {
        let PlayInit { sim, catalog, config } = self.init.lock().expect("init lock").take().expect("GalaxyPlayPlugin added twice");
        let human = if config.human_is_ai { None } else { sim.world().victory.active_player };
        let world = sim.world().clone();
        let mut driver = SimDriver::spawn(sim, human);
        let current = driver.wait_initial();

        if !app.world().contains_resource::<GalaxyLayout>() {
            app.insert_resource(GalaxyLayout::from_world(&world));
        }
        app.insert_resource(GameClock::new(&config))
            .insert_resource(PlayerFaction(human))
            .insert_resource(SimState { driver, current, previous: None, received_at: 0.0, world, awaiting_external: Default::default() })
            .insert_resource(catalog)
            .insert_resource(config)
            .init_resource::<PlayerSelection>()
            .init_resource::<Notifications>()
            .init_resource::<PlayUiModel>()
            .init_resource::<PlayHover>()
            .init_resource::<input::DragState>()
            .init_resource::<visuals::FleetEntities>()
            .add_message::<PlayAction>()
            .add_message::<PlayEvent>()
            .add_systems(Startup, (visuals::setup_assets, input::spawn_box_rect))
            .configure_sets(Update, (PlaySet::Logic, PlaySet::Visuals, PlaySet::Input, PlaySet::Ui).chain())
            .add_systems(
                Update,
                (
                    (logic::pump_snapshots, logic::hotkeys, logic::handle_actions, logic::drive_clock, logic::prune_selection).chain().in_set(PlaySet::Logic),
                    (visuals::sync_fleets, visuals::fit_fleet_models, visuals::draw_fleet_markers, visuals::update_fleet_labels).chain().in_set(PlaySet::Visuals),
                    (input::update_hover, input::handle_mouse, input::draw_drag).chain().in_set(PlaySet::Input),
                    logic::update_ui_model.in_set(PlaySet::Ui),
                ),
            );
    }
}
