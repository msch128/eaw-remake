//! `--label-probe` (offscreen): proof that the planets in the lower part of the map keep their names and stay usable. Their names are
//! drawn below the command bar (`galaxy_map::LABEL_Z`): behind the translucent build strip they show through it like in the Remake,
//! and nothing may keep a planet whose globe shows above the bar art from being hovered, clicked or dropped on.
//!
//! Once the map has settled the probe lists every planet whose globe or name lies in the bottom 42 % of the screen (the height of the
//! command bar), with its globe, its name rectangle and whether the name shows. Then, through the real input path (a window that
//! exists only as data receives cursor moves and button presses, as in `--drag-pointer`; nothing reaches the desktop) and with the game
//! clock paused, so that no fleet moves away from under the pointer (the dragged one leaves its planet with every drop):
//! 1. still: 120 frames with the camera and the pointer still. Every frame logs the number of names shown, the largest move of a name
//!    and the names in the lower part with their positions; a name that shows, hides or moves by more than half a pixel between two
//!    frames is a toggle.
//! 2. per listed planet, twice: the pointer goes to the first of a few points that is not over the bar art and rests there for 12
//!    frames: the planet must be hovered in every one of them (no fleet) and the hover card must name it; a click must select it; the own
//!    fleet of the start planet is dragged onto it and the drop must send the fleet there. The first time the point is one of the planet's
//!    own that no fleet's pick area reaches ("free"); when every such point has a fleet over it, the first one with a fleet over it
//!    is used and the planet fails (a click on a fleet selects the fleet). Only the visible globe is the planet's own: points of its pick
//!    disc outside it that a fleet reaches do not count, and a planet whose visible globe is behind the interface has nothing to prove. The second time the point lies on the visible globe
//!    (`GalaxyLayout::globe_radius`) under the pick area of a parked fleet ("under a fleet"): the planet must win there too, and the drop
//!    must be a plain move (not a merge into the fleet). A planet whose globe is all behind the bar art is logged as such.
//! 3. fleets: every parked fleet in view is hovered at its icon centre (it must be the one hovered) and the own ones are clicked there
//!    and on the edge of their ring away from their planet (the fleet must be selected, where `input_test` clicks): the planet's globe
//!    takes no fleet out of reach.
//! 4. zoom: 120 frames of slow zoom out (8 % of the zoom range) and 120 back in. Every frame logs the names shown and those in the
//!    lower part with their positions. A frame in which most names come or go at once is the far-zoom threshold passing; it may happen
//!    once on the way out and once on the way back. Apart from it a name that changes between shown and hidden (its planet entering or
//!    leaving the view) more than once on the way out or on the way back, or that changes between below and above its globe again
//!    within `SIDE_FLIP_FRAMES` frames, blinks (a name may go above its globe when a panel meets its place and come back below when the
//!    place clears, tens of frames later: that is the layout working).
//!
//! In every still and zoom frame the selected planet must show its name (at any zoom, also past `galaxy_map::NAMES_HIDE_FROM`). The
//! verdict is one log line `label-probe: ... PASS|FAIL`.

use bevy::input::ButtonState;
use bevy::input::InputSystems;
use bevy::input::mouse::MouseButtonInput;
use bevy::prelude::*;
use bevy::window::{CursorMoved, PrimaryWindow, WindowResolution};
use galaxy_play::input::{fleet_icons, pick_fleet};
use galaxy_play::{FleetEntities, GalaxyLayout, GameClock, PlayAction, PlayConfig, PlayCursorBlocked, PlayHover, PlayerFaction, PlayerSelection, SimState};
use galaxy_sim::{FleetId, FleetLocation, PlanetId};

use crate::camera::{CameraBounds, GalaxyCamera};
use crate::cli::Args;
use crate::galaxy_map::{Hover, PlanetLabel, Selection, galaxy_to_world};
use crate::galaxy_view::GalaxyView;
use crate::planet_visual::PlanetRadii;
use crate::play::{PlayBridge, ScriptHold};

/// Registers the probe.
pub struct LabelProbePlugin;

impl Plugin for LabelProbePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreUpdate,
            probe.before(InputSystems).run_if(
                resource_exists::<SimState>
                    .and_then(resource_exists::<PlayBridge>)
                    .and_then(resource_exists::<crate::offscreen::OffscreenTarget>),
            ),
        );
    }
}

/// Galaxy frames before the probe starts (the map, the labels and the fleets have settled).
const START_FRAME: u32 = 90;
const STILL_FRAMES: u32 = 120;
const ZOOM_FRAMES: u32 = 120;
/// How far the slow zoom goes out (and back in), as a share of the zoom range.
const ZOOM_SPAN: f32 = 0.08;
/// Frames from a camera move to the names it gives (camera transform, then the label layout, read by the probe two frames after it set
/// the distance). Measured on the far-zoom threshold: the camera frame whose zoom first reads 0.470 hides the names in the frame two later
/// (Imperial 0.42, zoom frames 74 and 76). With 3 a planet whose name shows for the one frame of the turnaround (its label crosses the
/// screen edge at the farthest zoom: shown at the peak, hidden at the first step back) counted as two changes on the way out.
const ZOOM_LAG: u32 = 2;
/// A name that changes its side of the globe again less than this many zoom frames after its last change flips back and forth (a blink).
/// A planet whose place under the globe meets a panel and later clears it changes twice, tens of frames apart: that is the layout working.
const SIDE_FLIP_FRAMES: u32 = 30;
/// The lower part of the screen: the bottom 42 %, the height of the command bar.
const LOWER: f32 = 0.58;
/// Frames after a pointer move or press before the probe reads what the input systems made of it.
const SETTLE: u32 = 3;
const HOVER_FRAMES: u32 = 12;
const DRAG_STEPS: u32 = 16;
const DRAG_REST: u32 = 6;
const DROP_WAIT: u32 = 8;
/// A label moving by more than this (logical px) between two still frames is a toggle.
const MOVE_EPS: f32 = 0.5;

/// A planet of the lower part of the screen, tried once on a point nothing reaches and once under the pick area of a fleet.
#[derive(Clone)]
struct Target {
    map: usize,
    sim: PlanetId,
    name: String,
    /// The visible globe under a fleet's pick area is tried, not the point of a free globe.
    covered: bool,
    /// The point of the globe the pointer uses (the first one not over the bar art), once found.
    point: Option<Vec2>,
    result: String,
    pass: bool,
}

impl Target {
    /// Name for the log lines: the planet and which of its two points.
    fn label(&self) -> String {
        format!("{} [{}]", self.name, if self.covered { "under a fleet" } else { "free" })
    }
}

/// Where the probe is.
#[derive(Default, Clone, Copy, PartialEq, Debug)]
enum Step {
    #[default]
    Wait,
    Still(u32),
    /// Planet, candidate point, frame.
    Aim(usize, usize, u32),
    Hover(usize, u32),
    Click(usize, u32),
    Drag(usize, u32),
    /// Parked fleet, frame.
    Fleets(usize, u32),
    Zoom(u32),
    Done,
}

/// A parked fleet in view and where the pointer goes: its icon centre, or (own fleets) the edge of its ring on the side away from its
/// planet, where `input_test` clicks.
#[derive(Clone, Copy)]
struct FleetAim {
    id: FleetId,
    /// The icon centre.
    centre: Vec2,
    /// Where the pointer rests.
    point: Vec2,
    own: bool,
    ring: bool,
}

/// What the pointer made of the aims of one kind: reached, hovered the right fleet, clicked (own fleets), selected the clicked fleet.
#[derive(Default, Clone, Copy)]
struct FleetTally {
    tested: u32,
    hovered: u32,
    clicked: u32,
    selected: u32,
}

#[derive(Default)]
struct Probe {
    galaxy_frames: u32,
    step: Step,
    targets: Vec<Target>,
    fleet: Option<FleetId>,
    /// Per planet: the name's top-left corner (logical px) and whether it sits above its globe, if it shows.
    last: Option<Vec<Option<(Vec2, bool)>>>,
    still_toggles: u32,
    /// Per planet during the zoom out [0] and back in [1]: changes between shown and hidden, and between below and above the globe.
    shown_changes: [Vec<u32>; 2],
    side_changes: [Vec<u32>; 2],
    /// Per planet and direction: the zoom frame of its last change of side, and the changes of side that came less than
    /// [`SIDE_FLIP_FRAMES`] after the one before (a name that flips back and forth).
    last_side: [Vec<Option<u32>>; 2],
    side_flips: [Vec<u32>; 2],
    /// Frames out [0] and back in [1] in which most names came or went at once (the far-zoom threshold).
    threshold_passes: [u32; 2],
    zoom_from: f32,
    zoom_to: f32,
    /// Still and zoom frames in which the selected planet, on screen, showed no name.
    selected_unnamed: u32,
    /// The first point on the visible globe of the current planet that is free of the bar art but has a fleet over it.
    fallback: Option<Vec2>,
    /// A point of the planet's pick disc outside its visible globe that is free of the bar art had a fleet over it (the fleet wins there).
    outside_fleet: bool,
    hover_ok: u32,
    /// Per hover frame: the map's hovered planet, galaxy-play's planet and fleet, whether the cursor is blocked and whether the hover card
    /// names the planet.
    hover_seen: Vec<(Option<usize>, Option<PlanetId>, Option<FleetId>, bool, bool)>,
    drag_from: Vec2,
    /// The clock's speed override before the probe paused time.
    saved_speed: Option<f32>,
    /// What the pointer was over when the button went down and when it was released: fleet, planet, cursor blocked.
    drag_seen: [(Option<FleetId>, Option<PlanetId>, bool); 2],
    orders: Vec<PlayAction>,
    /// The parked fleets in view and what the pointer made of them, at the icon centre [0] and at the ring edge [1]; the aims behind the
    /// interface or left by their fleet meanwhile, and what failed.
    fleet_aims: Vec<FleetAim>,
    fleets: [FleetTally; 2],
    fleets_blocked: u32,
    fleets_moved: u32,
    fleets_failed: Vec<String>,
}

/// The names now: per planet its top-left corner (logical px) and whether it sits above its globe, if it shows.
fn names_now(
    labels: &Query<(&PlanetLabel, &Node, &Visibility)>,
    n: usize,
    us: f32,
    globe_y: &[Option<f32>],
) -> Vec<Option<(Vec2, bool)>> {
    let mut out = vec![None; n];
    for (label, node, vis) in labels {
        if *vis == Visibility::Hidden {
            continue;
        }
        let (Val::Px(x), Val::Px(y)) = (node.left, node.top) else { continue };
        let pos = Vec2::new(x, y) * us;
        let above = globe_y.get(label.0).copied().flatten().is_some_and(|g| pos.y < g);
        if let Some(slot) = out.get_mut(label.0) {
            *slot = Some((pos, above));
        }
    }
    out
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn probe(
    mut commands: Commands,
    mut st: Local<Probe>,
    (args, view, bridge, target_image, radii): (Res<Args>, Res<GalaxyView>, Res<PlayBridge>, Res<crate::offscreen::OffscreenTarget>, Res<PlanetRadii>),
    (state, player, mut play_selection, entities): (Res<SimState>, Res<PlayerFaction>, ResMut<PlayerSelection>, Res<FleetEntities>),
    (layout, play_cfg, mut clock): (Res<GalaxyLayout>, Res<PlayConfig>, ResMut<GameClock>),
    (blocked, play_hover, hover, mut selection, mut hold): (Res<PlayCursorBlocked>, Res<PlayHover>, Res<Hover>, ResMut<Selection>, ResMut<ScriptHold>),
    (bounds, mut galaxy_camera): (Res<CameraBounds>, Single<&mut GalaxyCamera>),
    camera: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    fleets: Query<&GlobalTransform, With<galaxy_play::FleetVisual>>,
    labels: Query<(&PlanetLabel, &Node, &Visibility)>,
    mut window: Query<(Entity, &mut Window), With<PrimaryWindow>>,
    mut actions: MessageReader<PlayAction>,
    (mut moved, mut buttons): (MessageWriter<CursorMoved>, MessageWriter<MouseButtonInput>),
    (screen, ui_scale, tip): (Res<State<crate::flow::AppScreen>>, Res<UiScale>, Res<eaw_ui::WorldTooltip>),
) {
    if !args.label_probe || st.step == Step::Done || *screen.get() != crate::flow::AppScreen::Galaxy {
        return;
    }
    st.galaxy_frames += 1;
    // The window exists only as data: offscreen runs have no winit, so nothing reaches the desktop or the real cursor.
    let Ok((win_entity, mut win)) = window.single_mut() else {
        let (w, h) = crate::offscreen::size();
        commands.spawn((Window { resolution: WindowResolution::new(w, h).with_scale_factor_override(target_image.scale_factor), focused: true, ..default() }, PrimaryWindow));
        return;
    };
    let Some((cam, cam_tf)) = camera.iter().next() else { return };
    let Some(viewport) = cam.logical_viewport_size() else { return };
    let us = ui_scale.0.max(0.1);
    let n = view.planets.len();
    // Globe centre and radius on screen (logical px).
    let globe = |i: usize| -> Option<(Vec2, f32)> {
        let c = galaxy_to_world(view.planets[i].position);
        let a = cam.world_to_viewport(cam_tf, c).ok()?;
        let b = cam.world_to_viewport(cam_tf, c + cam_tf.right() * radii.0[i]).ok()?;
        Some((a, a.distance(b)))
    };
    // Radius on screen of the visible globe of sim planet `s`, which is what the pick gives to the planet over fleets.
    let visible_globe = |i: usize, s: PlanetId| -> Option<f32> {
        let c = galaxy_to_world(view.planets[i].position);
        let a = cam.world_to_viewport(cam_tf, c).ok()?;
        let b = cam.world_to_viewport(cam_tf, c + cam_tf.right() * layout.globe_radius(s)).ok()?;
        Some(a.distance(b))
    };
    // The icons of the fleets the pointer can pick (as `PlayHover` computes them).
    let icons = || fleet_icons(&layout, &play_cfg, entities.0.iter().filter_map(|(&id, &e)| Some((id, fleets.get(e).ok()?.translation()))), |p| cam.world_to_viewport(cam_tf, p).ok(), *cam_tf.right());
    let globe_y: Vec<Option<f32>> = (0..n).map(|i| globe(i).map(|(a, _)| a.y)).collect();
    let names = names_now(&labels, n, us, &globe_y);
    let lower = viewport.y * LOWER;
    // The selected planet keeps its name at every zoom while its globe is on screen.
    let selected = selection.0.filter(|&i| globe(i).is_some_and(|(a, _)| a.x >= 0.0 && a.y >= 0.0 && a.x <= viewport.x && a.y <= viewport.y));
    let selected_note = selected.map_or_else(
        || "none on screen".to_string(),
        |i| format!("{} {}", view.planets[i].display_name, if names[i].is_some() { "named" } else { "UNNAMED" }),
    );
    let selected_unnamed = selected.is_some_and(|i| names[i].is_none());
    let mut move_to = |p: Vec2| {
        win.bypass_change_detection().set_cursor_position(Some(p));
        moved.write(CursorMoved { window: win_entity, position: p, delta: None });
    };
    // Orders given by the input systems (a drop sends the fleet).
    let new_orders: Vec<PlayAction> = actions.read().cloned().collect();

    match st.step {
        Step::Wait => {
            let Some(from) = play_selection.planet else { return };
            if st.galaxy_frames < START_FRAME || state.current.loading {
                return;
            }
            let Some(me) = player.0 else { return };
            st.fleet = state.galaxy().fleets.values().filter(|f| f.owner == me && f.location == FleetLocation::AtPlanet(from)).min_by_key(|f| (f.slot, f.id)).map(|f| f.id);
            // During the still frames the pointer rests in the upper right quarter, away from the lower part of the map and the bar.
            move_to(Vec2::new(viewport.x * 0.75, viewport.y * 0.12));
            for (i, &name) in names.iter().enumerate() {
                let Some((a, r)) = globe(i) else { continue };
                let on_screen = a.x + r > 0.0 && a.x - r < viewport.x && a.y - r < viewport.y && a.y + r > 0.0;
                let name_lower = name.is_some_and(|(p, _)| p.y + 10.0 > lower);
                if !on_screen || !(a.y + r > lower || name_lower) {
                    continue;
                }
                let Some(sim) = bridge.planet_to_sim.get(i).copied().flatten() else { continue };
                let p = &view.planets[i];
                info!(
                    "label-probe: planet {} globe ({:.0}, {:.0}) r {:.0} (world radius {:.2}) visible globe r {:.1} name {}",
                    p.display_name,
                    a.x,
                    a.y,
                    r,
                    layout.radii[sim as usize],
                    visible_globe(i, sim).unwrap_or_default(),
                    match name {
                        Some((pos, above)) => format!("shown at ({:.0}, {:.0}) {}", pos.x, pos.y, if above { "above the globe" } else { "below the globe" }),
                        None => "hidden".into(),
                    }
                );
                for covered in [false, true] {
                    st.targets.push(Target { map: i, sim, name: p.display_name.clone(), covered, point: None, result: String::new(), pass: false });
                }
            }
            info!(
                "label-probe: {} planets in the bottom 42 % (from y {:.0}), fleet {:?}, zoom {:.3}, {} fleet icons in view, icon unit {:.2}",
                st.targets.len() / 2,
                lower,
                st.fleet,
                zoom_fraction(&bounds, galaxy_camera.distance),
                icons().len(),
                layout.unit
            );
            hold.active = true;
            // Time stands still while the probe runs: no fleet moves under the pointer (the dragged one leaves its planet with every drop).
            st.saved_speed = clock.override_multiplier;
            clock.override_multiplier = Some(0.0);
            st.last = Some(names);
            st.step = Step::Still(0);
        }
        Step::Still(t) => {
            let last = st.last.take().unwrap_or_default();
            let changed = (0..n).filter(|&i| match (last.get(i).copied().flatten(), names[i]) {
                (None, None) => false,
                (Some((a, sa)), Some((b, sb))) => sa != sb || a.distance(b) > MOVE_EPS,
                _ => true,
            }).count() as u32;
            let moved = (0..n)
                .filter_map(|i| Some(last.get(i).copied().flatten()?.0.distance(names[i]?.0)))
                .fold(0.0f32, f32::max);
            st.still_toggles += changed;
            st.selected_unnamed += u32::from(selected_unnamed);
            let shown = names.iter().filter(|x| x.is_some()).count();
            let low = names.iter().filter(|x| x.is_some_and(|(p, _)| p.y > lower)).count();
            info!(
                "label-probe: still {t} shown {shown} lower {low} changed {changed} max move {moved:.2} px, selected {selected_note} |{}",
                lower_names(&view, &names, lower)
            );
            st.last = Some(names);
            st.step = if t + 1 >= STILL_FRAMES { Step::Aim(0, 0, 0) } else { Step::Still(t + 1) };
        }
        Step::Aim(k, c, t) => {
            let Some(target) = st.targets.get(k).cloned() else {
                // Every planet is done: the parked fleets in view are next.
                let g = state.galaxy();
                let on_screen = |p: Vec2| p.x > 8.0 && p.y > 8.0 && p.x < viewport.x - 8.0 && p.y < viewport.y - 8.0;
                // Every parked fleet on screen at its icon centre; the own ones also at the edge of their ring, away from their planet.
                let mut aims = Vec::new();
                for (id, centre, _) in icons().into_iter().filter(|&(id, p, _)| on_screen(p) && g.fleets.get(&id).is_some_and(|f| f.planet().is_some())) {
                    let fleet = &g.fleets[&id];
                    let own = Some(fleet.owner) == player.0;
                    aims.push(FleetAim { id, centre, point: centre, own, ring: false });
                    let root = entities.0.get(&id).and_then(|&e| fleets.get(e).ok()).map(|t| t.translation());
                    let edge = root.zip(fleet.planet()).and_then(|(root, p)| crate::input_test::ring_edge(cam, cam_tf, &layout, &play_cfg, root, layout.positions[p as usize]));
                    if let Some((_, point)) = edge.filter(|&(_, p)| own && on_screen(p)) {
                        aims.push(FleetAim { id, centre, point, own, ring: true });
                    }
                }
                aims.sort_by_key(|a| (a.id, a.ring));
                info!("label-probe: {} parked fleets in view ({} of them own), {} aims", aims.iter().filter(|a| !a.ring).count(), aims.iter().filter(|a| !a.ring && a.own).count(), aims.len());
                st.fleet_aims = aims;
                st.step = Step::Fleets(0, 0);
                return;
            };
            let (Some((a, r)), Some(g)) = (globe(target.map), visible_globe(target.map, target.sim)) else {
                st.targets[k].result = "globe not on screen".into();
                st.step = Step::Aim(k + 1, 0, 0);
                return;
            };
            // Points on the globe: the centre, then up toward the top edge and to the sides (the bar covers the lower part first). A
            // point must not be over the bar art. A free point has no fleet under it (a click on a fleet selects the fleet); a point
            // under a fleet lies on the visible globe and in the pick area of a parked fleet (as the pick stood before a planet won its
            // globe), the offsets then count in visible globe radii, and the point 9/10 of the way from the centre to the nearest fleet
            // icon comes first (where that fleet's pick is strongest).
            let icons_now = icons();
            let reach = if target.covered { g } else { r };
            let toward = icons_now.iter().map(|i| i.1).min_by(|x, y| x.distance(a).total_cmp(&y.distance(a))).map(|p| a + (p - a).normalize_or_zero() * 0.9 * g);
            let candidates: Vec<Vec2> = toward
                .filter(|_| target.covered)
                .into_iter()
                .chain(
                    [
                        Vec2::ZERO,
                        Vec2::new(0.0, -0.5),
                        Vec2::new(0.0, -0.85),
                        Vec2::new(0.6, -0.45),
                        Vec2::new(-0.6, -0.45),
                        Vec2::new(0.4, 0.0),
                        Vec2::new(-0.4, 0.0),
                        Vec2::new(0.0, 0.45),
                        Vec2::new(0.35, -0.75),
                        Vec2::new(-0.35, -0.75),
                    ]
                    .iter()
                    .map(|o| a + *o * reach),
                )
                .filter(|p| p.x > 8.0 && p.y > 8.0 && p.x < viewport.x - 8.0 && p.y < viewport.y - 8.0)
                .filter(|&p| !target.covered || pick_fleet(p, icons_now.iter().copied(), None).is_some())
                .collect();
            if c == 0 && t == 0 {
                st.fallback = None;
                st.outside_fleet = false;
            }
            let Some(&point) = candidates.get(c) else {
                if target.covered {
                    st.targets[k].result = if candidates.is_empty() {
                        format!("no fleet's pick area reaches the visible globe ({:.0}, {:.0}) r {:.1}: nothing to prove", a.x, a.y, g)
                    } else {
                        "every point of the visible globe under a fleet is behind the bar art: the cursor is over the interface there".into()
                    };
                    st.targets[k].pass = true;
                    st.step = Step::Aim(k + 1, 0, 0);
                    return;
                }
                match st.fallback {
                    // Every free point of the globe has a fleet over it: use the first one anyway (the click selects that fleet).
                    Some(p) => {
                        move_to(p);
                        st.targets[k].point = Some(p);
                        st.targets[k].result = "fleets cover every free point of the globe; ".into();
                        st.hover_ok = 0;
                        st.hover_seen.clear();
                        st.step = Step::Hover(k, 0);
                    }
                    // The visible globe is behind the interface and the rest of the pick disc, which the interface leaves free, is under a
                    // parked fleet: a fleet wins there by design (only the visible globe is the planet's own).
                    None if st.outside_fleet => {
                        st.targets[k].result = format!("the visible globe ({:.0}, {:.0}) r {:.1} is behind the interface and the rest of the pick disc is under a parked fleet, which wins there: nothing to prove", a.x, a.y, g);
                        st.targets[k].pass = true;
                        st.step = Step::Aim(k + 1, 0, 0);
                    }
                    None => {
                        st.targets[k].result = format!("globe ({:.0}, {:.0}) r {:.0} all behind the bar art: the cursor is over the interface at every point of it, no hover, click or drop", a.x, a.y, r);
                        st.targets[k].pass = true;
                        st.step = Step::Aim(k + 1, 0, 0);
                    }
                }
                return;
            };
            if t == 0 {
                move_to(point);
            }
            if t < SETTLE {
                st.step = Step::Aim(k, c, t + 1);
            } else if blocked.0 || (target.covered && hover.0 != Some(target.map)) {
                st.step = Step::Aim(k, c + 1, 0);
            } else if !target.covered && (play_hover.fleet.is_some() || hover.0 != Some(target.map)) {
                if play_hover.fleet.is_some() && hover.0 == Some(target.map) && point.distance(a) > g {
                    // Outside the visible globe a parked fleet is picked as before: no failure of the planet.
                    st.outside_fleet = true;
                } else if st.fallback.is_none() {
                    st.fallback = Some(point);
                }
                st.step = Step::Aim(k, c + 1, 0);
            } else {
                if target.covered {
                    // The fleet that held this point before: where its icon stands against the visible globe.
                    let under = pick_fleet(point, icons_now.iter().copied(), None);
                    st.targets[k].result = icons_now
                        .iter()
                        .find(|i| Some(i.0) == under)
                        .map(|&(id, p, reach)| format!("fleet {id} held it (icon {:.1} px from the globe centre = {:.2} visible globe radii of {:.1} px, pick reach {:.0} px); ", p.distance(a), p.distance(a) / g, g, reach))
                        .unwrap_or_default();
                }
                st.targets[k].point = Some(point);
                st.hover_ok = 0;
                st.hover_seen.clear();
                st.step = Step::Hover(k, 0);
            }
        }
        Step::Hover(k, t) => {
            let target = st.targets[k].clone();
            // The hover card (eaw-ui's world tooltip) starts with the planet's name.
            let card = tip.text.as_deref().and_then(|t| t.lines().next()) == Some(target.name.as_str());
            let now = (hover.0, play_hover.planet, play_hover.fleet, blocked.0, card);
            if t >= 2 {
                if now.0 == Some(target.map) && now.1 == Some(target.sim) && now.2.is_none() && !now.3 && now.4 {
                    st.hover_ok += 1;
                }
                st.hover_seen.push(now);
            }
            if t + 1 >= HOVER_FRAMES {
                let toggles = st.hover_seen.windows(2).filter(|w| w[0] != w[1]).count();
                let ok = st.hover_ok == HOVER_FRAMES - 2 && toggles == 0;
                let p = target.point.unwrap_or_default();
                let note = format!(
                    "point ({:.0}, {:.0}) hover {} ({} of {} frames, map {:?} play {:?} fleet {:?} card {}, {toggles} toggles)",
                    p.x,
                    p.y,
                    if ok { "PASS" } else { "FAIL" },
                    st.hover_ok,
                    HOVER_FRAMES - 2,
                    now.0,
                    now.1,
                    now.2,
                    if now.4 { "names it" } else { "does not name it" }
                );
                st.targets[k].result += &note;
                st.targets[k].pass = ok;
                st.step = Step::Click(k, 0);
            } else {
                st.step = Step::Hover(k, t + 1);
            }
        }
        Step::Click(k, t) => {
            let target = st.targets[k].clone();
            if t == 0 {
                // Nothing selected before the click, so the planet is selected by it (the same planet is clicked once on a free point
                // and once under a fleet).
                selection.0 = None;
                buttons.write(MouseButtonInput { button: MouseButton::Left, state: ButtonState::Pressed, window: win_entity });
            }
            if t == 1 {
                buttons.write(MouseButtonInput { button: MouseButton::Left, state: ButtonState::Released, window: win_entity });
            }
            if t >= 1 + SETTLE + 2 {
                let ok = selection.0 == Some(target.map) && play_selection.planet == Some(target.sim);
                st.targets[k].result += &format!(", click {} (map {:?} play {:?})", if ok { "PASS" } else { "FAIL" }, selection.0, play_selection.planet);
                st.targets[k].pass &= ok;
                st.orders.clear();
                st.step = Step::Drag(k, 0);
            } else {
                st.step = Step::Click(k, t + 1);
            }
        }
        Step::Drag(k, t) => {
            let target = st.targets[k].clone();
            let point = target.point.unwrap_or_default();
            let Some(fleet) = st.fleet else {
                st.targets[k].result += ", drop FAIL (no own fleet at the start planet)";
                st.targets[k].pass = false;
                st.step = Step::Aim(k + 1, 0, 0);
                return;
            };
            let press = SETTLE;
            let release = press + DRAG_STEPS + DRAG_REST;
            if t <= press {
                // The pointer takes the fleet by its icon centre, where a player aims, and stays on it until the button goes down (the
                // fleet's icon still settles from the last drop: a pointer left where the icon was is on the planet's globe, not on it).
                let at = icons().into_iter().find(|i| i.0 == fleet).map(|i| i.1);
                match at {
                    Some(p) => {
                        st.drag_from = p;
                        move_to(p);
                    }
                    None => {
                        st.targets[k].result += ", drop FAIL (the fleet is not on screen)";
                        st.targets[k].pass = false;
                        st.step = Step::Aim(k + 1, 0, 0);
                        return;
                    }
                }
            }
            if t == press {
                st.drag_seen[0] = (play_hover.fleet, play_hover.planet, blocked.0);
                buttons.write(MouseButtonInput { button: MouseButton::Left, state: ButtonState::Pressed, window: win_entity });
            }
            if t > press && t <= press + DRAG_STEPS {
                let p = st.drag_from.lerp(point, (t - press) as f32 / DRAG_STEPS as f32);
                move_to(p);
            }
            if t == release {
                st.drag_seen[1] = (play_hover.fleet, play_hover.planet, blocked.0);
                buttons.write(MouseButtonInput { button: MouseButton::Left, state: ButtonState::Released, window: win_entity });
            }
            if t > release {
                st.orders.extend(new_orders.into_iter().filter(|a| !matches!(a, PlayAction::SelectFleets(_) | PlayAction::SelectPlanet(_) | PlayAction::SetSpeedLevel(_))));
            }
            if t >= release + DROP_WAIT {
                // Under a fleet the planet wins the drop too: the fleet is sent to it (or into one of its pads), not merged into a fleet.
                let ok = st.orders.iter().any(|o| if target.covered { moves_to(o, target.sim) } else { crate::drag_demo::drops_on(o, target.sim, None) });
                let left = state.galaxy().fleets.get(&fleet).map(|f| match f.location {
                    FleetLocation::AtPlanet(p) => format!("at planet {p}"),
                    FleetLocation::Moving { to, .. } => format!("moving to {to}"),
                });
                let [(pf, _, pb), (rf, rp, rb)] = st.drag_seen;
                let note = format!(
                    ", drop {} (orders {:?}, fleet {:?}; pressed on fleet {pf:?} blocked {pb}, released over planet {rp:?} fleet {rf:?} blocked {rb})",
                    if ok { "PASS" } else { "FAIL" },
                    st.orders,
                    left
                );
                st.targets[k].result += &note;
                st.targets[k].pass &= ok;
                let t = &st.targets[k];
                info!("label-probe: {}: {}", t.label(), t.result);
                st.step = Step::Aim(k + 1, 0, 0);
            } else {
                st.step = Step::Drag(k, t + 1);
            }
        }
        Step::Fleets(j, t) => {
            let Some(aim) = st.fleet_aims.get(j).copied() else {
                let [c, r] = st.fleets;
                info!(
                    "label-probe: {} parked fleets in view: at the icon centre {} of {} hovered right and {} of {} own clicked fleets selected; at the ring edge away from the planet {} of {} hovered right and {} of {} clicked fleets selected ({} aims behind the interface, {} moved on), failed {:?}",
                    st.fleet_aims.iter().filter(|a| !a.ring).count(),
                    c.hovered,
                    c.tested,
                    c.selected,
                    c.clicked,
                    r.hovered,
                    r.tested,
                    r.selected,
                    r.clicked,
                    st.fleets_blocked,
                    st.fleets_moved,
                    st.fleets_failed
                );
                st.zoom_from = galaxy_camera.distance;
                let z = zoom_fraction(&bounds, galaxy_camera.distance);
                st.zoom_to = bounds.min_distance + (bounds.max_distance - bounds.min_distance) * (z + ZOOM_SPAN).min(1.0);
                st.shown_changes = [vec![0; n], vec![0; n]];
                st.side_changes = [vec![0; n], vec![0; n]];
                st.last_side = [vec![None; n], vec![None; n]];
                st.side_flips = [vec![0; n], vec![0; n]];
                st.last = Some(names);
                st.step = Step::Zoom(0);
                return;
            };
            // The pointer rests on the aim; the fleet must be the one hovered, and a click on an own fleet must select it.
            let kind = usize::from(aim.ring);
            let what = if aim.ring { "ring edge" } else { "icon centre" };
            if t == 0 {
                move_to(aim.point);
            }
            if t == SETTLE {
                // A fleet that left its planet since the list was made (the game goes on) is no longer under the pointer.
                if !icons().into_iter().any(|i| i.0 == aim.id && i.1.distance(aim.centre) < 1.0) {
                    st.fleets_moved += 1;
                    st.step = Step::Fleets(j + 1, 0);
                    return;
                }
                if blocked.0 {
                    st.fleets_blocked += 1;
                    st.step = Step::Fleets(j + 1, 0);
                    return;
                }
                st.fleets[kind].tested += 1;
                let hovered = play_hover.fleet == Some(aim.id);
                if hovered {
                    st.fleets[kind].hovered += 1;
                } else {
                    st.fleets_failed.push(format!("fleet {} {what} ({:.0}, {:.0}) hovered {:?}", aim.id, aim.point.x, aim.point.y, play_hover.fleet));
                }
                if aim.own && hovered {
                    st.fleets[kind].clicked += 1;
                    // Nothing selected before the click, so the fleet is selected by it.
                    play_selection.fleets.clear();
                    buttons.write(MouseButtonInput { button: MouseButton::Left, state: ButtonState::Pressed, window: win_entity });
                } else {
                    st.step = Step::Fleets(j + 1, 0);
                    return;
                }
            }
            if t == SETTLE + 1 {
                buttons.write(MouseButtonInput { button: MouseButton::Left, state: ButtonState::Released, window: win_entity });
            }
            if t >= SETTLE + 1 + SETTLE + 2 {
                if play_selection.fleets.contains(&aim.id) {
                    st.fleets[kind].selected += 1;
                } else {
                    st.fleets_failed.push(format!("fleet {} {what} ({:.0}, {:.0}) clicked, selection {:?}", aim.id, aim.point.x, aim.point.y, play_selection.fleets));
                }
                st.step = Step::Fleets(j + 1, 0);
            } else {
                st.step = Step::Fleets(j, t + 1);
            }
        }
        Step::Zoom(t) => {
            // What the names show now comes from the camera of a few frames ago: count it to the way out until then.
            let back = usize::from(t >= ZOOM_FRAMES + ZOOM_LAG);
            let last = st.last.take().unwrap_or_default();
            let flips: Vec<(usize, bool)> = (0..n).filter_map(|i| (last.get(i).copied().flatten().is_some() != names[i].is_some()).then_some((i, names[i].is_some()))).collect();
            let changed = flips.len();
            // A frame in which most names come or go at once is the far-zoom threshold passing (`galaxy_map::NAMES_HIDE_FROM`): it counts
            // once for the whole map, not per name.
            let (before, after) = (last.iter().filter(|x| x.is_some()).count(), names.iter().filter(|x| x.is_some()).count());
            if changed >= 10 && changed * 2 >= before.max(after) {
                st.threshold_passes[back] += 1;
                info!("label-probe: zoom {t}: {changed} names {} at once (zoom threshold), {before} -> {after}", if after < before { "hidden" } else { "shown" });
            } else {
                for &(i, shown) in &flips {
                    st.shown_changes[back][i] += 1;
                    info!("label-probe: zoom {t}: {} name {}", view.planets[i].display_name, if shown { "shown" } else { "hidden" });
                }
            }
            for (i, &now) in names.iter().enumerate() {
                if let (Some((_, sa)), Some((pos, sb))) = (last.get(i).copied().flatten(), now)
                    && sa != sb
                {
                    st.side_changes[back][i] += 1;
                    if st.last_side[back][i].is_some_and(|prev| t - prev < SIDE_FLIP_FRAMES) {
                        st.side_flips[back][i] += 1;
                    }
                    st.last_side[back][i] = Some(t);
                    info!("label-probe: zoom {t}: {} name now {} its globe at ({:.0}, {:.0})", view.planets[i].display_name, if sb { "above" } else { "below" }, pos.x, pos.y);
                }
            }
            st.selected_unnamed += u32::from(selected_unnamed);
            // Out over ZOOM_FRAMES frames, back in over as many, then a few frames at the start zoom for the last names to settle.
            let f = if t < ZOOM_FRAMES { (t + 1) as f32 } else { (2 * ZOOM_FRAMES).saturating_sub(t + 1) as f32 } / ZOOM_FRAMES as f32;
            let d = st.zoom_from + (st.zoom_to - st.zoom_from) * f;
            galaxy_camera.distance = d;
            galaxy_camera.target_distance = d;
            let shown = names.iter().filter(|x| x.is_some()).count();
            let low = names.iter().filter(|x| x.is_some_and(|(p, _)| p.y > lower)).count();
            info!(
                "label-probe: zoom {t} z {:.3} shown {shown} lower {low} changed {changed}, selected {selected_note} |{}",
                zoom_fraction(&bounds, d),
                lower_names(&view, &names, lower)
            );
            st.last = Some(names);
            if t + 1 < 2 * ZOOM_FRAMES + ZOOM_LAG {
                st.step = Step::Zoom(t + 1);
                return;
            }
            // On the way out and on the way back a name may come or go once (its planet enters or leaves the view), it may change its
            // side of the globe as its place meets or clears a panel as long as it does not flip back within `SIDE_FLIP_FRAMES`, and the
            // far-zoom threshold may pass once; more is a blink.
            let mut blinks: Vec<String> = (0..n)
                .filter(|&i| (0..2).any(|h| st.shown_changes[h][i] > 1 || st.side_flips[h][i] > 0))
                .map(|i| {
                    let p = &view.planets[i];
                    format!(
                        "{} (shown/hidden {} out {} back, below/above {} out {} back, {} out {} back flipped within {SIDE_FLIP_FRAMES} frames)",
                        p.display_name, st.shown_changes[0][i], st.shown_changes[1][i], st.side_changes[0][i], st.side_changes[1][i], st.side_flips[0][i], st.side_flips[1][i]
                    )
                })
                .collect();
            if st.threshold_passes.iter().any(|&c| c > 1) {
                blinks.push(format!("all names ({} threshold passes out, {} back)", st.threshold_passes[0], st.threshold_passes[1]));
            }
            let planets = st.targets.iter().filter(|t| !t.covered).count();
            let tested = |covered: bool| st.targets.iter().filter(|t| t.covered == covered && t.point.is_some()).count();
            let failed: Vec<String> = st.targets.iter().filter(|t| !t.pass).map(Target::label).collect();
            let ok = st.still_toggles == 0 && blinks.is_empty() && failed.is_empty() && !st.targets.is_empty() && st.selected_unnamed == 0 && st.fleets_failed.is_empty();
            info!(
                "label-probe: {planets} planets in the lower part, {} with a usable free globe point, {} with a fleet over the visible globe, failed {:?}, parked fleets failed {:?}, still toggles {}, zoom blinks {:?}, threshold passes {} out {} back, selected unnamed in {} frames: {}",
                tested(false),
                tested(true),
                failed,
                st.fleets_failed,
                st.still_toggles,
                blinks,
                st.threshold_passes[0],
                st.threshold_passes[1],
                st.selected_unnamed,
                if ok { "PASS" } else { "FAIL" }
            );
            for t in st.targets.iter().filter(|t| t.point.is_none()) {
                info!("label-probe: {}: {}", t.label(), t.result);
            }
            hold.active = false;
            clock.override_multiplier = st.saved_speed;
            hold.done_at = Some(std::time::Instant::now());
            st.step = Step::Done;
        }
        Step::Done => {}
    }
}

/// Whether `order` sends the dragged fleet to planet `to` itself (a plain move or a drop into one of its pads), not into a fleet parked there.
fn moves_to(order: &PlayAction, to: PlanetId) -> bool {
    matches!(order, PlayAction::MoveSelected(t) | PlayAction::MoveToSlot { to: t, .. } if *t == to)
}

/// The names shown in the lower part with their top-left corners, for the per-frame log lines.
fn lower_names(view: &GalaxyView, names: &[Option<(Vec2, bool)>], lower: f32) -> String {
    names
        .iter()
        .enumerate()
        .filter_map(|(i, x)| x.filter(|(p, _)| p.y > lower).map(|(p, _)| format!(" {} ({:.1}, {:.1})", view.planets[i].display_name, p.x, p.y)))
        .collect()
}

/// Zoom as a fraction: 0 at the closest camera distance, 1 at the farthest (as `galaxy_map` computes it).
fn zoom_fraction(bounds: &CameraBounds, distance: f32) -> f32 {
    ((distance - bounds.min_distance) / (bounds.max_distance - bounds.min_distance).max(1.0)).clamp(0.0, 1.0)
}
