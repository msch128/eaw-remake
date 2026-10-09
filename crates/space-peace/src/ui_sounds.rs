//! Galaxy and command-bar sounds, all taken from the game data:
//!
//! * Command-bar buttons: `Click_SFX` / `Mouse_Over_SFX` of each component in `CommandBarComponents.xml`
//!   (`GUI_Button_Press_2_SFX` for most, `GUI_Dial_Switch_SFX` for the filter / tab dials, `GUI_Unit_Scroll_SFX` for the
//!   scroll arrows, `GUI_Highlight_Mouse_Over` on hover). A click on a locked button plays `SFXEvent_GUI_Negative_Feedback`
//!   of `Audio.xml` (`GUI_Bad_Sound`).
//! * Production: the object's own `SFXEvent_Build_Started` when the order is accepted (a refused order plays the negative
//!   feedback instead) and `SFXEvent_Build_Complete` when it is done; objects without a complete line fall back to the
//!   faction advisor line (`GalaxyEvent`).
//! * Fleets: selecting fleets plays the lead unit's `SFXEvent_Select`; a move or retreat order its `SFXEvent_Fleet_Move`
//!   (several ships) or `SFXEvent_Move` (the lead ship's unit XML).
//! * A planet captured by the player plays `SFXEvent_Planet_Capture_Bonus_Reward` of `Audio.xml`.
//!
//! Everything is silent without `AlamoAudioPlugin`. `SP_AUDIO_LOG=1` logs each request as `audio sfx <name>`.

use std::collections::HashMap;

use alamo_audio::plugin::{AlamoAudio, GalaxyAudio, PlaySfx};
use alamo_audio::{GalaxyEvent, SfxCommands};
use bevy::prelude::*;
use eaw_ui::{EawButton, EawButtonClicked, EawLockedClicked};
use galaxy_play::{BuildTarget, PlayAction, PlayEvent, PlayerFaction, PlayerSelection, SimState};
use galaxy_sim::{Command, Event as SimEvent, FleetId, FleetLocation, PlanetId, ProductionKind, UnitTypeId};

use crate::flow::CurrentCampaign;
use crate::game::Game;

/// Click sound of command-bar components without a `Click_SFX` of their own (the value of 243 of 267 components).
const DEFAULT_CLICK: &str = "GUI_Button_Press_2_SFX";
/// Hover sound used when a component names none (the value of 617 of 619 components).
const DEFAULT_HOVER: &str = "GUI_Highlight_Mouse_Over";
/// Seconds a build order waits for the simulation's verdict before the "started" line plays (estimate: the answer of the
/// simulation thread arrives within a few frames; a refusal inside the window plays the negative feedback instead).
const BUILD_VERDICT_WINDOW: f64 = 0.3;

/// `Click_SFX` / `Mouse_Over_SFX` per command-bar component (lower-case name).
#[derive(Resource, Default, Debug)]
pub struct ButtonSfx {
    loaded: bool,
    table: HashMap<String, (Option<String>, Option<String>)>,
}

impl ButtonSfx {
    /// Click sound of a component.
    pub fn click(&self, name: &str) -> &str {
        self.table.get(name).and_then(|e| e.0.as_deref()).unwrap_or(DEFAULT_CLICK)
    }

    /// Hover sound of a component.
    pub fn hover(&self, name: &str) -> &str {
        self.table.get(name).and_then(|e| e.1.as_deref()).unwrap_or(DEFAULT_HOVER)
    }

    /// Reads `Xml/CommandBarComponents.xml`.
    fn parse(bytes: &[u8]) -> Self {
        let mut warnings = Vec::new();
        let mut table = HashMap::new();
        if let Some(root) = alamo_data::xml::parse(bytes, "CommandBarComponents.xml", &mut warnings) {
            for c in root.children_named("commandbarcomponent") {
                let Some(name) = c.attr("name") else { continue };
                let tag = |t: &str| c.child_text(t).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
                table.insert(name.trim().to_ascii_lowercase(), (tag("click_sfx"), tag("mouse_over_sfx")));
            }
        }
        Self { loaded: true, table }
    }
}

/// A build order waiting for the simulation's verdict.
#[derive(Debug, Clone)]
struct PendingBuild {
    at: f64,
    planet: PlanetId,
    target: BuildTarget,
}

#[derive(Resource, Default)]
struct GalaxySoundState {
    pending: Vec<PendingBuild>,
    /// Fleet selection as the UI last sent it (the simulation's own copy updates a frame later).
    selected: Vec<FleetId>,
}

/// Registers the button and galaxy sound systems.
pub struct UiSoundsPlugin;

impl Plugin for UiSoundsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ButtonSfx>().init_resource::<GalaxySoundState>().add_systems(
            Update,
            (load_button_sfx, button_sounds, locked_button_sound, build_sounds, fleet_sounds, production_complete_sounds, capture_bonus_sound)
                .chain()
                .run_if(resource_exists::<AlamoAudio>),
        );
    }
}

fn load_button_sfx(audio: Res<AlamoAudio>, mut table: ResMut<ButtonSfx>) {
    if !table.loaded {
        *table = match audio.fs.read("Xml/CommandBarComponents.xml") {
            Ok(bytes) => ButtonSfx::parse(&bytes),
            Err(_) => ButtonSfx { loaded: true, ..default() },
        };
    }
}

/// Hover (pointer enters a button) and click (the button reports a click) sounds.
fn button_sounds(
    hovered: Query<(&Interaction, &EawButton), Changed<Interaction>>,
    mut clicks: MessageReader<EawButtonClicked>,
    table: Res<ButtonSfx>,
    mut out: MessageWriter<PlaySfx>,
) {
    let mut play = |event: &str| {
        out.write(PlaySfx { event: event.into(), position: None, source: None });
    };
    for (i, b) in &hovered {
        if *i == Interaction::Hovered {
            play(table.hover(&b.name));
        }
    }
    for c in clicks.read() {
        play(table.click(&c.name));
    }
}

/// A click on a greyed-out button: the general negative feedback of `Audio.xml`.
fn locked_button_sound(mut locked: MessageReader<EawLockedClicked>, audio: Res<AlamoAudio>, mut sfx: SfxCommands) {
    for _ in locked.read() {
        negative(&audio, &mut sfx);
    }
}

fn negative(audio: &AlamoAudio, sfx: &mut SfxCommands) {
    if let Some(n) = &audio.constants.negative_feedback {
        sfx.play(n.clone());
    }
}

/// Value of a sound tag of a game object (`sfxevent_build_started` ...), `None` when absent or empty.
fn object_sfx(game: &Game, object: &str, tag: &str) -> Option<String> {
    let data = &game.0.as_ref()?.data;
    data.objects.resolve(object)?.string(tag)
}

/// Object name of a build target.
fn target_object(state: &SimState, target: BuildTarget) -> Option<String> {
    match target {
        BuildTarget::Unit(u) => state.world.units.get(u as usize).map(|u| u.name.clone()),
        BuildTarget::Structure(s) => state.world.structures.get(s as usize).map(|s| s.name.clone()),
        BuildTarget::Starbase(s) => state.world.starbases.get(s as usize).map(|s| s.name.clone()),
        BuildTarget::Tech(_) => None,
    }
}

/// Whether a refused command is the pending build order.
fn rejects(command: &Command, pending: &PendingBuild) -> bool {
    match (command, pending.target) {
        (Command::BuildUnit { planet, unit, .. }, BuildTarget::Unit(u)) => *planet == pending.planet && *unit == u,
        (Command::BuildStructure { planet, structure, .. }, BuildTarget::Structure(s)) => *planet == pending.planet && *structure == s,
        (Command::UpgradeStarbase { planet, .. }, BuildTarget::Starbase(_)) => *planet == pending.planet,
        (Command::UpgradeTech { planet, .. }, BuildTarget::Tech(_)) => *planet == pending.planet,
        _ => false,
    }
}

/// Build orders: `SFXEvent_Build_Started` of the ordered object when the simulation accepts the order; refusals (and any
/// other rejected command) play the negative feedback.
#[allow(clippy::too_many_arguments)]
fn build_sounds(
    mut actions: MessageReader<PlayAction>,
    mut events: MessageReader<PlayEvent>,
    mut st: ResMut<GalaxySoundState>,
    state: Option<Res<SimState>>,
    game: Res<Game>,
    audio: Res<AlamoAudio>,
    time: Res<Time>,
    mut sfx: SfxCommands,
) {
    let now = time.elapsed_secs_f64();
    for a in actions.read() {
        if let PlayAction::Build { planet, target } = a {
            st.pending.push(PendingBuild { at: now, planet: *planet, target: *target });
        }
    }
    for e in events.read() {
        if let PlayEvent::CommandRejected { command, .. } = e {
            if let Some(i) = st.pending.iter().position(|p| rejects(command, p)) {
                st.pending.remove(i);
            }
            negative(&audio, &mut sfx);
        }
    }
    let Some(state) = state else { return };
    let (due, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut st.pending).into_iter().partition(|p| now - p.at >= BUILD_VERDICT_WINDOW);
    st.pending = waiting;
    for p in due {
        if let Some(n) = target_object(&state, p.target).and_then(|o| object_sfx(&game, &o, "sfxevent_build_started")) {
            sfx.play(n);
        }
    }
}

/// The unit type that answers for a group of fleets: the most expensive ship.
fn lead_unit(state: &SimState, fleets: &[FleetId], me: u32) -> Option<(UnitTypeId, u32)> {
    let g = state.galaxy();
    let mut total = 0;
    let mut best: Option<(UnitTypeId, f32)> = None;
    for f in fleets.iter().filter_map(|id| g.fleets.get(id)).filter(|f| f.owner == me) {
        total += f.unit_count();
        for u in f.units.keys() {
            let cost = state.world.units.get(*u as usize).map_or(0.0, |u| u.cost);
            if best.is_none_or(|(_, c)| cost > c) {
                best = Some((*u, cost));
            }
        }
    }
    best.map(|(u, _)| (u, total))
}

/// Fleet selection and orders on the galaxy map answer with the lead unit's voice lines.
#[allow(clippy::too_many_arguments)]
fn fleet_sounds(
    mut actions: MessageReader<PlayAction>,
    mut st: ResMut<GalaxySoundState>,
    state: Option<Res<SimState>>,
    player: Res<PlayerFaction>,
    live: Res<PlayerSelection>,
    game: Res<Game>,
    audio: Res<AlamoAudio>,
    mut sfx: SfxCommands,
) {
    let (Some(state), Some(me)) = (state, player.0) else {
        actions.clear();
        return;
    };
    for a in actions.read() {
        match a {
            PlayAction::SelectFleets(list) => {
                st.selected = list.clone();
                if let Some((u, _)) = lead_unit(&state, list, me)
                    && let Some(n) = object_sfx(&game, &state.world.units[u as usize].name, "sfxevent_select")
                {
                    sfx.play(n);
                }
            }
            PlayAction::MoveSelected(to) => {
                let list: Vec<FleetId> = if st.selected.is_empty() { live.fleets.iter().copied().collect() } else { st.selected.clone() };
                let g = state.galaxy();
                let owned = || list.iter().filter_map(|id| g.fleets.get(id)).filter(|f| f.owner == me);
                let movable: Vec<FleetId> = owned().filter(|f| matches!(f.location, FleetLocation::AtPlanet(p) if p != *to)).map(|f| f.id).collect();
                if movable.is_empty() {
                    // Fleets in hyperspace cannot be redirected; the order is refused.
                    if owned().any(|f| matches!(f.location, FleetLocation::Moving { .. })) {
                        negative(&audio, &mut sfx);
                    }
                } else {
                    order_line(&state, &game, &movable, me, &mut sfx);
                }
            }
            PlayAction::RetreatSelected => {
                let list: Vec<FleetId> = if st.selected.is_empty() { live.fleets.iter().copied().collect() } else { st.selected.clone() };
                if !list.is_empty() {
                    order_line(&state, &game, &list, me, &mut sfx);
                }
            }
            _ => {}
        }
    }
}

/// `SFXEvent_Fleet_Move` for a group of ships, `SFXEvent_Move` for a single ship (each falls back to the other).
fn order_line(state: &SimState, game: &Game, fleets: &[FleetId], me: u32, sfx: &mut SfxCommands) {
    let Some((u, ships)) = lead_unit(state, fleets, me) else { return };
    let name = &state.world.units[u as usize].name;
    let (first, second) = if ships > 1 { ("sfxevent_fleet_move", "sfxevent_move") } else { ("sfxevent_move", "sfxevent_fleet_move") };
    if let Some(n) = object_sfx(game, name, first).or_else(|| object_sfx(game, name, second)) {
        sfx.play(n);
    }
}

/// A finished production item: the object's `SFXEvent_Build_Complete`, or the advisor's generic line when it has none.
#[allow(clippy::too_many_arguments)]
fn production_complete_sounds(
    mut events: MessageReader<PlayEvent>,
    state: Option<Res<SimState>>,
    player: Res<PlayerFaction>,
    current: Res<CurrentCampaign>,
    game: Res<Game>,
    mut sfx: SfxCommands,
    mut advisor: MessageWriter<GalaxyAudio>,
) {
    let (Some(state), Some(me)) = (state, player.0) else {
        events.clear();
        return;
    };
    let faction = current.faction.clone().unwrap_or_else(|| "Rebel".to_owned());
    for e in events.read() {
        let PlayEvent::Sim(SimEvent::ConstructionComplete { faction: owner, kind, .. }) = e else { continue };
        if *owner != me {
            continue;
        }
        let (object, fallback) = match kind {
            ProductionKind::Unit(u) => {
                let unit = &state.world.units[*u as usize];
                let research = matches!(unit.role, galaxy_sim::UnitRole::Research(_));
                (Some(unit.name.clone()), if research { GalaxyEvent::ResearchComplete } else { GalaxyEvent::UnitBuilt })
            }
            ProductionKind::Structure(s) => (state.world.structures.get(*s as usize).map(|s| s.name.clone()), GalaxyEvent::StructureBuilt),
            ProductionKind::Starbase(s) => (state.world.starbases.get(*s as usize).map(|s| s.name.clone()), GalaxyEvent::StructureBuilt),
            // The new tech level is announced by `TechAdvanced` (see `play::galaxy_audio`).
            ProductionKind::Tech(_) => continue,
        };
        match object.and_then(|o| object_sfx(&game, &o, "sfxevent_build_complete")) {
            Some(n) => sfx.play(n),
            None => {
                advisor.write(GalaxyAudio { faction: faction.clone(), event: fallback });
            }
        }
    }
}

/// `SFXEvent_Planet_Capture_Bonus_Reward` when the player takes a planet.
fn capture_bonus_sound(mut events: MessageReader<PlayEvent>, player: Res<PlayerFaction>, audio: Res<AlamoAudio>, mut sfx: SfxCommands) {
    let Some(me) = player.0 else {
        events.clear();
        return;
    };
    for e in events.read() {
        if let PlayEvent::Sim(SimEvent::PlanetCaptured { old_owner, new_owner, .. }) = e
            && *new_owner == me
            && *old_owner != Some(me)
            && let Some(n) = &audio.constants.planet_capture_bonus
        {
            sfx.play(n.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_and_hover_sounds_come_from_the_component_data() {
        let t = ButtonSfx::parse(
            br#"<CommandBarComponents>
            <CommandBarComponent Name="b_filters0"><Click_SFX>GUI_Dial_Switch_SFX</Click_SFX><Mouse_Over_SFX>GUI_Highlight_Mouse_Over</Mouse_Over_SFX></CommandBarComponent>
            <CommandBarComponent Name="b_scroll_left"><Click_SFX>GUI_Unit_Scroll_SFX</Click_SFX></CommandBarComponent>
            <CommandBarComponent Name="b_other"><Mouse_Over_SFX>GUI_Text_SFX</Mouse_Over_SFX></CommandBarComponent>
            </CommandBarComponents>"#,
        );
        assert_eq!(t.click("b_filters0"), "GUI_Dial_Switch_SFX");
        assert_eq!(t.click("b_scroll_left"), "GUI_Unit_Scroll_SFX");
        assert_eq!(t.hover("b_scroll_left"), DEFAULT_HOVER);
        assert_eq!(t.hover("b_other"), "GUI_Text_SFX");
        assert_eq!(t.click("b_other"), DEFAULT_CLICK);
        assert_eq!(t.click("unknown_button"), DEFAULT_CLICK);
    }

    #[test]
    fn a_rejected_command_cancels_exactly_its_pending_build() {
        let p = PendingBuild { at: 0.0, planet: 4, target: BuildTarget::Unit(7) };
        assert!(rejects(&Command::BuildUnit { faction: 0, planet: 4, unit: 7 }, &p));
        assert!(!rejects(&Command::BuildUnit { faction: 0, planet: 5, unit: 7 }, &p));
        assert!(!rejects(&Command::BuildUnit { faction: 0, planet: 4, unit: 8 }, &p));
        assert!(!rejects(&Command::BuildStructure { faction: 0, planet: 4, structure: 7 }, &p));
    }
}
