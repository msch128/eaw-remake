//! Integration seam for tactical space battles started from the galaxy map.
//!
//! Nothing here depends on the unfinished `galaxy-play`, `galaxy-sim` or `battle-view` APIs; it
//! only fixes the screen, the messages and the placeholder screen so those crates can be plugged
//! in later without touching the rest of the flow.
//!
//! # Plan
//!
//! 1. **Pending battle.** `galaxy-sim` reports `Event::BattlePending(PendingBattle { id, planet,
//!    fleets, factions, defender })`. `galaxy-play` shows its battle dialog; with
//!    `PlayConfig::tactical_hook` on and the player choosing "Fight (tactical)" it emits
//!    `PlayEvent::OpenTacticalBattle(PendingBattle)`. A host system maps that event to
//!    [`OpenBattle`] (planet name from the [`GalaxyView`](crate::galaxy_view::GalaxyView), sides
//!    from the factions).
//! 2. **Entering the battle.** [`open_battle`] stores the [`BattleRequest`] and switches to
//!    [`AppScreen::SpaceBattle`]. The galaxy scene is kept (hidden), not despawned, so the map,
//!    selection and the running `galaxy-play` state survive; the galaxy clock stays paused.
//!    The host then builds the `battle_sim::BattleSetup` from the fleets (one `SideSetup` per
//!    faction, units from the fleet contents, planetary defences as `structures`, `ai: true` for
//!    all sides but the player) using the `Catalog` the simulation already holds
//!    (`Sim::set_tactical_catalog`), a `VisualCatalog` and the `SpaceScene` read from the planet's
//!    `Space_Tactical_Map` (`.ted`), and inserts `BattleSession::new(setup, visuals, scene)`.
//!    `battle-view` spawns its scene, camera and worker thread; the galaxy camera is deactivated
//!    while it runs. Music switches to `MusicScene::Space(faction)` and the host feeds the
//!    battle intensity to `MusicDirector::set_battle_intensity`.
//! 3. **Result.** When `Battle::is_over()` the host reads `Battle::result()` (`BattleResult`, one
//!    `GroupResult` per tagged group with its survivors), maps it to a [`BattleOutcome`] and writes
//!    [`BattleFinished`]. `galaxy-sim` needs an entry point that applies a tactical result to the
//!    pending battle (today `Sim::resolve_battle_tactical` simulates headless and
//!    `PlayAction::ResolveBattle` only knows `Fight`/`AutoResolve`); until then the host
//!    answers with `AutoResolve`.
//! 4. **Return.** [`finish_battle`] despawns the battle (`BattleEntity`), re-enables the galaxy
//!    camera, switches back to [`AppScreen::Galaxy`], restores the galaxy music scene and sends
//!    the result into the simulation, which resumes the galaxy clock.
//!
//! The placeholder screen below stands in for step 2 and 3 so the whole round trip can be
//! exercised today (F9 on the map opens it for the selected planet).

use bevy::prelude::*;

use crate::flow::AppScreen;
use crate::galaxy_map::{GalaxyEntity, Selection};
use crate::galaxy_view::GalaxyView;
use crate::menu::{MenuAction, MenuColumn, MenuItem, MenuSpec, spawn_menu};

/// What the battle screen needs to know about the battle that was requested.
#[derive(Resource, Debug, Clone)]
pub struct BattleRequest {
    /// Id of the pending battle in the galaxy simulation.
    pub id: u32,
    /// Planet the battle takes place at.
    pub planet_name: String,
    /// Display names of the sides.
    pub sides: Vec<String>,
}

/// Start the tactical battle for a pending galaxy battle.
#[derive(Message, Debug, Clone)]
pub struct OpenBattle(pub BattleRequest);

/// How a tactical battle ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BattleOutcome {
    /// The side with this index (into [`BattleRequest::sides`]) won.
    Victory(usize),
    /// Nobody won.
    Draw,
    /// The battle was left without a result.
    Aborted,
}

/// A tactical battle is over; the result goes back into the galaxy simulation.
#[derive(Message, Debug, Clone)]
pub struct BattleFinished {
    /// Id of the pending battle.
    pub id: u32,
    /// Result.
    pub outcome: BattleOutcome,
}

/// Root of the placeholder battle screen.
#[derive(Component)]
struct BattleScreen;

/// Screen switching around tactical battles.
pub struct BattleSeamPlugin;

impl Plugin for BattleSeamPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<OpenBattle>()
            .add_message::<BattleFinished>()
            .add_systems(
                Update,
                (debug_open_battle.run_if(in_state(AppScreen::Galaxy)), open_battle, finish_battle),
            )
            .add_systems(OnEnter(AppScreen::SpaceBattle), hide_galaxy)
            .add_systems(OnEnter(AppScreen::SpaceBattle), spawn_placeholder.run_if(not(resource_exists::<crate::battle_play::PendingTactical>)))
            .add_systems(OnExit(AppScreen::SpaceBattle), (show_galaxy, despawn_placeholder));
    }
}

/// F9 on the map: opens the placeholder battle for the selected planet.
fn debug_open_battle(
    keys: Res<ButtonInput<KeyCode>>,
    selection: Res<Selection>,
    view: Res<GalaxyView>,
    mut open: MessageWriter<OpenBattle>,
) {
    if !keys.just_pressed(KeyCode::F9) {
        return;
    }
    if let Some(p) = selection.0.and_then(|i| view.planets.get(i)) {
        open.write(OpenBattle(BattleRequest {
            id: 0,
            planet_name: p.display_name.clone(),
            sides: view.factions.iter().take(2).map(|f| f.name.clone()).collect(),
        }));
    }
}

fn open_battle(
    mut requests: MessageReader<OpenBattle>,
    mut commands: Commands,
    mut next: ResMut<NextState<AppScreen>>,
) {
    if let Some(OpenBattle(request)) = requests.read().last() {
        commands.insert_resource(request.clone());
        next.set(AppScreen::SpaceBattle);
    }
}

/// Leaves the battle screen when a result arrived.
fn finish_battle(mut results: MessageReader<BattleFinished>, mut next: ResMut<NextState<AppScreen>>) {
    if results.read().last().is_some() {
        next.set(AppScreen::Galaxy);
    }
}

/// The galaxy scene stays alive during a battle but is not drawn.
#[allow(clippy::type_complexity)]
fn hide_galaxy(
    mut scene: Query<&mut Visibility, Or<(With<GalaxyEntity>, With<galaxy_play::FleetVisual>, With<galaxy_play::visuals::FleetLabel>)>>,
) {
    for mut v in &mut scene {
        *v = Visibility::Hidden;
    }
}

#[allow(clippy::type_complexity)]
fn show_galaxy(
    mut scene: Query<&mut Visibility, Or<(With<GalaxyEntity>, With<galaxy_play::FleetVisual>, With<galaxy_play::visuals::FleetLabel>)>>,
    mut selection: ResMut<Selection>,
) {
    for mut v in &mut scene {
        *v = Visibility::Inherited;
    }
    // Makes the info panel refresh itself.
    selection.set_changed();
}

fn spawn_placeholder(mut commands: Commands, request: Option<Res<BattleRequest>>) {
    let (planet, sides) = request
        .map(|r| (r.planet_name.clone(), r.sides.join(" vs ")))
        .unwrap_or_default();
    let spec = MenuSpec {
        title: "Space Battle".into(),
        subtitle: format!("{planet}  -  {sides}"),
        columns: vec![MenuColumn {
            heading: "Tactical battle view".into(),
            subheading: "not connected yet".into(),
            items: vec![MenuItem::new("Return to the galaxy map", MenuAction::EndBattle)],
        }],
        dim_background: true,
        ..default()
    };
    spawn_menu(&mut commands, &spec, BattleScreen);
}

fn despawn_placeholder(mut commands: Commands, screen: Query<Entity, With<BattleScreen>>) {
    for e in &screen {
        commands.entity(e).despawn();
    }
    commands.remove_resource::<BattleRequest>();
}
