//! `--drag-to <planet>` (offscreen): shows the fleets of the selected planet picked up and carried over another planet, as while the
//! player drags them with the mouse. An offscreen run has no cursor, so this fills the drag state of `galaxy-play` by hand; the pick up,
//! the dashed origin ring, the carried fleet, the drop target ring and the route preview are then drawn by the normal systems.

use bevy::prelude::*;
use galaxy_play::{FleetEntities, GalaxyLayout, PlayAction, PlayHover, PlaySet, PlayerFaction, PlayerSelection, SimState, input::DragState};
use galaxy_sim::FleetLocation;

use crate::cli::Args;
use crate::galaxy_view::GalaxyView;
use crate::play::PlayBridge;

/// Registers the demo.
pub struct DragDemoPlugin;

impl Plugin for DragDemoPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            drag_demo
                .after(galaxy_play::input::update_hover)
                .before(galaxy_play::input::draw_drag)
                .in_set(PlaySet::Input)
                .run_if(resource_exists::<SimState>.and(resource_exists::<PlayBridge>)),
        );
    }
}

/// Frames to wait before the fleets are selected (the map settles), and before they are picked up (the selection arrives).
const SELECT_FRAME: u32 = 90;
const PICK_UP_FRAME: u32 = 96;

#[allow(clippy::too_many_arguments)]
fn drag_demo(
    args: Res<Args>,
    view: Res<GalaxyView>,
    bridge: Res<PlayBridge>,
    state: Res<SimState>,
    player: Res<PlayerFaction>,
    selection: Res<PlayerSelection>,
    layout: Res<GalaxyLayout>,
    entities: Res<FleetEntities>,
    mut drag: ResMut<DragState>,
    mut hover: ResMut<PlayHover>,
    mut actions: MessageWriter<PlayAction>,
    mut frames: Local<u32>,
    mut ids: Local<Vec<galaxy_sim::FleetId>>,
) {
    let Some(target) = args.drag_to.as_deref() else { return };
    *frames += 1;
    let names: Vec<(&str, &str)> = view.planets.iter().map(|p| (p.id.as_str(), p.display_name.as_str())).collect();
    let Some(to) = crate::cli::resolve_name("planet", target, &names).ok().and_then(|i| bridge.planet_to_sim.get(i).copied().flatten()) else { return };
    let (Some(me), Some(from)) = (player.0, selection.planet) else { return };
    if *frames == SELECT_FRAME {
        *ids = state
            .galaxy()
            .fleets
            .values()
            .filter(|f| f.owner == me && matches!(f.location, FleetLocation::AtPlanet(p) if p == from))
            .map(|f| f.id)
            .collect();
        actions.write(PlayAction::SelectFleets(ids.clone()));
    }
    if *frames >= PICK_UP_FRAME && !ids.is_empty() && ids.iter().all(|id| entities.0.contains_key(id)) {
        let at = layout.positions[to as usize] + Vec3::new(-layout.unit * 2.2, 0.0, layout.unit * 2.0);
        drag.dragging = true;
        drag.carry = ids.clone();
        drag.cursor_ground = Some(at);
        hover.planet = Some(to);
        let g = state.galaxy();
        drag.preview = ids.first().and_then(|id| g.fleets.get(id)).and_then(|f| galaxy_play::geometry::plan_route(&state.world, g, me, state.world.fleet_speed(f), from, to)).map(|plan| (to, plan));
    }
}
