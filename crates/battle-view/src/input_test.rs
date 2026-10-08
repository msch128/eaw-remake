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
}

const STEPS: [Step; 21] = [
    Step::SelectShip,
    Step::RightClickGround,
    Step::RightClickEnemyRim,
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
    Step::Button("b_reinforcement", Check::ReinforceWindow),
    Step::Button("r_0000", Check::Armed(ArmedKind::Reinforce)),
    Step::PlaceReinforcement,
    Step::BarAreaKeepsSelection,
    Step::EmptyClickDeselects,
    Step::MiddleDrag,
    Step::PageUpRotates,
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
    settings: Res<BattleViewSettings>,
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
        Query<&BattleCamera>,
        MessageWriter<MouseMotion>,
        MessageWriter<KeyboardInput>,
        Res<eaw_ui::TacticalData>,
        MessageReader<eaw_ui::EawAbilityClicked>,
    ),
) {
    let (block, pointer, hover, names, all_nodes, interactions, mut commands, action, armed, reinforce, cams, mut motion, mut keyboard, tactical, mut ability_clicks) = extra;
    st.ability_clicks += ability_clicks.read().count() as u32;
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
