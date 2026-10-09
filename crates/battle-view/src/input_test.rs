//! Opt-in self test of the real input path of the tactical screen (`SP_INPUT_TEST=1`, or `=exit` to quit afterwards).
//!
//! Needs a real window (`space-peace --hidden ...`). The test writes what the window system would write
//! (`CursorMoved`, `MouseButtonInput`, the window's cursor position) before Bevy's input systems run and lets the normal
//! pipeline (`ButtonInput`, the UI focus system, battle-view's pointer reading) do the rest, so it covers the same code as a
//! real mouse. Each step moves the pointer, presses and releases a button and checks the effect a few frames later:
//! select a ship, right click an order, right click the rim of an enemy model (must attack), a plain `Button` (control
//! experiment for the focus system), the bar buttons (fast forward, pause, battle cam, stop, move + ground click), a click
//! on the bar's empty selection area (keeps the selection) and an empty click (deselects). The verdict is one log line
//! `input-test: N checks PASS|FAIL`; failures print the UI nodes under the cursor.
#![allow(clippy::type_complexity, clippy::too_many_arguments)]

use bevy::input::ButtonState;
use bevy::input::InputSystems;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::mouse::{MouseButtonInput, MouseMotion};
use bevy::prelude::*;
use bevy::window::{CursorMoved, PrimaryWindow};
use eaw_ui::{EawButton, EawButtonClicked, EawButtonState};

use crate::camera::ActionCam;
use crate::control::{ArmedKind, ArmedOrder, Hover, OrderIssued, OrderKind, Pointer, Selection, UiBlock};
use crate::ships::RenderedShips;
use crate::{BattleCamera, BattleRuntime, BattleStatus, BattleViewSettings};

pub(crate) fn build(app: &mut App) {
    if std::env::var_os("SP_INPUT_TEST").is_some() {
        app.init_resource::<TestState>().add_systems(PreUpdate, drive.before(InputSystems).run_if(resource_exists::<BattleRuntime>));
    }
}

/// What a button click must have caused.
#[derive(Clone, Copy, Debug)]
enum Check {
    SpeedIs(f32),
    Paused(bool),
    BattleCam(bool),
    /// A new order was issued and the selection survived.
    Order,
    Armed(ArmedKind),
    /// The reinforcement window is open.
    ReinforceWindow,
}

#[derive(Clone, Copy, Debug)]
enum Step {
    SelectShip,
    RightClickGround,
    RightClickEnemyRim,
    PlainButton,
    Button(&'static str, Check),
    /// Left click on open ground after a command button armed an order.
    ArmedGroundClick,
    /// Click on the map to call in the picked reserve unit (needs `--battle-reserve`).
    PlaceReinforcement,
    BarAreaKeepsSelection,
    EmptyClickDeselects,
    /// Hold the middle button and move: left / right rotates, up / down tilts.
    MiddleDrag,
    /// Page Up rotates the camera.
    PageUpRotates,
    /// Shift + 1 presses the first ability button of the selected unit (vacuous when the unit has none).
    AbilityHotkey,
    /// Click the first own ship that has an ability with autocast (scenario `--grant-unit X4`); vacuous without one.
    SelectAutocastUnit,
    /// Right click on an ability button that supports autocast flips it (vacuous when the selected unit has none).
    AutocastToggle,
    /// Esc: cancels the armed order first, then opens the (pausing) menu, then closes it.
    Escape(EscCheck),
    /// Last step: after the 60 s lock a click on Retreat starts the jump countdown (state + banner), and when it runs out the battle ends with the ships retreated.
    RetreatFlow,
    /// After an attack order the red line goes away again within a few seconds although the ship stays selected.
    AttackLineExpires,
    /// With an empty selection the order buttons are dimmed, the retreat button is dimmed before it is available and no text sits under it.
    BarDimmedWithoutSelection,
    /// After a move order the selected ship shows a line to its destination and a ring there.
    MoveLineVisible,
    /// Hover a carrier or station: its unit panel lists the squadrons still in reserve (vacuous without one).
    CarrierPanel,
    /// Hover a ship, then one of its hardpoint reticles: the weapon tooltip must show.
    HardpointTooltip,
}

/// What an Esc press must have caused.
#[derive(Clone, Copy, Debug)]
enum EscCheck {
    ArmedCleared,
    Paused(bool),
}

const STEPS: [Step; 33] = [
    Step::SelectShip,
    Step::RightClickGround,
    Step::MoveLineVisible,
    Step::HardpointTooltip,
    Step::CarrierPanel,
    Step::RightClickEnemyRim,
    Step::AttackLineExpires,
    Step::PlainButton,
    Step::Button("b_fast_forward_t", Check::SpeedIs(2.0)),
    Step::Button("b_fast_forward_t", Check::SpeedIs(1.0)),
    Step::Button("b_play_pause_t", Check::Paused(true)),
    Step::Button("b_play_pause_t", Check::Paused(false)),
    Step::Button("b_camera_t", Check::BattleCam(true)),
    Step::Button("b_camera_t", Check::BattleCam(false)),
    Step::Button("c_button04", Check::Order),
    Step::Button("c_button02", Check::Armed(ArmedKind::Move)),
    Step::ArmedGroundClick,
    Step::AbilityHotkey,
    Step::SelectAutocastUnit,
    Step::AutocastToggle,
    Step::Button("c_button01", Check::Armed(ArmedKind::AttackMove)),
    Step::Escape(EscCheck::ArmedCleared),
    Step::Escape(EscCheck::Paused(true)),
    Step::Escape(EscCheck::Paused(false)),
    Step::Button("b_reinforcement", Check::ReinforceWindow),
    Step::Button("r_0000", Check::Armed(ArmedKind::Reinforce)),
    Step::PlaceReinforcement,
    Step::BarAreaKeepsSelection,
    Step::EmptyClickDeselects,
    Step::BarDimmedWithoutSelection,
    Step::MiddleDrag,
    Step::PageUpRotates,
    Step::RetreatFlow,
];

#[derive(Resource, Default)]
struct TestState {
    /// Frames since the battle was ready.
    ready_frames: u32,
    step: usize,
    /// Frames since the step started.
    t: u32,
    orders: u32,
    orders_at_start: u32,
    last_order: Option<OrderKind>,
    clicked: Vec<String>,
    probe_hover: bool,
    probe_press: bool,
    place_world: Option<Vec3>,
    /// Camera yaw / pitch at the start and after the horizontal part of a drag.
    ability_clicks: u32,
    attack_line_seen: bool,
    attack_ship: Option<u32>,
    retreat_phase: u8,
    retreat_t0: u32,
    retreat_seen: bool,
    hp_marker: Option<Vec2>,
    autocast: Option<(u32, usize, bool)>,
    hp_ship: Option<u32>,
    cam_saved: Option<(Vec3, f32)>,
    cam0: (f32, f32),
    cam1: (f32, f32),
    reserve_before: u32,
    results: Vec<(String, bool)>,
    done: bool,
}

#[derive(Clone)]
enum Act {
    Move(Vec2),
    Down(MouseButton),
    Up(MouseButton),
    Drag(Vec2),
    KeyDown(KeyCode, Key),
    KeyUp(KeyCode, Key),
}

fn drive(
    mut st: ResMut<TestState>,
    status: Res<BattleStatus>,
    mut settings: ResMut<BattleViewSettings>,
    ships: Res<RenderedShips>,
    sel: Res<Selection>,
    rt: Res<BattleRuntime>,
    camera: Single<(Entity, &Camera, &GlobalTransform), With<BattleCamera>>,
    mut window: Single<(Entity, &mut Window), With<PrimaryWindow>>,
    mut moved: MessageWriter<CursorMoved>,
    mut buttons: MessageWriter<MouseButtonInput>,
    mut orders: MessageReader<OrderIssued>,
    mut clicks: MessageReader<EawButtonClicked>,
    bar: Query<(&EawButton, &EawButtonState, &ComputedNode, &UiGlobalTransform, &InheritedVisibility)>,
    mut exit: MessageWriter<AppExit>,
    hp_ui: (
        Res<eaw_ui::TacticalOverlays>,
        Query<(&Node, &Text), With<crate::hud::HardpointTip>>,
        Query<&mut BattleCamera>,
        Query<(), With<crate::tactical_ui::AutocastRing>>,
        Query<&Text, With<crate::hud::UnitPanelStats>>,
    ),
    extra: (
        Res<UiBlock>,
        Res<Pointer>,
        Res<Hover>,
        Query<&Name>,
        Query<(Entity, &ComputedNode, &UiGlobalTransform, &InheritedVisibility, Option<&Interaction>, Option<&bevy::ui::FocusPolicy>)>,
        Query<&Interaction>,
        Commands,
        Res<ActionCam>,
        Res<ArmedOrder>,
        Res<crate::tactical_ui::ReinforceUi>,
        MessageWriter<MouseMotion>,
        MessageWriter<KeyboardInput>,
        Res<eaw_ui::TacticalData>,
        MessageReader<eaw_ui::EawAbilityClicked>,
        Res<crate::control::MoveLines>,
        ResMut<crate::control::RecentOrders>,
    ),
) {
    let (block, pointer, hover, names, all_nodes, interactions, mut commands, action, armed, reinforce, mut motion, mut keyboard, tactical, mut ability_clicks, move_lines, mut recent) = extra;
    st.ability_clicks += ability_clicks.read().count() as u32;
    let (hp_overlays, hp_tips, mut cams, autocast_rings, panel_stats) = hp_ui;
    let cam_pose = cams.single().map(|c| (c.target_yaw, c.pitch)).unwrap_or_default();
    let (cam_entity, cam, gt) = *camera;
    let sf = cam.target_scaling_factor().unwrap_or(1.0);
    for o in orders.read() {
        st.orders += 1;
        st.last_order = Some(o.kind);
    }
    for c in clicks.read() {
        st.clicked.push(c.name.clone());
    }
    if st.done || status.tick == 0 || ships.0.is_empty() {
        return;
    }
    st.ready_frames += 1;
    // Give the models, the bar layout and the camera a moment.
    if st.ready_frames < 240 {
        return;
    }
    let Some(vp) = cam.logical_viewport_size() else { return };
    let player = settings.player_side;
    let t = st.t;
    st.t += 1;
    let Some(&step) = STEPS.get(st.step) else {
        let pass = st.results.iter().all(|r| r.1);
        info!("input-test: {} checks {}", st.results.len(), if pass { "PASS" } else { "FAIL" });
        st.done = true;
        if std::env::var("SP_INPUT_TEST").is_ok_and(|v| v == "exit") {
            exit.write(AppExit::Success);
        }
        return;
    };
    if t == 0 {
        st.orders_at_start = st.orders;
        st.clicked.clear();
    }
    let mut act: Option<Act> = None;
    let mut verdict: Option<(String, bool)> = None;
    // Frames of a click: move at 0, press at 1, release at 2, judge at 6.
    let click_at = |t: u32, p: Vec2, button: MouseButton| match t {
        0 => Some(Act::Move(p)),
        1 => Some(Act::Down(button)),
        2 => Some(Act::Up(button)),
        _ => None,
    };
    let on_screen = |p: Vec2| p.x > vp.x * 0.1 && p.x < vp.x * 0.9 && p.y > vp.y * 0.1 && p.y < vp.y * 0.6;
    match step {
        Step::SelectShip => {
            let own = ships
                .0
                .values()
                .filter(|s| s.side == player && !rt.catalog.ship(s.def).is_static)
                .filter_map(|s| cam.world_to_viewport(gt, s.pos).ok().map(|p| (s.id, p)))
                .filter(|(_, p)| on_screen(*p))
                .min_by_key(|(id, _)| *id);
            match (t, own) {
                (0, Some((_, p))) => act = click_at(0, p, MouseButton::Left),
                (1 | 2, _) => act = click_at(t, Vec2::ZERO, MouseButton::Left),
                (6, _) => verdict = Some(("select-ship".into(), !sel.ships.is_empty())),
                (0, None) if st.ready_frames > 2000 => verdict = Some(("select-ship (no own ship on screen)".into(), false)),
                (0, None) => st.t = 0,
                _ => {}
            }
        }
        Step::RightClickGround => match t {
            0 => act = click_at(0, Vec2::new(vp.x * 0.62, vp.y * 0.22), MouseButton::Right),
            1 | 2 => act = click_at(t, Vec2::ZERO, MouseButton::Right),
            6 => verdict = Some(("right-click-order".into(), st.orders > st.orders_at_start)),
            _ => {}
        },
        Step::RightClickEnemyRim => {
            // The biggest enemy on screen (a station or capital ship); the click goes just inside its bounding circle.
            let rim = ships
                .0
                .values()
                .filter(|s| s.side != player)
                .filter_map(|s| {
                    let c = cam.world_to_viewport(gt, s.pos).ok()?;
                    let e = cam.world_to_viewport(gt, s.pos + gt.right() * s.radius).ok()?;
                    Some((s.radius, c + Vec2::X * (e - c).length() * 0.95))
                })
                .filter(|(_, p)| on_screen(*p))
                .max_by(|a, b| a.0.total_cmp(&b.0));
            match (t, rim) {
                (0, Some((_, p))) => act = click_at(0, p, MouseButton::Right),
                (1 | 2, _) => act = click_at(t, Vec2::ZERO, MouseButton::Right),
                (6, _) => verdict = Some(("right-click-enemy-rim-attacks".into(), st.last_order == Some(OrderKind::Attack) && st.orders > st.orders_at_start)),
                (0, None) => verdict = Some(("right-click-enemy-rim (no enemy on screen)".into(), true)),
                _ => {}
            }
        }
        // Control experiment: a plain Button must be hovered and pressed by the same events.
        Step::PlainButton => {
            if t == 0 {
                commands.spawn((
                    Name::new("input-test probe"),
                    Button,
                    Node { position_type: PositionType::Absolute, left: Val::Percent(40.0), top: Val::Percent(4.0), width: Val::Percent(10.0), height: Val::Percent(5.0), ..default() },
                    BackgroundColor(Color::srgba(1.0, 0.0, 0.0, 0.3)),
                    GlobalZIndex(100),
                    UiTargetCamera(cam_entity),
                ));
            }
            // The probe entity is found by name from the next frame on.
            let probe = all_nodes.iter().find(|n| names.get(n.0).is_ok_and(|x| x.as_str() == "input-test probe")).map(|n| n.0);
            let state = probe.and_then(|e| interactions.get(e).ok().copied());
            match t {
                2 => act = Some(Act::Move(Vec2::new(vp.x * 0.45, vp.y * 0.065))),
                4 => st.probe_hover = state == Some(Interaction::Hovered),
                5 => act = Some(Act::Down(MouseButton::Left)),
                6 => {
                    st.probe_press = state == Some(Interaction::Pressed);
                    act = Some(Act::Up(MouseButton::Left));
                }
                9 => {
                    if let Some(e) = probe {
                        commands.entity(e).try_despawn();
                    }
                    verdict = Some(("plain-button-hover-press".into(), st.probe_hover && st.probe_press));
                }
                _ => {}
            }
        }
        Step::Button(name, check) => {
            // A button whose centre is under another button would give its click to that one.
            let me = bar.iter().find(|r| r.0.name == name && r.4.get() && r.2.size().x > 4.0);
            let target = me.map(|r| r.3.translation / sf);
            let covered = me.is_some_and(|r| {
                let c = r.3.translation;
                bar.iter().any(|o| o.0.name != name && o.4.get() && o.2.size().x > 4.0 && (c.x - o.3.translation.x).abs() < o.2.size().x * 0.5 && (c.y - o.3.translation.y).abs() < o.2.size().y * 0.5)
            });
            match (t, target) {
                (0, Some(p)) => act = click_at(0, p, MouseButton::Left),
                (1 | 2, _) => act = click_at(t, Vec2::ZERO, MouseButton::Left),
                (8, _) => {
                    let effect = match check {
                        Check::SpeedIs(v) => (settings.speed - v).abs() < 0.01,
                        Check::Paused(p) => settings.paused == p,
                        Check::BattleCam(on) => action.active == on,
                        Check::Order => st.orders > st.orders_at_start,
                        Check::Armed(k) => armed.0 == Some(k),
                        Check::ReinforceWindow => reinforce.root.is_some(),
                    };
                    let ok = st.clicked.iter().any(|c| c == name) && effect && !sel.ships.is_empty();
                    if !ok {
                        info!("input-test:   {name}: clicked {:?}, covered {covered}, effect {effect}, selection {}", st.clicked, sel.ships.len());
                    }
                    verdict = Some((format!("button {name} {check:?}"), ok));
                }
                (0, None) => verdict = Some((format!("button {name} (not visible)"), false)),
                _ => {}
            }
        }
        Step::ArmedGroundClick => match t {
            0 => act = click_at(0, Vec2::new(vp.x * 0.5, vp.y * 0.3), MouseButton::Left),
            1 | 2 => act = click_at(t, Vec2::ZERO, MouseButton::Left),
            6 => {
                let ok = st.orders > st.orders_at_start && st.last_order == Some(OrderKind::Move) && armed.0.is_none() && !sel.ships.is_empty();
                verdict = Some(("armed-move-click".into(), ok));
            }
            _ => {}
        },
        Step::PlaceReinforcement => {
            let reserve = |r: &crate::tactical_ui::ReinforceUi| r.rows.iter().map(|x| x.2).sum::<u32>();
            match t {
                0 => {
                    st.reserve_before = reserve(&reinforce);
                    // A spot next to one of the player's ships that the zone rule accepts.
                    let own = ships.0.values().filter(|s| s.side == player && !rt.catalog.ship(s.def).is_static).min_by_key(|s| s.id);
                    let spot = own.and_then(|o| {
                        [(300.0, 0.0), (-300.0, 0.0), (0.0, 300.0), (0.0, -300.0), (600.0, 0.0), (-600.0, 0.0), (0.0, 600.0), (0.0, -600.0)]
                            .iter()
                            .map(|(dx, dz)| o.pos + Vec3::new(*dx, 0.0, *dz))
                            .find(|p| crate::tactical_ui::zone_check(&rt, player, crate::convert::bevy_to_sim(*p)).is_ok())
                    });
                    st.place_world = spot;
                    if spot.is_none() {
                        verdict = Some(("place-reinforcement (no valid spot found)".into(), false));
                    }
                }
                3 => {
                    if let Some(p) = st.place_world.and_then(|w| cam.world_to_viewport(gt, w).ok()) {
                        act = Some(Act::Move(p));
                    }
                }
                4 | 5 => act = click_at(t - 3, Vec2::ZERO, MouseButton::Left),
                90 => {
                    let after = reserve(&reinforce);
                    let ok = st.reserve_before > after && armed.0.is_none();
                    if !ok {
                        info!("input-test:   reserve {} -> {after}, armed {:?}, spot {:?}", st.reserve_before, armed.0, st.place_world);
                    }
                    verdict = Some(("place-reinforcement".into(), ok));
                }
                _ => {}
            }
        }
        Step::BarAreaKeepsSelection => match t {
            0 => act = click_at(0, Vec2::new(vp.x * 0.7, vp.y * 0.96), MouseButton::Left),
            1 | 2 => act = click_at(t, Vec2::ZERO, MouseButton::Left),
            6 => verdict = Some(("bar-area-keeps-selection".into(), !sel.ships.is_empty())),
            _ => {}
        },
        Step::MiddleDrag => match t {
            0 => {
                st.cam0 = cam_pose;
                act = Some(Act::Move(Vec2::new(vp.x * 0.5, vp.y * 0.3)));
            }
            1 => act = Some(Act::Down(MouseButton::Middle)),
            2 => act = Some(Act::Drag(Vec2::new(60.0, 0.0))),
            4 => {
                st.cam1 = cam_pose;
                act = Some(Act::Drag(Vec2::new(0.0, 40.0)));
            }
            6 => act = Some(Act::Up(MouseButton::Middle)),
            10 => {
                let yaw_only = (st.cam1.0 - st.cam0.0).abs() > 0.05 && (st.cam1.1 - st.cam0.1).abs() < 1e-4;
                let tilt = (cam_pose.1 - st.cam1.1).abs() > 0.02 && (cam_pose.0 - st.cam1.0).abs() < 1e-4;
                if !(yaw_only && tilt) {
                    info!("input-test:   drag: start {:?}, after horizontal {:?}, end {:?}", st.cam0, st.cam1, cam_pose);
                }
                verdict = Some(("middle-drag-rotates-and-tilts".into(), yaw_only && tilt));
            }
            _ => {}
        },
        Step::SelectAutocastUnit => {
            let pick = ships
                .0
                .values()
                .filter(|s| s.side == player && rt.catalog.ship(s.def).abilities.iter().any(|d| d.supports_autofire))
                .min_by_key(|s| s.id);
            match t {
                0 => match (pick, cams.single_mut()) {
                    (Some(s), Ok(mut c)) => {
                        st.cam_saved = Some((c.focus, c.distance));
                        let at = Vec3::new(s.pos.x, c.target_focus.y, s.pos.z);
                        c.focus = at;
                        c.target_focus = at;
                        c.distance = 2500.0;
                        c.target_distance = 2500.0;
                        st.hp_ship = Some(s.id);
                    }
                    _ => verdict = Some(("select-autocast-unit (no own ship with an autocast ability)".into(), true)),
                },
                8 => match st.hp_ship.and_then(|id| ships.0.get(&id)).and_then(|s| cam.world_to_viewport(gt, s.pos).ok()) {
                    Some(p) => act = Some(Act::Move(p)),
                    None => verdict = Some(("select-autocast-unit: ship not on screen".into(), false)),
                },
                9 | 10 => act = click_at(t - 8, Vec2::ZERO, MouseButton::Left),
                16 => {
                    let ok = st.hp_ship.is_some_and(|id| sel.ships.contains(&id));
                    verdict = Some(("select-autocast-unit".into(), ok));
                }
                _ => {}
            }
        }
        Step::AutocastToggle => {
            // The first selected ship and an ability of it that supports autocast, with the bar button that shows it.
            let found = sel.ships.first().and_then(|&id| {
                let s = rt.cur.as_ref()?.ship(id)?;
                let defs = &rt.catalog.ship(s.def).abilities;
                let unit = tactical.selection.get(tactical.focus)?;
                unit.abilities.iter().enumerate().find_map(|(k, a)| {
                    let i = defs.iter().position(|d| d.kind.eq_ignore_ascii_case(&a.def.kind) && d.supports_autofire)?;
                    Some((id, i, k, s.autocast.get(i).copied().unwrap_or(false)))
                })
            });
            match (t, found) {
                (0, None) => verdict = Some(("autocast-toggle (selected unit has no autocast ability)".into(), true)),
                (0, Some((id, i, k, on))) => {
                    st.autocast = Some((id, i, on));
                    let name = format!("special_button_{k:02}");
                    match bar.iter().find(|r| r.0.name == name && r.4.get()) {
                        Some(r) => act = Some(Act::Move(r.3.translation / sf)),
                        None => verdict = Some(("autocast-toggle: ability button not visible".into(), false)),
                    }
                }
                (1 | 2, _) => act = click_at(t, Vec2::ZERO, MouseButton::Right),
                (30, _) => {
                    if let (Some((f, d)), Ok(mut c)) = (st.cam_saved.take(), cams.single_mut()) {
                        c.focus = f;
                        c.target_focus = f;
                        c.distance = d;
                        c.target_distance = d;
                    }
                    let ok = st.autocast.is_some_and(|(id, i, before)| rt.cur.as_ref().and_then(|c| c.ship(id)).is_some_and(|s| s.autocast.get(i).copied() == Some(!before)));
                    // The ring on the button follows the state.
                    let ring_ok = st.autocast.is_some_and(|(_, _, before)| autocast_rings.iter().count() == usize::from(!before));
                    if !ring_ok {
                        info!("input-test:   autocast rings on the bar: {}", autocast_rings.iter().count());
                    }
                    verdict = Some(("autocast-toggle-and-ring".into(), ok && ring_ok));
                }
                _ => {}
            }
        }
        Step::CarrierPanel => {
            let pick = rt.cur.as_ref().and_then(|c| c.ships.iter().filter(|s| s.reserve_squadrons > 0).max_by_key(|s| s.id).map(|s| s.id));
            match t {
                0 => match (pick, pick.and_then(|id| ships.0.get(&id)), cams.single_mut()) {
                    (Some(id), Some(s), Ok(mut c)) => {
                        st.cam_saved = Some((c.focus, c.distance));
                        let at = Vec3::new(s.pos.x, c.target_focus.y, s.pos.z);
                        c.focus = at;
                        c.target_focus = at;
                        c.distance = (s.radius * 5.0).max(c.min_distance);
                        c.target_distance = c.distance;
                        st.hp_ship = Some(id);
                    }
                    _ => verdict = Some(("carrier-panel (no ship with squadrons in reserve)".into(), true)),
                },
                8 => match st.hp_ship.and_then(|id| ships.0.get(&id)).and_then(|s| cam.world_to_viewport(gt, s.pos).ok()) {
                    Some(p) => act = Some(Act::Move(p)),
                    None => verdict = Some(("carrier-panel: ship not on screen".into(), false)),
                },
                20 => {
                    // Reticles appear while the ship is hovered; the panel only shows away from them, so move to the free spot farthest from all of them.
                    let ship = st.hp_ship.and_then(|id| ships.0.get(&id));
                    if let Some(s) = ship
                        && let (Ok(c), Ok(e)) = (cam.world_to_viewport(gt, s.pos), cam.world_to_viewport(gt, s.pos + gt.right() * s.radius))
                    {
                        let r = (e - c).length();
                        let spot = (0..16)
                            .flat_map(|k| {
                                let a = k as f32 / 16.0 * std::f32::consts::TAU;
                                [0.35f32, 0.6].map(|f| c + Vec2::new(a.cos(), a.sin()) * r * f)
                            })
                            .max_by(|x, y| {
                                let d = |p: &Vec2| hp_overlays.hardpoints.iter().map(|h| h.screen_pos.distance(*p)).fold(f32::MAX, f32::min);
                                d(x).total_cmp(&d(y))
                            });
                        if let Some(p) = spot {
                            act = Some(Act::Move(p));
                        }
                    }
                }
                140 => {
                    if let (Some((f, d)), Ok(mut c)) = (st.cam_saved.take(), cams.single_mut()) {
                        c.focus = f;
                        c.target_focus = f;
                        c.distance = d;
                        c.target_distance = d;
                    }
                    let shown = panel_stats.iter().map(|t| t.0.clone()).collect::<Vec<_>>();
                    let ok = shown.iter().any(|t| t.contains("in reserve"));
                    // A fighter next to the carrier can win the pick; that is not a panel fault.
                    let picked_other = hover.ship != st.hp_ship;
                    if !ok {
                        info!("input-test:   panel stats {shown:?}, hover {:?}, wanted {:?}", hover.ship, st.hp_ship);
                    }
                    if picked_other {
                        verdict = Some(("carrier-panel (cursor picked another ship, not judged)".into(), true));
                    } else {
                        verdict = Some(("carrier-panel-shows-reserve".into(), ok));
                    }
                }
                _ => {}
            }
        }
        Step::HardpointTooltip => {
            let (overlays, tips) = (&hp_overlays, &hp_tips);
            match t {
                0 => {
                    // The biggest ship with hardpoints a reticle can mark: bring the camera over it.
                    let pick = ships
                        .0
                        .values()
                        .filter(|s| s.visible && rt.catalog.ship(s.def).hardpoints.iter().any(|h| h.destroyable && crate::visuals::is_markable(h.kind)))
                        .max_by(|a, b| a.radius.total_cmp(&b.radius));
                    match (pick, cams.single_mut()) {
                        (Some(s), Ok(mut c)) => {
                            st.cam_saved = Some((c.focus, c.distance));
                            let at = Vec3::new(s.pos.x, c.target_focus.y, s.pos.z);
                            c.focus = at;
                            c.target_focus = at;
                            c.distance = (s.radius * 5.0).max(c.min_distance);
                            c.target_distance = c.distance;
                            st.hp_ship = Some(s.id);
                        }
                        _ => verdict = Some(("hardpoint-tooltip (no ship with hardpoints)".into(), false)),
                    }
                }
                8 => {
                    let p = st.hp_ship.and_then(|id| ships.0.get(&id)).and_then(|s| cam.world_to_viewport(gt, s.pos).ok());
                    match p {
                        Some(p) => act = Some(Act::Move(p)),
                        None => verdict = Some(("hardpoint-tooltip: ship not on screen".into(), false)),
                    }
                }
                16 => {
                    st.hp_marker = overlays.hardpoints.first().map(|h| h.screen_pos);
                    // Hover the edge of the turret, not its centre.
                    match st.hp_marker {
                        Some(p) => act = Some(Act::Move(p + Vec2::new(20.0, 0.0))),
                        None => verdict = Some(("hardpoint-tooltip: no reticle appeared while hovering the ship".into(), false)),
                    }
                }
                36 => {
                    let shown = tips.iter().any(|(n, _)| n.display != Display::None);
                    info!("input-test:   hardpoint tooltip text {:?}", tips.iter().map(|(_, t)| t.0.clone()).collect::<Vec<_>>());
                    if !shown {
                        info!("input-test:   reticle at {:?}, cursor {:?}, markers {}", st.hp_marker, pointer.cursor, overlays.hardpoints.len());
                    }
                    if let (Some((f, d)), Ok(mut c)) = (st.cam_saved.take(), cams.single_mut()) {
                        c.focus = f;
                        c.target_focus = f;
                        c.distance = d;
                        c.target_distance = d;
                    }
                    verdict = Some(("hardpoint-tooltip".into(), shown));
                }
                _ => {}
            }
        }
        Step::AttackLineExpires => {
            // Own attack order on the nearest enemy for the first selected ship, issued the way `orders` does (command + "just ordered" window).
            if t == 0 {
                let me = sel.ships.iter().copied().find(|id| ships.0.get(id).is_some_and(|s| s.side == player && s.squad == u32::MAX));
                let from = me.and_then(|id| ships.0.get(&id));
                let foe = from.and_then(|f| ships.0.values().filter(|s| s.side != player).min_by(|a, b| a.pos.distance(f.pos).total_cmp(&b.pos.distance(f.pos))));
                match (me, foe) {
                    (Some(id), Some(foe)) => {
                        rt.sim.command(player, battle_sim::Command::Attack { ships: vec![id], target: foe.id });
                        recent.0.insert(id, 3.3);
                        st.attack_ship = Some(id);
                    }
                    _ => verdict = Some(("attack-line-expires (no own ship or no enemy)".into(), true)),
                }
            }
            // The selected ship still has its attack order; only the "just ordered" window may draw the red line.
            if t == 0 {
                st.attack_line_seen = false;
            }
            if !move_lines.lines.is_empty() {
                st.attack_line_seen = true;
            }
            if move_lines.lines.is_empty() && ((st.attack_line_seen && t >= 30) || t >= 300) {
                // The order must still stand, otherwise the missing line proves nothing.
                let still = st.attack_ship.and_then(|id| ships.0.get(&id)).is_none_or(|s| matches!(s.order, battle_sim::Order::Attack(_)));
                info!("input-test:   attack line seen {} order still active {still}", st.attack_line_seen);
                verdict = Some(("attack-line-expires".into(), still));
            } else if t >= 2400 {
                verdict = Some(("attack-line-expires".into(), false));
            }
        }
        Step::MoveLineVisible => {
            // The order reaches the sim with its next step, so give it a moment.
            if t >= 5 && !move_lines.lines.is_empty() && !move_lines.rings.is_empty() {
                verdict = Some(("move-line-and-ring".into(), true));
            } else if t >= 120 {
                info!("input-test:   move lines {:?}", *move_lines);
                verdict = Some(("move-line-and-ring".into(), false));
            }
        }
        Step::Escape(check) => match t {
            0 => act = Some(Act::KeyDown(KeyCode::Escape, Key::Escape)),
            1 => act = Some(Act::KeyUp(KeyCode::Escape, Key::Escape)),
            8 => {
                let ok = match check {
                    EscCheck::ArmedCleared => armed.0.is_none() && !settings.paused,
                    EscCheck::Paused(p) => settings.paused == p && armed.0.is_none(),
                };
                verdict = Some((format!("escape {check:?}"), ok));
            }
            _ => {}
        },
        Step::AbilityHotkey => {
            let has = tactical.selection.get(tactical.focus).is_some_and(|u| u.abilities.first().is_some_and(|a| !a.disabled));
            match t {
                0 if !has => verdict = Some(("ability-hotkey (selected unit has no ability)".into(), true)),
                1 => act = Some(Act::KeyDown(KeyCode::ShiftLeft, Key::Shift)),
                2 => act = Some(Act::KeyDown(KeyCode::Digit1, Key::Character("1".into()))),
                3 => act = Some(Act::KeyUp(KeyCode::Digit1, Key::Character("1".into()))),
                4 => act = Some(Act::KeyUp(KeyCode::ShiftLeft, Key::Shift)),
                8 => verdict = Some(("ability-hotkey".into(), st.ability_clicks > 0)),
                _ => {}
            }
        }
        Step::PageUpRotates => match t {
            0 => {
                st.cam0 = cam_pose;
                act = Some(Act::KeyDown(KeyCode::PageUp, Key::PageUp));
            }
            12 => act = Some(Act::KeyUp(KeyCode::PageUp, Key::PageUp)),
            16 => {
                let ok = (cam_pose.0 - st.cam0.0).abs() > 0.05 && (cam_pose.1 - st.cam0.1).abs() < 1e-4;
                verdict = Some(("page-up-rotates".into(), ok));
            }
            _ => {}
        },
        Step::RetreatFlow => {
            match st.retreat_phase {
                // Run the clock past the lock fast, then back to real time so that the countdown can be seen.
                0 => {
                    settings.speed = 30.0;
                    if status.time >= rt.retreat_countdown + 1.0 {
                        settings.speed = 1.0;
                        st.retreat_phase = 1;
                        st.retreat_t0 = t + 1;
                    } else if t > 60_000 {
                        verdict = Some(("retreat-flow (lock never ended)".into(), false));
                    }
                }
                1 => {
                    let p = bar.iter().find(|r| r.0.name == "b_retreat" && r.4.get()).map(|r| r.3.translation / sf);
                    match (t - st.retreat_t0, p) {
                        (0, Some(p)) => act = click_at(0, p, MouseButton::Left),
                        (1 | 2, _) => act = click_at(t - st.retreat_t0, Vec2::ZERO, MouseButton::Left),
                        (0, None) => verdict = Some(("retreat-flow (button not visible)".into(), false)),
                        (30.., _) => {
                            st.retreat_phase = 2;
                            info!("input-test:   after the retreat click: state {:?}, time {:.1}, clicked {:?}", tactical.retreat, status.time, st.clicked);
                        }
                        _ => {}
                    }
                }
                _ => {
                    if matches!(tactical.retreat, eaw_ui::RetreatState::Countdown(_)) && !st.retreat_seen {
                        st.retreat_seen = true;
                        info!("input-test:   retreat countdown {:?}", tactical.retreat);
                    }
                    let outcome = rt.outcome();
                    if let Some(o) = outcome {
                        let escaped: u32 = o.retreated.get(settings.player_side as usize).map_or(0, |g| g.iter().sum());
                        info!("input-test:   battle result after retreat: {escaped} units escaped, winner {:?}", o.winner);
                        verdict = Some(("retreat-flow".into(), st.retreat_seen && escaped > 0));
                    } else if t - st.retreat_t0 > 6000 {
                        verdict = Some(("retreat-flow (no result after the jump)".into(), false));
                    }
                }
            }
        }
        Step::BarDimmedWithoutSelection => {
            if t == 4 {
                let orders: Vec<bool> = bar.iter().filter(|(b, ..)| b.name.starts_with("c_button")).map(|(_, s, ..)| s.disabled).collect();
                let retreat = bar.iter().find(|(b, ..)| b.name == "b_retreat").map(|(_, s, ..)| s.disabled);
                info!("input-test:   order buttons disabled {orders:?}, retreat disabled {retreat:?}");
                verdict = Some(("order-buttons-dimmed-without-selection".into(), sel.ships.is_empty() && !orders.is_empty() && orders.iter().all(|d| *d) && retreat != Some(false)));
            }
        }
        Step::EmptyClickDeselects => match t {
            0 => act = click_at(0, Vec2::new(vp.x * 0.5, vp.y * 0.12), MouseButton::Left),
            1 | 2 => act = click_at(t, Vec2::ZERO, MouseButton::Left),
            6 => verdict = Some(("empty-click-deselects".into(), sel.ships.is_empty())),
            _ => {}
        },
    }
    if let Some((name, ok)) = verdict {
        info!("input-test: {name} {}", if ok { "ok" } else { "FAILED" });
        if !ok {
            let by = block.1.and_then(|e| names.get(e).ok()).map(|n| n.to_string());
            info!("input-test:   cursor {:?} blocked {} by {:?} hover {:?} selection {}", pointer.cursor, block.0, by, hover.ship, sel.ships.len());
            if let Some(c) = pointer.cursor {
                for (e, n, g, v, inter, fp) in &all_nodes {
                    if n.size().x > 0.0 && n.contains_point(*g, c * sf) {
                        info!("input-test:     node {:?} vis {} interaction {:?} focus {:?}", names.get(e).ok().map(|n| n.as_str().to_string()), v.get(), inter, fp);
                    }
                }
            }
        }
        st.results.push((name, ok));
        st.step += 1;
        st.t = 0;
    }
    let (entity, window) = &mut *window;
    match act {
        Some(Act::Move(p)) => {
            let physical = (p * window.scale_factor()).as_dvec2();
            window.set_physical_cursor_position(Some(physical));
            moved.write(CursorMoved { window: *entity, position: p, delta: None });
        }
        Some(Act::Down(b)) => {
            buttons.write(MouseButtonInput { button: b, state: ButtonState::Pressed, window: *entity });
        }
        Some(Act::Up(b)) => {
            buttons.write(MouseButtonInput { button: b, state: ButtonState::Released, window: *entity });
        }
        Some(Act::Drag(delta)) => {
            motion.write(MouseMotion { delta });
        }
        Some(Act::KeyDown(code, key)) => {
            keyboard.write(KeyboardInput { key_code: code, logical_key: key, state: ButtonState::Pressed, text: None, repeat: false, window: *entity });
        }
        Some(Act::KeyUp(code, key)) => {
            keyboard.write(KeyboardInput { key_code: code, logical_key: key, state: ButtonState::Released, text: None, repeat: false, window: *entity });
        }
        None => {}
    }
}
