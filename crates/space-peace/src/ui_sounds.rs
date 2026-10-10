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
//! * Hyperspace: `Galactic_Fleet_Hyperspace` when one of the player's fleets leaves for hyperspace and when it drops out again
//!   (the data names one event; using it for the arrival too is an estimate). `GUI_Drag_Fleet` / `GUI_Drop_Fleet` (not referenced
//!   by any XML, engine-side names) stand for the original's drag-and-drop of a fleet: our galaxy has no drag gesture, so the
//!   pick-up is the fleet selection and the drop the accepted move order.
//! * Superweapon (`--fire`, Death Star): after the simulation accepted the order the unit's `TSW_SFXEvent_GUI_Bttton_Press`,
//!   `TSW_SFXEvent_Activate_Voice` and `TSW_SFXEvent_Weapon_Power_Up` play and `TSW_MusicEvent_Activation_Start` plays once over
//!   the map music; when the planet dies the unit's `SFXEvent_Fire` and the ability's `SFXEvent_Target_Affected` (`Planet_Death_SFX`)
//!   play (any faction's planet destruction plays the latter).
//! * "Start Game" on the campaign list plays the chosen faction's `SFXEvent_GUI_Start_Campaign` (`Factions.xml`).
//! * A planet captured by the player plays `SFXEvent_Planet_Capture_Bonus_Reward` of `Audio.xml`.
//!
//! Everything is silent without `AlamoAudioPlugin`. `SP_AUDIO_LOG=1` logs each request as `audio sfx <name>`.

use std::collections::{HashMap, HashSet};

use alamo_audio::plugin::{AlamoAudio, GalaxyAudio, PlaySfx};
use alamo_audio::{GalaxyEvent, MusicDirector, MusicScene, SfxCommands};
use bevy::prelude::*;
use eaw_ui::{EawButton, EawButtonClicked, EawLockedClicked};
use galaxy_play::{BuildTarget, PlayAction, PlayEvent, PlayerFaction, PlayerSelection, SimState};
use galaxy_sim::{Command, Event as SimEvent, FleetId, FleetLocation, PlanetId, ProductionKind, UnitTypeId};

use crate::campaign_select::SetupState;
use crate::flow::{CurrentCampaign, StartCampaign};
use crate::game::Game;

/// Click sound of command-bar components without a `Click_SFX` of their own (the value of 243 of 267 components).
const DEFAULT_CLICK: &str = "GUI_Button_Press_2_SFX";
/// Hover sound used when a component names none (the value of 617 of 619 components).
const DEFAULT_HOVER: &str = "GUI_Highlight_Mouse_Over";
/// Seconds a build order waits for the simulation's verdict before the "started" line plays (estimate: the answer of the
/// simulation thread arrives within a few frames; a refusal inside the window plays the negative feedback instead).
const BUILD_VERDICT_WINDOW: f64 = 0.3;
/// Engine-side GUI events for a fleet in hyperspace and for dragging / dropping a fleet (`SFXEventsGUI.xml`).
const FLEET_HYPERSPACE: &str = "Galactic_Fleet_Hyperspace";
const DRAG_FLEET: &str = "GUI_Drag_Fleet";
const DROP_FLEET: &str = "GUI_Drop_Fleet";

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
    /// Player fleets that were in hyperspace last frame.
    moving: HashSet<FleetId>,
    /// Superweapon orders waiting for the simulation's verdict.
    fire: Vec<PendingFire>,
    /// Superweapon unit (object name) that fired last, for its fire sound when the planet dies.
    last_unit: Option<String>,
}

/// A superweapon order waiting for the simulation's verdict.
#[derive(Debug, Clone)]
struct PendingFire {
    at: f64,
    planet: PlanetId,
    unit: String,
}

/// Registers the button and galaxy sound systems.
pub struct UiSoundsPlugin;

impl Plugin for UiSoundsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ButtonSfx>().init_resource::<GalaxySoundState>().add_systems(
            Update,
            (
                load_button_sfx,
                button_sounds,
                locked_button_sound,
                build_sounds,
                fleet_sounds,
                hyperspace_sounds,
                superweapon_sounds,
                start_campaign_sound,
                production_complete_sounds,
                capture_bonus_sound,
            )
                .chain()
                .run_if(resource_exists::<AlamoAudio>)
                .run_if(resource_exists::<Messages<PlayAction>>)
                .run_if(resource_exists::<Messages<PlayEvent>>)
                .run_if(resource_exists::<Messages<EawButtonClicked>>)
                .run_if(resource_exists::<Messages<EawLockedClicked>>),
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
    current: Option<Res<CurrentCampaign>>,
    game: Res<Game>,
    audio: Res<AlamoAudio>,
    time: Res<Time>,
    mut sfx: SfxCommands,
    mut voice: MessageWriter<GalaxyAudio>,
) {
    let now = time.elapsed_secs_f64();
    let faction = current.as_ref().and_then(|c| c.faction.clone()).unwrap_or_else(|| "Rebel".to_owned());
    for a in actions.read() {
        // A cancelled order: the object's own `SFXEvent_Build_Cancelled` ("Building cancelled", "Production cancelled").
        if let (PlayAction::CancelProduction { planet, lane, index }, Some(state)) = (a, &state) {
            let kind = state.galaxy().planets.get(*planet as usize).and_then(|p| p.queues.get(*lane as usize)).and_then(|q| q.get(*index as usize)).map(|i| i.kind.clone());
            let object = match &kind {
                Some(ProductionKind::Unit(u)) => state.world.units.get(*u as usize).map(|u| u.name.clone()),
                Some(ProductionKind::Structure(s)) => state.world.structures.get(*s as usize).map(|s| s.name.clone()),
                Some(ProductionKind::Starbase(s)) => state.world.starbases.get(*s as usize).map(|s| s.name.clone()),
                _ => None,
            };
            if let Some(n) = object.as_ref().and_then(|o| object_sfx(&game, o, "sfxevent_build_cancelled")) {
                sfx.play(n);
            } else {
                match kind {
                    Some(ProductionKind::Unit(_)) => { voice.write(GalaxyAudio { faction: faction.clone(), event: GalaxyEvent::ProductionCancelled }); }
                    Some(ProductionKind::Structure(_)) => { voice.write(GalaxyAudio { faction: faction.clone(), event: GalaxyEvent::ConstructionCancelled }); }
                    Some(ProductionKind::Starbase(_)) => { voice.write(GalaxyAudio { faction: faction.clone(), event: GalaxyEvent::UpgradeCancelled }); }
                    Some(ProductionKind::Tech(_)) => { voice.write(GalaxyAudio { faction: faction.clone(), event: GalaxyEvent::TechUpgradeCancelled }); }
                    _ => {}
                }
            }
        }
        if let PlayAction::SellStructure { .. } = a {
            voice.write(GalaxyAudio { faction: faction.clone(), event: GalaxyEvent::ConstructionCancelled });
        }
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
        } else {
            match p.target {
                BuildTarget::Unit(_) => { voice.write(GalaxyAudio { faction: faction.clone(), event: GalaxyEvent::ProductionStarted }); }
                BuildTarget::Structure(_) => { voice.write(GalaxyAudio { faction: faction.clone(), event: GalaxyEvent::ConstructionStarted }); }
                BuildTarget::Starbase(_) => { voice.write(GalaxyAudio { faction: faction.clone(), event: GalaxyEvent::UpgradeStarted }); }
                BuildTarget::Tech(_) => { voice.write(GalaxyAudio { faction: faction.clone(), event: GalaxyEvent::TechUpgradeStarted }); }
            }
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
    player: Option<Res<PlayerFaction>>,
    live: Option<Res<PlayerSelection>>,
    game: Res<Game>,
    audio: Res<AlamoAudio>,
    mut sfx: SfxCommands,
) {
    let (Some(state), Some(me), Some(live)) = (state, player.and_then(|p| p.0), live) else {
        actions.clear();
        return;
    };
    for a in actions.read() {
        match a {
            PlayAction::SelectFleets(list) => {
                st.selected = list.clone();
                if list.iter().any(|id| state.galaxy().fleets.get(id).is_some_and(|f| f.owner == me)) {
                    sfx.play(DRAG_FLEET);
                }
                if let Some((u, _)) = lead_unit(&state, list, me)
                    && let Some(n) = object_sfx(&game, &state.world.units[u as usize].name, "sfxevent_select")
                {
                    sfx.play(n);
                }
            }
            PlayAction::MoveSelected(to) | PlayAction::MoveAndMerge { to, .. } => {
                let list: Vec<FleetId> = if st.selected.is_empty() { live.fleets.iter().copied().collect() } else { st.selected.clone() };
                let g = state.galaxy();
                let owned = || list.iter().filter_map(|id| g.fleets.get(id)).filter(|f| f.owner == me);
                let movable: Vec<FleetId> = owned().filter(|f| matches!(f.location, FleetLocation::AtPlanet(p) if p != *to)).map(|f| f.id).collect();
                if movable.is_empty() {
                    if !list.is_empty() && owned().next().is_none() {
                        // Only foreign fleets selected: they cannot be commanded.
                        negative(&audio, &mut sfx);
                    }
                    // Fleets in hyperspace cannot be redirected; the order is refused.
                    if owned().any(|f| matches!(f.location, FleetLocation::Moving { .. })) {
                        negative(&audio, &mut sfx);
                    }
                } else {
                    sfx.play(DROP_FLEET);
                    order_line(&state, &game, &movable, me, &mut sfx);
                }
            }
            PlayAction::MergeFleets { .. } => {
                sfx.play(DROP_FLEET);
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

/// "Start Game" on the campaign list: `SFXEvent_GUI_Start_Campaign` of the chosen side (`GUI_Select_<Faction>_SFX`).
fn start_campaign_sound(mut starts: MessageReader<StartCampaign>, setup: Option<Res<SetupState>>, audio: Res<AlamoAudio>, mut sfx: SfxCommands) {
    for _ in starts.read() {
        let faction = setup.as_ref().and_then(|s| s.rows.get(s.selected)?.sides.get(s.side?)).map(|(f, _)| f.clone());
        if let Some(n) = faction.and_then(|f| audio.constants.start_campaign_sfx(&f).map(str::to_owned)) {
            sfx.play(n);
        }
    }
}

/// `Galactic_Fleet_Hyperspace` when a fleet of the player jumps into hyperspace and when it arrives.
fn hyperspace_sounds(mut st: ResMut<GalaxySoundState>, state: Option<Res<SimState>>, player: Option<Res<PlayerFaction>>, mut sfx: SfxCommands) {
    let (Some(state), Some(me)) = (state, player.and_then(|p| p.0)) else { return };
    let now: HashSet<FleetId> = state
        .galaxy()
        .fleets
        .values()
        .filter(|f| f.owner == me && matches!(f.location, FleetLocation::Moving { .. }))
        .map(|f| f.id)
        .collect();
    let departed = now.difference(&st.moving).count();
    let arrived = st.moving.difference(&now).count();
    st.moving = now;
    // One sound per frame and direction: a whole fleet group leaving together is one jump.
    if departed > 0 {
        sfx.play(FLEET_HYPERSPACE);
    }
    if arrived > 0 {
        sfx.play(FLEET_HYPERSPACE);
    }
}

/// Text of the first `tag` below `element` (any depth), lower-case tag names.
fn descendant_text(element: &alamo_data::xml::Element, tag: &str) -> Option<String> {
    if element.name == tag {
        let t = element.text_trimmed();
        return (!t.is_empty()).then(|| t.to_string());
    }
    element.children.iter().find_map(|c| descendant_text(c, tag))
}

/// `SFXEvent_Target_Affected` of the unit's `Planet_Destruction_Ability` (`Planet_Death_SFX`).
fn planet_death_sfx(game: &Game, unit: &str) -> Option<String> {
    let data = &game.0.as_ref()?.data;
    let r = data.objects.resolve(unit)?;
    let abilities = r.element("abilities")?;
    let ability = abilities.children_named("planet_destruction_ability").next()?;
    descendant_text(ability, "sfxevent_target_affected")
}

/// Whether a refused command is the pending superweapon order.
fn rejects_fire(command: &Command, pending: &PendingFire) -> bool {
    matches!(command, Command::DestroyPlanet { planet, .. } if *planet == pending.planet)
}

/// Superweapon orders: activation sounds once the simulation accepts the order, then the fire and planet-death sounds when the
/// planet is destroyed. See the module docs for the tags.
#[allow(clippy::too_many_arguments)]
fn superweapon_sounds(
    mut actions: MessageReader<PlayAction>,
    mut events: MessageReader<PlayEvent>,
    mut st: ResMut<GalaxySoundState>,
    state: Option<Res<SimState>>,
    player: Option<Res<PlayerFaction>>,
    current: Res<CurrentCampaign>,
    game: Res<Game>,
    time: Res<Time>,
    director: Option<ResMut<MusicDirector>>,
    mut sfx: SfxCommands,
) {
    let now = time.elapsed_secs_f64();
    let (Some(state), Some(me)) = (state, player.and_then(|p| p.0)) else {
        actions.clear();
        events.clear();
        return;
    };
    let faction = current.faction.clone().unwrap_or_else(|| "Rebel".to_owned());
    let mut director = director;
    let mut activate = |st: &mut GalaxySoundState, unit: &str, sfx: &mut SfxCommands| {
        st.last_unit = Some(unit.to_owned());
        for tag in ["tsw_sfxevent_gui_bttton_press", "tsw_sfxevent_activate_voice", "tsw_sfxevent_weapon_power_up"] {
            if let Some(n) = object_sfx(&game, unit, tag) {
                sfx.play(n);
            }
        }
        if let (Some(m), Some(d)) = (object_sfx(&game, unit, "tsw_musicevent_activation_start"), director.as_mut()) {
            d.play_once_then(&m, MusicScene::Galactic(faction.clone()));
        }
    };
    for a in actions.read() {
        let PlayAction::DestroyPlanet(planet) = a else { continue };
        let g = state.galaxy();
        let unit = g
            .fleets
            .values()
            .filter(|f| f.owner == me && matches!(f.location, FleetLocation::AtPlanet(p) if p == *planet))
            .flat_map(|f| f.units.keys())
            .filter_map(|u| state.world.units.get(*u as usize).map(|u| u.name.clone()))
            .find(|n| object_sfx(&game, n, "tsw_sfxevent_weapon_power_up").is_some());
        if let Some(unit) = unit {
            st.fire.push(PendingFire { at: now, planet: *planet, unit });
        }
    }
    for e in events.read() {
        match e {
            PlayEvent::CommandRejected { command, .. } => {
                if let Some(i) = st.fire.iter().position(|p| rejects_fire(command, p)) {
                    st.fire.remove(i);
                }
            }
            PlayEvent::Sim(SimEvent::PlanetDestroyed { planet, by, .. }) => {
                // The order is accepted: the activation sounds come first (even when the verdict window is still open).
                let mine = *by == me;
                let mut unit = None;
                if mine && let Some(i) = st.fire.iter().position(|p| p.planet == *planet) {
                    let p = st.fire.remove(i);
                    activate(&mut st, &p.unit, &mut sfx);
                }
                if mine {
                    unit = st.last_unit.clone();
                }
                if let Some(n) = unit.as_deref().and_then(|u| object_sfx(&game, u, "sfxevent_fire")) {
                    sfx.play(n);
                }
                // Every faction's planet destruction is heard; the sound is the ability's `SFXEvent_Target_Affected`.
                let death = unit.as_deref().and_then(|u| planet_death_sfx(&game, u)).or_else(|| find_planet_death_sfx(&game));
                if let Some(n) = death {
                    sfx.play(n);
                }
            }
            _ => {}
        }
    }
    let (due, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut st.fire).into_iter().partition(|p| now - p.at >= BUILD_VERDICT_WINDOW);
    st.fire = waiting;
    for p in due {
        activate(&mut st, &p.unit, &mut sfx);
    }
}

/// `Planet_Death_SFX` as defined by any superweapon unit (used when another faction destroys a planet).
fn find_planet_death_sfx(game: &Game) -> Option<String> {
    let data = &game.0.as_ref()?.data;
    data.objects.iter().find_map(|o| planet_death_sfx(game, &o.name))
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
    player: Option<Res<PlayerFaction>>,
    current: Res<CurrentCampaign>,
    game: Res<Game>,
    mut sfx: SfxCommands,
    mut advisor: MessageWriter<GalaxyAudio>,
) {
    let (Some(state), Some(me)) = (state, player.and_then(|p| p.0)) else {
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
fn capture_bonus_sound(mut events: MessageReader<PlayEvent>, player: Option<Res<PlayerFaction>>, audio: Res<AlamoAudio>, mut sfx: SfxCommands) {
    let Some(me) = player.and_then(|p| p.0) else {
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
    fn planet_death_sound_is_found_below_the_ability() {
        let mut w = Vec::new();
        let root = alamo_data::xml::parse(b"<A><Planet_Destruction_Ability Name=\"x\"><Activation_Style>g</Activation_Style><SFXEvent_Target_Affected> Planet_Death_SFX </SFXEvent_Target_Affected></Planet_Destruction_Ability></A>", "t.xml", &mut w).unwrap();
        let ability = root.children_named("planet_destruction_ability").next().unwrap();
        assert_eq!(descendant_text(ability, "sfxevent_target_affected").as_deref(), Some("Planet_Death_SFX"));
        assert_eq!(descendant_text(ability, "missing"), None);
    }

    #[test]
    fn only_the_fired_planet_cancels_a_superweapon_order() {
        let p = PendingFire { at: 0.0, planet: 3, unit: "Death_Star".into() };
        assert!(rejects_fire(&Command::DestroyPlanet { faction: 0, planet: 3 }, &p));
        assert!(!rejects_fire(&Command::DestroyPlanet { faction: 0, planet: 4 }, &p));
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
