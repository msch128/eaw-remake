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
    bar: Query<(&EawButton, &eaw_ui::EawButtonState, &ComputedNode, &UiGlobalTransform, &InheritedVisibility)>,
    mut exit: MessageWriter<AppExit>,
    diag: (Res<crate::control::UiBlock>, Res<crate::control::Pointer>, Res<crate::control::Hover>, Query<&Name>),
) {
    let (block, pointer, hover, names) = diag;
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
        // 3: click a command bar button: it fires and the selection survives.
        2 => {
            let target = bar
                .iter()
                .filter(|(b, st, n, _, v)| {
                    !st.disabled && v.get() && n.size().x > 4.0 && n.size().y > 4.0 && b.name.starts_with("b_") && !b.name.contains("retreat") && !b.name.contains("reinforce")
                })
                .min_by(|a, b| a.0.name.cmp(&b.0.name))
                .map(|(b, _, n, g, _)| (b.name.clone(), g.translation * n.inverse_scale_factor()));
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
        3 => match t {
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
        4 => match t {
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
