//! Opt-in self test of the real pointer path of the galaxy screens (`SP_UI_TEST=1`, or `=exit` to quit afterwards).
//!
//! Needs a real window (`space-peace --hidden ...`) and a scenario in which a battle is pending for the player
//! (`--select <planet> --weeks 8 --move <planet>`). Like `battle-view`'s input test it writes what the window system
//! would write (`CursorMoved`, `MouseButtonInput`, `KeyboardInput`, the window's cursor position) and lets Bevy's input
//! and UI focus systems do the rest. Checks: a click on the battle banner's Auto-Resolve button decides the battle;
//! Esc opens the pause menu; Save Game and a slot click write the save file; Options opens and Esc closes it; Main Menu
//! leaves the game. The verdict is one log line `ui-test: ... PASS|FAIL`. `SP_SAVES_DIR` should point at a scratch
//! folder so that the player's saves stay untouched.

use bevy::input::ButtonState;
use bevy::input::InputSystems;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::mouse::MouseButtonInput;
use bevy::prelude::*;
use bevy::window::{CursorMoved, PrimaryWindow};
use eaw_ui::EawButton;

use crate::flow::{AppScreen, Paused};
use crate::menu::{MenuAction, MenuButton};
use crate::newgame::{SlotMode, SlotScreen};
use crate::options::OptionsOpen;
use crate::play_ui::BattleDialogState;

pub(crate) fn build(app: &mut App) {
    if std::env::var_os("SP_UI_TEST").is_some() {
        app.init_resource::<TestState>().add_systems(PreUpdate, drive.before(InputSystems));
    }
}

#[derive(Resource, Default)]
struct TestState {
    phase: u32,
    t: u32,
    started: Option<std::time::SystemTime>,
    results: Vec<(&'static str, bool)>,
    done: bool,
    /// Phase clock value at which the phase's target was first on screen.
    found_at: Option<u32>,
    /// Value of the filter under test before its click.
    before: bool,
}

#[derive(Clone)]
enum Act {
    Move(Vec2),
    Down,
    Up,
    Key(KeyCode, Key),
}

/// Presses a button: move, press, release at fixed frame offsets of the phase clock.
fn click(st: &mut TestState, target: Option<Vec2>) -> Option<Act> {
    if target.is_some() && st.found_at.is_none() {
        st.found_at = Some(st.t.saturating_sub(1));
    }
    let t = st.found_at.map_or(u32::MAX, |f| (st.t - 1).saturating_sub(f));
    match (t, target) {
        (0, Some(p)) => {
            info!("ui-test: moving the pointer to {p:?}");
            Some(Act::Move(p))
        }
        (2, _) => Some(Act::Down),
        (4, _) => Some(Act::Up),
        _ => None,
    }
}

fn menu_button(nodes: &Query<(&MenuButton, &ComputedNode, &UiGlobalTransform, &InheritedVisibility)>, wanted: &MenuAction) -> Option<Vec2> {
    nodes
        .iter()
        .find(|(b, n, _, v)| &b.0 == wanted && v.get() && n.size().x > 4.0)
        .map(|(_, _, g, _)| g.translation)
}

#[allow(clippy::too_many_arguments)]
fn drive(
    mut st: ResMut<TestState>,
    screen: Res<State<AppScreen>>,
    dialog: Option<Res<BattleDialogState>>,
    paused: Res<Paused>,
    slots: Res<SlotScreen>,
    options: Res<OptionsOpen>,
    (filters, bar, frames): (Res<crate::galaxy_map::MapFilters>, Res<crate::play_ui::BarState>, Res<bevy::diagnostic::FrameCount>),
    sim: Option<Res<galaxy_play::SimState>>,
    banner: Query<(&EawButton, &ComputedNode, &UiGlobalTransform, &InheritedVisibility)>,
    menu: Query<(&MenuButton, &ComputedNode, &UiGlobalTransform, &InheritedVisibility)>,
    diag: Query<(Entity, Option<&Name>, &ComputedNode, &UiGlobalTransform, &InheritedVisibility, Option<&Interaction>, Option<&GlobalZIndex>)>,
    window: Option<Single<(Entity, &mut Window), With<PrimaryWindow>>>,
    mut moved: MessageWriter<CursorMoved>,
    mut buttons: MessageWriter<MouseButtonInput>,
    mut keys: MessageWriter<KeyboardInput>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(mut window) = window else { return };
    if st.done {
        return;
    }
    let t = st.t;
    st.t += 1;
    let mut act: Option<Act> = None;
    let finish = |st: &mut TestState, name: &'static str, ok: bool| {
        if !ok {
            for (b, n, g, v) in &menu {
                info!("ui-test:   menu button {:?} visible {} size {:?} at {:?}", b.0, v.get(), n.size(), g.translation);
            }
            if let Some(c) = window.1.physical_cursor_position() {
                for (e, n, node, g, v, i, z) in &diag {
                    if v.get() && node.size().x > 0.0 && node.contains_point(*g, c) {
                        info!("ui-test:   under the cursor: {e:?} {:?} interaction {i:?} z {:?}", n.map(|n| n.as_str()), z.map(|z| z.0));
                    }
                }
            }
        }
        info!("ui-test: {name} {}", if ok { "ok" } else { "FAILED" });
        st.results.push((name, ok));
        st.phase += 1;
        st.t = 0;
        st.found_at = None;
    };
    if t > 1500 && st.phase > 0 && (st.phase < 10 || (100..115).contains(&st.phase)) {
        finish(&mut st, "phase timed out", false);
        return;
    }
    match st.phase {
        // 1: the battle banner's Auto-Resolve button.
        0 => {
            let open = dialog.as_ref().is_some_and(|d| d.open.is_some());
            if st.started.is_none() && open {
                st.started = Some(std::time::SystemTime::now());
                st.t = 0;
                return;
            }
            let target = banner
                .iter()
                .find(|(b, n, _, v)| b.name == eaw_ui::BANNER_AUTO && v.get() && n.size().x > 4.0)
                .map(|(_, _, g, _)| g.translation);
            if st.started.is_some() && target.is_some() {
                act = click(&mut st, target);
            }
            if st.started.is_some() && !open && dialog.as_ref().is_some_and(|d| d.decided.is_some()) {
                finish(&mut st, "banner-auto-resolve-click", true);
                st.phase = 100;
            } else if t > 3000 {
                finish(&mut st, "banner-auto-resolve-click (no banner, or the click did not arrive)", false);
            }
        }
        // Filter toggles next to the minimap: each click flips its map filter (credits, base level, ability, heroes).
        100..=103 => {
            let i = st.phase - 100;
            let name = format!("b_filters{i}");
            let read = |f: &crate::galaxy_map::MapFilters| [f.credits, f.base_level, f.weather, f.heroes][i as usize];
            if st.found_at.is_none() {
                st.before = read(&filters);
            }
            let target = banner.iter().find(|(b, n, _, v)| b.name == name && v.get() && n.size().x > 4.0).map(|(_, _, g, _)| g.translation);
            act = click(&mut st, target);
            if st.found_at.is_some_and(|f| t == f + 20) {
                let ok = read(&filters) != st.before;
                let label: &'static str = ["filter-credits-click", "filter-base-level-click", "filter-ability-click", "filter-heroes-click"][i as usize];
                finish(&mut st, label, ok);
                if i == 3 {
                    st.phase = 110;
                }
            }
        }
        // Class buttons of the left grid: each click sets the build-bar class filter, a second click on the same one clears it.
        110..=114 => {
            use crate::play_ui::BarClass::{Capital, Corvette, Frigate, Station};
            let (name, want, label): (&str, Option<crate::play_ui::BarClass>, &'static str) = match st.phase {
                110 => ("b_filters_corvette", Some(Corvette), "class-corvette-click"),
                111 => ("b_filters_frigate", Some(Frigate), "class-frigate-click"),
                112 => ("b_filters_capital", Some(Capital), "class-capital-click"),
                113 => ("b_filters_station", Some(Station), "class-station-click"),
                _ => ("b_filters_station", None, "class-station-second-click-clears"),
            };
            let target = banner.iter().find(|(b, n, _, v)| b.name == name && v.get() && n.size().x > 4.0).map(|(_, _, g, _)| g.translation);
            act = click(&mut st, target);
            if st.found_at.is_some_and(|f| t == f + 20) {
                let ok = bar.class == want;
                finish(&mut st, label, ok);
                if st.phase == 115 {
                    st.phase = 1;
                }
            }
        }
        // 2: Esc opens the pause menu.
        1 => match t {
            0 => act = Some(Act::Key(KeyCode::Escape, Key::Escape)),
            10 => {
                let ok = paused.0;
                finish(&mut st, "escape-opens-pause-menu", ok);
            }
            _ => {}
        },
        // 3: Save Game opens the slot list.
        2 => {
            let target = menu_button(&menu, &MenuAction::OpenSlots(SlotMode::Save));
            act = click(&mut st, target);
            if st.found_at.is_some_and(|f| t == f + 20) {
                let ok = slots.0 == Some(SlotMode::Save);
                finish(&mut st, "save-game-opens-slots", ok);
            }
        }
        // 4: a slot click writes the file.
        3 => {
            let target = menu_button(&menu, &MenuAction::Slot(SlotMode::Save, 1));
            act = click(&mut st, target);
            if t > 4 && t.is_multiple_of(20) {
                let fresh = crate::newgame::slot_path(1)
                    .and_then(|f| std::fs::metadata(f).ok())
                    .and_then(|m| m.modified().ok())
                    .is_some_and(|m| st.started.is_some_and(|s| m >= s));
                if fresh || t > 1200 {
                    finish(&mut st, "slot-click-writes-save", fresh);
                }
            }
        }
        // 5: back out of the slot list with its Back button.
        4 => {
            if slots.0.is_none() {
                finish(&mut st, "slot-list-closes-after-save", true);
            } else {
                let target = menu_button(&menu, &MenuAction::CloseSlots);
                act = click(&mut st, target);
                if st.found_at.is_some_and(|f| t == f + 20) {
                    let ok = slots.0.is_none();
                    finish(&mut st, "slot-list-back", ok);
                }
            }
        }
        // 6: Options opens.
        5 => {
            // Saving resumes the game: bring the pause menu back first.
            let target = if paused.0 { menu_button(&menu, &MenuAction::Options) } else { None };
            if !paused.0 && t.is_multiple_of(30) {
                act = Some(Act::Key(KeyCode::Escape, Key::Escape));
            }
            if paused.0 {
                act = click(&mut st, target);
            }
            if st.found_at.is_some_and(|f| t == f + 20) {
                let ok = options.0;
                finish(&mut st, "options-opens", ok);
            }
        }
        // 7: Esc closes it.
        6 => match t {
            0 => act = Some(Act::Key(KeyCode::Escape, Key::Escape)),
            10 => {
                let ok = !options.0;
                finish(&mut st, "escape-closes-options", ok);
            }
            _ => {}
        },
        // 8: Load Game opens the slot list.
        7 => {
            let target = menu_button(&menu, &MenuAction::OpenSlots(SlotMode::Load));
            act = click(&mut st, target);
            if st.found_at.is_some_and(|f| t == f + 20) {
                let ok = slots.0 == Some(SlotMode::Load);
                finish(&mut st, "load-game-opens-slots", ok);
            }
        }
        // 9: a slot click loads the save and closes the menu.
        8 => {
            let target = menu_button(&menu, &MenuAction::Slot(SlotMode::Load, 1));
            act = click(&mut st, target);
            if st.found_at.is_some_and(|f| t == f + 40) {
                let ok = slots.0.is_none() && !paused.0;
                finish(&mut st, "slot-click-loads-save", ok);
            }
        }
        // 10: Main Menu leaves the game.
        9 => {
            // The load restarts the galaxy behind its loading overlay; wait for the map.
            if *screen.get() == AppScreen::Galaxy && (sim.as_ref().is_none_or(|s| s.current.loading) || t < 200) {
                st.found_at = None;
                return;
            }
            let target = if paused.0 { menu_button(&menu, &MenuAction::MainMenu) } else { None };
            if !paused.0 && t.is_multiple_of(30) {
                act = Some(Act::Key(KeyCode::Escape, Key::Escape));
            }
            if paused.0 {
                act = click(&mut st, target);
            }
            if st.found_at.is_some_and(|f| t == f + 60) {
                let ok = *screen.get() == AppScreen::MainMenu;
                finish(&mut st, "main-menu-button", ok);
            }
        }
        _ => {
            let pass = st.results.iter().all(|r| r.1);
            info!("ui-test: {} checks {} (frame {})", st.results.len(), if pass { "PASS" } else { "FAIL" }, frames.0);
            st.done = true;
            if std::env::var("SP_UI_TEST").is_ok_and(|v| v == "exit") {
                exit.write(AppExit::Success);
            }
        }
    }
    let (entity, win) = &mut *window;
    match act {
        Some(Act::Move(p)) => {
            win.set_physical_cursor_position(Some(p.as_dvec2()));
            moved.write(CursorMoved { window: *entity, position: p / win.scale_factor(), delta: None });
        }
        Some(Act::Down) => {
            buttons.write(MouseButtonInput { button: MouseButton::Left, state: ButtonState::Pressed, window: *entity });
        }
        Some(Act::Up) => {
            buttons.write(MouseButtonInput { button: MouseButton::Left, state: ButtonState::Released, window: *entity });
        }
        Some(Act::Key(code, key)) => {
            keys.write(KeyboardInput { key_code: code, logical_key: key.clone(), state: ButtonState::Pressed, text: None, repeat: false, window: *entity });
            keys.write(KeyboardInput { key_code: code, logical_key: key, state: ButtonState::Released, text: None, repeat: false, window: *entity });
        }
        None => {}
    }
}

/// Registers the test when `SP_UI_TEST` is set.
pub(crate) fn plugin(app: &mut App) {
    build(app);
}
