//! `--pointer <steps>` (offscreen): a synthetic cursor on named points of the screen, through the real input path.
//!
//! Offscreen runs have no winit, so a window that exists only as data (no OS window, no OS cursor; the same one `--drag-pointer` uses)
//! receives what the OS would write: cursor moves and button presses. Bevy's own focus, hover and click systems do the rest, so the
//! capture shows what a player sees with the mouse on that point: a button's mouse-over or pressed art, the hover card of a number of
//! the header, of a planet or of a building of the planet view.
//!
//! `<steps>` is a list separated by `;`. Each step is `<target>[!<action>]`:
//! * target `x,y`: a point in logical pixels of the picture (fractions of its size when both are at most 1);
//! * target `node:<name>`: the centre of the interface node with that name (a command bar component such as `b_filters0` or
//!   `Text_Day_Credits`);
//! * target `planet:<name>`: the globe of a planet on the map;
//! * target `view:<orbital0|ground0|station>`: a building slot of the open planet view;
//! * action `press` (the left button goes down and stays down), `click` (down and up) or `right` (a right click).
//!
//! The pointer rests [`REST_FRAMES`] frames on every step; the capture waits for the last one ([`crate::offscreen::CaptureGate`]). A
//! magenta cross marks the cursor in the picture. One log line per step: `pointer: step <n> <target> at <x>,<y>`.

use bevy::input::ButtonState;
use bevy::input::InputSystems;
use bevy::input::mouse::MouseButtonInput;
use bevy::prelude::*;
use bevy::ui::{ComputedNode, UiGlobalTransform};
use bevy::window::{CursorMoved, PrimaryWindow, WindowResolution};

use crate::cli::Args;
use crate::galaxy_view::GalaxyView;

/// Frames the pointer rests on a step before the next one (or the capture): hover cards need three frames to lay out.
const REST_FRAMES: u32 = 30;
/// Frame of a step at which its button action starts, and how long a click holds the button.
const ACTION_FRAME: u32 = 8;
const CLICK_HOLD: u32 = 4;

/// Registers the probe.
pub struct PointerProbePlugin;

impl Plugin for PointerProbePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreUpdate, pointer_probe.before(InputSystems).run_if(resource_exists::<crate::offscreen::OffscreenTarget>));
    }
}

/// One step of `--pointer`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Step {
    pub(crate) target: Target,
    pub(crate) action: Option<Action>,
}

/// Where a step puts the cursor.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Target {
    Point(Vec2),
    Node(String),
    Planet(String),
    View(String),
}

/// What a step does with the mouse buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    Press,
    Click,
    Right,
}

/// Parses the `--pointer` value; `Err` names the step that is not understood.
pub(crate) fn parse_steps(spec: &str) -> Result<Vec<Step>, String> {
    spec.split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            let (target, action) = match s.rsplit_once('!') {
                Some((t, a)) => (t.trim(), Some(a.trim())),
                None => (s, None),
            };
            let action = match action {
                None => None,
                Some("press") => Some(Action::Press),
                Some("click") => Some(Action::Click),
                Some("right") => Some(Action::Right),
                Some(a) => return Err(format!("unknown pointer action `{a}` (press, click, right)")),
            };
            let target = if let Some(n) = target.strip_prefix("node:") {
                Target::Node(n.trim().to_owned())
            } else if let Some(n) = target.strip_prefix("planet:") {
                Target::Planet(n.trim().to_owned())
            } else if let Some(n) = target.strip_prefix("view:") {
                Target::View(n.trim().to_owned())
            } else {
                let xy = target.split_once(',').and_then(|(x, y)| Some(Vec2::new(x.trim().parse().ok()?, y.trim().parse().ok()?)));
                Target::Point(xy.ok_or_else(|| format!("bad pointer target `{target}` (x,y | node:<name> | planet:<name> | view:<slot>)"))?)
            };
            Ok(Step { target, action })
        })
        .collect()
}

#[derive(Default)]
struct Probe {
    steps: Option<Vec<Step>>,
    /// Frames on the galaxy screen with the scripted scenario settled.
    ready_frames: u32,
    step: usize,
    /// Frames spent on the current step since its target was found.
    t: u32,
    /// Frames the current step's target could not be found.
    missing: u32,
    done: bool,
}

/// Marks the cross drawn at the synthetic cursor.
#[derive(Component)]
struct ProbeCross;

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn pointer_probe(
    mut commands: Commands,
    mut st: Local<Probe>,
    (args, target_image, mut gate): (Res<Args>, Res<crate::offscreen::OffscreenTarget>, ResMut<crate::offscreen::CaptureGate>),
    (hold, screen, ui_scale): (Res<crate::play::ScriptHold>, Res<State<crate::flow::AppScreen>>, Res<UiScale>),
    (view, planet_view): (Res<GalaxyView>, Res<crate::planet_view::PlanetViewState>),
    camera: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    nodes: Query<(&Name, &ComputedNode, &UiGlobalTransform, &InheritedVisibility)>,
    mut window: Query<(Entity, &mut Window), With<PrimaryWindow>>,
    mut cross: Query<&mut Node, With<ProbeCross>>,
    (mut moved, mut buttons): (MessageWriter<CursorMoved>, MessageWriter<MouseButtonInput>),
) {
    let Some(spec) = args.pointer.as_deref() else { return };
    if st.done {
        return;
    }
    if st.steps.is_none() {
        match parse_steps(spec) {
            Ok(steps) => st.steps = Some(steps),
            Err(e) => {
                eprintln!("error: --pointer: {e}");
                std::process::exit(2);
            }
        }
        gate.0 += 1;
    }
    if *screen.get() != crate::flow::AppScreen::Galaxy || !hold.settled() {
        return;
    }
    // The window exists only as data: offscreen runs have no winit, so nothing reaches the desktop or the real cursor.
    let Ok((win_entity, mut win)) = window.single_mut() else {
        let (w, h) = crate::offscreen::size();
        commands.spawn((Window { resolution: WindowResolution::new(w, h).with_scale_factor_override(target_image.scale_factor), focused: true, ..default() }, PrimaryWindow));
        commands.spawn((
            ProbeCross,
            Node { position_type: PositionType::Absolute, width: Val::Px(9.0), height: Val::Px(9.0), border: UiRect::all(Val::Px(1.0)), left: Val::Px(-50.0), ..default() },
            BorderColor::all(Color::srgb(1.0, 0.2, 1.0)),
            GlobalZIndex(2000),
            bevy::ui::FocusPolicy::Pass,
        ));
        return;
    };
    st.ready_frames += 1;
    // The planet view (if any) flies in first; a plain map needs a moment for the bar and the labels.
    let wait = if args.planet_view.is_some() { 190 } else { 45 };
    if st.ready_frames < wait {
        return;
    }
    let steps = st.steps.clone().unwrap_or_default();
    let Some(step) = steps.get(st.step) else {
        st.done = true;
        gate.0 = gate.0.saturating_sub(1);
        return;
    };
    let logical = Vec2::new(win.width(), win.height());
    let inv = 1.0 / win.scale_factor().max(1e-3);
    let cam = camera.iter().next();
    let project = |p: Vec3| cam.and_then(|(c, gt)| c.world_to_viewport(gt, p).ok());
    let at = match &step.target {
        Target::Point(p) if p.x <= 1.0 && p.y <= 1.0 => Some(*p * logical),
        Target::Point(p) => Some(*p),
        Target::Node(name) => nodes.iter().find(|(n, c, _, v)| v.get() && c.size().x > 1.0 && n.as_str().eq_ignore_ascii_case(name)).map(|(_, _, g, _)| g.translation * inv),
        Target::Planet(name) => view
            .planets
            .iter()
            .find(|p| p.id.eq_ignore_ascii_case(name) || p.display_name.eq_ignore_ascii_case(name))
            .and_then(|p| project(crate::galaxy_map::galaxy_to_world(p.position))),
        Target::View(what) => crate::planet_view::probe_point(&planet_view, what).and_then(project),
    };
    let Some(at) = at else {
        st.missing += 1;
        if st.missing > 240 {
            warn!("pointer: step {} {:?}: target not found, skipped", st.step, step.target);
            st.step += 1;
            st.t = 0;
            st.missing = 0;
        }
        return;
    };
    if st.t == 0 {
        info!("pointer: step {} {:?} at {:.0},{:.0}", st.step, step.target, at.x, at.y);
    }
    // The cursor follows its target every frame (a planet or a card can still move), like a hand that keeps the mouse on it.
    if win.cursor_position() != Some(at) {
        win.bypass_change_detection().set_cursor_position(Some(at));
        moved.write(CursorMoved { window: win_entity, position: at, delta: None });
    }
    let us = ui_scale.0.max(0.1);
    for mut n in &mut cross {
        n.left = Val::Px(at.x / us - 4.5);
        n.top = Val::Px(at.y / us - 4.5);
    }
    let press = |b: MouseButton, s: ButtonState, out: &mut MessageWriter<MouseButtonInput>| {
        out.write(MouseButtonInput { button: b, state: s, window: win_entity });
    };
    match (step.action, st.t) {
        (Some(Action::Press | Action::Click), ACTION_FRAME) => press(MouseButton::Left, ButtonState::Pressed, &mut buttons),
        (Some(Action::Click), t) if t == ACTION_FRAME + CLICK_HOLD => press(MouseButton::Left, ButtonState::Released, &mut buttons),
        (Some(Action::Right), ACTION_FRAME) => press(MouseButton::Right, ButtonState::Pressed, &mut buttons),
        (Some(Action::Right), t) if t == ACTION_FRAME + CLICK_HOLD => press(MouseButton::Right, ButtonState::Released, &mut buttons),
        _ => {}
    }
    st.t += 1;
    if st.t >= REST_FRAMES {
        let last = st.step + 1 >= steps.len();
        // A held button is let go when the pointer moves on, never before the capture.
        if step.action == Some(Action::Press) && !last {
            press(MouseButton::Left, ButtonState::Released, &mut buttons);
        }
        st.step += 1;
        st.t = 0;
        st.missing = 0;
        if last {
            st.done = true;
            gate.0 = gate.0.saturating_sub(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_are_parsed() {
        let steps = parse_steps("node:b_filters0!click; 0.5,0.3 ;planet:The Kuat System!right;view:orbital1!press").unwrap();
        assert_eq!(
            steps,
            vec![
                Step { target: Target::Node("b_filters0".into()), action: Some(Action::Click) },
                Step { target: Target::Point(Vec2::new(0.5, 0.3)), action: None },
                Step { target: Target::Planet("The Kuat System".into()), action: Some(Action::Right) },
                Step { target: Target::View("orbital1".into()), action: Some(Action::Press) },
            ]
        );
    }

    #[test]
    fn a_bad_step_is_named() {
        assert!(parse_steps("node:x!hover").unwrap_err().contains("hover"));
        assert!(parse_steps("somewhere").unwrap_err().contains("somewhere"));
    }
}
