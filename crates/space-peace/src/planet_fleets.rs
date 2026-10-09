//! The fleet organizer of the planet view (U13g): the three framed boxes at the top hold the player's own fleets at the planet (box n = n-th fleet),
//! the first, brighter slot of each box is its hyperspace slot, and units are dragged between the boxes (and into a hyperspace slot) with the mouse.
//!
//! * Dropping a stack on another box moves the stack (Shift: one ship) with `Command::TransferUnits`; an empty box receives a new fleet.
//! * Dropping a stack (or the hyperspace ship of another box) on a hyperspace slot puts one ship there: `Command::TransferUnits { hyperspace: true }` across
//!   boxes, `Command::SetHyperspace` inside one box. The choice lives in the sim fleet (`Galaxy::hyperspace`), where the battle setup can read it later.
//! * Forces of other factions in orbit are not in the boxes; they are listed in one line above them ([`OtherForces`]).
//!
//! `SP_ORGANIZER_TEST=1` (needs a real window: `--hidden`) drives the whole thing with synthetic pointer input and logs `organizer-test: ... PASS|FAIL`.

use std::collections::HashMap;

use bevy::input::ButtonState;
use bevy::input::mouse::MouseButtonInput;
use bevy::prelude::*;
use bevy::window::{CursorMoved, PrimaryWindow};
use eaw_ui::PanelSlot;
use galaxy_play::{Fog, PlayerFaction, SimState};
use galaxy_sim::{Command, FactionId, FleetId, PlanetId, UnitTypeId};

use crate::flow::AppScreen;
use crate::planet_view::PlanetViewState;

/// Slots of a box (5 rows of 7); slot 0 is the hyperspace slot.
const BOX_SLOTS: usize = 35;

/// What one box shows.
#[derive(Clone, Debug, PartialEq, Default)]
pub(crate) struct BoxContent {
    /// The fleet behind the box, if there is one.
    pub fleet: Option<FleetId>,
    /// Unit type of the ship in the hyperspace slot.
    pub hyper: Option<UnitTypeId>,
    /// Stacks of the other slots (slot 1 onwards): unit type and ship count without the hyperspace ship.
    pub stacks: Vec<(UnitTypeId, u32)>,
}

/// The player's fleets at `planet` in fleet id order, one per box (at most three).
pub(crate) fn box_contents(galaxy: &galaxy_sim::Galaxy, me: FactionId, planet: PlanetId) -> [BoxContent; 3] {
    let mut out: [BoxContent; 3] = Default::default();
    let fleets = galaxy.fleets.values().filter(|f| f.owner == me && f.planet() == Some(planet));
    for (b, f) in fleets.take(3).enumerate() {
        let hyper = galaxy.hyperspace_unit(f.id).filter(|u| f.units.get(u).copied().unwrap_or(0) > 0);
        out[b] = BoxContent {
            fleet: Some(f.id),
            hyper,
            stacks: f.units.iter().map(|(&u, &c)| (u, c - u32::from(hyper == Some(u)))).filter(|(_, c)| *c > 0).take(BOX_SLOTS - 1).collect(),
        };
    }
    out
}

/// Component name of slot `k` of box `b` (both 0-based): `q1_0000` is the hyperspace slot of box 1.
pub(crate) fn slot_name(b: usize, k: usize) -> String {
    eaw_ui::fleet_slot_name(b as u32 + 1, (k / 7) as u32, (k % 7) as u32)
}

/// `(box, slot)` of a component name like `q2_0103`.
pub(crate) fn parse_slot(name: &str) -> Option<(usize, usize)> {
    let n = name.to_ascii_lowercase();
    let rest = n.strip_prefix('q')?;
    let (b, rc) = rest.split_once('_')?;
    let b: usize = b.parse().ok()?;
    if !(1..=3).contains(&b) || rc.len() != 4 {
        return None;
    }
    let (r, c): (usize, usize) = (rc[..2].parse().ok()?, rc[2..].parse().ok()?);
    (r < 5 && c < 7).then_some((b - 1, r * 7 + c))
}

/// Where a drag ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// The hyperspace slot of a box.
    Hyperspace(usize),
    /// Anywhere else in a box.
    Box(usize),
}

/// The commands a drag from slot `from` (`(box, slot)`) to `target` stands for; empty if the drop means nothing.
pub(crate) fn drop_commands(me: FactionId, boxes: &[BoxContent; 3], from: (usize, usize), target: Target, shift: bool) -> Vec<Command> {
    let (b1, k1) = from;
    let src = &boxes[b1];
    let Some(f1) = src.fleet else { return Vec::new() };
    let (unit, stack) = if k1 == 0 {
        match src.hyper {
            Some(u) => (u, 1),
            None => return Vec::new(),
        }
    } else {
        match src.stacks.get(k1 - 1) {
            Some(&(u, c)) => (u, c),
            None => return Vec::new(),
        }
    };
    let transfer = |to: usize, count: u32, hyperspace: bool| Command::TransferUnits { faction: me, from: f1, to: boxes[to].fleet, units: vec![(unit, count)], hyperspace };
    match target {
        Target::Hyperspace(b2) if b2 == b1 => {
            if k1 == 0 {
                Vec::new()
            } else {
                vec![Command::SetHyperspace { faction: me, fleet: f1, unit: Some(unit) }]
            }
        }
        Target::Hyperspace(b2) => {
            let mut cmds = Vec::new();
            if k1 == 0 {
                cmds.push(Command::SetHyperspace { faction: me, fleet: f1, unit: None });
            }
            cmds.push(transfer(b2, 1, true));
            cmds
        }
        Target::Box(b2) if b2 == b1 => {
            if k1 == 0 {
                vec![Command::SetHyperspace { faction: me, fleet: f1, unit: None }]
            } else {
                Vec::new()
            }
        }
        Target::Box(b2) => {
            let mut cmds = Vec::new();
            if k1 == 0 {
                cmds.push(Command::SetHyperspace { faction: me, fleet: f1, unit: None });
            }
            cmds.push(transfer(b2, if k1 == 0 || shift { 1 } else { stack }, false));
            cmds
        }
    }
}

/// Everything `play_ui::planet_view_ui` needs to fill the boxes.
pub(crate) struct Organizer {
    /// The three boxes.
    pub boxes: [BoxContent; 3],
    /// One line about the forces of other factions in orbit; empty if there are none.
    pub others: String,
    /// Changes whenever the boxes or that line change (rebuild trigger).
    pub key: u64,
}

/// Reads the boxes and the other forces of `planet`.
pub(crate) fn organizer(state: &SimState, fog: &Fog, me: Option<FactionId>, planet: PlanetId) -> Organizer {
    use std::hash::{Hash, Hasher};
    let galaxy = state.galaxy();
    let boxes = me.map(|m| box_contents(galaxy, m, planet)).unwrap_or_default();
    // Other factions: names of what is known, "unknown forces" for fleets that are only seen.
    let mut known: std::collections::BTreeMap<String, u32> = Default::default();
    let mut hidden = 0u32;
    for f in galaxy.fleets.values().filter(|f| f.planet() == Some(planet) && Some(f.owner) != me) {
        match fog.fleet(f.id) {
            galaxy_sim::Sight::Hidden => {}
            galaxy_sim::Sight::Detailed => {
                for (&u, &c) in &f.units {
                    *known.entry(state.world.units[u as usize].name.replace('_', " ")).or_default() += c;
                }
            }
            _ => hidden += 1,
        }
    }
    let mut parts: Vec<String> = known.iter().map(|(n, c)| format!("{c}x {n}")).collect();
    if hidden > 0 {
        parts.push(if hidden == 1 { "1 unknown fleet".to_owned() } else { format!("{hidden} unknown fleets") });
    }
    let others = if parts.is_empty() { String::new() } else { format!("Other forces in orbit: {}", parts.join(", ")) };
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for b in &boxes {
        (b.fleet, b.hyper, &b.stacks).hash(&mut h);
    }
    others.hash(&mut h);
    Organizer { boxes, others, key: h.finish() }
}

impl Organizer {
    /// Slot contents for `PanelData` and the unit behind each slot (for tooltips; `"!hyperspace"` for an empty hyperspace slot).
    pub fn slots(&self, world: &galaxy_sim::World, game: Option<&alamo_data::GameData>) -> (Vec<(String, PanelSlot)>, HashMap<String, String>) {
        let icon = |u: UnitTypeId| game.and_then(|g| eaw_ui::unit_icon(g, &world.units[u as usize].name)).unwrap_or_default();
        let mut slots = Vec::new();
        let mut names = HashMap::new();
        for (b, content) in self.boxes.iter().enumerate() {
            // The hyperspace slot is always there (brighter than the rest); an empty box has just this slot.
            let hyper = PanelSlot { icon: content.hyper.map(icon).unwrap_or_default(), selected: true, ..default() };
            slots.push((slot_name(b, 0), hyper));
            names.insert(slot_name(b, 0), content.hyper.map_or_else(|| "!hyperspace".to_owned(), |u| world.units[u as usize].name.clone()));
            for (i, &(u, c)) in content.stacks.iter().enumerate() {
                let name = slot_name(b, i + 1);
                slots.push((name.clone(), PanelSlot { icon: icon(u), label: if c > 1 { format!("x{c}") } else { String::new() }, ..default() }));
                names.insert(name, world.units[u as usize].name.clone());
            }
        }
        (slots, names)
    }
}

/// The line about other forces, filled by `planet_view_ui`.
#[derive(Resource, Default)]
pub(crate) struct OtherForces(pub String);

#[derive(Component)]
struct OtherForcesText;

#[derive(Component)]
struct DragGhost;

/// Where the player took hold of a slot.
#[derive(Default)]
struct Drag {
    from: Option<(usize, usize)>,
    ghost: Option<Entity>,
}

pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<OtherForces>().add_systems(Update, (other_forces_line, organizer_drag, organizer_test).chain().run_if(in_state(AppScreen::Galaxy)));
}

/// The one-line strip of other forces above the boxes.
fn other_forces_line(mut commands: Commands, state: Res<PlanetViewState>, others: Res<OtherForces>, mut line: Query<(Entity, &mut Text, &mut Visibility), With<OtherForcesText>>) {
    let show = state.active && state.blend > 0.8 && !others.0.is_empty();
    match line.single_mut() {
        Ok((_, mut text, mut vis)) => {
            if text.0 != others.0 {
                text.0.clone_from(&others.0);
            }
            vis.set_if_neq(if show { Visibility::Inherited } else { Visibility::Hidden });
        }
        Err(_) => {
            commands.spawn((
                OtherForcesText,
                Name::new("other forces in orbit"),
                Text::new(""),
                TextFont { font: FontSource::Family("Bahnschrift".into()), font_size: FontSize::Px(12.0), ..default() },
                TextColor(Color::srgb(1.0, 0.55, 0.45)),
                Node { position_type: PositionType::Absolute, left: Val::Percent(23.0), top: Val::Percent(4.2), ..default() },
                GlobalZIndex(9),
                Visibility::Hidden,
            ));
        }
    }
}

/// Drag and drop of the slots: press on a slot with something in it, drop on a box or a hyperspace slot.
#[allow(clippy::too_many_arguments)]
fn organizer_drag(
    mut drag: Local<Drag>,
    mut commands: Commands,
    state: Res<PlanetViewState>,
    sim: Option<ResMut<SimState>>,
    player: Option<Res<PlayerFaction>>,
    bridge: Option<Res<crate::play::PlayBridge>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    assets: Option<Res<eaw_ui::EawUiAssets>>,
    mut textures: Option<ResMut<eaw_ui::textures::UiTextures>>,
    mut images: ResMut<Assets<Image>>,
    slots: Query<(&eaw_ui::panels::PanelSlotName, &ComputedNode, &UiGlobalTransform, &InheritedVisibility)>,
    frames: Query<(&Name, &ComputedNode, &UiGlobalTransform)>,
    mut ghost: Query<&mut Node, With<DragGhost>>,
    ui_scale: Res<UiScale>,
) {
    let (Some(mut sim), Some(player), Some(bridge), Some(window)) = (sim, player, bridge, window) else { return };
    let Some(me) = player.0 else { return };
    let Some(sp) = state.planet.and_then(|p| bridge.planet_to_sim.get(p).copied().flatten()).filter(|_| state.active && state.blend >= 1.0) else {
        if let Some(g) = drag.ghost.take() {
            commands.entity(g).despawn();
        }
        drag.from = None;
        return;
    };
    let cursor = window.physical_cursor_position();
    let boxes = box_contents(sim.galaxy(), me, sp);
    let hit = |node: &ComputedNode, gt: &UiGlobalTransform, c: Vec2| node.size().x > 4.0 && node.contains_point(*gt, c);
    if mouse.just_pressed(MouseButton::Left)
        && let Some(c) = cursor
    {
        let found = slots.iter().find(|(name, n, g, v)| v.get() && hit(n, g, c) && parse_slot(&name.0).is_some()).and_then(|(name, ..)| parse_slot(&name.0));
        if let Some((b, k)) = found {
            let filled = if k == 0 { boxes[b].hyper.is_some() } else { boxes[b].stacks.get(k - 1).is_some() };
            if filled {
                drag.from = Some((b, k));
                let unit = if k == 0 { boxes[b].hyper } else { boxes[b].stacks.get(k - 1).map(|s| s.0) };
                let icon = unit.and_then(|u| assets.as_deref()?.game.as_deref().and_then(|g| eaw_ui::unit_icon(g, &sim.world.units[u as usize].name)));
                if let Some(h) = icon.and_then(|i| textures.as_mut()?.get(&mut images, &i)).map(|t| t.handle) {
                    // Val::Px is multiplied by UiScale: physical px / (DPI factor * UiScale).
                    let scale = window.scale_factor() * ui_scale.0;
                    let e = commands
                        .spawn((
                            DragGhost,
                            ImageNode::new(h).with_color(Color::srgba(1.0, 1.0, 1.0, 0.8)),
                            Node { position_type: PositionType::Absolute, width: Val::Px(44.0), height: Val::Px(44.0), left: Val::Px(c.x / scale - 22.0), top: Val::Px(c.y / scale - 22.0), ..default() },
                            GlobalZIndex(40),
                            bevy::ui::FocusPolicy::Pass,
                        ))
                        .id();
                    drag.ghost = Some(e);
                }
            }
        }
    }
    if let (Some(c), Ok(mut node)) = (cursor, ghost.single_mut()) {
        let scale = window.scale_factor() * ui_scale.0;
        node.left = Val::Px(c.x / scale - 22.0);
        node.top = Val::Px(c.y / scale - 22.0);
    }
    if mouse.just_released(MouseButton::Left)
        && let Some(from) = drag.from.take()
    {
        if let Some(g) = drag.ghost.take() {
            commands.entity(g).despawn();
        }
        let Some(c) = cursor else { return };
        let hyper_slot = slots.iter().find_map(|(name, n, g, v)| (v.get() && hit(n, g, c)).then(|| parse_slot(&name.0)).flatten().filter(|(_, k)| *k == 0)).map(|(b, _)| b);
        let in_box = frames.iter().find_map(|(name, n, g)| {
            let b = name.as_str().to_ascii_lowercase();
            let idx = b.strip_prefix('q').and_then(|r| r.strip_suffix("_frame")).and_then(|d| d.parse::<usize>().ok())?;
            (hit(n, g, c) && (1..=3).contains(&idx)).then_some(idx - 1)
        });
        let target = hyper_slot.map(Target::Hyperspace).or(in_box.map(Target::Box));
        let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        if let Some(t) = target {
            for cmd in drop_commands(me, &boxes, from, t, shift) {
                sim.driver.send_command(cmd);
            }
        }
    }
}

/// `SP_ORGANIZER_TEST=1`: drags a stack of box 1 onto box 2, then onto the hyperspace slot of box 2, then the hyperspace ship back, with synthetic pointer input.
#[derive(Default)]
struct OrganizerTest {
    t: u32,
    phase: u32,
    wait: u32,
    before: Option<(usize, usize)>,
    results: Vec<(&'static str, bool)>,
    done: bool,
}

#[allow(clippy::too_many_arguments)]
fn organizer_test(
    mut st: Local<OrganizerTest>,
    mut selection: ResMut<crate::galaxy_map::Selection>,
    state: Res<PlanetViewState>,
    sim: Option<Res<SimState>>,
    player: Option<Res<PlayerFaction>>,
    bridge: Option<Res<crate::play::PlayBridge>>,
    window: Option<Single<(Entity, &mut Window), With<PrimaryWindow>>>,
    slots: Query<(&eaw_ui::panels::PanelSlotName, &ComputedNode, &UiGlobalTransform, &InheritedVisibility)>,
    frames: Query<(&Name, &ComputedNode, &UiGlobalTransform)>,
    mut moved: MessageWriter<CursorMoved>,
    mut buttons: MessageWriter<MouseButtonInput>,
    mut exit: MessageWriter<AppExit>,
) {
    if st.done || std::env::var_os("SP_ORGANIZER_TEST").is_none() {
        return;
    }
    if !state.active && selection.0.is_none() {
        // No scripted selection in a hidden run: take the planet of the first own fleet with two ships or more.
        let pick = sim.as_deref().zip(player.as_deref().and_then(|p| p.0)).zip(bridge.as_deref()).and_then(|((s, me), b)| {
            let p = s.galaxy().fleets.values().find(|f| f.owner == me && f.unit_count() >= 2)?.planet()?;
            b.sim_to_planet.get(p as usize).copied().flatten()
        });
        selection.0 = pick;
    }
    st.wait += 1;
    if st.wait % 300 == 1 {
        info!("organizer-test: waiting: active {} blend {} planet {:?}", state.active, state.blend, state.planet);
    }
    let (Some(sim), Some(player), Some(bridge), Some(window)) = (sim, player, bridge, window) else { return };
    let Some(me) = player.0 else { return };
    let Some(sp) = state.planet.and_then(|p| bridge.planet_to_sim.get(p).copied().flatten()).filter(|_| state.active && state.blend >= 1.0) else { return };
    let (entity, mut win) = window.into_inner();
    st.t += 1;
    let boxes = box_contents(sim.galaxy(), me, sp);
    let center = |name: &str| slots.iter().find(|(n, c, _, v)| n.0.eq_ignore_ascii_case(name) && v.get() && c.size().x > 4.0).map(|(_, _, g, _)| g.translation);
    let frame_center = |name: &str| frames.iter().find(|(n, c, _)| n.as_str().eq_ignore_ascii_case(name) && c.size().x > 4.0).map(|(_, _, g)| g.translation);
    let finish = |st: &mut OrganizerTest, name: &'static str, ok: bool| {
        info!("organizer-test: {name} {}", if ok { "ok" } else { "FAILED" });
        st.results.push((name, ok));
        st.phase += 1;
        st.t = 0;
    };
    // One drag takes 3 frames per step: move to the source, press, move to the target, release.
    let mut drag = |st: &OrganizerTest, from: Vec2, to: Vec2| match st.t {
        2 => {
            win.bypass_change_detection().set_physical_cursor_position(Some(from.as_dvec2()));
            moved.write(CursorMoved { window: entity, position: from / win.scale_factor(), delta: None });
        }
        6 => {
            buttons.write(MouseButtonInput { button: MouseButton::Left, state: ButtonState::Pressed, window: entity });
        }
        10 => {
            win.bypass_change_detection().set_physical_cursor_position(Some(to.as_dvec2()));
            moved.write(CursorMoved { window: entity, position: to / win.scale_factor(), delta: None });
        }
        14 => {
            buttons.write(MouseButtonInput { button: MouseButton::Left, state: ButtonState::Released, window: entity });
        }
        _ => {}
    };
    let units_of = |b: &BoxContent| -> usize { b.stacks.iter().map(|s| s.1 as usize).sum::<usize>() + usize::from(b.hyper.is_some()) };
    match st.phase {
        // Setup: the player needs a fleet of at least two ships at the viewed planet, in box 1.
        0 => {
            if st.t < 30 {
                return;
            }
            let Some(f1) = boxes[0].fleet else {
                info!("organizer-test: no own fleet at the planet FAIL");
                st.done = true;
                return;
            };
            if units_of(&boxes[0]) < 2 {
                info!("organizer-test: the fleet has fewer than two ships FAIL");
                st.done = true;
                return;
            }
            st.before = Some((units_of(&boxes[0]), usize::from(boxes[1].fleet.is_some())));
            let _ = f1;
            st.phase = 1;
            st.t = 0;
        }
        // Drag the first stack of box 1 onto the empty box 2: a new fleet appears.
        1 => {
            let (Some(from), Some(to)) = (center(&slot_name(0, 1)), frame_center("q2_frame")) else { return };
            drag(&st, from, to);
            if st.t == 40 {
                let ok = boxes[1].fleet.is_some() && st.before.is_some_and(|(n, _)| units_of(&boxes[0]) < n);
                finish(&mut st, "drag-stack-to-empty-box-makes-a-fleet", ok);
            }
        }
        // Drag the first stack of box 1 onto the hyperspace slot of box 2.
        2 => {
            let (Some(from), Some(to)) = (center(&slot_name(0, 1)), center(&slot_name(1, 0))) else {
                // Box 1 has no stack left (it was one stack): nothing to drag, the previous step moved it all.
                if st.t > 60 {
                    finish(&mut st, "drag-stack-to-hyperspace-slot (no stack left to drag)", false);
                }
                return;
            };
            drag(&st, from, to);
            if st.t == 40 {
                let ok = boxes[1].hyper.is_some();
                finish(&mut st, "drag-stack-to-hyperspace-slot", ok);
            }
        }
        // Drag the hyperspace ship of box 2 back into box 1.
        3 => {
            let (Some(from), Some(to)) = (center(&slot_name(1, 0)), frame_center("q1_frame")) else { return };
            drag(&st, from, to);
            if st.t == 40 {
                let ok = boxes[1].hyper.is_none();
                finish(&mut st, "drag-hyperspace-ship-out-empties-the-slot", ok);
            }
        }
        _ => {
            let pass = st.results.iter().all(|r| r.1);
            info!("organizer-test: {} checks {}", st.results.len(), if pass { "PASS" } else { "FAIL" });
            st.done = true;
            st.wait = 0;
            if std::env::var("SP_ORGANIZER_TEST").is_ok_and(|v| v == "exit") {
                exit.write(AppExit::Success);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxes() -> [BoxContent; 3] {
        [
            BoxContent { fleet: Some(10), hyper: Some(7), stacks: vec![(1, 3), (2, 1)] },
            BoxContent { fleet: Some(11), hyper: None, stacks: vec![(1, 2)] },
            BoxContent::default(),
        ]
    }

    #[test]
    fn slot_names_round_trip() {
        assert_eq!(slot_name(0, 0), "q1_0000");
        assert_eq!(slot_name(2, 9), "q3_0102");
        for (b, k) in [(0, 0), (1, 8), (2, 34)] {
            assert_eq!(parse_slot(&slot_name(b, k)), Some((b, k)));
        }
        assert_eq!(parse_slot("q4_0000"), None);
        assert_eq!(parse_slot("q1_frame"), None);
        assert_eq!(parse_slot("b_create_00"), None);
    }

    #[test]
    fn a_stack_dropped_on_another_box_moves_whole_or_one_with_shift() {
        let t = |count| Command::TransferUnits { faction: 4, from: 10, to: Some(11), units: vec![(1, count)], hyperspace: false };
        assert_eq!(drop_commands(4, &boxes(), (0, 1), Target::Box(1), false), vec![t(3)]);
        assert_eq!(drop_commands(4, &boxes(), (0, 1), Target::Box(1), true), vec![t(1)]);
        // Into an empty box: a new fleet (`to: None`).
        assert_eq!(drop_commands(4, &boxes(), (0, 2), Target::Box(2), false), vec![Command::TransferUnits { faction: 4, from: 10, to: None, units: vec![(2, 1)], hyperspace: false }]);
        // Dropped on its own box: nothing.
        assert!(drop_commands(4, &boxes(), (0, 1), Target::Box(0), false).is_empty());
    }

    #[test]
    fn hyperspace_drops() {
        // A stack onto the slot of its own box, onto another box's slot.
        assert_eq!(drop_commands(4, &boxes(), (0, 2), Target::Hyperspace(0), false), vec![Command::SetHyperspace { faction: 4, fleet: 10, unit: Some(2) }]);
        assert_eq!(
            drop_commands(4, &boxes(), (0, 1), Target::Hyperspace(1), false),
            vec![Command::TransferUnits { faction: 4, from: 10, to: Some(11), units: vec![(1, 1)], hyperspace: true }]
        );
        // The slot ship dragged out of its box empties the slot first; onto its own box it just empties it.
        assert_eq!(
            drop_commands(4, &boxes(), (0, 0), Target::Box(1), false),
            vec![
                Command::SetHyperspace { faction: 4, fleet: 10, unit: None },
                Command::TransferUnits { faction: 4, from: 10, to: Some(11), units: vec![(7, 1)], hyperspace: false }
            ]
        );
        assert_eq!(drop_commands(4, &boxes(), (0, 0), Target::Box(0), false), vec![Command::SetHyperspace { faction: 4, fleet: 10, unit: None }]);
        // Nothing to drag from an empty slot or an empty box.
        assert!(drop_commands(4, &boxes(), (1, 0), Target::Box(0), false).is_empty());
        assert!(drop_commands(4, &boxes(), (2, 1), Target::Box(0), false).is_empty());
    }
}
