//! `--drag-to <planet>` (offscreen): shows the fleets of the selected planet picked up and carried over another planet, as while the
//! player drags them with the mouse. An offscreen run has no cursor, so this fills the drag state of `galaxy-play` by hand; the pick up,
//! the dashed origin ring, the carried fleet, the drop target ring and the route preview are then drawn by the normal systems.
//!
//! `--drag-pointer <planet>[:<pad>]` (offscreen): the same drag through the real input path. Offscreen runs have no winit, so a window
//! that exists only as data (no OS window, no OS cursor) receives what the OS would write: cursor moves and a left press and release. The
//! pointer goes to the first own fleet at the selected planet, presses, moves to the target planet (or to its fleet pad 1..3) in small
//! steps, rests there and releases (`--drag-hold` keeps the button down, so the capture shows the carried fleet). Every frame between the
//! pick-up and the release must still carry the fleet, the carried model's centre must stay within 3 px of the cursor, the cursor stays
//! drawn as a cross in the capture, and the verdict is one log line `drag-pointer: ... PASS|FAIL`.

use bevy::input::ButtonState;
use bevy::input::InputSystems;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::mouse::MouseButtonInput;
use bevy::prelude::*;
use bevy::window::{CursorMoved, PrimaryWindow, WindowResolution};
use galaxy_play::{FleetEntities, GalaxyLayout, PlayAction, PlayCursorBlocked, PlayHover, PlaySet, PlayerFaction, PlayerSelection, SimState, input::DragState};
use galaxy_sim::{FleetId, FleetLocation, PlanetId};

use crate::cli::Args;
use crate::galaxy_view::GalaxyView;
use crate::play::{PlayBridge, ScriptHold};

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
        )
        .add_systems(
            PreUpdate,
            drag_pointer
                .before(InputSystems)
                .run_if(resource_exists::<SimState>.and_then(resource_exists::<PlayBridge>).and_then(resource_exists::<crate::offscreen::OffscreenTarget>)),
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

/// Frame of the pointer scenario (counted from its start) of each step: move onto the fleet, press, start moving, steps of the move,
/// frames resting on the target before the release, frames after the release until the verdict.
const POINTER_MOVE: u32 = 2;
const POINTER_PRESS: u32 = 6;
const POINTER_PATH: u32 = 8;
const POINTER_STEPS: u32 = 48;
const POINTER_REST: u32 = 24;
const POINTER_AFTER: u32 = 40;
/// Most the carried model's centre may be off the cursor on screen (logical px): it hangs exactly at the pointer.
const ICON_TOLERANCE: f32 = 3.0;
/// Most a dropped fleet's drawn size may change from one frame to the next (a fraction): it eases from the carry scale 1.5 to 1, the first
/// frame by 7 % at 60 Hz, and nothing may make it jump.
const DROP_STEP_MAX: f32 = 0.10;

/// Progress of `--drag-pointer`.
#[derive(Default)]
struct PointerDrag {
    galaxy_frames: u32,
    /// Frame of the scenario, `None` before it started.
    t: Option<u32>,
    fleet: Option<FleetId>,
    from: Vec2,
    /// Target planet and pad (1..3) of the drop.
    target: Option<(PlanetId, Option<u8>)>,
    /// The aimed pad takes the drop into its slot (own or allied planet); on other planets the fleet is just sent there.
    pad_drop: bool,
    cursor: Vec2,
    picked_up: bool,
    /// First frame the carried fleet was gone before the release: frame, cursor, cursor blocked, hovered planet.
    aborted: Option<(u32, Vec2, bool, Option<PlanetId>)>,
    /// Largest distance on screen (logical px) between the cursor and the centre of the carried fleet model while resting on the target.
    icon_offset: f32,
    /// World scale of the carried fleet while resting on the target (the same at every zoom).
    carry_scale: f32,
    /// World scale of the fleet in the last frame, and the largest change of it between two frames after the release (a fraction).
    prev_scale: Option<f32>,
    drop_step: f32,
    /// What the pointer is over while it rests on the target (cursor blocked, hovered planet and pad), and how often that changed.
    rest: Option<(bool, Option<PlanetId>, Option<u8>)>,
    toggles: u32,
    /// The fleet still hung at the cursor after the release (or the cancel).
    carried_after: bool,
    orders: Vec<PlayAction>,
    done: bool,
}

/// Whether `order` drops the dragged fleet on planet `to` (and into pad `pad`, if one was aimed at).
pub(crate) fn drops_on(order: &PlayAction, to: PlanetId, pad: Option<u8>) -> bool {
    match (order, pad) {
        // Onto a pad: into exactly that slot (the simulation merges it into the fleet holding the slot).
        (PlayAction::MoveToSlot { to: t, slot, .. }, Some(s)) => *t == to && *slot == s,
        (_, Some(_)) => false,
        (PlayAction::MoveToSlot { to: t, .. }, None) | (PlayAction::MoveSelected(t), None) => *t == to,
        // Onto an own fleet: the dragged fleet merges into it.
        (PlayAction::MoveAndMerge { to: t, .. }, None) => *t == to,
        (PlayAction::MergeFleets { .. }, None) => true,
        _ => false,
    }
}

/// Marks the cross drawn at the synthetic cursor.
#[derive(Component)]
struct PointerCross;

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn drag_pointer(
    mut commands: Commands,
    mut st: Local<PointerDrag>,
    (args, view, bridge, target_image): (Res<Args>, Res<GalaxyView>, Res<PlayBridge>, Res<crate::offscreen::OffscreenTarget>),
    (state, player, selection, layout, entities): (Res<SimState>, Res<PlayerFaction>, Res<PlayerSelection>, Res<GalaxyLayout>, Res<FleetEntities>),
    (drag, blocked, hover, mut hold, paused): (Res<DragState>, Res<PlayCursorBlocked>, Res<PlayHover>, ResMut<ScriptHold>, Res<crate::flow::Paused>),
    camera: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    fleets: Query<(&galaxy_play::FleetVisual, &GlobalTransform)>,
    mut window: Query<(Entity, &mut Window), With<PrimaryWindow>>,
    mut cross: Query<&mut Node, With<PointerCross>>,
    mut actions: MessageReader<PlayAction>,
    (mut moved, mut buttons, mut keys): (MessageWriter<CursorMoved>, MessageWriter<MouseButtonInput>, MessageWriter<KeyboardInput>),
    (screen, ui_scale): (Res<State<crate::flow::AppScreen>>, Res<UiScale>),
    time: Res<Time>,
) {
    let Some(spec) = args.drag_pointer.as_deref() else { return };
    if st.done || *screen.get() != crate::flow::AppScreen::Galaxy {
        return;
    }
    st.galaxy_frames += 1;
    // The window exists only as data: offscreen runs have no winit, so nothing reaches the desktop or the real cursor.
    let Ok((win_entity, mut win)) = window.single_mut() else {
        let (w, h) = crate::offscreen::size();
        commands.spawn((Window { resolution: WindowResolution::new(w, h).with_scale_factor_override(target_image.scale_factor), focused: true, ..default() }, PrimaryWindow));
        commands.spawn((
            PointerCross,
            Node { position_type: PositionType::Absolute, width: Val::Px(15.0), height: Val::Px(15.0), border: UiRect::all(Val::Px(1.0)), ..default() },
            BorderColor::all(Color::srgb(1.0, 0.2, 1.0)),
            GlobalZIndex(100),
        ));
        return;
    };
    let names: Vec<(&str, &str)> = view.planets.iter().map(|p| (p.id.as_str(), p.display_name.as_str())).collect();
    let Some((camera, cam_tf)) = camera.iter().next() else { return };
    let screen_of = |p: Vec3| camera.world_to_viewport(cam_tf, p).ok();
    let me = player.0;
    let Some(t) = st.t else {
        // Start once the map settled and the fleets of the selected planet are on it.
        let (Some(me), Some(from)) = (me, selection.planet) else { return };
        if st.galaxy_frames < SELECT_FRAME || state.current.loading {
            return;
        }
        let (name, pad) = match spec.rsplit_once(':') {
            Some((n, p)) => (n, p.trim().parse::<u8>().ok().filter(|p| (1..=3).contains(p))),
            None => (spec, None),
        };
        let to = crate::cli::resolve_name("planet", name, &names).ok().and_then(|i| bridge.planet_to_sim.get(i).copied().flatten());
        let fleet = state.galaxy().fleets.values().filter(|f| f.owner == me && f.location == FleetLocation::AtPlanet(from)).min_by_key(|f| (f.slot, f.id)).map(|f| f.id);
        let at = fleet.and_then(|f| entities.0.get(&f)).and_then(|&e| fleets.get(e).ok()).and_then(|(_, g)| screen_of(g.translation()));
        match (to, fleet, at) {
            (Some(to), Some(fleet), Some(at)) => {
                let g = state.galaxy();
                let owner = g.planets.get(to as usize).and_then(|p| p.owner);
                st.pad_drop = pad.is_some() && galaxy_play::input::pads_open(owner, me, |o| g.hostile(&state.world, me, o));
                info!("drag-pointer: fleet {fleet} at planet {from} (screen {at:.0}) to planet {to} pad {pad:?} (pad takes the drop: {})", st.pad_drop);
                st.fleet = Some(fleet);
                st.from = at;
                st.target = Some((to, pad));
                st.t = Some(0);
                hold.active = true;
            }
            _ => {
                info!("drag-pointer: no own fleet at the selected planet or unknown target `{spec}` FAIL");
                st.done = true;
            }
        }
        return;
    };
    st.t = Some(t + 1);
    let (Some(fleet), Some((to, pad))) = (st.fleet, st.target) else { return };
    // The target on screen: the planet centre or the centre of the pad (as the hover picks it).
    let aim = match pad {
        Some(s) => {
            let a = galaxy_play::visuals::slot_angle(s as usize - 1);
            layout.positions[to as usize] + Vec3::new(a.cos(), 0.0, a.sin()) * layout.orbit_radius(to)
        }
        None => layout.positions[to as usize],
    };
    let Some(goal) = screen_of(aim) else { return };
    let release = POINTER_PATH + POINTER_STEPS + POINTER_REST;
    let next = match t {
        POINTER_MOVE => Some(st.from),
        t if (POINTER_PATH..POINTER_PATH + POINTER_STEPS).contains(&t) => Some(st.from.lerp(goal, (t - POINTER_PATH + 1) as f32 / POINTER_STEPS as f32)),
        _ => None,
    };
    if let Some(p) = next {
        st.cursor = p;
        win.bypass_change_detection().set_cursor_position(Some(p));
        moved.write(CursorMoved { window: win_entity, position: p, delta: None });
    }
    if t == POINTER_PRESS {
        buttons.write(MouseButtonInput { button: MouseButton::Left, state: ButtonState::Pressed, window: win_entity });
    }
    // The end of the drag: release the button, or cancel with Esc / a right click and release a little later.
    let cancel = args.drag_cancel.as_deref();
    if t == release {
        match cancel {
            Some("esc") => {
                keys.write(KeyboardInput { key_code: KeyCode::Escape, logical_key: Key::Escape, state: ButtonState::Pressed, text: None, repeat: false, window: win_entity });
                keys.write(KeyboardInput { key_code: KeyCode::Escape, logical_key: Key::Escape, state: ButtonState::Released, text: None, repeat: false, window: win_entity });
            }
            Some(_) => {
                buttons.write(MouseButtonInput { button: MouseButton::Right, state: ButtonState::Pressed, window: win_entity });
                buttons.write(MouseButtonInput { button: MouseButton::Right, state: ButtonState::Released, window: win_entity });
            }
            None if !args.drag_hold => {
                buttons.write(MouseButtonInput { button: MouseButton::Left, state: ButtonState::Released, window: win_entity });
            }
            None => {}
        }
    }
    if cancel.is_some() && t == release + 4 {
        buttons.write(MouseButtonInput { button: MouseButton::Left, state: ButtonState::Released, window: win_entity });
    }
    // UI nodes are multiplied by the UI scale; the cross is 15 node px, centred on the cursor.
    let us = ui_scale.0.max(0.1);
    for mut n in &mut cross {
        n.left = Val::Px(st.cursor.x / us - 7.5);
        n.top = Val::Px(st.cursor.y / us - 7.5);
    }
    // What the input systems made of the previous frame.
    let carrying = drag.dragging && drag.carry.contains(&fleet);
    // The world scale the fleet was drawn at in the last frame. After the release it shrinks back to its normal size while it glides home:
    // no frame may change it by more than `DROP_STEP_MAX` (a fleet that popped to 1.4 times its size half a second after the drop, 39.5 % in
    // one frame, went unseen by this scenario). `SP_DRAG_TRACE=1` logs it with the frame time, one line per frame.
    let scale_now = entities.0.get(&fleet).and_then(|&e| fleets.get(e).ok()).map(|(_, g)| g.compute_transform().scale.x);
    if std::env::var_os("SP_DRAG_TRACE").is_some()
        && let Some(scale) = scale_now
    {
        info!("drag-trace: t {t} dt {:.2} ms scale {scale:.3} carrying {carrying} cursor {:.0}", time.delta_secs() * 1000.0, st.cursor);
    }
    if t > release
        && !args.drag_hold
        && let (Some(before), Some(now)) = (st.prev_scale, scale_now)
    {
        st.drop_step = st.drop_step.max(((now - before) / before).abs());
    }
    st.prev_scale = scale_now;
    if (POINTER_PATH..=release).contains(&t) {
        if carrying {
            st.picked_up = true;
        } else if st.picked_up && st.aborted.is_none() {
            st.aborted = Some((t, st.cursor, blocked.0, hover.planet));
        }
        if t % 8 == 0 {
            info!("drag-pointer: t {t} cursor {:.0} carrying {carrying} blocked {} hover {:?} pad {:?}", st.cursor, blocked.0, hover.planet, hover.slot);
        }
        // Resting on the target with a fixed pointer: nothing may toggle from frame to frame.
        if t > POINTER_PATH + POINTER_STEPS + 2 {
            let now = (blocked.0, hover.planet, hover.slot);
            if st.rest.is_some_and(|r| r != now) {
                st.toggles += 1;
            }
            st.rest = Some(now);
            // The carried model's centre on screen (the model stands `FLEET_MODEL_LIFT` pads above its root, see `fit_fleet_models`).
            if carrying
                && let Some((v, g)) = entities.0.get(&fleet).and_then(|&e| fleets.get(e).ok())
                && let Some(p) = screen_of(g.transform_point(Vec3::Y * v.pad * galaxy_play::visuals::FLEET_MODEL_LIFT))
            {
                st.icon_offset = st.icon_offset.max(p.distance(st.cursor));
                st.carry_scale = g.compute_transform().scale.x;
            }
        }
    }
    if t == release + 2 && !args.drag_hold {
        st.carried_after = carrying;
    }
    for a in actions.read() {
        if t > release && !matches!(a, PlayAction::SelectFleets(_) | PlayAction::SelectPlanet(_) | PlayAction::SetSpeedLevel(_)) {
            st.orders.push(a.clone());
        }
    }
    let end = if args.drag_hold { release } else { release + POINTER_AFTER };
    if t >= end {
        st.done = true;
        hold.active = false;
        hold.done_at = Some(std::time::Instant::now());
        // A drop gives the order to the aimed planet / pad; a cancelled drag gives none and (Esc) opens no pause menu.
        let ended = match cancel {
            Some(c) => st.orders.is_empty() && !(c == "esc" && paused.0),
            None if args.drag_hold => true,
            // A pad of an enemy or neutral planet takes no drop into its slot: the fleet is just sent there.
            None if pad.is_some() && !st.pad_drop => {
                st.orders.iter().any(|o| matches!(o, PlayAction::MoveSelected(t) if *t == to)) && !st.orders.iter().any(|o| matches!(o, PlayAction::MoveToSlot { .. }))
            }
            None => st.orders.iter().any(|o| drops_on(o, to, pad)),
        };
        let ok = st.picked_up && st.aborted.is_none() && st.toggles == 0 && !st.carried_after && st.icon_offset <= ICON_TOLERANCE && st.drop_step <= DROP_STEP_MAX && ended;
        let left = state.galaxy().fleets.get(&fleet).map(|f| match f.location {
            FleetLocation::AtPlanet(p) => format!("at planet {p} slot {}", f.slot),
            FleetLocation::Moving { to, .. } => format!("moving to {to}"),
        });
        info!(
            "drag-pointer: picked up {} aborted {:?} toggles {} carried after the end {} icon offset {:.1} px carried scale {:.2} drop size step {:.1} % orders {:?} paused {} fleet {:?}: {}",
            st.picked_up,
            st.aborted,
            st.toggles,
            st.carried_after,
            st.icon_offset,
            st.carry_scale,
            st.drop_step * 100.0,
            st.orders,
            paused.0,
            left,
            if ok { "PASS" } else { "FAIL" }
        );
    }
}
