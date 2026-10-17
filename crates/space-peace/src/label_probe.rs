//! `--label-probe` (offscreen): proof that the planets in the lower part of the map keep their names and stay usable. Their names are
//! drawn below the command bar (`galaxy_map::LABEL_Z`): behind the translucent build strip they show through it like in the Remake,
//! and nothing may keep a planet whose globe shows above the bar art from being hovered, clicked or dropped on.
//!
//! Once the map has settled the probe lists every planet whose globe or name lies in the bottom 42 % of the screen (the height of the
//! command bar), with its globe, its name rectangle and whether the name shows. Then, through the real input path (a window that
//! exists only as data receives cursor moves and button presses, as in `--drag-pointer`; nothing reaches the desktop):
//! 1. still: 120 frames with the camera and the pointer still. Every frame logs the number of names shown, the largest move of a name
//!    and the names in the lower part with their positions; a name that shows, hides or moves by more than half a pixel between two
//!    frames is a toggle.
//! 2. per listed planet: the pointer goes to the first of a few points on the globe that is not over the bar art and has no fleet over
//!    it (a click on a fleet selects the fleet), and rests there for 12 frames: the planet must be hovered in every one of them and the
//!    hover card must name it; a click must select it; the own fleet of the start planet is dragged onto it and the drop must send the
//!    fleet there. A planet whose globe is all behind the bar art is logged as such.
//! 3. zoom: 120 frames of slow zoom out (8 % of the zoom range) and 120 back in. Every frame logs the names shown and those in the
//!    lower part with their positions. A frame in which most names come or go at once is the far-zoom threshold passing; it may happen
//!    once on the way out and once on the way back. Apart from it a name that changes between shown and hidden (its planet entering or
//!    leaving the view) more than once on the way out or on the way back, or between below and above its globe, blinks.
//!
//! In every still and zoom frame the selected planet must show its name (at any zoom, also past `galaxy_map::NAMES_HIDE_FROM`). The
//! verdict is one log line `label-probe: ... PASS|FAIL`.

use bevy::input::ButtonState;
use bevy::input::InputSystems;
use bevy::input::mouse::MouseButtonInput;
use bevy::prelude::*;
use bevy::window::{CursorMoved, PrimaryWindow, WindowResolution};
use galaxy_play::{FleetEntities, PlayAction, PlayCursorBlocked, PlayHover, PlayerFaction, PlayerSelection, SimState};
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
/// Frames from a camera move to the names it gives (camera transform, then the label layout).
const ZOOM_LAG: u32 = 3;
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

/// A planet of the lower part of the screen.
#[derive(Clone)]
struct Target {
    map: usize,
    sim: PlanetId,
    name: String,
    /// The point of the globe the pointer uses (the first one not over the bar art), once found.
    point: Option<Vec2>,
    result: String,
    pass: bool,
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
    Zoom(u32),
    Done,
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
    /// Frames out [0] and back in [1] in which most names came or went at once (the far-zoom threshold).
    threshold_passes: [u32; 2],
    zoom_from: f32,
    zoom_to: f32,
    /// Still and zoom frames in which the selected planet, on screen, showed no name.
    selected_unnamed: u32,
    /// The first globe point of the current planet that is free of the bar art but has a fleet over it.
    fallback: Option<Vec2>,
    hover_ok: u32,
    /// Per hover frame: the map's hovered planet, galaxy-play's, whether the cursor is blocked and whether the hover card names it.
    hover_seen: Vec<(Option<usize>, Option<PlanetId>, bool, bool)>,
    drag_from: Vec2,
    orders: Vec<PlayAction>,
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
    (state, player, play_selection, entities): (Res<SimState>, Res<PlayerFaction>, Res<PlayerSelection>, Res<FleetEntities>),
    (blocked, play_hover, hover, selection, mut hold): (Res<PlayCursorBlocked>, Res<PlayHover>, Res<Hover>, Res<Selection>, ResMut<ScriptHold>),
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
                    "label-probe: planet {} globe ({:.0}, {:.0}) r {:.0} name {}",
                    p.display_name,
                    a.x,
                    a.y,
                    r,
                    match name {
                        Some((pos, above)) => format!("shown at ({:.0}, {:.0}) {}", pos.x, pos.y, if above { "above the globe" } else { "below the globe" }),
                        None => "hidden".into(),
                    }
                );
                st.targets.push(Target { map: i, sim, name: p.display_name.clone(), point: None, result: String::new(), pass: false });
            }
            info!("label-probe: {} planets in the bottom 42 % (from y {:.0}), fleet {:?}, zoom {:.3}", st.targets.len(), lower, st.fleet, zoom_fraction(&bounds, galaxy_camera.distance));
            hold.active = true;
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
                st.zoom_from = galaxy_camera.distance;
                let z = zoom_fraction(&bounds, galaxy_camera.distance);
                st.zoom_to = bounds.min_distance + (bounds.max_distance - bounds.min_distance) * (z + ZOOM_SPAN).min(1.0);
                st.shown_changes = [vec![0; n], vec![0; n]];
                st.side_changes = [vec![0; n], vec![0; n]];
                st.last = Some(names);
                st.step = Step::Zoom(0);
                return;
            };
            let Some((a, r)) = globe(target.map) else {
                st.targets[k].result = "globe not on screen".into();
                st.step = Step::Aim(k + 1, 0, 0);
                return;
            };
            // Points on the globe: the centre, then up toward the top edge and to the sides (the bar covers the lower part first). A
            // point must not be over the bar art, and no fleet may be under it (a click there selects the fleet, not the planet).
            let candidates: Vec<Vec2> = [
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
            .map(|o| a + *o * r)
            .filter(|p| p.x > 8.0 && p.y > 8.0 && p.x < viewport.x - 8.0 && p.y < viewport.y - 8.0)
            .collect();
            if c == 0 && t == 0 {
                st.fallback = None;
            }
            let Some(&point) = candidates.get(c) else {
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
            } else if blocked.0 {
                st.step = Step::Aim(k, c + 1, 0);
            } else if play_hover.fleet.is_some() || hover.0 != Some(target.map) {
                if st.fallback.is_none() {
                    st.fallback = Some(point);
                }
                st.step = Step::Aim(k, c + 1, 0);
            } else {
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
            let now = (hover.0, play_hover.planet, blocked.0, card);
            if t >= 2 {
                if now.0 == Some(target.map) && !now.2 && now.3 {
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
                    play_hover.fleet,
                    if card { "names it" } else { "does not name it" }
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
            if t == 0 {
                let at = entities.0.get(&fleet).and_then(|&e| fleets.get(e).ok()).and_then(|g| cam.world_to_viewport(cam_tf, g.translation()).ok());
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
            let press = SETTLE;
            let release = press + DRAG_STEPS + DRAG_REST;
            if t == press {
                buttons.write(MouseButtonInput { button: MouseButton::Left, state: ButtonState::Pressed, window: win_entity });
            }
            if t > press && t <= press + DRAG_STEPS {
                let p = st.drag_from.lerp(point, (t - press) as f32 / DRAG_STEPS as f32);
                move_to(p);
            }
            if t == release {
                buttons.write(MouseButtonInput { button: MouseButton::Left, state: ButtonState::Released, window: win_entity });
            }
            if t > release {
                st.orders.extend(new_orders.into_iter().filter(|a| !matches!(a, PlayAction::SelectFleets(_) | PlayAction::SelectPlanet(_) | PlayAction::SetSpeedLevel(_))));
            }
            if t >= release + DROP_WAIT {
                let ok = st.orders.iter().any(|o| crate::drag_demo::drops_on(o, target.sim, None));
                let left = state.galaxy().fleets.get(&fleet).map(|f| match f.location {
                    FleetLocation::AtPlanet(p) => format!("at planet {p}"),
                    FleetLocation::Moving { to, .. } => format!("moving to {to}"),
                });
                let note = format!(", drop {} (orders {:?}, fleet {:?})", if ok { "PASS" } else { "FAIL" }, st.orders, left);
                st.targets[k].result += &note;
                st.targets[k].pass &= ok;
                let t = &st.targets[k];
                info!("label-probe: {}: {}", t.name, t.result);
                st.step = Step::Aim(k + 1, 0, 0);
            } else {
                st.step = Step::Drag(k, t + 1);
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
            // On the way out and on the way back a name may come or go once (its planet enters or leaves the view) and change its side
            // once, and the far-zoom threshold may pass once; more is a blink.
            let mut blinks: Vec<String> = (0..n)
                .filter(|&i| (0..2).any(|h| st.shown_changes[h][i] > 1 || st.side_changes[h][i] > 1))
                .map(|i| {
                    let p = &view.planets[i];
                    format!("{} (shown/hidden {} out {} back, below/above {} out {} back)", p.display_name, st.shown_changes[0][i], st.shown_changes[1][i], st.side_changes[0][i], st.side_changes[1][i])
                })
                .collect();
            if st.threshold_passes.iter().any(|&c| c > 1) {
                blinks.push(format!("all names ({} threshold passes out, {} back)", st.threshold_passes[0], st.threshold_passes[1]));
            }
            let tested = st.targets.iter().filter(|t| t.point.is_some()).count();
            let failed: Vec<&str> = st.targets.iter().filter(|t| !t.pass).map(|t| t.name.as_str()).collect();
            let ok = st.still_toggles == 0 && blinks.is_empty() && failed.is_empty() && !st.targets.is_empty() && st.selected_unnamed == 0;
            info!(
                "label-probe: {} planets in the lower part, {tested} with a usable globe point, failed {:?}, still toggles {}, zoom blinks {:?}, threshold passes {} out {} back, selected unnamed in {} frames: {}",
                st.targets.len(),
                failed,
                st.still_toggles,
                blinks,
                st.threshold_passes[0],
                st.threshold_passes[1],
                st.selected_unnamed,
                if ok { "PASS" } else { "FAIL" }
            );
            for t in st.targets.iter().filter(|t| t.point.is_none()) {
                info!("label-probe: {}: {}", t.name, t.result);
            }
            hold.active = false;
            hold.done_at = Some(std::time::Instant::now());
            st.step = Step::Done;
        }
        Step::Done => {}
    }
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
