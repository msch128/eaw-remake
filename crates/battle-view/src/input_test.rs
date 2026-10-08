//! Opt-in self test of the real input path of the tactical screen (`SP_INPUT_TEST=1`, or `=exit` to quit afterwards).
//!
//! Needs a real window (`space-peace --hidden ...`). The test writes what the window system would write
//! (`CursorMoved`, `MouseButtonInput`, the window's cursor position) and lets Bevy's normal input systems, the UI focus
//! system and battle-view's pointer reading do the rest, so it covers the same code as a real mouse. It checks:
//! a left click on a ship selects it, a right click on open space issues an order, a click on a command bar button
//! reaches the button and does not touch the selection, a click on the bar's empty selection area does not deselect, a
//! left click on open space deselects. The verdict is one log line `input-test: ... PASS|FAIL`.

use bevy::input::ButtonState;
use bevy::input::InputSystems;
use bevy::input::mouse::MouseButtonInput;
use bevy::prelude::*;
use bevy::window::{CursorMoved, PrimaryWindow};
use eaw_ui::{EawButton, EawButtonClicked};

use crate::control::{OrderIssued, Selection};
use crate::ships::RenderedShips;
use crate::{BattleCamera, BattleRuntime, BattleStatus, BattleViewSettings};

pub(crate) fn build(app: &mut App) {
    if std::env::var_os("SP_INPUT_TEST").is_some() {
        app.init_resource::<TestState>().add_systems(PreUpdate, drive.before(InputSystems).run_if(resource_exists::<BattleRuntime>));
    }
}

#[derive(Resource, Default)]
struct TestState {
    /// Frames since the battle was ready.
    ready_frames: u32,
    phase: u32,
    t: u32,
    orders: u32,
    clicked_button: Option<String>,
    button_name: Option<String>,
    probe: Option<Entity>,
    probe_hover: bool,
    probe_press: bool,
    results: Vec<(&'static str, bool)>,
    done: bool,
}

#[derive(Clone, Copy)]
enum Act {
    Move(Vec2),
    Down(MouseButton),
    Up(MouseButton),
}

#[allow(clippy::too_many_arguments)]
fn drive(
    mut st: ResMut<TestState>,
    status: Res<BattleStatus>,
    settings: Res<BattleViewSettings>,
    ships: Res<RenderedShips>,
    sel: Res<Selection>,
    rt: Res<BattleRuntime>,
    camera: Single<(&Camera, &GlobalTransform), With<BattleCamera>>,
    mut window: Single<(Entity, &mut Window), With<PrimaryWindow>>,
    mut moved: MessageWriter<CursorMoved>,
    mut buttons: MessageWriter<MouseButtonInput>,
    mut orders: MessageReader<OrderIssued>,
    mut clicks: MessageReader<EawButtonClicked>,
    bar: Query<(&EawButton, &eaw_ui::EawButtonState, &ComputedNode, &UiGlobalTransform, &InheritedVisibility, &Interaction)>,
    mut exit: MessageWriter<AppExit>,
    diag: (
        Res<crate::control::UiBlock>,
        Res<crate::control::Pointer>,
        Res<crate::control::Hover>,
        Query<&Name>,
        Query<(Entity, &ComputedNode, &UiGlobalTransform, &InheritedVisibility, Has<ImageNode>, Option<&Interaction>, Option<&BackgroundColor>, Option<&bevy::ui::FocusPolicy>, &ComputedUiTargetCamera, Option<&GlobalZIndex>)>,
        Query<Entity, With<BattleCamera>>,
        Res<ButtonInput<MouseButton>>,
        Res<bevy::ui::UiStack>,
        Query<(Entity, &Interaction)>,
        Query<(&Camera, &bevy::camera::RenderTarget)>,
        Commands,
    ),
) {
    let sf = camera.0.target_scaling_factor().unwrap_or(1.0);
    let (block, pointer, hover, names, all_nodes, battle_cam, mouse, ui_stack, interactions, cams, mut diag_commands) = diag;
    st.orders += orders.read().count() as u32;
    for c in clicks.read() {
        st.clicked_button = Some(c.name.clone());
    }
    if st.done {
        return;
    }
    if status.tick == 0 || ships.0.is_empty() {
        return;
    }
    st.ready_frames += 1;
    // Give the models, the bar layout and the camera a moment.
    if st.ready_frames < 240 {
        return;
    }
    let (cam, gt) = *camera;
    let Some(vp) = cam.logical_viewport_size() else { return };
    let player = settings.player_side;
    let t = st.t;
    st.t += 1;
    let mut act: Option<Act> = None;
    let finish_phase = |st: &mut TestState, name: &'static str, ok: bool| {
        info!("input-test: {name} {}", if ok { "ok" } else { "FAILED" });
        if !ok {
            let by = block.1.and_then(|e| names.get(e).ok()).map(|n| n.to_string());
            info!("input-test:   cursor {:?} blocked {} by {:?} hover {:?} selection {:?}", pointer.cursor, block.0, by, hover.ship, sel.ships.len());
            if let Some(c) = pointer.cursor {
                info!("input-test:   battle camera {:?}", battle_cam.iter().collect::<Vec<_>>());
                for (e, n, g, v, img, inter, bg, fp, cam, z) in &all_nodes {
                    if n.size().x > 0.0 && n.contains_point(*g, c * sf) {
                        info!("input-test:     node {:?} vis {} image {} interaction {:?} focus {:?} z {:?} camera {:?} bg {:?} size {:?}", names.get(e).ok().map(|n| n.as_str().to_string()), v.get(), img, inter, fp, z.map(|z| z.0), cam.get(), bg.map(|b| b.0.alpha()), n.size());
                    }
                }
            }
        }
        st.results.push((name, ok));
        st.phase += 1;
        st.t = 0;
    };
    match st.phase {
        // 1: click a ship of the player.
        0 => {
            let own = ships
                .0
                .values()
                .filter(|s| s.side == player && !rt.catalog.ship(s.def).is_static)
                .filter_map(|s| cam.world_to_viewport(gt, s.pos).ok().map(|p| (s.id, p)))
                .filter(|(_, p)| p.x > vp.x * 0.1 && p.x < vp.x * 0.9 && p.y > vp.y * 0.1 && p.y < vp.y * 0.6)
                .min_by(|a, b| a.0.cmp(&b.0));
            match (t, own) {
                (0, Some((_, p))) => act = Some(Act::Move(p)),
                (1, _) => act = Some(Act::Down(MouseButton::Left)),
                (2, _) => act = Some(Act::Up(MouseButton::Left)),
                (5, _) => {
                    let ok = !sel.ships.is_empty();
                    finish_phase(&mut st, "select-ship", ok);
                }
                (0, None) if st.ready_frames > 2000 => finish_phase(&mut st, "select-ship (no own ship on screen)", false),
                _ => {}
            }
        }
        // 2: right click on open space orders the selection.
        1 => match t {
            0 => act = Some(Act::Move(Vec2::new(vp.x * 0.62, vp.y * 0.22))),
            1 => act = Some(Act::Down(MouseButton::Right)),
            2 => act = Some(Act::Up(MouseButton::Right)),
            5 => {
                let ok = st.orders > 0;
                finish_phase(&mut st, "right-click-order", ok);
            }
            _ => {}
        },
        // 2.5: control experiment, a plain Button node of our own must get Hovered/Pressed from the same events.
        2 => {
            if t == 0 {
                let mut e = diag_commands.spawn((
                    Name::new("input-test probe"),
                    Button,
                    Node { position_type: PositionType::Absolute, left: Val::Percent(40.0), top: Val::Percent(4.0), width: Val::Percent(10.0), height: Val::Percent(5.0), ..default() },
                    BackgroundColor(Color::srgba(1.0, 0.0, 0.0, 0.3)),
                    GlobalZIndex(100),
                ));
                if let Some(cam) = battle_cam.iter().next() {
                    e.insert(UiTargetCamera(cam));
                }
                st.probe = Some(e.id());
            }
            let state = st.probe.and_then(|e| interactions.get(e).ok().map(|(_, i)| *i));
            match t {
                1 => act = Some(Act::Move(Vec2::new(vp.x * 0.45, vp.y * 0.065))),
                3 => {
                    info!("input-test:   probe button after move: {state:?}");
                    st.probe_hover = state == Some(Interaction::Hovered);
                }
                4 => act = Some(Act::Down(MouseButton::Left)),
                5 => {
                    info!("input-test:   probe button after press: {state:?}");
                    st.probe_press = state == Some(Interaction::Pressed);
                    act = Some(Act::Up(MouseButton::Left));
                }
                8 => {
                    if let Some(e) = st.probe.take() {
                        diag_commands.entity(e).despawn();
                    }
                    let ok = st.probe_hover && st.probe_press;
                    finish_phase(&mut st, "plain-button-hover-press", ok);
                }
                _ => {}
            }
        }
        // 3: click a command bar button: it fires and the selection survives.
        3 => {
            // Buttons whose centre is covered by another button are skipped (their click would go to the other one).
            let centres: Vec<(String, Vec2, Vec2)> = bar.iter().filter(|r| r.4.get() && r.2.size().x > 4.0).map(|r| (r.0.name.clone(), r.3.translation, r.2.size())).collect();
            let covered = |name: &str, c: Vec2| {
                centres.iter().any(|(o, oc, os)| o != name && (c.x - oc.x).abs() < os.x * 0.5 && (c.y - oc.y).abs() < os.y * 0.5)
            };
            if t == 0 {
                let overlaps: Vec<String> = centres.iter().filter(|(n, c, _)| covered(n, *c)).map(|(n, ..)| n.clone()).collect();
                info!("input-test:   bar buttons whose centre lies under another button: {overlaps:?}");
            }
            let target = bar
                .iter()
                .filter(|(b, _, n, g, v, _)| v.get() && n.size().x > 4.0 && !covered(&b.name, g.translation))
                .filter(|(b, st, n, _, v, _)| {
                    !st.disabled && v.get() && n.size().x > 4.0 && n.size().y > 4.0 && b.name.starts_with("b_") && !b.name.contains("droid") && !b.name.contains("retreat") && !b.name.contains("reinforce")
                })
                .min_by_key(|(b, ..)| (b.name.contains("camera") || b.name.contains("droid"), !(b.name.contains("stop") || b.name.contains("move") || b.name.contains("attack")), b.name.clone()))
                .map(|(b, _, _, g, _, _)| (b.name.clone(), g.translation / sf));
            if t == 0 {
                let all: Vec<String> = bar.iter().filter(|r| r.4.get() && r.2.size().x > 4.0).map(|r| format!("{}({}x{} dis {})", r.0.name, r.2.size().x, r.2.size().y, r.1.disabled)).collect();
                info!("input-test:   visible bar buttons: {all:?}");
            }
            if t <= 5 {
                    let active: Vec<String> = interactions.iter().filter(|(_, i)| **i != Interaction::None).map(|(e, i)| format!("{:?}={i:?}", names.get(e).ok().map(|n| n.as_str().to_string()))).collect();
                    let btn = bar.iter().find(|r| Some(&r.0.name) == st.button_name.as_ref()).map(|r| r.5.clone());
                    info!(
                        "input-test:   probe t={t} cursor_phys {:?} pressed {} just_pressed {} just_released {} active interactions {:?} button {:?}",
                        window.1.physical_cursor_position(), mouse.pressed(MouseButton::Left), mouse.just_pressed(MouseButton::Left), mouse.just_released(MouseButton::Left), active, btn
                    );
                    if t == 3 {
                        let target_entity = bar.iter().find(|r| Some(&r.0.name) == st.button_name.as_ref()).map(|_| ());
                        let _ = target_entity;
                        if let Some(entity) = all_nodes.iter().find(|n| names.get(n.0).ok().map(|x| x.as_str()) == st.button_name.as_deref()).map(|n| n.0) {
                            let pos = ui_stack.uinodes.iter().position(|x| *x == entity);
                            let part = pos.and_then(|p| ui_stack.partition.iter().find(|r| r.contains(&p)).cloned());
                            if let Some(range) = part {
                                let root = ui_stack.uinodes[range.start];
                                let rn = all_nodes.iter().find(|n| n.0 == root);
                                info!("input-test:   button stack pos {pos:?} partition {range:?} root {:?} root cam {:?} root vis {:?} focus {:?}", names.get(root).ok().map(|n| n.as_str().to_string()), rn.map(|n| n.8.get()), rn.map(|n| n.3.get()), rn.map(|n| n.7.copied()));
                                let above: Vec<String> = ui_stack.uinodes[pos.unwrap_or(0)..range.end].iter().filter_map(|e| all_nodes.iter().find(|n| n.0 == *e)).filter(|n| n.1.size().x > 0.0 && n.1.contains_point(*n.2, window.1.physical_cursor_position().unwrap_or_default())).map(|n| format!("{:?}/{:?}/vis{}", names.get(n.0).ok().map(|x| x.as_str().to_string()), n.7.copied(), n.3.get())).collect();
                                info!("input-test:   nodes above the button under the cursor in its partition: {above:?}");
                            } else {
                                info!("input-test:   button entity {entity:?} not in any partition (stack pos {pos:?})");
                            }
                        }
                        let cur = window.1.physical_cursor_position().unwrap_or_default();
                        let mut hit: Vec<(usize, String)> = Vec::new();
                        for (e, n, g, v, _img, inter, _bg, fp, cam, _z) in &all_nodes {
                            if n.size().x > 0.0 && n.contains_point(*g, cur) {
                                let pos = ui_stack.uinodes.iter().position(|x| *x == e);
                                hit.push((pos.unwrap_or(usize::MAX), format!("{:?} vis {} focus {:?} inter {:?} cam {:?}", names.get(e).ok().map(|n| n.as_str().to_string()), v.get(), fp, inter, cam.get())));
                            }
                        }
                        hit.sort_by(|a, b| b.0.cmp(&a.0));
                        for (pos, d) in hit {
                            info!("input-test:   stack {pos}: {d}");
                        }
                        info!("input-test:   partitions {}", ui_stack.partition.len());
                    }
                    if t == 0 {
                        let cs: Vec<String> = cams.iter().map(|(c, r)| format!("active {} order {} viewport {:?} target {:?}", c.is_active, c.order, c.physical_viewport_rect(), r)).collect();
                        info!("input-test:   cameras {:?}; ui stack nodes {}", cs, ui_stack.uinodes.len());
                    }
            }
            match (t, target) {
                (0, Some((name, p))) => {
                    st.button_name = Some(name);
                    act = Some(Act::Move(p));
                }
                (1, _) => act = Some(Act::Down(MouseButton::Left)),
                (2, _) => act = Some(Act::Up(MouseButton::Left)),
                (6, _) => {
                    let ok = st.button_name.is_some() && st.button_name == st.clicked_button && !sel.ships.is_empty();
                    info!("input-test: button {:?}, got {:?}", st.button_name, st.clicked_button);
                    finish_phase(&mut st, "bar-button-click", ok);
                }
                (0, None) => finish_phase(&mut st, "bar-button-click (no button found)", false),
                _ => {}
            }
        }
        // 4: a click on the bar's selection area does not deselect.
        4 => match t {
            0 => act = Some(Act::Move(Vec2::new(vp.x * 0.7, vp.y * 0.96))),
            1 => act = Some(Act::Down(MouseButton::Left)),
            2 => act = Some(Act::Up(MouseButton::Left)),
            5 => {
                let ok = !sel.ships.is_empty();
                finish_phase(&mut st, "bar-area-keeps-selection", ok);
            }
            _ => {}
        },
        // 5: a left click on open space deselects.
        5 => match t {
            0 => act = Some(Act::Move(Vec2::new(vp.x * 0.5, vp.y * 0.12))),
            1 => act = Some(Act::Down(MouseButton::Left)),
            2 => act = Some(Act::Up(MouseButton::Left)),
            5 => {
                let ok = sel.ships.is_empty();
                finish_phase(&mut st, "empty-click-deselects", ok);
            }
            _ => {}
        },
        _ => {
            let pass = st.results.iter().all(|r| r.1);
            info!("input-test: {} checks {}", st.results.len(), if pass { "PASS" } else { "FAIL" });
            st.done = true;
            if std::env::var("SP_INPUT_TEST").is_ok_and(|v| v == "exit") {
                exit.write(AppExit::Success);
            }
        }
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
        None => {}
    }
}
