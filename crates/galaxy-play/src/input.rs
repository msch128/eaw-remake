//! Mouse interaction: click and box selection of fleets, drag-and-drop and right-click orders.

use std::f32::consts::FRAC_PI_2;

use bevy::prelude::*;
use bevy::ui::FocusPolicy;
use bevy::window::PrimaryWindow;
use galaxy_sim::{FleetId, PlanetId};

use crate::geometry::{GalaxyLayout, RoutePlan, format_duration, plan_route};
use crate::state::*;
use crate::visuals::{FleetEntities, FleetVisual};

/// Marks UI panels so clicks on them do not reach the map.
#[derive(Component, Default)]
#[require(Interaction, FocusPolicy = FocusPolicy::Block)]
pub struct PlayUiBlock;

/// What the cursor is over.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq)]
pub struct PlayHover {
    /// Fleet under the cursor.
    pub fleet: Option<FleetId>,
    /// Planet under the cursor.
    pub planet: Option<PlanetId>,
    /// Fleet slot circle (`1..=3`) of that planet under the cursor: a fleet dropped there takes (or merges into) that slot.
    pub slot: Option<u8>,
}

/// Set by the host while the cursor is over its own UI or the map is not active; the plugin then
/// ignores the mouse.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct PlayCursorBlocked(pub bool);

/// The press that started a drag.
#[derive(Debug, Clone, Copy)]
struct Press {
    pos: Vec2,
    fleet: Option<FleetId>,
    shift: bool,
}

/// Input gesture state.
#[derive(Resource, Default)]
pub struct DragState {
    left: Option<Press>,
    right: Option<Vec2>,
    /// True once the left button moved far enough to count as a drag.
    pub dragging: bool,
    /// Screen rectangle of an ongoing box selection.
    pub box_rect: Option<(Vec2, Vec2)>,
    /// Route of the drag in progress.
    pub preview: Option<(PlanetId, RoutePlan)>,
}

/// Marks the UI rectangle drawn for box selection.
#[derive(Component)]
pub struct BoxSelectRect;

const DRAG_PIXELS: f32 = 6.0;
const FLEET_PICK_PIXELS: f32 = 20.0;

pub(crate) fn spawn_box_rect(mut commands: Commands) {
    commands.spawn((
        Node { position_type: PositionType::Absolute, border: UiRect::all(Val::Px(1.0)), ..default() },
        BackgroundColor(Color::srgba(0.3, 0.7, 1.0, 0.12)),
        BorderColor::all(Color::srgba(0.5, 0.85, 1.0, 0.9)),
        Visibility::Hidden,
        GlobalZIndex(5),
        BoxSelectRect,
    ));
}

/// Nearest of `points` within `max` pixels of `cursor`.
pub fn pick_nearest<T: Copy>(cursor: Vec2, points: impl Iterator<Item = (T, Vec2, f32)>) -> Option<T> {
    points
        .filter_map(|(id, p, reach)| {
            let d = p.distance(cursor);
            (d <= reach).then_some((id, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

/// Computes what is under the cursor.
#[allow(clippy::too_many_arguments)]
pub fn update_hover(
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    camera: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    layout: Res<GalaxyLayout>,
    entities: Res<FleetEntities>,
    transforms: Query<&Transform, With<FleetVisual>>,
    cfg: Res<PlayConfig>,
    mut hover: ResMut<PlayHover>,
) {
    let new = (|| {
        let cursor = window?.cursor_position()?;
        let (camera, cam_tf) = camera.iter().next()?;
        let right = cam_tf.right();
        // The hit area follows the drawn owner ring (its on-screen radius at the current zoom, with a margin) and
        // never drops below a fixed minimum, so clicking the ring works at every zoom level.
        let ring = layout.unit * cfg.fleet_icon_scale * 1.5;
        let fleet = pick_nearest(
            cursor,
            entities.0.iter().filter_map(|(&id, &e)| {
                let t = transforms.get(e).ok()?.translation;
                let center = Vec3::new(t.x, t.y - layout.unit * 0.1, t.z);
                let p = camera.world_to_viewport(cam_tf, center).ok()?;
                let edge = camera.world_to_viewport(cam_tf, center + right * ring * 1.2).ok()?;
                Some((id, p, (p.distance(edge) * 1.1).max(FLEET_PICK_PIXELS)))
            }),
        );
        // The planet under the cursor is always known (also with a fleet icon next to it): orders need it as their target.
        let planet = pick_nearest(
            cursor,
            layout.positions.iter().enumerate().filter_map(|(i, &c)| {
                let a = camera.world_to_viewport(cam_tf, c).ok()?;
                let r_world = (layout.radii[i] * 1.6).max(layout.orbit_radius(i as PlanetId) * 1.25);
                let b = camera.world_to_viewport(cam_tf, c + right * r_world).ok()?;
                Some((i as PlanetId, a, (a.distance(b) * 1.25).max(36.0)))
            }),
        );
        // The three slot circles around the hovered planet (the ones `visuals` draws).
        let slot = planet.and_then(|p| {
            let c = layout.positions[p as usize];
            let r = layout.orbit_radius(p);
            pick_nearest(
                cursor,
                (0..3u8).filter_map(|s| {
                    let a = crate::visuals::slot_angle(s as usize);
                    let at = c + Vec3::new(a.cos() * r, 0.0, a.sin() * r);
                    let sp = camera.world_to_viewport(cam_tf, at).ok()?;
                    let edge = camera.world_to_viewport(cam_tf, at + right * layout.unit * 0.95).ok()?;
                    Some((s + 1, sp, sp.distance(edge).max(10.0)))
                }),
            )
        });
        Some(PlayHover { fleet, planet, slot })
    })()
    .unwrap_or_default();
    if *hover != new {
        *hover = new;
    }
}

/// Selection, box selection, drag-and-drop and right-click orders.
#[allow(clippy::too_many_arguments)]
pub fn handle_mouse(
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    hover: Res<PlayHover>,
    cfg: Res<PlayConfig>,
    camera: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    blockers: Query<&Interaction, With<PlayUiBlock>>,
    blocked: Res<PlayCursorBlocked>,
    state: Res<SimState>,
    player: Res<PlayerFaction>,
    selection: Res<PlayerSelection>,
    entities: Res<FleetEntities>,
    transforms: Query<&Transform, With<FleetVisual>>,
    mut drag: ResMut<DragState>,
    mut actions: MessageWriter<PlayAction>,
    mut model: ResMut<PlayUiModel>,
) {
    let Some(cursor) = window.and_then(|w| w.cursor_position()) else { return };
    let Some((camera, cam_tf)) = camera.iter().next() else { return };
    let over_ui = blocked.0 || blockers.iter().any(|i| *i != Interaction::None);
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let g = state.galaxy();
    let mine = |id: FleetId| g.fleets.get(&id).is_some_and(|f| player.0 == Some(f.owner));

    if blocked.0 {
        drag.left = None;
        drag.right = None;
        drag.dragging = false;
        drag.box_rect = None;
        drag.preview = None;
        return;
    }
    if buttons.just_pressed(MouseButton::Left) && !over_ui {
        drag.left = Some(Press { pos: cursor, fleet: hover.fleet, shift });
        drag.dragging = false;
        // Pressing a fleet selects it right away so it can be dragged.
        if let Some(f) = hover.fleet
            && !selection.fleets.contains(&f)
        {
            let mut set: Vec<FleetId> = if shift { selection.fleets.iter().copied().collect() } else { Vec::new() };
            set.push(f);
            actions.write(PlayAction::SelectFleets(set));
        }
    }
    if buttons.just_pressed(MouseButton::Right) && !over_ui {
        drag.right = Some(cursor);
    }

    if let Some(press) = drag.left {
        if !drag.dragging && press.pos.distance(cursor) > DRAG_PIXELS {
            drag.dragging = true;
        }
        if drag.dragging {
            match press.fleet.filter(|&f| mine(f)) {
                Some(src_fid) => {
                    // Drag-and-drop order: plan the route to the planet under the cursor.
                    drag.box_rect = None;
                    let target_fleet = hover.fleet.filter(|&f| f != src_fid);
                    let target = hover.planet.or_else(|| target_fleet.and_then(|fid| g.fleets.get(&fid)).and_then(|f| f.planet()));
                    model.route_hint = None;
                    drag.preview = None;
                    if let Some(to) = target {
                        let from = g.fleets.get(&src_fid).and_then(|f| f.planet());
                        let h = player.0;
                        if let (Some(_dst_fid), true) = (target_fleet, target_fleet.is_some_and(mine)) {
                            // Target is a friendly fleet -> show merge route hint!
                            let dst_name = state.world.planets[to as usize].display_name.as_str();
                            if from == Some(to) {
                                model.route_hint = Some(format!("Merge into fleet at {}", dst_name));
                            } else if let (Some(from), Some(h), Some(sf)) = (from, h, g.fleets.get(&src_fid)) {
                                if let Some(plan) = plan_route(&state.world, g, h, state.world.fleet_speed(sf), from, to) {
                                    let names: Vec<&str> = plan.path.iter().map(|&p| state.world.planets[p as usize].display_name.as_str()).collect();
                                    let mut hint = format!("{} > Merge into fleet at {} - ETA {}", names.join(" > "), dst_name, format_duration(plan.eta_seconds, state.world.config.week_seconds));
                                    if !plan.blockades.is_empty() {
                                        hint.push_str(" - blockade on the way");
                                    }
                                    model.route_hint = Some(hint);
                                    drag.preview = Some((to, plan));
                                }
                            }
                        } else {
                            let sel: Vec<FleetId> = selection.fleets.iter().copied().filter(|&f| mine(f)).collect();
                            let plan = sel.first().and_then(|id| g.fleets.get(id)).and_then(|f| {
                                let from = f.planet()?;
                                let h = player.0?;
                                plan_route(&state.world, g, h, state.world.fleet_speed(f), from, to)
                            });
                            if let Some(plan) = plan {
                                let names: Vec<&str> = plan.path.iter().map(|&p| state.world.planets[p as usize].display_name.as_str()).collect();
                                let mut hint = format!("{} - ETA {}", names.join(" > "), format_duration(plan.eta_seconds, state.world.config.week_seconds));
                                if !plan.blockades.is_empty() {
                                    hint.push_str(" - blockade on the way");
                                }
                                model.route_hint = Some(hint);
                                drag.preview = Some((to, plan));
                            }
                        }
                    }
                }
                None => drag.box_rect = Some((press.pos.min(cursor), press.pos.max(cursor))),
            }
        }
    }

    if buttons.just_released(MouseButton::Left)
        && let Some(press) = drag.left.take()
    {
        let was_dragging = std::mem::take(&mut drag.dragging);
        let rect = drag.box_rect.take();
        drag.preview = None;
        model.route_hint = None;
        if !was_dragging {
            match (hover.fleet, hover.planet) {
                (Some(f), _) => {
                    if press.shift && selection.fleets.contains(&f) {
                        let set: Vec<FleetId> = selection.fleets.iter().copied().filter(|&x| x != f).collect();
                        actions.write(PlayAction::SelectFleets(set));
                    }
                    // Plain click on a fleet was already selected at press time.
                }
                (None, planet) => {
                    if !over_ui && (planet.is_some() || !press.shift) {
                        actions.write(PlayAction::SelectFleets(Vec::new()));
                        if cfg.pick_planets {
                            actions.write(PlayAction::SelectPlanet(planet));
                        }
                    }
                }
            }
        } else if press.fleet.is_some_and(mine) {
            let src_fid = press.fleet.unwrap();
            let target_fleet = hover.fleet.filter(|&f| f != src_fid);
            let target_planet = hover.planet.or_else(|| target_fleet.and_then(|fid| g.fleets.get(&fid)).and_then(|f| f.planet()));
            if let Some(dst_fid) = target_fleet.filter(|&f| mine(f)) {
                let from = g.fleets.get(&src_fid).and_then(|f| f.planet());
                let to = g.fleets.get(&dst_fid).and_then(|f| f.planet());
                if from.is_some() && from == to {
                    // Both fleets already at same planet: merge immediately!
                    actions.write(PlayAction::MergeFleets { into: dst_fid, from: src_fid });
                } else if let Some(to) = to {
                    let mut sel: Vec<FleetId> = selection.fleets.iter().copied().filter(|&f| mine(f) && f != dst_fid).collect();
                    if !sel.contains(&src_fid) {
                        sel.push(src_fid);
                    }
                    actions.write(PlayAction::MoveAndMerge { fleets: sel, into: dst_fid, to });
                } else if let Some(to) = target_planet {
                    actions.write(PlayAction::MoveSelected(to));
                }
            } else if let (Some(to), Some(slot)) = (hover.planet, hover.slot) {
                // Dropped on a slot circle: the dragged fleet (and the rest of the selection) goes into that slot.
                let mut sel: Vec<FleetId> = selection.fleets.iter().copied().filter(|&f| mine(f)).collect();
                if !sel.contains(&src_fid) {
                    sel.push(src_fid);
                }
                actions.write(PlayAction::MoveToSlot { fleets: sel, to, slot });
            } else if let Some(to) = target_planet {
                actions.write(PlayAction::MoveSelected(to));
            }
        } else if let Some((lo, hi)) = rect {
            let mut set: Vec<FleetId> = if press.shift { selection.fleets.iter().copied().collect() } else { Vec::new() };
            for (&id, &e) in &entities.0 {
                let Ok(t) = transforms.get(e) else { continue };
                if let Ok(p) = camera.world_to_viewport(cam_tf, t.translation)
                    && p.cmpge(lo).all()
                    && p.cmple(hi).all()
                    && mine(id)
                    && !set.contains(&id)
                {
                    set.push(id);
                }
            }
            actions.write(PlayAction::SelectFleets(set));
        }
    }

    if buttons.just_released(MouseButton::Right)
        && let Some(pos) = drag.right.take()
        && pos.distance(cursor) <= DRAG_PIXELS
        && !selection.fleets.is_empty()
    {
        let target_planet = hover.planet.or_else(|| hover.fleet.and_then(|fid| g.fleets.get(&fid)).and_then(|f| f.planet()));
        if let Some(to) = target_planet {
            actions.write(PlayAction::MoveSelected(to));
        }
    }
}

/// Draws the box selection rectangle and the drag route preview.
pub fn draw_drag(
    mut gizmos: Gizmos,
    drag: Res<DragState>,
    layout: Res<GalaxyLayout>,
    hover: Res<PlayHover>,
    mut rect: Single<(&mut Node, &mut Visibility), With<BoxSelectRect>>,
) {
    let (node, vis) = &mut *rect;
    match drag.box_rect {
        Some((lo, hi)) => {
            node.left = Val::Px(lo.x);
            node.top = Val::Px(lo.y);
            node.width = Val::Px(hi.x - lo.x);
            node.height = Val::Px(hi.y - lo.y);
            **vis = Visibility::Inherited;
        }
        None => **vis = Visibility::Hidden,
    }
    if drag.dragging
        && let Some(p) = hover.planet
    {
        let c = layout.positions[p as usize];
        gizmos.circle(Isometry3d::new(c, Quat::from_rotation_x(-FRAC_PI_2)), layout.radii[p as usize] * 1.3, Color::srgb(0.4, 1.0, 0.6));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_nearest_selects_planet_within_extended_orbit_radius() {
        let cursor = Vec2::new(100.0, 100.0);
        let planets = vec![
            (0 as PlanetId, Vec2::new(100.0, 125.0), 30.0), // within 30px reach
            (1 as PlanetId, Vec2::new(300.0, 300.0), 20.0),
        ];
        assert_eq!(pick_nearest(cursor, planets.into_iter()), Some(0));

        // Outside reach returns None
        let far_planets = vec![
            (0 as PlanetId, Vec2::new(100.0, 150.0), 30.0),
        ];
        assert_eq!(pick_nearest(cursor, far_planets.into_iter()), None);
    }
}


