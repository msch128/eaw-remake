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

/// True while the cursor is over an opaque part of the interface (command bar, panels, portraits): world clicks,
/// hover and orders are ignored then. Written by [`ui_block`].
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiBlock(pub bool, pub Option<Entity>);

/// Present while the EaW overlays of `eaw-ui` draw brackets, bars and hardpoint reticles; the gizmo versions
/// of [`overlays`] are then switched off.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct OverlaysActive;

/// What the next left click does after one of the command buttons was pressed (`c_button00..02`); a right click or Esc cancels.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArmedOrder(pub Option<ArmedKind>);

/// The armed command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmedKind {
    /// Attack the enemy that is clicked.
    Attack,
    /// Attack-move to the clicked point.
    AttackMove,
    /// Move to the clicked point.
    Move,
    /// Place a reinforcement wave (handled by `tactical_ui::reinforce_place`).
    Reinforce,
    /// Pick the target point of a targeted ability (handled by `tactical_ui::ability_at`).
    AbilityAt,
}

/// Ask the camera to centre on the selection (double click on a selected ship, space bar, hero portrait).
#[derive(Message, Debug, Clone, Copy)]
pub struct FocusCamera;

/// Kind of order the player gave (for unit responses).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderKind {
    /// Move.
    Move,
    /// Attack or attack-move.
    Attack,
    /// Stop.
    Stop,
    /// Guard (hold position, shoot what comes in range).
    Guard,
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

/// Two clicks on the same ship within this time count as a double click. Estimate (the engine value is not in the data).
const DOUBLE_CLICK_SECONDS: f32 = 0.4;

/// Ships that were ordered lately and the seconds their move line stays up (selected ships always show theirs).
#[derive(Resource, Default)]
pub struct RecentOrders(pub std::collections::HashMap<u32, f32>);

/// What the move lines drew this frame (for tests): line start / end per ship and the destination rings.
#[derive(Resource, Default, Debug, Clone)]
pub struct MoveLines {
    /// Ship position and destination (Bevy space).
    pub lines: Vec<(Vec3, Vec3)>,
    /// Ring centres.
    pub rings: Vec<Vec3>,
}

/// Move marker ring radius in hull radii of the biggest ship sent there, and its limits in world units (estimates from the video frames b0641..b0643).
const RING_HULL_RADII: f32 = 4.0;
const RING_MIN: f32 = 250.0;
const RING_MAX: f32 = 3500.0;

/// Seconds the move line of an ordered ship stays up after the order when it is not selected (estimate).
const ORDER_LINE_SECONDS: f32 = 3.0;

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

/// Sets [`UiBlock`]: the cursor is inside a visible interface node that has a picture or a background. The
/// transparent full-screen layers of the world-space overlays (ship bars, hardpoint reticles) do not count, so
/// reticles stay clickable and right-click orders on them reach the world.
#[allow(clippy::type_complexity)]
pub fn ui_block(
    pointer: Res<Pointer>,
    mut block: ResMut<UiBlock>,
    nodes: Query<(Entity, &ComputedNode, &UiGlobalTransform, &InheritedVisibility, Option<&ImageNode>, Option<&BackgroundColor>, Option<&Interaction>)>,
    images: Res<Assets<Image>>,
    camera: Single<&Camera, With<BattleCamera>>,
    parents: Query<&ChildOf>,
    overlay_roots: Query<(), With<eaw_ui::TacticalOverlayRoot>>,
) {
    // Node transforms are in physical pixels of the camera target (UI scale only changes node sizes).
    let target_scale = camera.target_scaling_factor().unwrap_or(1.0);
    let mut hit = false;
    let mut by = None;
    if let Some(cursor) = pointer.cursor {
        for (e, node, gt, vis, image, bg, interaction) in &nodes {
            if !vis.get() || node.size().x <= 0.0 || node.size().y <= 0.0 {
                continue;
            }
            let visual = image.is_some() || interaction.is_some() || bg.is_some_and(|b| b.0.alpha() > 0.01);
            let point = cursor * target_scale;
            if !visual || !node.contains_point(*gt, point) {
                continue;
            }
            // Pictures count where they are not transparent (the frame art of the bar spans a big mostly empty rectangle).
            let solid = interaction.is_some() || bg.is_some_and(|b| b.0.alpha() > 0.01);
            if !solid
                && let Some(img) = image
                && img.texture_atlas.is_none()
                && img.rect.is_none()
                && let Some(tex) = images.get(&img.image)
                && let Some(n) = node.normalize_point(*gt, point)
            {
                let uv = n + Vec2::splat(0.5);
                let (w, h) = (tex.width() as f32, tex.height() as f32);
                let (x, y) = ((uv.x * w).clamp(0.0, w - 1.0) as u32, (uv.y * h).clamp(0.0, h - 1.0) as u32);
                if tex.get_color_at(x, y).is_ok_and(|c| c.alpha() < 0.06) {
                    continue;
                }
            }
            let mut cur = e;
            let mut overlay = false;
            for _ in 0..8 {
                if overlay_roots.contains(cur) {
                    overlay = true;
                    break;
                }
                match parents.get(cur) {
                    Ok(p) => cur = p.parent(),
                    Err(_) => break,
                }
            }
            if !overlay {
                hit = true;
                by = Some(e);
                break;
            }
        }
    }
    if block.0 != hit || block.1 != by {
        *block = UiBlock(hit, by);
    }
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
    for (&id, s) in ships.0.iter().filter(|(_, s)| s.visible) {
        let Ok(c) = camera.world_to_viewport(gt, s.pos) else {
            continue;
        };
        let edge = camera
            .world_to_viewport(gt, s.pos + right * s.radius)
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

/// World position of a hardpoint of a rendered ship: its bone in the loaded model, else the sim offset (the sim scales the model bones by `Scale_Factor` itself).
pub fn hardpoint_world(rt: &BattleRuntime, s: &ShipRender, hp: usize) -> Vec3 {
    if let Some(p) = s.hp_pos.get(hp).copied().flatten() {
        return p;
    }
    let def = rt.catalog.ship(s.def);
    let off = def.hardpoints[hp].offset.rotate_yaw(s.yaw);
    s.pos + sim_dir_to_bevy(off) * rt.visuals.ship(s.def).scale
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
    block: Res<UiBlock>,
    time: Res<Time>,
    mut last_click: Local<(f32, Option<u32>)>,
    mut focus: MessageWriter<FocusCamera>,
    armed: Res<ArmedOrder>,
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
    // Over the interface only a drag that started in the world may end; nothing is hovered or started.
    hover.ship = if block.0 { None } else { pick(&projected, cursor, |_| true) };
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
    let shift_held = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    for (i, k) in DIGITS.iter().enumerate() {
        if keys.just_pressed(*k) && !shift_held {
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
    if armed.0.is_some() {
        // The next click is an order, not a selection.
        drag.start = None;
    }
    if pointer.left_pressed && !block.0 && armed.0.is_none() {
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
        // Double click on a ship focuses the camera on the selection.
        let now = time.elapsed_secs();
        if !moved && let &[only] = picked.as_slice() {
            if last_click.1 == Some(only) && now - last_click.0 < DOUBLE_CLICK_SECONDS {
                focus.write(FocusCamera);
                *last_click = (0.0, None);
            } else {
                *last_click = (now, Some(only));
            }
        } else {
            *last_click = (0.0, None);
        }
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
    block: Res<UiBlock>,
    mut armed: ResMut<ArmedOrder>,
    mut recent: ResMut<RecentOrders>,
) {
    let player = settings.player_side;
    // Esc is handled by the host (space-peace battle_menu): it cancels an armed order first and only opens the menu when none is armed.
    // A command button arms an order for the next left click; a right click cancels it (also without a selection).
    if pointer.right_pressed && armed.0.is_some() {
        armed.0 = None;
        return;
    }
    let mine = selected_mobile(&rt, &ships, &sel, player);
    if mine.is_empty() {
        return;
    }
    let lead = mine[0];
    if keys.just_pressed(KeyCode::KeyS) && !keys.pressed(KeyCode::ControlLeft) {
        rt.sim.command(player, Command::Stop { ships: mine.clone() });
        issued.write(OrderIssued { kind: OrderKind::Stop, lead });
    }
    let armed_click = armed.0.filter(|_| pointer.left_released && !block.0);
    if armed_click.is_none() && (!pointer.right_pressed || block.0) {
        return;
    }
    // Every ship that gets an order shows its move line for a moment (the order reaches the sim next step, the line follows it).
    for &id in &mine {
        recent.0.insert(id, ORDER_LINE_SECONDS + 0.3);
    }
    // Any enemy under the cursor, also when the click is on the outer part of a big model (station, capital ship).
    let enemy_target = hover.ship.filter(|id| ships.0.get(id).is_some_and(|s| s.side != player)).or_else(|| {
        let g = hover.ground?;
        enemy_at(&ships, player, g)
    });
    let mark_enemy = |markers: &mut OrderMarkers, target: u32| {
        if let Some(s) = ships.0.get(&target) {
            markers.0.push((s.pos, 1.0, Color::srgb(1.0, 0.25, 0.2)));
        }
    };
    match armed_click {
        Some(ArmedKind::Attack) => {
            let Some(target) = enemy_target else { return };
            rt.sim.command(player, Command::Attack { ships: mine, target });
            issued.write(OrderIssued { kind: OrderKind::Attack, lead });
            mark_enemy(&mut markers, target);
            armed.0 = None;
        }
        Some(ArmedKind::Reinforce | ArmedKind::AbilityAt) => {}
        Some(kind) => {
            let Some(g) = hover.ground else { return };
            let to = bevy_to_sim(g);
            if kind == ArmedKind::AttackMove {
                rt.sim.command(player, Command::AttackMove { ships: mine, to });
                issued.write(OrderIssued { kind: OrderKind::Attack, lead });
            } else {
                rt.sim.command(player, Command::Move { ships: mine, to });
                issued.write(OrderIssued { kind: OrderKind::Move, lead });
            }
            markers.0.push((g, 1.2, Color::srgb(0.3, 1.0, 0.4)));
            armed.0 = None;
        }
        None => {
            // Right click: an enemy of any kind (ship, station, hardpoint) is attacked, open space is a move.
            if let Some((target, hp)) = hover.hardpoint {
                rt.sim.command(player, Command::AttackHardpoint { ships: mine, target, hardpoint: hp });
                issued.write(OrderIssued { kind: OrderKind::Attack, lead });
                mark_enemy(&mut markers, target);
            } else if let Some(target) = enemy_target {
                rt.sim.command(player, Command::Attack { ships: mine, target });
                issued.write(OrderIssued { kind: OrderKind::Attack, lead });
                mark_enemy(&mut markers, target);
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
    }
}

/// The selected ships of the player that can move (structures cannot).
pub fn selected_mobile(rt: &BattleRuntime, ships: &RenderedShips, sel: &Selection, player: u8) -> Vec<u32> {
    sel.ships
        .iter()
        .copied()
        .filter(|id| ships.0.get(id).is_some_and(|s| s.side == player && !rt.catalog.ship(s.def).is_static))
        .collect()
}

/// The enemy whose footprint on the battle plane contains `ground` (closest centre wins). Big models are only
/// picked on a sphere around their centre on screen, so a click on a station's rim would otherwise be a move order.
fn enemy_at(ships: &RenderedShips, player: u8, ground: Vec3) -> Option<u32> {
    ships
        .0
        .values()
        .filter(|s| s.side != player && s.visible)
        .map(|s| (s.id, Vec2::new(s.pos.x - ground.x, s.pos.z - ground.z).length() / (s.radius * ENEMY_CLICK_RADIUS).max(1.0)))
        .filter(|(_, d)| *d <= 1.0)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

/// How far beyond the model radius a click still counts as a click on the enemy (estimate).
const ENEMY_CLICK_RADIUS: f32 = 1.3;

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
    eaw_overlays: Option<Res<OverlaysActive>>,
    mut recent: ResMut<RecentOrders>,
    mut move_lines: ResMut<MoveLines>,
) {
    let gizmo_marks = eaw_overlays.is_none();
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

    for id in sel.ships.iter().filter(|_| gizmo_marks) {
        if let Some(s) = ships.0.get(id) {
            ring(&mut gizmos, s, Color::srgb(0.2, 1.0, 0.35), 1.05);
            bars(&mut gizmos, s);
        }
    }
    if let Some(s) = hover.ship.and_then(|id| ships.0.get(&id)).filter(|_| gizmo_marks) {
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
    if let Some(s) = sel.enemy.and_then(|id| ships.0.get(&id)).filter(|_| gizmo_marks) {
        ring(&mut gizmos, s, Color::srgb(1.0, 0.35, 0.2), 1.12);
        bars(&mut gizmos, s);
    }

    // Hardpoint markers on the targeted enemy while own ships are selected.
    if !sel.ships.is_empty() && gizmo_marks {
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

    // Move lines: from every selected or just ordered ship of the player to its destination (a straight line until the sim exposes
    // the planned path), fading out towards the end, with a ring at the destination.
    let dt_lines = time.delta_secs();
    recent.0.retain(|_, t| {
        *t -= dt_lines;
        *t > 0.0
    });
    let mut listed: Vec<u32> = sel.ships.clone();
    listed.extend(recent.0.keys().copied().filter(|id| !sel.ships.contains(id)));
    let mut rings: Vec<(Vec3, Color, f32)> = Vec::new();
    let mut drawn: Vec<(Vec3, Vec3)> = Vec::new();
    for id in listed {
        let Some(s) = ships.0.get(&id).filter(|s| s.side == player && s.visible) else { continue };
        // Ships only (corvettes, frigates, capitals): fighters and bombers show no line.
        if s.squad != u32::MAX || matches!(rt.catalog.ship(s.def).class, battle_sim::TargetClass::Fighter | battle_sim::TargetClass::Bomber) {
            continue;
        }
        let (dest, base) = match s.order {
            battle_sim::Order::Move(d) | battle_sim::Order::Guard(d) => (crate::convert::sim_to_bevy(d), Color::srgb(0.35, 1.0, 0.4)),
            battle_sim::Order::AttackMove(d) => (crate::convert::sim_to_bevy(d), Color::srgb(0.95, 0.9, 0.3)),
            // Attack lines show only for a moment after the order, not for the plain selection (EaW draws none persistently).
            battle_sim::Order::Attack(t) if recent.0.contains_key(&id) => match ships.0.get(&t) {
                Some(target) => (target.pos, Color::srgb(1.0, 0.35, 0.3)),
                None => continue,
            },
            _ => continue,
        };
        // The ring is world sized like in the real game (about four hull radii of the biggest ship sent there, estimate from the video frames);
        // it goes away once the ships are inside it.
        let ring_radius = (s.radius * RING_HULL_RADII).clamp(RING_MIN, RING_MAX);
        if (dest - s.pos).length() < ring_radius * 0.3 {
            continue;
        }
        drawn.push((s.pos, dest));
        // Along the planned path (it bends around rocks and hulls); a straight line when the sim gives none.
        let mut points: Vec<Vec3> = Vec::with_capacity(s.path.len() + 1);
        points.push(s.pos);
        if matches!(s.order, battle_sim::Order::Attack(_)) || s.path.is_empty() {
            points.push(dest);
        } else {
            points.extend(s.path.iter().copied());
        }
        let total: f32 = points.windows(2).map(|w| w[0].distance(w[1])).sum::<f32>().max(1.0);
        let mut done = 0.0;
        for w in points.windows(2) {
            let len = w[0].distance(w[1]);
            // Split long legs so that the fade is smooth.
            let pieces = (len / (total / 16.0)).ceil().max(1.0) as usize;
            for k in 0..pieces {
                let (u0, u1) = (k as f32 / pieces as f32, (k + 1) as f32 / pieces as f32);
                let (t0, t1) = ((done + len * u0) / total, (done + len * u1) / total);
                gizmos.line_gradient(w[0].lerp(w[1], u0), w[0].lerp(w[1], u1), base.with_alpha(0.9 * (1.0 - t0 * 0.8)), base.with_alpha(0.9 * (1.0 - t1 * 0.8)));
            }
            done += len;
        }
        if !matches!(s.order, battle_sim::Order::Attack(_)) {
            match rings.iter_mut().find(|(p, ..)| p.distance(dest) < 30.0) {
                Some(ring) => ring.2 = ring.2.max(ring_radius),
                None => rings.push((dest, base, ring_radius)),
            }
        }
    }
    *move_lines = MoveLines { lines: drawn, rings: rings.iter().map(|r| r.0).collect() };
    for (p, c, r) in rings {
        // A few hairline circles side by side make the glowing 5 px band of the real marker.
        for (k, a) in [(0.0, 0.95), (0.006, 0.55), (-0.006, 0.55), (0.012, 0.25), (-0.012, 0.25)] {
            gizmos.circle(Isometry3d::new(p, flat), r * (1.0 + k), c.with_alpha(a)).resolution(96);
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



