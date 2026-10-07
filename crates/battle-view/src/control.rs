//! Selection, orders and the overlays that go with them (EaW style): left click / drag box selects
//! own ships, right click moves, attacks a ship or a single hardpoint of an enemy ship.

use battle_sim::{Command, HpKind};
use bevy::input::ButtonInput;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::camera::BattleCamera;
use crate::convert::{bevy_to_sim, sim_dir_to_bevy};
use crate::ships::{RenderedShips, ShipRender};
use crate::visuals::is_markable;
use crate::{BattleRuntime, BattleViewSettings};

/// Currently selected ships (own side only) and the enemy whose hardpoints are shown.
#[derive(Resource, Default, Debug, Clone)]
pub struct Selection {
    /// Selected own ships.
    pub ships: Vec<u32>,
    /// Enemy ship selected for hardpoint targeting.
    pub enemy: Option<u32>,
}

/// Kind of order the player gave (for unit responses).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderKind {
    /// Move.
    Move,
    /// Attack or attack-move.
    Attack,
    /// Stop.
    Stop,
}

/// The player gave an order; `lead` is the first ordered ship.
#[derive(Message, Debug, Clone, Copy)]
pub struct OrderIssued {
    /// What was ordered.
    pub kind: OrderKind,
    /// First ordered ship.
    pub lead: u32,
}

/// Control groups (Ctrl + digit stores, digit recalls).
#[derive(Resource, Default, Debug, Clone)]
pub struct ControlGroups(pub [Vec<u32>; 10]);

/// Mouse state in logical window pixels. Filled from the window unless
/// [`BattleViewSettings::synthetic_pointer`] is set, in which case callers (tests, offscreen
/// scripts) write it themselves.
#[derive(Resource, Default, Debug, Clone)]
pub struct Pointer {
    /// Cursor position, `None` when outside the window.
    pub cursor: Option<Vec2>,
    /// Left button is down.
    pub left_down: bool,
    /// Left button went down this frame.
    pub left_pressed: bool,
    /// Left button went up this frame.
    pub left_released: bool,
    /// Right button went down this frame.
    pub right_pressed: bool,
    /// Shift is held.
    pub shift: bool,
}

/// What is under the cursor.
#[derive(Resource, Default, Debug, Clone)]
pub struct Hover {
    /// Ship under the cursor (any side).
    pub ship: Option<u32>,
    /// Hardpoint of the enemy ship (hovered or selected) closest to the cursor.
    pub hardpoint: Option<(u32, u16)>,
    /// Point on the battle plane under the cursor (Bevy world space).
    pub ground: Option<Vec3>,
}

#[derive(Resource, Default)]
pub struct Drag {
    start: Option<Vec2>,
}

/// Marker flashes where orders were given.
#[derive(Resource, Default)]
pub struct OrderMarkers(Vec<(Vec3, f32, Color)>);

/// Reads the mouse from the primary window.
pub fn read_pointer(
    settings: Res<BattleViewSettings>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    mut pointer: ResMut<Pointer>,
) {
    if settings.synthetic_pointer {
        return;
    }
    pointer.cursor = window.as_ref().and_then(|w| w.cursor_position());
    pointer.left_down = buttons.pressed(MouseButton::Left);
    pointer.left_pressed = buttons.just_pressed(MouseButton::Left);
    pointer.left_released = buttons.just_released(MouseButton::Left);
    pointer.right_pressed = buttons.just_pressed(MouseButton::Right);
    pointer.shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
}

/// Screen-space description of a ship for picking.
struct Projected {
    id: u32,
    screen: Vec2,
    radius_px: f32,
}

fn project_ships(
    camera: &Camera,
    gt: &GlobalTransform,
    ships: &RenderedShips,
    scale: f32,
) -> Vec<Projected> {
    let right = gt.right();
    let mut out = Vec::with_capacity(ships.0.len());
    for (&id, s) in &ships.0 {
        let Ok(c) = camera.world_to_viewport(gt, s.pos) else {
            continue;
        };
        let edge = camera
            .world_to_viewport(gt, s.pos + right * s.radius * 0.8)
            .map_or(8.0, |e| (e - c).length());
        out.push(Projected {
            id,
            screen: c,
            radius_px: edge.max(9.0 * scale),
        });
    }
    out
}

fn pick(projected: &[Projected], cursor: Vec2, accept: impl Fn(u32) -> bool) -> Option<u32> {
    projected
        .iter()
        .filter(|p| accept(p.id))
        .map(|p| (p.id, (p.screen - cursor).length() / p.radius_px))
        .filter(|(_, d)| *d <= 1.0)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

/// World position of a hardpoint of a rendered ship.
pub fn hardpoint_world(rt: &BattleRuntime, s: &ShipRender, hp: usize) -> Vec3 {
    let def = rt.catalog.ship(s.def);
    let off = def.hardpoints[hp].offset.rotate_yaw(s.yaw);
    s.pos + sim_dir_to_bevy(off)
}

/// Hover detection, click selection and box selection.
#[allow(clippy::too_many_arguments)]
pub fn selection(
    rt: Res<BattleRuntime>,
    settings: Res<BattleViewSettings>,
    pointer: Res<Pointer>,
    ships: Res<RenderedShips>,
    keys: Res<ButtonInput<KeyCode>>,
    camera: Single<(&Camera, &GlobalTransform), With<BattleCamera>>,
    mut sel: ResMut<Selection>,
    mut groups: ResMut<ControlGroups>,
    mut hover: ResMut<Hover>,
    mut drag: ResMut<Drag>,
) {
    let (cam, gt) = *camera;
    let player = settings.player_side;
    // Drop dead ships from the selection.
    sel.ships.retain(|id| ships.0.contains_key(id));
    if sel.enemy.is_some_and(|id| !ships.0.contains_key(&id)) {
        sel.enemy = None;
    }
    *hover = Hover::default();
    let Some(cursor) = pointer.cursor else {
        return;
    };
    let scale = settings.ui_scale;
    let projected = project_ships(cam, gt, &ships, scale);
    hover.ship = pick(&projected, cursor, |_| true);
    if let Ok(ray) = cam.viewport_to_world(gt, cursor)
        && let Some(d) = ray.intersect_plane(Vec3::ZERO, InfinitePlane3d::new(Vec3::Y))
    {
        hover.ground = Some(ray.get_point(d));
    }

    // Hardpoints of the enemy under the cursor (or the selected enemy).
    let enemy = hover
        .ship
        .filter(|id| ships.0.get(id).is_some_and(|s| s.side != player))
        .or(sel.enemy);
    if let Some(eid) = enemy
        && !sel.ships.is_empty()
        && let Some(s) = ships.0.get(&eid)
    {
        let def = rt.catalog.ship(s.def);
        let mut best: Option<(u16, f32)> = None;
        for (i, h) in def.hardpoints.iter().enumerate() {
            if !h.targetable || !h.destroyable || !is_markable(h.kind) {
                continue;
            }
            if !s.hp_alive.get(i).copied().unwrap_or(false) {
                continue;
            }
            let w = hardpoint_world(&rt, s, i);
            if let Ok(p) = cam.world_to_viewport(gt, w) {
                let d = (p - cursor).length();
                if d < 16.0 * scale && best.is_none_or(|(_, bd)| d < bd) {
                    best = Some((i as u16, d));
                }
            }
        }
        hover.hardpoint = best.map(|(i, _)| (eid, i));
    }

    // Control groups.
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    const DIGITS: [KeyCode; 10] = [
        KeyCode::Digit0,
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ];
    for (i, k) in DIGITS.iter().enumerate() {
        if keys.just_pressed(*k) {
            if ctrl {
                groups.0[i] = sel.ships.clone();
            } else {
                sel.ships = groups.0[i]
                    .iter()
                    .copied()
                    .filter(|id| ships.0.contains_key(id))
                    .collect();
            }
        }
    }
    if ctrl && keys.just_pressed(KeyCode::KeyA) {
        sel.ships = ships
            .0
            .iter()
            .filter(|(_, s)| s.side == player && !rt.catalog.ship(s.def).is_static)
            .map(|(id, _)| *id)
            .collect();
    }

    // Click / drag selection.
    if pointer.left_pressed {
        drag.start = Some(cursor);
    }
    if pointer.left_released
        && let Some(start) = drag.start.take()
    {
        let moved = (cursor - start).length() > 5.0 * scale;
        let own = |id: u32| ships.0.get(&id).is_some_and(|s| s.side == player);
        let picked: Vec<u32> = if moved {
            let (lo, hi) = (start.min(cursor), start.max(cursor));
            projected
                .iter()
                .filter(|p| {
                    own(p.id)
                        && p.screen.x >= lo.x
                        && p.screen.x <= hi.x
                        && p.screen.y >= lo.y
                        && p.screen.y <= hi.y
                })
                .map(|p| p.id)
                .collect()
        } else {
            pick(&projected, cursor, own).into_iter().collect()
        };
        if moved || !picked.is_empty() {
            if pointer.shift {
                for id in picked {
                    if !sel.ships.contains(&id) {
                        sel.ships.push(id);
                    }
                }
            } else {
                sel.ships = picked;
            }
        } else if let Some(e) = hover
            .ship
            .filter(|id| ships.0.get(id).is_some_and(|s| s.side != player))
        {
            // Clicking an enemy marks it for hardpoint targeting; clicking empty space deselects.
            sel.enemy = Some(e);
        } else if !pointer.shift {
            sel.ships.clear();
            sel.enemy = None;
        }
    }
    if !pointer.left_down {
        drag.start = None;
    }
}

/// Right click orders and the stop key.
#[allow(clippy::too_many_arguments)]
pub fn orders(
    rt: Res<BattleRuntime>,
    settings: Res<BattleViewSettings>,
    pointer: Res<Pointer>,
    keys: Res<ButtonInput<KeyCode>>,
    ships: Res<RenderedShips>,
    sel: Res<Selection>,
    hover: Res<Hover>,
    mut markers: ResMut<OrderMarkers>,
    mut issued: MessageWriter<OrderIssued>,
) {
    let player = settings.player_side;
    let mine: Vec<u32> = sel
        .ships
        .iter()
        .copied()
        .filter(|id| {
            ships
                .0
                .get(id)
                .is_some_and(|s| s.side == player && !rt.catalog.ship(s.def).is_static)
        })
        .collect();
    if mine.is_empty() {
        return;
    }
    let lead = mine[0];
    if keys.just_pressed(KeyCode::KeyS) && !keys.pressed(KeyCode::ControlLeft) {
        rt.sim.command(player, Command::Stop { ships: mine.clone() });
        issued.write(OrderIssued { kind: OrderKind::Stop, lead });
    }
    if !pointer.right_pressed {
        return;
    }
    if let Some((target, hp)) = hover.hardpoint {
        rt.sim.command(
            player,
            Command::AttackHardpoint {
                ships: mine,
                target,
                hardpoint: hp,
            },
        );
        issued.write(OrderIssued { kind: OrderKind::Attack, lead });
        if let Some(s) = ships.0.get(&target) {
            markers.0.push((s.pos, 1.0, Color::srgb(1.0, 0.25, 0.2)));
        }
    } else if let Some(target) = hover
        .ship
        .filter(|id| ships.0.get(id).is_some_and(|s| s.side != player))
    {
        rt.sim.command(player, Command::Attack { ships: mine, target });
        issued.write(OrderIssued { kind: OrderKind::Attack, lead });
        if let Some(s) = ships.0.get(&target) {
            markers.0.push((s.pos, 1.0, Color::srgb(1.0, 0.25, 0.2)));
        }
    } else if let Some(g) = hover.ground {
        let to = bevy_to_sim(g);
        if pointer.shift {
            rt.sim.command(player, Command::AttackMove { ships: mine, to });
            issued.write(OrderIssued { kind: OrderKind::Attack, lead });
        } else {
            rt.sim.command(player, Command::Move { ships: mine, to });
            issued.write(OrderIssued { kind: OrderKind::Move, lead });
        }
        markers.0.push((g, 1.2, Color::srgb(0.3, 1.0, 0.4)));
    }
}

fn hp_color(kind: HpKind) -> Color {
    match kind {
        HpKind::Weapon => Color::srgb(1.0, 0.45, 0.15),
        HpKind::ShieldGenerator => Color::srgb(0.35, 0.65, 1.0),
        HpKind::Engine => Color::srgb(1.0, 0.9, 0.3),
        HpKind::FighterBay => Color::srgb(0.4, 1.0, 0.9),
        HpKind::TractorBeam => Color::srgb(0.8, 0.5, 1.0),
        _ => Color::srgb(0.8, 0.8, 0.8),
    }
}

/// Draws selection circles, bars, hardpoint markers and the drag box.
#[allow(clippy::too_many_arguments)]
pub fn overlays(
    time: Res<Time>,
    rt: Res<BattleRuntime>,
    settings: Res<BattleViewSettings>,
    pointer: Res<Pointer>,
    ships: Res<RenderedShips>,
    sel: Res<Selection>,
    hover: Res<Hover>,
    drag: Res<Drag>,
    mut markers: ResMut<OrderMarkers>,
    camera: Single<(&Camera, &GlobalTransform, &Projection), With<BattleCamera>>,
    mut gizmos: Gizmos,
) {
    let (cam, gt, projection) = *camera;
    let player = settings.player_side;
    let fov = match projection {
        Projection::Perspective(p) => p.fov,
        _ => 0.8,
    };
    let vp_h = cam.logical_viewport_size().map_or(1080.0, |s| s.y).max(1.0);
    let (right, up, fwd) = (gt.right().as_vec3(), gt.up().as_vec3(), gt.forward().as_vec3());
    let scale = settings.ui_scale;
    let wpp = |pos: Vec3| 2.0 * (pos - gt.translation()).dot(fwd).max(1.0) * (fov * 0.5).tan() / vp_h;
    let flat = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);

    let ring = |gizmos: &mut Gizmos, s: &ShipRender, color: Color, k: f32| {
        gizmos
            .circle(Isometry3d::new(s.pos, flat), s.radius * k, color)
            .resolution(56);
    };
    let bars = |gizmos: &mut Gizmos, s: &ShipRender| {
        let def = rt.catalog.ship(s.def);
        let w = wpp(s.pos);
        let half = (s.radius * 0.9).clamp(24.0 * w * scale, 90.0 * w * scale);
        let center = s.pos + up * (s.radius * 0.55 + 10.0 * w * scale) + right * 0.0;
        let thick = 2.0 * w * scale;
        let mut bar = |row: f32, frac: f32, fill: Color| {
            let o = center + up * (row * thick * 2.4);
            let a = o - right * half;
            let b = o + right * half;
            let e = a + (b - a) * frac.clamp(0.0, 1.0);
            let dark = Color::srgba(0.0, 0.0, 0.0, 0.85);
            // Border lines, the empty part of the bar, then the filled part (never overlapping).
            for k in [-2, 2] {
                gizmos.line(a + up * (k as f32 * thick * 0.5), b + up * (k as f32 * thick * 0.5), dark);
            }
            for k in -1..=1 {
                let off = up * (k as f32 * thick * 0.5);
                gizmos.line(e + off, b + off, dark);
                gizmos.line(a + off, e + off, fill);
            }
        };
        let hull_color = if s.hull > 0.5 {
            Color::srgb(0.25, 0.95, 0.3)
        } else if s.hull > 0.25 {
            Color::srgb(1.0, 0.85, 0.2)
        } else {
            Color::srgb(1.0, 0.25, 0.2)
        };
        bar(0.0, s.hull, hull_color);
        if def.shield > 0.0 {
            bar(1.0, s.shield, Color::srgb(0.3, 0.7, 1.0));
        }
    };

    for id in &sel.ships {
        if let Some(s) = ships.0.get(id) {
            ring(&mut gizmos, s, Color::srgb(0.2, 1.0, 0.35), 1.05);
            bars(&mut gizmos, s);
        }
    }
    if let Some(s) = hover.ship.and_then(|id| ships.0.get(&id)) {
        let enemy = s.side != player;
        if !sel.ships.contains(&s.id) {
            ring(
                &mut gizmos,
                s,
                if enemy {
                    Color::srgb(1.0, 0.2, 0.15)
                } else {
                    Color::srgba(0.7, 1.0, 0.8, 0.8)
                },
                1.0,
            );
            bars(&mut gizmos, s);
        }
    }
    if let Some(s) = sel.enemy.and_then(|id| ships.0.get(&id)) {
        ring(&mut gizmos, s, Color::srgb(1.0, 0.35, 0.2), 1.12);
        bars(&mut gizmos, s);
    }

    // Hardpoint markers on the targeted enemy while own ships are selected.
    if !sel.ships.is_empty() {
        let enemy = hover
            .ship
            .filter(|id| ships.0.get(id).is_some_and(|s| s.side != player))
            .or(sel.enemy);
        if let Some(s) = enemy.and_then(|id| ships.0.get(&id)) {
            let def = rt.catalog.ship(s.def);
            for (i, h) in def.hardpoints.iter().enumerate() {
                if !h.targetable || !h.destroyable || !is_markable(h.kind) {
                    continue;
                }
                if !s.hp_alive.get(i).copied().unwrap_or(false) {
                    continue;
                }
                let w = hardpoint_world(&rt, s, i);
                let hot = hover.hardpoint == Some((s.id, i as u16));
                let size = wpp(w) * scale * if hot { 9.0 } else { 5.0 };
                let color = if hot { Color::WHITE } else { hp_color(h.kind) };
                let pts = [
                    w + up * size,
                    w + right * size,
                    w - up * size,
                    w - right * size,
                ];
                for k in 0..4 {
                    gizmos.line(pts[k], pts[(k + 1) % 4], color);
                }
            }
        }
    }

    // Order flashes.
    let dt = time.delta_secs();
    markers.0.retain_mut(|m| {
        m.1 -= dt;
        m.1 > 0.0
    });
    for (p, t, c) in &markers.0 {
        let r = 60.0 + (1.2 - t) * 120.0;
        gizmos
            .circle(Isometry3d::new(*p, flat), r, c.with_alpha(t.min(1.0)))
            .resolution(40);
    }

    // Selection rectangle at a fixed depth in front of the camera.
    if let (Some(start), Some(cur)) = (drag.start, pointer.cursor)
        && (cur - start).length() > 5.0 * scale
    {
        let at = |p: Vec2| cam.viewport_to_world(gt, p).map(|r| r.get_point(20.0)).ok();
        let corners = [
            start,
            Vec2::new(cur.x, start.y),
            cur,
            Vec2::new(start.x, cur.y),
        ];
        let w: Vec<Option<Vec3>> = corners.iter().map(|c| at(*c)).collect();
        for k in 0..4 {
            if let (Some(a), Some(b)) = (w[k], w[(k + 1) % 4]) {
                gizmos.line(a, b, Color::srgb(0.3, 1.0, 0.4));
            }
        }
    }
    let _ = &rt;
}



