//! `SP_PLAY_SCRIPT=<file>` (offscreen runs only): plays a text script through the real input path, for play tests without a window.
//!
//! An offscreen run has no winit, so a window that exists only as data receives what the OS would write: cursor moves, mouse buttons, the
//! wheel and keys. The galaxy map, the planet view and the tactical battle read that window like a real one (the battle's
//! `synthetic_pointer` is switched off). Bevy's UI focus system only serves cameras that render to a window, so `emulate_focus` repeats
//! its rules for the offscreen target: the top node under the cursor is hovered / pressed, `FocusPolicy::Block` stops the search, a
//! release clears the press. Nothing reaches the desktop, the real cursor or an audio device.
//!
//! The script is read line by line; `#` starts a comment. With `SP_PLAY_LIVE=1` the run does not end at the last line: it blocks (no
//! frame is simulated or drawn, set `SP_FRAME_DT_MS` so game time does not follow the wall clock) until more lines are appended to the
//! file, and gives up after `SP_PLAY_IDLE_EXIT` seconds (default 900) without one. After every batch `<file>.done` holds the number of
//! lines consumed; dumps are appended to `<file>.out`. Pictures go next to the `--offscreen` path as `<name>.png`.
//!
//! Pointer targets: `X Y` (logical pixels), `@btn:<name>` (an `EawButton`), `@text:<part>[#n]` (the n-th visible text that contains
//! the part; the click goes to its clickable ancestor), `@planet:<name>`, `@fleet:<id>`, `@ship:<id>`, `@world:<x>,<z>` (battle plane).
//!
//! Commands:
//! `wait N` frames, `waitscreen menu|campaigns|galaxy|battle [N]`, `waitweek N`, `waitbattle S` (battle clock), `waitover`,
//! `waitbegin` (the battle's Begin button), `shot NAME`, `move T`, `glide T [frames]`, `click T`, `rclick T`, `dclick T`, `ldown`,
//! `lup`, `rdown`, `rup`, `mdown`, `mup`, `wheel N`, `key K`, `keydown K`, `keyup K`, `send BUTTON` (the button's message, no pointer),
//! `speed L` (galaxy speed level), `advance WEEKS` (galaxy fast forward), `bspeed X`, `gcam PLANET ZOOM`, `gcamrel DX DZ`,
//! `bcam X Z DIST [YAW PITCH]`, `bcam @ship:ID DIST [YAW PITCH]`, `grant UNIT PLANET [COUNT]`, `save FILE`,
//! `dump ui|text [X0 Y0 X1 Y1]`, `dump galaxy|planets|planet NAME|battle|squads`, `burst NAME N` (N consecutive frames),
//! `cross on|off` (the square that marks the pointer in the captures), `echo TEXT`, `quit`.

use std::{collections::HashMap, io::Write, path::PathBuf};

use bevy::{
    camera::{ImageRenderTarget, RenderTarget},
    ecs::change_detection::Tick,
    input::{
        ButtonState, InputSystems,
        keyboard::{Key, KeyboardInput},
        mouse::{MouseButtonInput, MouseMotion, MouseScrollUnit, MouseWheel},
    },
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
    ui::{ComputedNode, FocusPolicy, OverrideClip, RelativeCursorPosition, UiGlobalTransform, UiStack, UiSystems},
    window::{CursorMoved, PrimaryWindow, WindowResolution},
};
use battle_view::{BattleCamera, BattleRuntime, BattleStatus, BattleViewSettings, RenderedShips};
use eaw_ui::{EawButton, EawButtonClicked, EawButtonState};
use galaxy_play::{FleetEntities, PlayAction, PlayerFaction, PlayerSelection, SimState};
use galaxy_sim::FleetLocation;

use crate::camera::{CameraBounds, GalaxyCamera};
use crate::flow::AppScreen;
use crate::galaxy_map::galaxy_to_world;
use crate::galaxy_view::GalaxyView;
use crate::offscreen::OffscreenTarget;

/// Registers the script player when `SP_PLAY_SCRIPT` names a file and the run is offscreen.
pub(crate) fn plugin(app: &mut App) {
    let Some(path) = std::env::var_os("SP_PLAY_SCRIPT").map(PathBuf::from) else { return };
    if app.world().get_resource::<OffscreenTarget>().is_none() {
        warn!("playscript: SP_PLAY_SCRIPT needs --offscreen; ignored");
        return;
    }
    let live = std::env::var_os("SP_PLAY_LIVE").is_some();
    let idle_exit = std::env::var("SP_PLAY_IDLE_EXIT").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(900.0);
    info!("playscript: {} (live {live})", path.display());
    app.insert_resource(Script { path, live, idle_exit, consumed: 0, current: None, frames: 0, aux: 0.0, cursor: Vec2::new(40.0, 40.0), frame: 0, shot_wait: None, glide_from: Vec2::ZERO, mark: true })
        .init_resource::<FocusEmu>()
        .add_systems(PreUpdate, (drive.before(InputSystems), remember_interactions.before(UiSystems::Focus), emulate_focus.after(UiSystems::Focus)));
}

/// The script and where it stands.
#[derive(Resource)]
struct Script {
    path: PathBuf,
    live: bool,
    idle_exit: f32,
    /// Complete lines of the file already taken.
    consumed: usize,
    /// The command being executed.
    current: Option<Vec<String>>,
    /// Frames the current command has run.
    frames: u32,
    /// Scratch value of the current command (galactic seconds sent by `advance`).
    aux: f32,
    /// Pointer position in logical pixels.
    cursor: Vec2,
    frame: u64,
    /// A requested picture and the frames waited for its file.
    shot_wait: Option<(PathBuf, u32)>,
    glide_from: Vec2,
    /// The pointer mark is drawn.
    mark: bool,
}

/// The square drawn at the pointer.
#[derive(Component)]
struct PointerMark;

/// `Interaction` values and change ticks as they were before Bevy's focus system reset them, and the nodes held pressed.
#[derive(Resource, Default)]
struct FocusEmu {
    before: HashMap<Entity, (Interaction, Tick)>,
}

fn remember_interactions(mut emu: ResMut<FocusEmu>, nodes: Query<(Entity, Ref<Interaction>)>) {
    emu.before.clear();
    for (e, i) in &nodes {
        emu.before.insert(e, (*i, i.last_changed()));
    }
}

/// What `bevy_ui::ui_focus_system` does for a window camera, for the offscreen target and the data window's cursor.
#[allow(clippy::type_complexity)]
fn emulate_focus(
    emu: Res<FocusEmu>,
    window: Query<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    ui_stack: Res<UiStack>,
    mut nodes: Query<(Entity, &ComputedNode, &UiGlobalTransform, Option<&mut Interaction>, Option<&mut RelativeCursorPosition>, Option<&FocusPolicy>, Option<&InheritedVisibility>)>,
    clipping: Query<(&ComputedNode, &UiGlobalTransform, &Node)>,
    child_of: Query<&ChildOf, Without<OverrideClip>>,
) {
    let cursor = window.single().ok().and_then(Window::physical_cursor_position);
    let released = mouse.just_released(MouseButton::Left);
    let clicked = mouse.just_pressed(MouseButton::Left);
    // Start from the state before Bevy's reset; a release ends every press.
    let mut want: HashMap<Entity, Interaction> = HashMap::new();
    for (e, _, _, interaction, ..) in &nodes {
        if interaction.is_some() {
            let before = emu.before.get(&e).map_or(Interaction::None, |b| b.0);
            want.insert(e, if before == Interaction::Pressed && released { Interaction::None } else { before });
        }
    }
    let mut hovered: Vec<Entity> = Vec::new();
    for range in ui_stack.partition.iter().rev() {
        for &entity in ui_stack.uinodes[range.clone()].iter().rev() {
            let Ok((_, node, tf, _, relative, _, visible)) = nodes.get_mut(entity) else { continue };
            let Some(visible) = visible else { continue };
            if !visible.get() {
                want.entry(entity).and_modify(|w| *w = Interaction::None);
                continue;
            }
            let over = cursor.is_some_and(|p| node.contains_point(*tf, p) && bevy::ui::clip_check_recursive(p, entity, &clipping, &child_of));
            if let Some(mut r) = relative {
                let value = RelativeCursorPosition { cursor_over: over, normalized: cursor.and_then(|p| node.normalize_point(*tf, p)) };
                r.set_if_neq(value);
            }
            if over {
                hovered.push(entity);
            } else if let Some(w) = want.get_mut(&entity)
                && (*w == Interaction::Hovered || cursor.is_none())
            {
                *w = Interaction::None;
            }
        }
    }
    let mut blocked = false;
    for &entity in &hovered {
        let Ok((_, _, _, _, _, policy, _)) = nodes.get(entity) else { continue };
        if blocked {
            if let Some(w) = want.get_mut(&entity)
                && *w != Interaction::Pressed
            {
                *w = Interaction::None;
            }
            continue;
        }
        if let Some(w) = want.get_mut(&entity) {
            if clicked {
                *w = Interaction::Pressed;
            } else if *w == Interaction::None {
                *w = Interaction::Hovered;
            }
        }
        if *policy.unwrap_or(&FocusPolicy::Block) == FocusPolicy::Block {
            blocked = true;
        }
    }
    for (e, _, _, interaction, ..) in &mut nodes {
        let (Some(mut interaction), Some(&w)) = (interaction, want.get(&e)) else { continue };
        match emu.before.get(&e) {
            // Unchanged for the game: put back the value and the tick Bevy's reset overwrote.
            Some(&(before, tick)) if before == w => {
                if *interaction != w {
                    *interaction.bypass_change_detection() = w;
                }
                interaction.set_last_changed(tick);
            }
            _ => {
                if *interaction != w {
                    *interaction = w;
                }
            }
        }
    }
}

fn key_of(name: &str) -> Option<(KeyCode, Key)> {
    use KeyCode as K;
    let named = |code: KeyCode, key: Key| Some((code, key));
    use Key as N;
    let lower = name.to_ascii_lowercase();
    if lower.len() == 1 {
        let c = lower.chars().next()?;
        let code = match c {
            'a' => K::KeyA, 'b' => K::KeyB, 'c' => K::KeyC, 'd' => K::KeyD, 'e' => K::KeyE, 'f' => K::KeyF, 'g' => K::KeyG, 'h' => K::KeyH, 'i' => K::KeyI,
            'j' => K::KeyJ, 'k' => K::KeyK, 'l' => K::KeyL, 'm' => K::KeyM, 'n' => K::KeyN, 'o' => K::KeyO, 'p' => K::KeyP, 'q' => K::KeyQ, 'r' => K::KeyR,
            's' => K::KeyS, 't' => K::KeyT, 'u' => K::KeyU, 'v' => K::KeyV, 'w' => K::KeyW, 'x' => K::KeyX, 'y' => K::KeyY, 'z' => K::KeyZ,
            '0' => K::Digit0, '1' => K::Digit1, '2' => K::Digit2, '3' => K::Digit3, '4' => K::Digit4, '5' => K::Digit5, '6' => K::Digit6, '7' => K::Digit7,
            '8' => K::Digit8, '9' => K::Digit9,
            _ => return None,
        };
        return Some((code, Key::Character(lower.as_str().into())));
    }
    match lower.as_str() {
        "esc" | "escape" => named(K::Escape, N::Escape),
        "space" => named(K::Space, N::Space),
        "enter" => named(K::Enter, N::Enter),
        "tab" => named(K::Tab, N::Tab),
        "backspace" => named(K::Backspace, N::Backspace),
        "delete" | "del" => named(K::Delete, N::Delete),
        "home" => named(K::Home, N::Home),
        "end" => named(K::End, N::End),
        "pageup" => named(K::PageUp, N::PageUp),
        "pagedown" => named(K::PageDown, N::PageDown),
        "up" => named(K::ArrowUp, N::ArrowUp),
        "down" => named(K::ArrowDown, N::ArrowDown),
        "left" => named(K::ArrowLeft, N::ArrowLeft),
        "right" => named(K::ArrowRight, N::ArrowRight),
        "shift" => named(K::ShiftLeft, N::Shift),
        "ctrl" | "control" => named(K::ControlLeft, N::Control),
        "alt" => named(K::AltLeft, N::Alt),
        "f1" => named(K::F1, N::F1),
        "f2" => named(K::F2, N::F2),
        "f3" => named(K::F3, N::F3),
        "f4" => named(K::F4, N::F4),
        "f5" => named(K::F5, N::F5),
        "f6" => named(K::F6, N::F6),
        "f7" => named(K::F7, N::F7),
        "f8" => named(K::F8, N::F8),
        "f9" => named(K::F9, N::F9),
        "f10" => named(K::F10, N::F10),
        "f11" => named(K::F11, N::F11),
        "f12" => named(K::F12, N::F12),
        _ => None,
    }
}

enum Step {
    /// The command needs more frames.
    Pending,
    /// The command is finished.
    Done,
    /// Leave the application.
    Quit,
}

fn out_line(script: &Script, text: &str) {
    info!("ps: {text}");
    let mut path = script.path.clone().into_os_string();
    path.push(".out");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{text}");
    }
}

/// Reads the complete lines of the script that were not taken yet.
fn next_line(script: &mut Script) -> Option<Vec<String>> {
    loop {
        let text = std::fs::read_to_string(&script.path).unwrap_or_default();
        let complete = match text.rfind('\n') {
            Some(i) => &text[..=i],
            None => "",
        };
        let line = complete.lines().nth(script.consumed)?;
        script.consumed += 1;
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        return Some(line.split_whitespace().map(str::to_owned).collect());
    }
}

fn write_done(script: &Script, note: &str) {
    let mut path = script.path.clone().into_os_string();
    path.push(".done");
    let _ = std::fs::write(path, format!("{} {note}\n", script.consumed));
}

/// Centre of a UI node in logical pixels.
fn node_centre(world: &World, e: Entity, scale: f32) -> Option<Vec2> {
    let tf = world.get::<UiGlobalTransform>(e)?;
    Some(tf.translation / scale)
}

/// Shortens a text to at most `n` bytes at a character boundary.
fn clip(t: &mut String, n: usize) {
    if t.len() > n {
        let mut cut = n;
        while !t.is_char_boundary(cut) {
            cut -= 1;
        }
        t.truncate(cut);
    }
}

fn node_text(world: &World, e: Entity, out: &mut String, depth: u32) {
    if let Some(t) = world.get::<Text>(e) {
        if !out.is_empty() {
            out.push_str(" | ");
        }
        out.push_str(t.0.replace('\n', " / ").trim());
    }
    if let Some(t) = world.get::<TextSpan>(e) {
        out.push_str(&t.0.replace('\n', " / "));
    }
    if depth < 6
        && let Some(children) = world.get::<Children>(e)
    {
        for c in children.iter() {
            node_text(world, c, out, depth + 1);
        }
    }
}

fn visible_node(world: &World, e: Entity) -> bool {
    world.get::<InheritedVisibility>(e).is_some_and(|v| v.get()) && world.get::<ComputedNode>(e).is_some_and(|n| n.size().x > 2.0 && n.size().y > 2.0)
}

/// Resolves a pointer target to logical pixels.
fn target(world: &mut World, script: &Script, args: &[String]) -> Result<Vec2, String> {
    let scale = world.resource::<OffscreenTarget>().scale_factor;
    let first = args.first().ok_or("missing target")?;
    if let Some(rest) = first.strip_prefix('@') {
        let (kind, value) = rest.split_once(':').ok_or("target needs @kind:value")?;
        return match kind {
            "btn" => {
                let want = value.to_ascii_lowercase();
                let mut q = world.query::<(Entity, &EawButton)>();
                let found: Vec<Entity> = q.iter(world).filter(|(_, b)| b.name == want).map(|(e, _)| e).collect();
                let e = found.into_iter().find(|e| visible_node(world, *e)).ok_or(format!("no visible button `{want}`"))?;
                node_centre(world, e, scale).ok_or("button has no transform".into())
            }
            "text" => {
                let (part, nth) = match value.rsplit_once('#') {
                    Some((p, n)) if n.parse::<usize>().is_ok() => (p, n.parse::<usize>().unwrap_or(0)),
                    _ => (value, 0),
                };
                let part = part.replace('_', " ").to_ascii_lowercase();
                let mut q = world.query::<(Entity, &Text)>();
                let mut found: Vec<(Entity, Vec2)> = Vec::new();
                let all: Vec<(Entity, String)> = q.iter(world).map(|(e, t)| (e, t.0.to_ascii_lowercase())).collect();
                for (e, text) in all {
                    // `@text:=word` wants the whole text, `@text:word` a part of it.
                    let hit = match part.strip_prefix('=') {
                        Some(whole) => text.trim() == whole,
                        None => text.contains(&part),
                    };
                    if hit && visible_node(world, e) {
                        // The clickable ancestor, if there is one.
                        let mut at = e;
                        let mut cur = e;
                        for _ in 0..8 {
                            if world.get::<Interaction>(cur).is_some() {
                                at = cur;
                                break;
                            }
                            match world.get::<ChildOf>(cur) {
                                Some(p) => cur = p.parent(),
                                None => break,
                            }
                        }
                        if let Some(c) = node_centre(world, at, scale) {
                            found.push((at, c));
                        }
                    }
                }
                found.sort_by(|a, b| (a.1.y, a.1.x).partial_cmp(&(b.1.y, b.1.x)).unwrap_or(std::cmp::Ordering::Equal));
                found.get(nth).map(|f| f.1).ok_or(format!("no visible text containing `{part}` (#{nth})"))
            }
            "planet" => {
                let at = {
                    let view = world.resource::<GalaxyView>();
                    let names: Vec<(&str, &str)> = view.planets.iter().map(|p| (p.id.as_str(), p.display_name.as_str())).collect();
                    let i = crate::cli::resolve_name("planet", &value.replace('_', " "), &names)?;
                    galaxy_to_world(view.planets[i].position)
                };
                let mut q = world.query_filtered::<(&Camera, &GlobalTransform), With<GalaxyCamera>>();
                let (cam, tf) = q.iter(world).next().ok_or("no galaxy camera")?;
                cam.world_to_viewport(tf, at).map_err(|e| format!("planet not on screen: {e:?}"))
            }
            "fleet" => {
                let id = value.parse().map_err(|_| "bad fleet id")?;
                let e = *world.get_resource::<FleetEntities>().ok_or("no fleets")?.0.get(&id).ok_or("fleet has no icon")?;
                let at = world.get::<GlobalTransform>(e).ok_or("fleet icon has no transform")?.translation();
                let mut q = world.query_filtered::<(&Camera, &GlobalTransform), With<GalaxyCamera>>();
                let (cam, tf) = q.iter(world).next().ok_or("no galaxy camera")?;
                cam.world_to_viewport(tf, at).map_err(|e| format!("fleet not on screen: {e:?}"))
            }
            "ship" | "world" => {
                let at = if kind == "ship" {
                    let id: u32 = value.parse().map_err(|_| "bad ship id")?;
                    world.get_resource::<RenderedShips>().and_then(|s| s.0.get(&id)).map(|s| s.pos).ok_or(format!("no ship {id}"))?
                } else {
                    let (x, z) = value.split_once(',').ok_or("@world:x,z")?;
                    Vec3::new(x.parse().map_err(|_| "bad x")?, 0.0, z.parse().map_err(|_| "bad z")?)
                };
                let mut q = world.query_filtered::<(&Camera, &GlobalTransform), With<BattleCamera>>();
                let (cam, tf) = q.iter(world).next().ok_or("no battle camera")?;
                cam.world_to_viewport(tf, at).map_err(|e| format!("not on screen: {e:?}"))
            }
            _ => Err(format!("unknown target kind `{kind}`")),
        };
    }
    let _ = script;
    let (x, y) = match first.split_once(',') {
        Some((x, y)) => (x.to_owned(), y.to_owned()),
        None => (first.clone(), args.get(1).cloned().ok_or("target needs X Y")?),
    };
    Ok(Vec2::new(x.parse().map_err(|_| format!("bad x `{x}`"))?, y.parse().map_err(|_| format!("bad y `{y}`"))?))
}

fn window_entity(world: &mut World) -> Option<Entity> {
    let mut q = world.query_filtered::<Entity, With<PrimaryWindow>>();
    q.iter(world).next()
}

fn move_pointer(world: &mut World, script: &mut Script, to: Vec2) {
    let Some(win) = window_entity(world) else { return };
    let delta = to - script.cursor;
    script.cursor = to;
    if let Some(mut w) = world.get_mut::<Window>(win) {
        w.bypass_change_detection().set_cursor_position(Some(to));
    }
    world.write_message(CursorMoved { window: win, position: to, delta: Some(delta) });
    world.write_message(MouseMotion { delta });
}

fn button(world: &mut World, button: MouseButton, state: ButtonState) {
    if let Some(window) = window_entity(world) {
        world.write_message(MouseButtonInput { button, state, window });
    }
}

fn key(world: &mut World, name: &str, state: ButtonState) -> Result<(), String> {
    let (key_code, logical_key) = key_of(name).ok_or(format!("unknown key `{name}`"))?;
    let window = window_entity(world).ok_or("no window")?;
    world.write_message(KeyboardInput { key_code, logical_key, state, text: None, repeat: false, window });
    Ok(())
}

fn screen_of(world: &World) -> AppScreen {
    *world.resource::<State<AppScreen>>().get()
}

fn dump(world: &mut World, script: &Script, what: &[String]) {
    let scale = world.resource::<OffscreenTarget>().scale_factor;
    let (w, h) = crate::offscreen::size();
    let size = Vec2::new(w as f32, h as f32) / scale;
    let kind = what.first().map(String::as_str).unwrap_or("ui");
    // `dump ui|text X0 Y0 X1 Y1` keeps what lies in that rectangle.
    let rect: Option<[f32; 4]> = (what.len() >= 5).then(|| [1, 2, 3, 4].map(|i| what[i].parse::<f32>().unwrap_or(0.0))).filter(|_| kind == "ui" || kind == "text");
    let inside = |c: Vec2| rect.is_none_or(|r| c.x >= r[0] && c.y >= r[1] && c.x <= r[2] && c.y <= r[3]);
    out_line(script, &format!("== dump {} (frame {}, screen {:?}) ==", what.join(" "), script.frame, screen_of(world)));
    match kind {
        "ui" => {
            let mut q = world.query_filtered::<Entity, With<Interaction>>();
            let list: Vec<Entity> = q.iter(world).collect();
            let mut rows: Vec<(Vec2, String)> = Vec::new();
            for e in list {
                if !visible_node(world, e) {
                    continue;
                }
                let Some(c) = node_centre(world, e, scale) else { continue };
                let s = world.get::<ComputedNode>(e).map_or(Vec2::ZERO, |n| n.size() / scale);
                if c.x < -5.0 || c.y < -5.0 || c.x > size.x + 5.0 || c.y > size.y + 5.0 || !inside(c) {
                    continue;
                }
                let mut text = String::new();
                node_text(world, e, &mut text, 0);
                clip(&mut text, 70);
                let name = world.get::<EawButton>(e).map(|b| b.name.clone()).or_else(|| world.get::<Name>(e).map(|n| n.as_str().to_owned())).unwrap_or_default();
                let state = world.get::<EawButtonState>(e).map(|s| format!("{}{}", if s.selected { " SEL" } else { "" }, if s.disabled { " DIS" } else { "" })).unwrap_or_default();
                let i = world.get::<Interaction>(e).copied().unwrap_or_default();
                if let Some(mut tip) = world.get::<EawButton>(e).and_then(|b| b.tooltip_literal.clone()) {
                    clip(&mut tip, 44);
                    text = format!("{text} [tip: {}]", tip.replace('\n', " / "));
                }
                rows.push((c, format!("{:>5.0},{:<5.0} {:>4.0}x{:<4.0} {:<26} {:?}{state} {text}", c.x, c.y, s.x, s.y, name, i)));
            }
            rows.sort_by(|a, b| (a.0.y, a.0.x).partial_cmp(&(b.0.y, b.0.x)).unwrap_or(std::cmp::Ordering::Equal));
            for r in rows {
                out_line(script, &r.1);
            }
        }
        "text" => {
            let mut q = world.query::<(Entity, &Text)>();
            let list: Vec<(Entity, String)> = q.iter(world).map(|(e, t)| (e, t.0.clone())).collect();
            let mut rows: Vec<(Vec2, String)> = Vec::new();
            for (e, text) in list {
                if text.trim().is_empty() || !world.get::<InheritedVisibility>(e).is_some_and(|v| v.get()) {
                    continue;
                }
                let Some(c) = node_centre(world, e, scale) else { continue };
                let s = world.get::<ComputedNode>(e).map_or(Vec2::ZERO, |n| n.size() / scale);
                if c.x < -5.0 || c.y < -5.0 || c.x > size.x + 5.0 || c.y > size.y + 5.0 || !inside(c) {
                    continue;
                }
                // A text wider or taller than its parent box, or off the screen edge, is worth a look.
                let mut flag = String::new();
                if let Some(p) = world.get::<ChildOf>(e).map(|p| p.parent())
                    && let (Some(pc), Some(pn)) = (node_centre(world, p, scale), world.get::<ComputedNode>(p))
                {
                    let ps = pn.size() / scale;
                    if ps.x > 2.0 && (c.x - s.x / 2.0 < pc.x - ps.x / 2.0 - 1.5 || c.x + s.x / 2.0 > pc.x + ps.x / 2.0 + 1.5 || c.y + s.y / 2.0 > pc.y + ps.y / 2.0 + 1.5 || c.y - s.y / 2.0 < pc.y - ps.y / 2.0 - 1.5) {
                        flag = format!(" OVERFLOWS-PARENT({:.0}x{:.0})", ps.x, ps.y);
                    }
                }
                if c.x - s.x / 2.0 < -1.0 || c.x + s.x / 2.0 > size.x + 1.0 || c.y - s.y / 2.0 < -1.0 || c.y + s.y / 2.0 > size.y + 1.0 {
                    flag.push_str(" OFF-SCREEN");
                }
                let mut t = String::new();
                node_text(world, e, &mut t, 0);
                clip(&mut t, 110);
                rows.push((c, format!("{:>5.0},{:<5.0} {:>4.0}x{:<4.0}{flag} {t}", c.x, c.y, s.x, s.y)));
            }
            rows.sort_by(|a, b| (a.0.y, a.0.x).partial_cmp(&(b.0.y, b.0.x)).unwrap_or(std::cmp::Ordering::Equal));
            for r in rows {
                out_line(script, &r.1);
            }
        }
        "galaxy" | "planets" | "planet" => {
            let Some(state) = world.get_resource::<SimState>() else {
                out_line(script, "no simulation");
                return;
            };
            let me = world.get_resource::<PlayerFaction>().and_then(|p| p.0);
            let g = state.galaxy();
            let wd = &state.world;
            let units = |f: &galaxy_sim::Fleet| f.units.iter().map(|(u, n)| format!("{}x{}", n, wd.units[*u as usize].name)).collect::<Vec<_>>().join(" ");
            let fname = |f: galaxy_sim::FactionId| wd.factions[f as usize].name.clone();
            let planet_line = |p: usize| {
                let ps = &g.planets[p];
                let owner = ps.owner.map_or("-".to_owned(), fname);
                let base = ps.starbase.map_or(0, |s| wd.starbases[s as usize].level);
                let structures: Vec<&str> = ps.structures.iter().map(|s| wd.structures[*s as usize].name.as_str()).collect();
                let queues: Vec<String> = ps.queues.iter().enumerate().filter(|(_, q)| !q.is_empty()).map(|(l, q)| format!("lane{l}:{}[{:.0}/{:.0}s]", q.len(), q[0].total - q[0].remaining, q[0].total)).collect();
                let fleets: Vec<String> = g.fleets.values().filter(|f| f.location == FleetLocation::AtPlanet(p as galaxy_sim::PlanetId)).map(|f| format!("fleet {} {} slot{} [{}]", f.id, fname(f.owner), f.slot, units(f))).collect();
                format!("{} ({}) owner {owner} base {base} structures {structures:?} queues {queues:?} {}", wd.planets[p].display_name, wd.planets[p].name, fleets.join("; "))
            };
            if kind == "planet" {
                let name = what[1..].join(" ").replace('_', " ");
                let names: Vec<(&str, &str)> = wd.planets.iter().map(|p| (p.name.as_str(), p.display_name.as_str())).collect();
                match crate::cli::resolve_name("planet", &name, &names) {
                    Ok(p) => {
                        out_line(script, &planet_line(p));
                        let near: Vec<String> = wd.adjacency[p].iter().map(|(n, _)| planet_line(*n as usize)).collect();
                        for n in near {
                            out_line(script, &format!("  next: {n}"));
                        }
                    }
                    Err(e) => out_line(script, &e),
                }
                return;
            }
            out_line(script, &format!("week {} time {:.0}s battles pending for the player {}", state.current.week, g.time, state.current.human_battles.len()));
            if let Some(me) = me {
                let f = &g.factions[me as usize];
                out_line(script, &format!("me {} credits {:.0} income/week {:.0} tech {} pop {}/{} planets {}", fname(me), f.credits, f.income_per_week, f.tech, f.population_used, f.population_cap, f.planets_owned));
            }
            if let Some(sel) = world.get_resource::<PlayerSelection>() {
                out_line(script, &format!("selection planet {:?} fleets {:?}", sel.planet.map(|p| wd.planets[p as usize].display_name.clone()), sel.fleets));
            }
            for (p, ps) in g.planets.iter().enumerate() {
                if !wd.real.get(p).copied().unwrap_or(true) {
                    continue;
                }
                if kind == "planets" || (me.is_some() && ps.owner == me) {
                    out_line(script, &planet_line(p));
                }
            }
            if kind == "galaxy" {
                for f in g.fleets.values().filter(|f| Some(f.owner) == me) {
                    let at = match &f.location {
                        FleetLocation::AtPlanet(p) => format!("at {}", wd.planets[*p as usize].display_name),
                        FleetLocation::Moving { from, to, .. } => format!("moving {} -> {}", wd.planets[*from as usize].display_name, wd.planets[*to as usize].display_name),
                    };
                    out_line(script, &format!("my fleet {} {at} slot{} [{}]", f.id, f.slot, units(f)));
                }
            }
        }
        "battle" | "squads" => {
            let status = world.get_resource::<BattleStatus>().cloned().unwrap_or_default();
            let settings = world.get_resource::<BattleViewSettings>().map(|s| (s.speed, s.paused, s.player_side));
            let sel = world.get_resource::<battle_view::Selection>().map(|s| s.ships.clone()).unwrap_or_default();
            let sel = format!("{} {:?}", sel.len(), sel.iter().take(12).collect::<Vec<_>>());
            out_line(script, &format!("battle t={:.1}s alive {:?} over {} winner {:?} (speed, paused, player side) {settings:?} selected {sel}", status.time, status.alive, status.over, status.winner));
            let mut cq = world.query::<(&Camera, &GlobalTransform, &BattleCamera)>();
            let Some((cam, tf, bc)) = cq.iter(world).next().map(|(c, t, b)| (c.clone(), *t, b.clone())) else {
                out_line(script, "no battle camera");
                return;
            };
            out_line(script, &format!("camera focus ({:.0}, {:.0}) distance {:.0} yaw {:.0} pitch {:.0}", bc.focus.x, bc.focus.z, bc.distance, bc.yaw.to_degrees(), bc.pitch.to_degrees()));
            let (Some(ships), Some(rt)) = (world.get_resource::<RenderedShips>(), world.get_resource::<BattleRuntime>()) else { return };
            let mut list: Vec<&battle_view::ShipRender> = ships.0.values().collect();
            list.sort_by_key(|s| (s.side, s.squad, s.id));
            let mut squads: HashMap<(u8, u32), u32> = HashMap::new();
            for s in &list {
                if s.squad != u32::MAX {
                    *squads.entry((s.side, s.squad)).or_default() += 1;
                }
            }
            let mut seen: std::collections::HashSet<(u8, u32)> = std::collections::HashSet::new();
            for s in list {
                let craft = if s.squad != u32::MAX {
                    if !seen.insert((s.side, s.squad)) {
                        continue;
                    }
                    format!(" squad {} x{}", s.squad, squads[&(s.side, s.squad)])
                } else if kind == "squads" {
                    continue;
                } else {
                    String::new()
                };
                let px = cam.world_to_viewport(&tf, s.pos).ok().filter(|p| p.x >= 0.0 && p.y >= 0.0 && p.x <= size.x && p.y <= size.y).map_or("off".to_owned(), |p| format!("{:.0},{:.0}", p.x, p.y));
                out_line(
                    script,
                    &format!(
                        "ship {:>4} side {} {:<34} px {:<9} world ({:>6.0},{:>6.0}) hull {:>3.0}% shield {:>3.0}% hp {}/{}{}{craft} {}",
                        s.id,
                        s.side,
                        rt.catalog.ship(s.def).name,
                        px,
                        s.pos.x,
                        s.pos.z,
                        s.hull * 100.0,
                        s.shield * 100.0,
                        s.hp_alive.iter().filter(|a| **a).count(),
                        s.hp_alive.len(),
                        if s.visible { "" } else { " HIDDEN" },
                        {
                            let mut o = format!("{:?}", s.order);
                            clip(&mut o, 40);
                            o
                        }
                    ),
                );
            }
        }
        other => out_line(script, &format!("unknown dump `{other}`")),
    }
}

/// One frame of one command.
fn step(world: &mut World, script: &mut Script, cmd: &[String]) -> Result<Step, String> {
    let n = script.frames;
    let arg = |i: usize| cmd.get(i).map(String::as_str);
    let num = |i: usize| -> Result<f32, String> { arg(i).ok_or(format!("`{}` needs an argument", cmd[0]))?.parse::<f32>().map_err(|e| format!("bad number `{}`: {e}", cmd[i])) };
    match cmd[0].as_str() {
        "wait" => Ok(if n as f32 >= num(1)? { Step::Done } else { Step::Pending }),
        "echo" => {
            out_line(script, &format!("-- {}", cmd[1..].join(" ")));
            Ok(Step::Done)
        }
        "quit" => Ok(Step::Quit),
        "cross" => {
            script.mark = arg(1) != Some("off");
            Ok(Step::Done)
        }
        "burst" => {
            // `burst NAME N`: N consecutive frames as NAME-000.png ..., to look for flicker.
            let name = arg(1).ok_or("burst NAME N")?;
            let count = num(2)? as u32;
            let out = world.resource::<crate::cli::Args>().offscreen.clone().ok_or("no offscreen path")?;
            let path = |k: u32| out.with_file_name(format!("{name}-{k:03}.png"));
            if n < count {
                let _ = std::fs::remove_file(path(n));
                let t = world.resource::<OffscreenTarget>();
                let target = RenderTarget::Image(ImageRenderTarget { handle: t.image.clone(), scale_factor: t.scale_factor });
                world.spawn(Screenshot(target)).observe(save_to_disk(path(n)));
                return Ok(Step::Pending);
            }
            let last = path(count.saturating_sub(1));
            if std::fs::metadata(&last).is_ok_and(|m| m.len() > 0) || n > count + 600 {
                out_line(script, &format!("burst {} frames {}", count, last.display()));
                Ok(Step::Done)
            } else {
                Ok(Step::Pending)
            }
        }
        "waitscreen" => {
            let want = match arg(1).unwrap_or("galaxy") {
                "menu" => AppScreen::MainMenu,
                "campaigns" => AppScreen::CampaignSelect,
                "battle" => AppScreen::SpaceBattle,
                "loading" => AppScreen::Loading,
                _ => AppScreen::Galaxy,
            };
            let max = arg(2).and_then(|v| v.parse::<u32>().ok()).unwrap_or(6000);
            let loading = world.get_resource::<SimState>().is_some_and(|s| s.current.loading || s.driver.load_progress().is_some());
            let ready = screen_of(world) == want && !(want == AppScreen::Galaxy && (loading || world.get_resource::<SimState>().is_none())) && !(want == AppScreen::SpaceBattle && world.get_resource::<BattleRuntime>().is_none());
            if ready {
                Ok(Step::Done)
            } else if n > max {
                Err(format!("screen {want:?} not reached in {max} frames (now {:?})", screen_of(world)))
            } else {
                Ok(Step::Pending)
            }
        }
        "waitweek" => {
            let week = world.get_resource::<SimState>().map_or(0, |s| s.current.week);
            let battle = world.get_resource::<SimState>().is_some_and(|s| !s.current.human_battles.is_empty());
            if week as f32 >= num(1)? || battle || screen_of(world) != AppScreen::Galaxy {
                if battle {
                    out_line(script, &format!("waitweek: stopped at week {week}, a battle waits for the player"));
                }
                Ok(Step::Done)
            } else if n > 200_000 {
                Err(format!("week {} not reached (now {week})", cmd[1]))
            } else {
                Ok(Step::Pending)
            }
        }
        "waitbattle" => {
            let s = world.get_resource::<BattleStatus>().cloned().unwrap_or_default();
            if screen_of(world) != AppScreen::SpaceBattle {
                out_line(script, "waitbattle: no battle screen");
                return Ok(Step::Done);
            }
            if world.get_resource::<BattleViewSettings>().is_some_and(|v| v.paused) && n > 120 {
                return Err(format!("the battle is paused at t={:.1}s", s.time));
            }
            Ok(if (s.tick > 0 && s.time >= num(1)?) || s.over { Step::Done } else if n > 400_000 { return Err("battle time not reached".into()) } else { Step::Pending })
        }
        "waitover" => {
            let s = world.get_resource::<BattleStatus>().cloned().unwrap_or_default();
            if world.get_resource::<BattleViewSettings>().is_some_and(|v| v.paused) && n > 120 && screen_of(world) == AppScreen::SpaceBattle {
                return Err(format!("the battle is paused at t={:.1}s", s.time));
            }
            Ok(if s.over || screen_of(world) != AppScreen::SpaceBattle { Step::Done } else if n > 400_000 { return Err("battle did not end".into()) } else { Step::Pending })
        }
        "waitbegin" => {
            // The Begin button of the battle's loading screen is the only text "BEGIN" there.
            let mut q = world.query::<(Entity, &Text)>();
            let found: Vec<Entity> = q.iter(world).filter(|(_, t)| t.0.eq_ignore_ascii_case("begin")).map(|(e, _)| e).collect();
            let shown = found.into_iter().any(|e| visible_node(world, e));
            Ok(if shown { Step::Done } else if n > 6000 { return Err("no Begin button".into()) } else { Step::Pending })
        }
        "shot" => {
            let name = arg(1).ok_or("shot needs a name")?;
            if n == 0 {
                let out = world.resource::<crate::cli::Args>().offscreen.clone().ok_or("no offscreen path")?;
                let path = out.with_file_name(format!("{name}.png"));
                let _ = std::fs::remove_file(&path);
                let t = world.resource::<OffscreenTarget>();
                let target = RenderTarget::Image(ImageRenderTarget { handle: t.image.clone(), scale_factor: t.scale_factor });
                world.spawn(Screenshot(target)).observe(save_to_disk(path.clone()));
                script.shot_wait = Some((path, 0));
                return Ok(Step::Pending);
            }
            let Some((path, waited)) = script.shot_wait.clone() else { return Ok(Step::Done) };
            if path.exists() && std::fs::metadata(&path).is_ok_and(|m| m.len() > 0) {
                out_line(script, &format!("shot {}", path.display()));
                script.shot_wait = None;
                Ok(Step::Done)
            } else if waited > 600 {
                script.shot_wait = None;
                Err("the picture never arrived".into())
            } else {
                script.shot_wait = Some((path, waited + 1));
                Ok(Step::Pending)
            }
        }
        "move" => {
            let to = target(world, script, &cmd[1..])?;
            move_pointer(world, script, to);
            Ok(Step::Done)
        }
        "glide" => {
            let to = target(world, script, &cmd[1..])?;
            // The target is one word (`@kind:value`, `X,Y`) or two (`X Y`); the frame count follows it.
            let words = if cmd.get(1).is_some_and(|a| a.starts_with('@') || a.contains(',')) { 1 } else { 2 };
            let frames = arg(1 + words).and_then(|v| v.parse::<u32>().ok()).unwrap_or(20).max(1);
            if n == 0 {
                script.glide_from = script.cursor;
            }
            let t = ((n + 1) as f32 / frames as f32).min(1.0);
            let at = script.glide_from.lerp(to, t);
            move_pointer(world, script, at);
            Ok(if t >= 1.0 { Step::Done } else { Step::Pending })
        }
        "click" | "rclick" | "dclick" => {
            let b = if cmd[0] == "rclick" { MouseButton::Right } else { MouseButton::Left };
            match n {
                0 => {
                    let to = target(world, script, &cmd[1..])?;
                    move_pointer(world, script, to);
                }
                3 => button(world, b, ButtonState::Pressed),
                5 => button(world, b, ButtonState::Released),
                8 if cmd[0] == "dclick" => button(world, b, ButtonState::Pressed),
                10 if cmd[0] == "dclick" => button(world, b, ButtonState::Released),
                _ => {}
            }
            let end = if cmd[0] == "dclick" { 13 } else { 8 };
            Ok(if n >= end { Step::Done } else { Step::Pending })
        }
        "ldown" | "lup" | "rdown" | "rup" | "mdown" | "mup" => {
            let b = match cmd[0].as_bytes()[0] {
                b'l' => MouseButton::Left,
                b'r' => MouseButton::Right,
                _ => MouseButton::Middle,
            };
            button(world, b, if cmd[0].ends_with("down") { ButtonState::Pressed } else { ButtonState::Released });
            Ok(Step::Done)
        }
        "wheel" => {
            let y = num(1)?;
            let window = window_entity(world).ok_or("no window")?;
            world.write_message(MouseWheel { unit: MouseScrollUnit::Line, x: 0.0, y, window, phase: bevy::input::touch::TouchPhase::Moved });
            Ok(Step::Done)
        }
        "key" => {
            match n {
                0 => key(world, arg(1).ok_or("key needs a name")?, ButtonState::Pressed)?,
                2 => key(world, arg(1).ok_or("key needs a name")?, ButtonState::Released)?,
                _ => {}
            }
            Ok(if n >= 3 { Step::Done } else { Step::Pending })
        }
        "keydown" | "keyup" => {
            key(world, arg(1).ok_or("needs a key name")?, if cmd[0] == "keydown" { ButtonState::Pressed } else { ButtonState::Released })?;
            Ok(Step::Done)
        }
        "send" => {
            world.write_message(EawButtonClicked { name: arg(1).ok_or("send needs a button name")?.to_lowercase(), entity: Entity::PLACEHOLDER, shift: false });
            Ok(Step::Done)
        }
        "speed" => {
            world.write_message(PlayAction::SetSpeedLevel(num(1)? as usize));
            Ok(Step::Done)
        }
        "advance" => {
            // Like `--weeks`: week-sized steps to the simulation worker, until the weeks are sent and worked off or a battle waits.
            let weeks = num(1)?;
            let mut state = world.get_resource_mut::<SimState>().ok_or("no simulation")?;
            let week_seconds = state.world.config.week_seconds;
            let goal = weeks * week_seconds;
            let battle = !state.current.human_battles.is_empty();
            let flying = state.driver.advances_in_flight(&state.current);
            if script.aux >= goal || battle {
                return Ok(if flying == 0 || n > 200_000 { Step::Done } else { Step::Pending });
            }
            if n >= 3 && flying < 3 {
                let step = week_seconds.min(goal - script.aux);
                state.driver.advance(step);
                script.aux += step;
            }
            Ok(Step::Pending)
        }
        "bspeed" => {
            let x = num(1)?;
            let mut s = world.get_resource_mut::<BattleViewSettings>().ok_or("no battle")?;
            s.speed = x;
            Ok(Step::Done)
        }
        "gcam" => {
            let at = {
                let view = world.resource::<GalaxyView>();
                let names: Vec<(&str, &str)> = view.planets.iter().map(|p| (p.id.as_str(), p.display_name.as_str())).collect();
                let i = crate::cli::resolve_name("planet", &arg(1).ok_or("gcam needs a planet")?.replace('_', " "), &names)?;
                galaxy_to_world(view.planets[i].position)
            };
            let bounds = *world.resource::<CameraBounds>();
            let zoom = arg(2).and_then(|v| v.parse::<f32>().ok());
            let mut q = world.query::<&mut GalaxyCamera>();
            let mut cam = q.iter_mut(world).next().ok_or("no galaxy camera")?;
            cam.focus = Vec3::new(at.x, 0.0, at.z);
            cam.target_focus = cam.focus;
            if let Some(z) = zoom {
                let d = bounds.min_distance + (bounds.max_distance - bounds.min_distance) * z.clamp(0.0, 1.0);
                cam.distance = d;
                cam.target_distance = d;
            }
            Ok(Step::Done)
        }
        "gcamrel" => {
            let (dx, dz) = (num(1)?, num(2)?);
            let mut q = world.query::<&mut GalaxyCamera>();
            let mut cam = q.iter_mut(world).next().ok_or("no galaxy camera")?;
            cam.target_focus += Vec3::new(dx, 0.0, dz);
            cam.focus = cam.target_focus;
            Ok(Step::Done)
        }
        "bcam" => {
            let (focus, rest) = if let Some(id) = arg(1).and_then(|a| a.strip_prefix("@ship:")) {
                let id: u32 = id.parse().map_err(|_| "bad ship id")?;
                let p = world.get_resource::<RenderedShips>().and_then(|s| s.0.get(&id)).map(|s| s.pos).ok_or(format!("no ship {id}"))?;
                (Vec3::new(p.x, 0.0, p.z), 2)
            } else {
                (Vec3::new(num(1)?, 0.0, num(2)?), 3)
            };
            let dist = num(rest)?;
            let yaw = arg(rest + 1).and_then(|v| v.parse::<f32>().ok());
            let pitch = arg(rest + 2).and_then(|v| v.parse::<f32>().ok());
            let mut q = world.query::<&mut BattleCamera>();
            let mut cam = q.iter_mut(world).next().ok_or("no battle camera")?;
            cam.focus = Vec3::new(focus.x, cam.focus.y, focus.z);
            cam.target_focus = cam.focus;
            cam.distance = dist;
            cam.target_distance = dist;
            if let Some(y) = yaw {
                cam.yaw = y.to_radians();
                cam.target_yaw = cam.yaw;
            }
            if let Some(p) = pitch {
                cam.pitch = p.to_radians();
            }
            Ok(Step::Done)
        }
        "grant" => {
            let unit = arg(1).ok_or("grant UNIT PLANET [COUNT]")?.to_owned();
            let state = world.get_resource::<SimState>().ok_or("no simulation")?;
            let names: Vec<(&str, &str)> = state.world.planets.iter().map(|p| (p.name.as_str(), p.display_name.as_str())).collect();
            let planet = crate::cli::resolve_name("planet", &arg(2).ok_or("grant needs a planet")?.replace('_', " "), &names)? as galaxy_sim::PlanetId;
            let count = arg(3).and_then(|v| v.parse::<u32>().ok()).unwrap_or(1);
            world.write_message(PlayAction::DebugGrant { unit_name: unit, planet, count });
            Ok(Step::Done)
        }
        "save" => {
            world.write_message(PlayAction::Save(PathBuf::from(arg(1).ok_or("save needs a file")?)));
            Ok(Step::Done)
        }
        "dump" => {
            dump(world, script, &cmd[1..]);
            Ok(Step::Done)
        }
        other => Err(format!("unknown command `{other}`")),
    }
}

/// Plays the script: one step of the current command per frame; blocks while a live script has no line left.
fn drive(world: &mut World) {
    let Some(mut script) = world.remove_resource::<Script>() else { return };
    script.frame += 1;
    // The data window, once; later frames keep its size (the options screen may write a window size).
    let (w, h) = crate::offscreen::size();
    let scale = world.resource::<OffscreenTarget>().scale_factor;
    match window_entity(world) {
        None => {
            world.spawn((Window { resolution: WindowResolution::new(w, h).with_scale_factor_override(scale), focused: true, ..default() }, PrimaryWindow));
            // The pointer has no picture offscreen: a small square marks it in the captures (`cross off` hides it).
            world.spawn((
                PointerMark,
                Node { position_type: PositionType::Absolute, width: Val::Px(11.0), height: Val::Px(11.0), border: UiRect::all(Val::Px(1.0)), ..default() },
                BorderColor::all(Color::srgb(1.0, 0.2, 1.0)),
                GlobalZIndex(5000),
                FocusPolicy::Pass,
            ));
            world.insert_resource(script);
            return;
        }
        Some(e) => {
            if let Some(mut win) = world.get_mut::<Window>(e)
                && (win.resolution.physical_width(), win.resolution.physical_height()) != (w, h)
            {
                win.bypass_change_detection().resolution.set_physical_resolution(w, h);
            }
        }
    }
    // The mark follows the pointer (node pixels are multiplied by the UI scale).
    let ui_scale = world.get_resource::<UiScale>().map_or(1.0, |s| s.0.max(0.1));
    let (at, shown) = (script.cursor, script.mark);
    let mut marks = world.query_filtered::<(&mut Node, &mut Visibility), With<PointerMark>>();
    for (mut n, mut v) in marks.iter_mut(world) {
        n.left = Val::Px(at.x / ui_scale - 5.5);
        n.top = Val::Px(at.y / ui_scale - 5.5);
        let want = if shown { Visibility::Inherited } else { Visibility::Hidden };
        if *v != want {
            *v = want;
        }
    }
    // The battle reads the window like a real one.
    if let Some(mut s) = world.get_resource_mut::<BattleViewSettings>()
        && s.synthetic_pointer
    {
        s.bypass_change_detection().synthetic_pointer = false;
    }
    if script.current.is_none() {
        script.current = next_line(&mut script);
        script.frames = 0;
        script.aux = 0.0;
        if script.current.is_none() {
            write_done(&script, "idle");
            if !script.live {
                info!("playscript: end of script");
                world.write_message(AppExit::Success);
                world.insert_resource(script);
                return;
            }
            // Live: nothing is simulated or drawn until the next line arrives.
            let started = std::time::Instant::now();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(40));
                script.current = next_line(&mut script);
                if script.current.is_some() {
                    break;
                }
                if started.elapsed().as_secs_f32() > script.idle_exit {
                    info!("playscript: no new line for {:.0} s, leaving", script.idle_exit);
                    write_done(&script, "exit");
                    world.write_message(AppExit::Success);
                    world.insert_resource(script);
                    return;
                }
            }
        }
    }
    // `<file>.abort` ends a command that waits for something that will not come (a wait on a paused battle).
    if script.current.is_some() && script.frame % 20 == 0 {
        let mut abort = script.path.clone().into_os_string();
        abort.push(".abort");
        if std::fs::remove_file(&abort).is_ok() {
            out_line(&script, &format!("ABORTED `{}`", script.current.as_ref().map_or(String::new(), |c| c.join(" "))));
            script.current = None;
            script.shot_wait = None;
        }
    }
    if let Some(cmd) = script.current.clone() {
        match step(world, &mut script, &cmd) {
            Ok(Step::Pending) => script.frames += 1,
            Ok(Step::Done) => script.current = None,
            Ok(Step::Quit) => {
                write_done(&script, "exit");
                world.write_message(AppExit::Success);
                script.current = None;
            }
            Err(e) => {
                out_line(&script, &format!("ERROR line {} `{}`: {e}", script.consumed, cmd.join(" ")));
                script.current = None;
            }
        }
    }
    world.insert_resource(script);
}
