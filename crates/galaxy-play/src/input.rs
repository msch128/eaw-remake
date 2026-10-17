//! Mouse interaction: click and box selection of fleets, drag-and-drop and right-click orders.

use std::f32::consts::FRAC_PI_2;

use bevy::prelude::*;
use bevy::ui::FocusPolicy;
use bevy::window::PrimaryWindow;
use galaxy_sim::{FactionId, FleetId, PlanetId};

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

/// Set by the host (with [`PlayCursorBlocked`]) when the block only comes from its interface under the cursor, not from a menu, another
/// screen or a loading galaxy. A fleet drag in progress keeps going over the interface: it only ends on release (no order while over
/// the interface), Esc or a right click.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct PlayCursorOverUi(pub bool);

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
    /// Fleets hanging at the cursor while a drag is in progress (the dragged fleet and the rest of the selection).
    pub carry: Vec<FleetId>,
    /// Point of the map plane under the cursor while a fleet is dragged.
    pub cursor_ground: Option<Vec3>,
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

/// The planet under `cursor`: the nearest planet centre on screen within its reach (the drawn orbit ring with a margin, at least
/// 36 px). Only pickable planets count ([`GalaxyLayout::pickable`]): `Icon_Republic_Rendili_WW` stands 15 units above Rendili, so
/// on the upper half of Rendili's globe the icon's centre was the nearer one and a fleet dropped there was sent to the icon.
pub fn pick_planet(cursor: Vec2, layout: &GalaxyLayout, screen: impl Fn(Vec3) -> Option<Vec2>, right: Vec3) -> Option<PlanetId> {
    pick_nearest(
        cursor,
        layout.positions.iter().enumerate().filter(|(i, _)| layout.pickable.get(*i).copied().unwrap_or(true)).filter_map(|(i, &c)| {
            let a = screen(c)?;
            let r_world = (layout.radii[i] * 1.6).max(layout.orbit_radius(i as PlanetId) * 1.25);
            let b = screen(c + right * r_world)?;
            Some((i as PlanetId, a, (a.distance(b) * 1.25).max(36.0)))
        }),
    )
}

/// Whether the fleet pads of a planet owned by `owner` take a dropped fleet of `me` into the chosen slot: own and allied planets (the
/// player's fleets park in their slots there), not enemy or neutral ones (there the fleet attacks or takes the first free slot).
pub fn pads_open(owner: Option<FactionId>, me: FactionId, hostile: impl Fn(FactionId) -> bool) -> bool {
    owner.is_some_and(|o| o == me || !hostile(o))
}

/// The fleet pad `(planet, slot 1..=3)` a carried fleet goes into when it is dropped now: the pad under the cursor of an own or allied
/// planet.
pub fn drop_pad(state: &SimState, me: Option<FactionId>, hover: &PlayHover) -> Option<(PlanetId, u8)> {
    let (me, planet, slot) = (me?, hover.planet?, hover.slot?);
    let g = state.galaxy();
    let owner = g.planets.get(planet as usize)?.owner;
    pads_open(owner, me, |o| g.hostile(&state.world, me, o)).then_some((planet, slot))
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
    drag: Res<DragState>,
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
            entities.0.iter().filter(|(id, _)| !drag.carry.contains(id)).filter_map(|(&id, &e)| {
                let t = transforms.get(e).ok()?.translation;
                let center = Vec3::new(t.x, t.y - layout.unit * 0.1, t.z);
                let p = camera.world_to_viewport(cam_tf, center).ok()?;
                let edge = camera.world_to_viewport(cam_tf, center + right * ring * 1.2).ok()?;
                Some((id, p, (p.distance(edge) * 1.1).max(FLEET_PICK_PIXELS)))
            }),
        );
        // The planet under the cursor is always known (also with a fleet icon next to it): orders need it as their target.
        let planet = pick_planet(cursor, &layout, |p| camera.world_to_viewport(cam_tf, p).ok(), *right);
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
                    let edge = camera.world_to_viewport(cam_tf, at + right * layout.pad_radius(p)).ok()?;
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
    (blocked, over_host_ui): (Res<PlayCursorBlocked>, Res<PlayCursorOverUi>),
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

    // A menu, another screen or a loading galaxy ends every gesture. The interface under the cursor does not: a fleet carried over the
    // command bar on its way to a planet stays at the cursor (it only ignores new presses there).
    let modal = blocked.0 && !(over_host_ui.0 && drag.left.is_some());
    // Esc or a right click puts a carried fleet back; the right click gives no order.
    let cancel = drag.dragging && (keys.just_pressed(KeyCode::Escape) || buttons.just_pressed(MouseButton::Right));
    if modal || cancel {
        // The route hint belongs to the drag (it stayed on screen after an aborted one).
        if drag.dragging && model.route_hint.is_some() {
            model.route_hint = None;
        }
        drag.left = None;
        drag.right = None;
        drag.dragging = false;
        drag.box_rect = None;
        drag.preview = None;
        drag.carry.clear();
        drag.cursor_ground = None;
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
                    // The group hangs at the cursor: the dragged fleet and the rest of the selection.
                    let mut carry: Vec<FleetId> = selection.fleets.iter().copied().filter(|&f| mine(f) && f != src_fid).collect();
                    carry.insert(0, src_fid);
                    drag.carry = carry;
                    drag.cursor_ground = camera.viewport_to_world(cam_tf, cursor).ok().and_then(|ray| ray.intersect_plane(Vec3::ZERO, InfinitePlane3d::new(Vec3::Y)).map(|d| ray.get_point(d)));
                    // Drag-and-drop order: plan the route to the planet under the cursor (none while the cursor is over the interface).
                    drag.box_rect = None;
                    let target_fleet = hover.fleet.filter(|&f| f != src_fid && !over_ui);
                    let target = hover.planet.filter(|_| !over_ui).or_else(|| target_fleet.and_then(|fid| g.fleets.get(&fid)).and_then(|f| f.planet()));
                    model.route_hint = None;
                    drag.preview = None;
                    if let Some(to) = target {
                        let from = g.fleets.get(&src_fid).and_then(|f| f.planet());
                        let h = player.0;
                        // Over a fleet pad of an own or allied planet the drop goes into that slot (at the planet it is at: at once), also
                        // when another fleet's pick area reaches over it: the pads sit a few pixels apart when zoomed out.
                        let pad = drop_pad(&state, player.0, &hover).filter(|&(p, _)| p == to).map(|(_, s)| s);
                        if let (None, Some(_dst_fid), true) = (pad, target_fleet, target_fleet.is_some_and(mine)) {
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
                            // A slot that holds an own fleet merges the drop into it.
                            let merges = |s: u8| h.and_then(|h| g.fleet_in_slot(h, to, s)).is_some_and(|f| f != src_fid);
                            if let (Some(s), true) = (pad, from == Some(to)) {
                                let what = if merges(s) { "Merge into fleet slot" } else { "Fleet slot" };
                                model.route_hint = Some(format!("{what} {s} at {}", state.world.planets[to as usize].display_name));
                            } else if let Some(plan) = plan {
                                let names: Vec<&str> = plan.path.iter().map(|&p| state.world.planets[p as usize].display_name.as_str()).collect();
                                let slot = pad.map(|s| format!(" > {} {s}", if merges(s) { "merge into fleet slot" } else { "fleet slot" })).unwrap_or_default();
                                let mut hint = format!("{}{slot} - ETA {}", names.join(" > "), format_duration(plan.eta_seconds, state.world.config.week_seconds));
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
        drag.carry.clear();
        drag.cursor_ground = None;
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
        } else if press.fleet.is_some_and(mine) && !over_ui {
            // (Released over the interface the carried fleet goes back without an order: no box either, so nothing below runs.)
            let src_fid = press.fleet.unwrap();
            let target_fleet = hover.fleet.filter(|&f| f != src_fid);
            let target_planet = hover.planet.or_else(|| target_fleet.and_then(|fid| g.fleets.get(&fid)).and_then(|f| f.planet()));
            if let Some((to, slot)) = drop_pad(&state, player.0, &hover) {
                // Dropped on a fleet pad of an own or allied planet: the dragged fleet (and the rest of the selection) goes into that slot,
                // at once when it is at that planet (group 1 to group 3), on arrival otherwise, and merges into the fleet holding it. The
                // pad wins over a fleet whose pick area reaches over it (the pads sit a few pixels apart when zoomed out). Enemy and
                // neutral planets have no pads to drop on: the fleet is sent there.
                let mut sel: Vec<FleetId> = selection.fleets.iter().copied().filter(|&f| mine(f)).collect();
                if !sel.contains(&src_fid) {
                    sel.push(src_fid);
                }
                actions.write(PlayAction::MoveToSlot { fleets: sel, to, slot });
            } else if let Some(dst_fid) = target_fleet.filter(|&f| mine(f)) {
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
    state: Res<SimState>,
    player: Res<PlayerFaction>,
    time: Res<Time>,
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
    // The drop target: two pulsing rings, green on an own planet, red on an enemy one, yellow otherwise; red and flat when the fleet
    // has no route there.
    if drag.dragging
        && !drag.carry.is_empty()
        && let Some(p) = hover.planet
    {
        let c = layout.positions[p as usize];
        let owner = state.galaxy().planets.get(p as usize).and_then(|pl| pl.owner);
        let reachable = drag.preview.as_ref().is_some_and(|(to, _)| *to == p) || drag.carry.iter().any(|f| state.galaxy().fleets.get(f).is_some_and(|fl| fl.planet() == Some(p)));
        let color = match owner {
            _ if !reachable => Color::srgb(1.0, 0.25, 0.2),
            o if o.is_some() && o == player.0 => Color::srgb(0.4, 1.0, 0.6),
            Some(_) => Color::srgb(1.0, 0.35, 0.3),
            None => Color::srgb(1.0, 0.85, 0.2),
        };
        let pulse = if reachable { 1.0 + 0.06 * (time.elapsed_secs() * 6.0).sin() } else { 1.0 };
        let iso = Isometry3d::new(c, Quat::from_rotation_x(-FRAC_PI_2));
        gizmos.circle(iso, layout.radii[p as usize] * 1.3 * pulse, color);
        gizmos.circle(iso, layout.radii[p as usize] * 1.55 * pulse, color.with_alpha(0.45));
        // On an own or allied planet its three fleet pads are drop targets: all outlined, the one under the cursor lit like a selected
        // fleet's rim (no Remake frame shows a drag; the look follows the pad outlines and selection rims of the map).
        let g = state.galaxy();
        if let Some(me) = player.0
            && pads_open(owner, me, |o| g.hostile(&state.world, me, o))
        {
            let (r, pad) = (layout.orbit_radius(p), layout.pad_radius(p));
            let green = Color::srgb(0.4, 1.0, 0.6);
            let lit = 1.0 + 0.08 * (time.elapsed_secs() * 4.0).sin();
            for s in 0..3u8 {
                let a = crate::visuals::slot_angle(s as usize);
                let at = Isometry3d::new(c + Vec3::new(a.cos() * r, 0.0, a.sin() * r), Quat::from_rotation_x(-FRAC_PI_2));
                if hover.slot == Some(s + 1) {
                    // Three rings read as one bright band (gizmo lines are a pixel wide); the carried fleet's own rings give way to it.
                    gizmos.circle(at, pad * lit, Color::WHITE);
                    gizmos.circle(at, pad * 1.15 * lit, green);
                    gizmos.circle(at, pad * 1.3 * lit, green.with_alpha(0.5));
                } else {
                    gizmos.circle(at, pad, green.with_alpha(0.45));
                }
            }
        }
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

    #[test]
    fn a_decoration_listed_as_a_planet_never_takes_the_pointer() {
        // Rendili at the origin and its faction icon 15 units above it; the tilted camera shows height as up on the screen.
        let mut layout = GalaxyLayout {
            positions: vec![Vec3::ZERO, Vec3::new(0.0, 15.0, 0.0)],
            radii: vec![2.0, 2.0],
            unit: 2.0,
            index: Default::default(),
            pickable: vec![true, false],
        };
        let screen = |p: Vec3| Some(Vec2::new(500.0 + p.x * 10.0, 500.0 + p.z * 10.0 - p.y * 4.0));
        // On the upper half of Rendili's globe the icon's centre (60 px up) is the nearer one.
        let cursor = Vec2::new(500.0, 465.0);
        assert_eq!(pick_planet(cursor, &layout, screen, Vec3::X), Some(0));
        // Without the mask the icon took the hover: a fleet dropped there was sent to the icon.
        layout.pickable.clear();
        assert_eq!(pick_planet(cursor, &layout, screen, Vec3::X), Some(1));
    }

    #[test]
    fn fleet_pads_take_a_drop_only_on_own_and_allied_planets() {
        // Faction 1 is the player, 2 an ally (not hostile), 3 an enemy.
        let hostile = |o: FactionId| o == 3;
        assert!(pads_open(Some(1), 1, hostile));
        assert!(pads_open(Some(2), 1, hostile));
        assert!(!pads_open(Some(3), 1, hostile));
        assert!(!pads_open(None, 1, hostile));
    }
}


