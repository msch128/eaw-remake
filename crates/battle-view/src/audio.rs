//! Battle sound hooks: weapon fire, impacts, explosions, ability and unit-response events are
//! sent as [`alamo_audio::PlaySfx`] messages; advisor lines (`EHD_*` HUD voice-over) as
//! [`alamo_audio::plugin::GalaxyAudio`].
//!
//! The view never plays anything itself: without `AlamoAudioPlugin` the messages are consumed
//! by Bevy's message cleanup and nothing is audible, which keeps tests and offscreen runs silent.
//! [`BattleViewSettings::audio`] switches the hooks off completely.
//!
//! Data sources: the unit XML names the response events (`SFXEvent_Select`, `_Move`, `_Fleet_Move`, `_Attack`,
//! `_Guard`, `_Stop`, `_Unit_Lost`, `_Unit_Under_Attack`, `_Health_Low_Warning`, `_Health_Critical_Warning`,
//! `_Enemy_Damaged_Health_*_Warning`); `Gameconstants.xml` gives the health thresholds and `Audio.xml` the
//! space-base announcement delay and the peace delay of the battle music.

use std::collections::{HashMap, HashSet};

use alamo_audio::plugin::AlamoAudio;
use alamo_audio::{AudioConstants, GalaxyEvent, MusicDirector, MusicScene, PlaySfx};
use battle_sim::{Event, HitKind, ShipId, TargetClass};
use bevy::prelude::*;

use crate::control::{OrderIssued, OrderKind, Selection};
use crate::convert::sim_to_bevy;
use crate::sim::Snapshot;
use crate::{BattleRuntime, BattleStatus, BattleViewSettings};

/// Most sounds of one event name per frame; the rest would only pile up volume.
const PER_NAME_PER_FRAME: u32 = 2;
/// Most battle sounds per frame.
const PER_FRAME: u32 = 24;
/// Seconds before the same unit type reports "under attack" again (estimate; `Max_Instances 1` of the
/// response events already stops overlapping lines).
const UNDER_ATTACK_GAP: f64 = 12.0;
/// Seconds between two "enemy fleeing" / "retreat succeeded" announcements (estimate).
const RETREAT_GAP: f64 = 30.0;
/// Battle seconds before "reinforcements available" is announced (estimate: the opening shots first).
const REINFORCEMENTS_AVAILABLE_AT: f64 = 3.0;
/// A hostile hit or shot within this many battle seconds keeps the music at battle intensity (estimate).
const COMBAT_WINDOW: f64 = 3.0;
/// Minimum gap between unit selection acknowledgments to prevent click spam.
pub const SELECT_VOICE_COOLDOWN: f64 = 2.0;

/// Counters of what the hooks sent (for tests and diagnostics).
#[derive(Resource, Debug, Default, Clone)]
pub struct BattleAudioStats {
    /// Messages written so far.
    pub sent: u64,
    /// Last event names, newest last (at most 32).
    pub recent: Vec<String>,
    /// Battle clock of the last shot or hit (drives the music intensity).
    pub last_combat: Option<f64>,
    /// Advisor lines requested so far (`GalaxyEvent` debug names).
    pub advisor: Vec<String>,
}

/// Per-battle memory of the sound hooks.
#[derive(Default)]
pub struct AudioMemory {
    /// Ship id -> (definition, side), kept after the ship died.
    ships: HashMap<u32, (ShipId, u8)>,
    /// Bit 0: low-health line played, bit 1: critical line played.
    health_flags: HashMap<u32, u8>,
    under_attack: HashMap<ShipId, f64>,
    last_station_alarm: Option<f64>,
    last_flee: Option<f64>,
    last_retreat_ok: Option<f64>,
    heroes_seen: HashSet<u32>,
    /// Enemy unit types already announced (`SFXEvent_Unit_Type_Spotted`).
    types_spotted: HashSet<ShipId>,
    reserve_announced: bool,
    last_selection: Vec<u32>,
    last_select_voice: Option<f64>,
    last_station_critical: Option<f64>,
    shield_down_warned: HashSet<u32>,
    own_heroes_entered: HashSet<u32>,
    /// `(ship, ability index)` of the player's abilities that were active last frame (their end plays the deactivation sounds).
    abilities_on: HashSet<(u32, usize)>,
    /// `Death_SFXEvent_End_Die` waiting for the end of the death sequence: (battle clock when due, event, position).
    end_die: Vec<(f64, String, Vec3)>,
}

/// Seconds from `Death_SFXEvent_Start_Die` to `Death_SFXEvent_End_Die` (estimate: the view has no death timeline of its own).
const DEATH_SEQUENCE_SECONDS: f64 = 1.5;

fn is_small(class: TargetClass) -> bool {
    matches!(class, TargetClass::Fighter | TargetClass::Bomber)
}

/// Translates the events of the displayed steps and the player's input into sound requests.
#[allow(clippy::too_many_arguments)]
pub fn battle_audio(
    settings: Res<BattleViewSettings>,
    rt: Res<BattleRuntime>,
    sel: Res<Selection>,
    audio: Option<Res<AlamoAudio>>,
    mut orders: MessageReader<OrderIssued>,
    mut hardpoint_orders: MessageReader<crate::control::HardpointOrderIssued>,
    mut out: MessageWriter<PlaySfx>,
    mut stats: ResMut<BattleAudioStats>,
    mut voice: MessageWriter<alamo_audio::plugin::GalaxyAudio>,
    mut mem: Local<AudioMemory>,
) {
    if !settings.audio {
        orders.clear();
        return;
    }
    let Some(cur) = rt.cur.clone() else { return };
    let visuals = &rt.visuals;
    let player = settings.player_side;
    let clock = rt.clock;
    let constants = audio.as_ref().map(|a| a.constants.clone()).unwrap_or_default();
    let AudioConstants { space_base_attack_announcement_delay, health_low, health_critical, .. } = constants;
    mem.ships.extend(cur.ships.iter().map(|s| (s.id, (s.def, s.side))));
    let mut per_name: HashMap<String, u32> = HashMap::new();
    let mut budget = PER_FRAME;
    let mut send = |names: &str, pos: Option<Vec3>, stats: &mut BattleAudioStats, out: &mut MessageWriter<PlaySfx>| {
        // Some tags hold a comma separated list of events (`HARD_POINT_WEAPON_LASER, Unit_Lost_...`): play each.
        for name in names.split(',').map(str::trim).filter(|n| !n.is_empty() && !n.starts_with("HARD_POINT_")) {
            if budget == 0 {
                return;
            }
            // An event the data leaves blank (the shield absorb detonation) is silent by design: it must not use up the frame's budget.
            // The static hook audit (`alamo_audio::hooks`) reports such events.
            if audio.as_ref().and_then(|a| a.sfx.get(name)).is_some_and(|e| {
                !e.is_preset && e.samples.is_empty() && e.pre_samples.is_empty() && e.post_samples.is_empty() && e.chained.is_none()
            }) {
                continue;
            }
            let n = match per_name.get_mut(name) {
                Some(n) => n,
                None => per_name.entry(name.to_string()).or_default(),
            };
            if *n >= PER_NAME_PER_FRAME {
                return;
            }
            *n += 1;
            budget -= 1;
            out.write(PlaySfx { event: name.to_string(), position: pos, source: None });
            stats.sent += 1;
            if stats.recent.len() >= 32 {
                stats.recent.remove(0);
            }
            stats.recent.push(name.to_string());
        }
    };
    let faction = rt.player_faction.clone();
    let mut say = |event: GalaxyEvent, stats: &mut BattleAudioStats| {
        if let Some(f) = &faction {
            voice.write(alamo_audio::plugin::GalaxyAudio { faction: f.clone(), event });
            stats.advisor.push(format!("{event:?}"));
        }
    };

    for ev in &rt.fresh_events {
        match ev {
            Event::ProjectileFired { ship, hardpoint, from, .. } => {
                stats.last_combat = Some(clock);
                let Some(s) = cur.ship(*ship) else { continue };
                let v = visuals.ship(s.def);
                let name = match hardpoint {
                    Some(h) => v.hardpoints.get(*h as usize).and_then(|h| h.fire_sfx.as_deref()),
                    None => v.sfx.fire.as_deref(),
                };
                if let Some(n) = name {
                    send(n, Some(sim_to_bevy(*from)), &mut stats, &mut out);
                }
            }
            Event::Hit { ship, attacker, projectile, pos, kind, .. } => {
                let attacker_side = mem.ships.get(attacker).map(|a| a.1);
                if let Some(v) = cur.ship(*ship) {
                    let hostile = attacker_side.is_some_and(|a| a != v.side);
                    if hostile {
                        stats.last_combat = Some(clock);
                    }
                    let def = rt.catalog.ship(v.def);
                    let sfx = &visuals.ship(v.def).sfx;
                    // An own unit is shot at: the unit complains, a station raises the alarm.
                    if v.side == player && hostile {
                        let frac = if def.hull > 0.0 { v.hull / def.hull } else { 1.0 };
                        if def.class == TargetClass::Structure {
                            if frac <= 0.25 && mem.last_station_critical.is_none_or(|t| clock - t >= 30.0) {
                                mem.last_station_critical = Some(clock);
                                say(GalaxyEvent::StationCriticalDamage, &mut stats);
                            } else if mem.last_station_alarm.is_none_or(|t| clock - t >= f64::from(space_base_attack_announcement_delay)) {
                                mem.last_station_alarm = Some(clock);
                                say(GalaxyEvent::StationUnderAttack, &mut stats);
                            }
                        } else if !is_small(def.class)
                            && mem.under_attack.get(&v.def).is_none_or(|t| clock - t >= UNDER_ATTACK_GAP)
                            && let Some(n) = sfx.under_attack.as_deref()
                        {
                            mem.under_attack.insert(v.def, clock);
                            send(n, None, &mut stats, &mut out);
                        }

                        // Shield collapse warning ("Shields are down!") for capital ships, supers and stations
                        if matches!(def.class, TargetClass::Capital | TargetClass::Super | TargetClass::Structure) && def.shield > 0.0 {
                            if v.shield <= 0.0 && mem.shield_down_warned.insert(*ship) {
                                say(GalaxyEvent::ShieldsDown, &mut stats);
                            } else if v.shield > def.shield * 0.25 {
                                mem.shield_down_warned.remove(ship);
                            }
                        }
                    }
                    // Health warnings at `Health_Low_Percent_Threshold` / `Health_Critical_Percent_Threshold`.
                    let mine = v.side == player;
                    if !def.is_squadron && !is_small(def.class) && def.hull > 0.0 && (mine || attacker_side == Some(player)) {
                        let frac = v.hull / def.hull;
                        let flags = mem.health_flags.entry(*ship).or_default();
                        let (low, crit) = if mine {
                            (sfx.health_low.as_deref(), sfx.health_critical.as_deref())
                        } else {
                            (sfx.enemy_health_low.as_deref(), sfx.enemy_health_critical.as_deref())
                        };
                        if frac <= health_critical && *flags & 2 == 0 {
                            *flags |= 3;
                            if let Some(n) = crit {
                                send(n, None, &mut stats, &mut out);
                            }
                        } else if frac <= health_low && *flags & 1 == 0 {
                            *flags |= 1;
                            if let Some(n) = low {
                                send(n, None, &mut stats, &mut out);
                            }
                        }
                    }
                }
                // A shield hit plays the projectile's shield-absorb event, a hull / hardpoint hit its detonation. Most laser bolts
                // name only `Projectile_SFXEvent_Detonate_Reduced_By_Armor`, which then stands in for the plain detonation
                // (estimate: the sim does not report armor reduction).
                let pi = *projectile as usize;
                let name = if matches!(kind, HitKind::Shield) {
                    visuals.proj_detonate_shield.get(pi)
                } else if matches!(visuals.proj_detonate.get(pi), Some(Some(_))) {
                    visuals.proj_detonate.get(pi)
                } else {
                    visuals.proj_detonate_armor.get(pi)
                };
                if let Some(Some(n)) = name {
                    send(n, Some(sim_to_bevy(*pos)), &mut stats, &mut out);
                }
            }
            Event::ShipDestroyed { ship, pos } => {
                mem.health_flags.remove(ship);
                mem.shield_down_warned.remove(ship);
                let Some(&(def, side)) = mem.ships.get(ship) else { continue };
                let vis = visuals.ship(def);
                if let Some(n) = vis.sfx.death.as_deref() {
                    send(n, Some(sim_to_bevy(*pos)), &mut stats, &mut out);
                }
                if let Some(n) = vis.sfx.death_end.as_deref() {
                    mem.end_die.push((clock + DEATH_SEQUENCE_SECONDS, n.to_string(), sim_to_bevy(*pos)));
                }
                let class = rt.catalog.ship(def).class;
                if is_small(class) {
                    continue;
                }
                let structure = class == TargetClass::Structure;
                let capital = matches!(class, TargetClass::Capital | TargetClass::Super);
                if side == player {
                    if let Some(n) = vis.sfx.unit_lost.as_deref() {
                        send(n, None, &mut stats, &mut out);
                    }
                    say(
                        match (vis.hero, structure, capital) {
                            (true, _, _) => GalaxyEvent::HeroLost,
                            (_, true, _) => GalaxyEvent::StructureDestroyed,
                            (_, _, true) => GalaxyEvent::CapitalShipLost,
                            _ => GalaxyEvent::UnitDestroyed,
                        },
                        &mut stats,
                    );
                } else {
                    say(
                        match (vis.hero, structure, capital) {
                            (true, _, _) => GalaxyEvent::HeroEliminated,
                            (_, true, _) => GalaxyEvent::EnemyStructureDestroyed,
                            _ => GalaxyEvent::EnemyUnitDestroyed,
                        },
                        &mut stats,
                    );
                }
            }
            // The unit's own report per hardpoint type (`SFXEvent_Hardpoint_Destroyed` lists `HARD_POINT_ENGINE, Unit_Lost_Engines_...`
            // pairs; `SFX_NA` = silent). Only the player's ships report; a lost enemy hardpoint is not announced.
            Event::HardpointDestroyed { ship, hardpoint, .. } => {
                if let Some(s) = cur.ship(*ship)
                    && s.side == player
                    && let Some(n) = hardpoint_destroyed_line(visuals.ship(s.def), *hardpoint)
                {
                    send(n, None, &mut stats, &mut out);
                }
            }
            Event::ReinforcementsCalled { side, .. } if *side == player => say(GalaxyEvent::ReinforcementsEnRoute, &mut stats),
            Event::ReinforcementsRefused { side, .. } if *side == player => say(GalaxyEvent::UnitCapReached, &mut stats),
            Event::ReinforcementsArrived { side, .. } if *side != player => say(GalaxyEvent::EnemyReinforcements, &mut stats),
            // A missile shield destroys a missile: the projectile's own detonation.
            Event::ProjectileIntercepted { projectile, pos } => {
                if let Some(Some(n)) = visuals.proj_detonate.get(*projectile as usize) {
                    send(n, Some(sim_to_bevy(*pos)), &mut stats, &mut out);
                }
            }
            // The retreat order was accepted (button, Esc menu or the AI): the own announcement, or the enemy's `Space_Retreat_Enemy_Begin_SFXEvent`.
            Event::RetreatOrdered { side, .. } => {
                if *side == player {
                    say(GalaxyEvent::RetreatStarted, &mut stats);
                } else if let Some(n) = faction.as_deref().and_then(|f| constants.faction_sfx(f, "space_retreat_enemy_begin_sfxevent")) {
                    send(n, None, &mut stats, &mut out);
                }
            }
            // An enemy gravity well came up while the fleet charged its hyperdrives (`Event::RetreatAborted`): the faction's "interdicted"
            // line (`Space_Retreat_Not_Allowed_Reason_1_SFXEvent`, "Reason 1: Gravity Well"), else `Space_Retreat_Cancel_SFXEvent`. Only the
            // player's own aborted retreat is announced.
            Event::RetreatAborted { side } if *side == player => {
                let line = faction.as_deref().and_then(|f| {
                    constants
                        .faction_sfx(f, "space_retreat_not_allowed_reason_1_sfxevent")
                        .or_else(|| constants.faction_sfx(f, "space_retreat_cancel_sfxevent"))
                });
                if let Some(n) = line {
                    send(n, None, &mut stats, &mut out);
                }
            }
            // The player called the own retreat off (Retreat pressed again during the countdown): `Space_Retreat_Cancel_SFXEvent`.
            Event::RetreatCancelled { side } if *side == player => {
                if let Some(n) = faction.as_deref().and_then(|f| constants.faction_sfx(f, "space_retreat_cancel_sfxevent")) {
                    send(n, None, &mut stats, &mut out);
                }
            }
            Event::ShipRetreated { ship } => {
                let Some(&(_, side)) = mem.ships.get(ship) else { continue };
                if side == player {
                    if mem.last_retreat_ok.is_none_or(|t| clock - t >= RETREAT_GAP) {
                        mem.last_retreat_ok = Some(clock);
                        say(GalaxyEvent::RetreatSucceeded, &mut stats);
                    }
                } else if mem.last_flee.is_none_or(|t| clock - t >= RETREAT_GAP) {
                    mem.last_flee = Some(clock);
                    say(GalaxyEvent::EnemyFleeing, &mut stats);
                }
            }
            Event::AbilityActivated { ship, ability } => {
                if let Some(s) = cur.ship(*ship) {
                    let vis = visuals.ship(s.def);
                    let kind = rt.catalog.ship(s.def).abilities.get(*ability as usize).map(|a| a.kind.to_ascii_uppercase()).unwrap_or_default();
                    let own = vis.sfx.abilities.iter().find(|a| a.kind == kind);
                    let mut line = None;
                    if s.side == player {
                        line = own.and_then(|a| a.activated.as_deref()).or(vis.sfx.ability.as_deref())
                            .or_else(|| if vis.hero { vis.sfx.attack.as_deref() } else { None });
                        if let Some(n) = line {
                            send(n, None, &mut stats, &mut out);
                        }
                        // A non-hero ability switched on also plays the faction's GUI toggle sound (`SFXEvent_GUI_Toggle_Non_Hero_Ability_On`).
                        if !vis.hero && let Some(n) = faction.as_deref().and_then(|f| constants.toggle_sfx(f, &kind, true)) {
                            send(n, None, &mut stats, &mut out);
                        }
                    }
                    // The ability's own start sound sits on the unit and is heard from every unit the player sees (estimate: the data names
                    // `SFXEvent_Activate` but not when the engine plays it; the start is the obvious moment). 2D events are GUI or voice lines
                    // that the hooks above already cover for the unit's own side, so only the 3D ones count.
                    let seen = s.side == player || (s.seen_by >> player.min(7)) & 1 == 1;
                    if seen
                        && let Some(n) = own.and_then(|a| a.start.as_deref()).filter(|n| line.is_none_or(|l| !l.eq_ignore_ascii_case(n)))
                        && audio.as_ref().and_then(|a| a.sfx.get(n)).is_some_and(|e| !e.is_2d)
                    {
                        send(n, Some(sim_to_bevy(s.pos)), &mut stats, &mut out);
                    }
                }
            }
            _ => {}
        }
    }

    // End of a death sequence.
    let (due, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut mem.end_die).into_iter().partition(|d| d.0 <= clock);
    mem.end_die = waiting;
    for (_, name, at) in due {
        send(&name, Some(at), &mut stats, &mut out);
    }
    // An ability of the player's ships ran out: its deactivation sound and the faction's toggle-off sound.
    for s in cur.ships.iter().filter(|s| s.side == player) {
        for (i, st) in s.abilities.iter().enumerate() {
            if st.0 > 0.0 {
                mem.abilities_on.insert((s.id, i));
            } else if mem.abilities_on.remove(&(s.id, i)) {
                let vis = visuals.ship(s.def);
                let kind = rt.catalog.ship(s.def).abilities.get(i).map(|a| a.kind.to_ascii_uppercase()).unwrap_or_default();
                if let Some(n) = vis.sfx.abilities.iter().find(|a| a.kind == kind).and_then(|a| a.deactivated.as_deref()) {
                    send(n, None, &mut stats, &mut out);
                }
                if !vis.hero && let Some(n) = faction.as_deref().and_then(|f| constants.toggle_sfx(f, &kind, false)) {
                    send(n, None, &mut stats, &mut out);
                }
            }
        }
    }

    // An own hero entering the battle for the first time announces their presence ("Grand Admiral Thrawn reporting", etc.)
    for s in &cur.ships {
        if s.side == player && visuals.ship(s.def).hero && mem.own_heroes_entered.insert(s.id) {
            let vis = visuals.ship(s.def);
            if let Some(n) = vis.sfx.select.as_deref().or_else(|| vis.sfx.fleet_move.as_deref()) {
                send(n, None, &mut stats, &mut out);
            }
        }
    }

    // An enemy hero entering the player's sight.
    for s in &cur.ships {
        if s.side != player && visuals.ship(s.def).hero && s.seen_by & (1 << player) != 0 && mem.heroes_seen.insert(s.id) {
            say(GalaxyEvent::EnemyHeroDetected, &mut stats);
        }
        // The faction's soldiers announce the first sight of certain enemy unit types (`SFXEvent_Unit_Type_Spotted` in Factions.xml:
        // heroes, Star Destroyers, Executor ...).
        if s.side != player && s.seen_by & (1 << player) != 0 && mem.types_spotted.insert(s.def) {
            let line = faction.as_deref().and_then(|f| constants.spotted_line(f, &rt.catalog.ship(s.def).name)).map(str::to_owned);
            if let Some(n) = line {
                send(&n, None, &mut stats, &mut out);
            }
        }
    }
    // Reserve groups of the player's side can be called in.
    if !mem.reserve_announced
        && clock >= REINFORCEMENTS_AVAILABLE_AT
        && rt.request.as_ref().and_then(|r| r.sides.get(player as usize)).is_some_and(|s| !s.reserve.is_empty())
    {
        mem.reserve_announced = true;
        say(GalaxyEvent::ReinforcementsAvailable, &mut stats);
    }

    // Unit responses of the player's own units: one voice for the whole selection. The ships are ranked (a named hero first, then
    // population, then hull + shield) and the line falls through the ranking until a ship has one; the generic fallbacks
    // (fleet move -> move, guard -> stop, hardpoint attack -> plain attack) are the default when no ship has the specific line.
    let class_priority = |class: TargetClass| match class {
        TargetClass::Super => 7,
        TargetClass::Capital => 6,
        TargetClass::Frigate => 5,
        TargetClass::Corvette => 4,
        TargetClass::Structure => 3,
        TargetClass::Transport => 2,
        TargetClass::Bomber => 1,
        TargetClass::Fighter | TargetClass::Other => 0,
    };
    let rank = |def| {
        let d = rt.catalog.ship(def);
        let hero = visuals.ship(def).hero;
        (hero, class_priority(d.class), d.population, (d.hull + d.shield) as i64)
    };
    let mut ranked: Vec<_> = sel.ships.iter().filter_map(|id| cur.ship(*id)).map(|s| s.def).collect();
    ranked.sort_by(|&a, &b| rank(b).cmp(&rank(a)));
    if sel.is_changed() && sel.ships != mem.last_selection {
        mem.last_selection = sel.ships.clone();
        if mem.last_select_voice.is_none_or(|t| clock - t >= SELECT_VOICE_COOLDOWN) {
            let line = ranked.iter().find_map(|&def| visuals.ship(def).sfx.select.as_deref())
                .or_else(|| match faction.as_deref() {
                    Some(f) if voice_faction(f) == Some("Empire") => Some("Unit_Select_Empire"),
                    Some(f) if voice_faction(f) == Some("Rebel") => Some("Unit_Select_Rebel"),
                    Some(f) if voice_faction(f) == Some("Underworld") => Some("Unit_Select_Underworld"),
                    _ => None,
                });
            if let Some(n) = line {
                mem.last_select_voice = Some(clock);
                send(n, None, &mut stats, &mut out);
            }
        }
    }
    // An attack on one hardpoint: the `SFXEvent_Attack_Hardpoint` line of that hardpoint's type ("Instructions on taking out those laser cannons!"),
    // instead of the plain attack line.
    let mut hardpoint_line = false;
    for o in hardpoint_orders.read() {
        let Some(target) = cur.ship(o.target) else { continue };
        let ty = visuals.ship(target.def).hardpoints.get(o.hardpoint as usize).map_or("", |h| h.hp_type.as_str());
        let line = attack_hardpoint_line(visuals, &ranked, ty, faction.as_deref());
        if let Some(n) = line {
            send(n, None, &mut stats, &mut out);
            hardpoint_line = true;
        }
    }
    for o in orders.read() {
        // `SFXEvent_Fleet_Move` answers a move order given to a group (estimate: the data does not say when the
        // engine prefers it; units without a fleet line use their single-unit move line).
        let group = sel.ships.len() > 1;
        let specific = |def| {
            let v = &visuals.ship(def).sfx;
            match o.kind {
            OrderKind::Move if group => v.fleet_move.as_deref().or(v.group_move.as_deref()),
            OrderKind::Move => v.mv.as_deref(),
            OrderKind::Attack if hardpoint_line => None,
            OrderKind::Attack if group => v.attack.as_deref().or(v.group_attack.as_deref()),
            OrderKind::Attack => v.attack.as_deref(),
            OrderKind::Guard => v.guard.as_deref(),
            OrderKind::Stop => v.stop.as_deref(),
            }
        };
        let fallback = |def| {
            let v = &visuals.ship(def).sfx;
            match o.kind {
            // `Assist_*` lines are the last resort of units that have no line of their own (estimate: the data does not say when they play).
            OrderKind::Move if group => v.mv.as_deref().or(v.assist_move.as_deref()),
            OrderKind::Move => v.fleet_move.as_deref().or(v.assist_move.as_deref()),
            OrderKind::Guard => v.stop.as_deref(),
            OrderKind::Attack if !hardpoint_line => v.assist_attack.as_deref(),
            _ => None,
            }
        };
        let mut lines = ranked.clone();
        if lines.is_empty() {
            if let Some(s) = cur.ship(o.lead) {
                lines.push(s.def);
            }
        }
        let faction_fallback = match o.kind {
            OrderKind::Move if group => match faction.as_deref() {
                Some(f) if voice_faction(f) == Some("Empire") => Some("Unit_Fleet_Move_Empire"),
                Some(f) if voice_faction(f) == Some("Rebel") => Some("Unit_Fleet_Move_Rebel"),
                Some(f) if voice_faction(f) == Some("Underworld") => Some("Unit_Fleet_Move_Underworld"),
                _ => None,
            },
            OrderKind::Move => match faction.as_deref() {
                Some(f) if voice_faction(f) == Some("Empire") => Some("Unit_Move_Empire"),
                Some(f) if voice_faction(f) == Some("Rebel") => Some("Unit_Move_Rebel"),
                Some(f) if voice_faction(f) == Some("Underworld") => Some("Unit_Move_Underworld"),
                _ => None,
            },
            OrderKind::Attack if !hardpoint_line => match faction.as_deref() {
                Some(f) if voice_faction(f) == Some("Empire") => Some("Unit_Attack_Empire"),
                Some(f) if voice_faction(f) == Some("Rebel") => Some("Unit_Attack_Rebel"),
                Some(f) if voice_faction(f) == Some("Underworld") => Some("Unit_Attack_Underworld"),
                _ => None,
            },
            _ => None,
        };
        let name = lines.iter().find_map(|&def| specific(def))
            .or_else(|| lines.iter().find_map(|&def| fallback(def)))
            .or(faction_fallback);
        if let Some(n) = name {
            send(n, None, &mut stats, &mut out);
        }
    }
}

/// The unit's report for a destroyed hardpoint: the `SFXEvent_Hardpoint_Destroyed` entry of the hardpoint's `Type`.
fn hardpoint_destroyed_line(v: &crate::visuals::ShipVisual, hardpoint: u16) -> Option<&str> {
    let ty = v.hardpoints.get(hardpoint as usize).map_or("", |h| h.hp_type.as_str());
    v.sfx.hardpoint_destroyed_by_type.iter().find(|(t, _)| t == ty).map(|(_, e)| e.as_str())
}

/// The unit's acknowledgment when targeted at a specific hardpoint: exact type match, category match, or faction fallback.
pub fn attack_hardpoint_line<'a>(
    visuals: &'a crate::visuals::VisualCatalog,
    ranked: &[ShipId],
    ty: &str,
    faction: Option<&str>,
) -> Option<&'a str> {
    // 1. Exact hardpoint type match on ranked ships
    if let Some(n) = ranked.iter().find_map(|&def| {
        visuals.ship(def).sfx.attack_hardpoint_by_type.iter().find(|(t, _)| t.eq_ignore_ascii_case(ty)).map(|(_, e)| e.as_str())
    }) {
        return Some(n);
    }
    // 2. Category match on ranked ships
    let category = if ty.contains("SHIELD") {
        Some("SHIELD")
    } else if ty.contains("ENGINE") {
        Some("ENGINE")
    } else if ty.contains("WEAPON") || ty.contains("LASER") || ty.contains("TURRET") || ty.contains("CANNON") || ty.contains("TORPEDO") || ty.contains("MISSILE") {
        Some("WEAPON")
    } else if ty.contains("BAY") || ty.contains("HANGAR") {
        Some("BAY")
    } else {
        None
    };

    if let Some(cat) = category {
        if let Some(n) = ranked.iter().find_map(|&def| {
            visuals.ship(def).sfx.attack_hardpoint_by_type.iter().find(|(t, _)| t.to_ascii_uppercase().contains(cat)).map(|(_, e)| e.as_str())
        }) {
            return Some(n);
        }
    }

    // 3. Category faction fallback
    match category {
        Some("SHIELD") => match faction {
            Some(f) if voice_faction(f) == Some("Empire") => Some("Unit_Attack_Hardpoint_Shield_Empire"),
            Some(f) if voice_faction(f) == Some("Rebel") => Some("Unit_Attack_Hardpoint_Shield_Rebel"),
            Some(f) if voice_faction(f) == Some("Underworld") => Some("Unit_Attack_Hardpoint_Shield_Underworld"),
            _ => None,
        },
        Some("ENGINE") => match faction {
            Some(f) if voice_faction(f) == Some("Empire") => Some("Unit_Attack_Hardpoint_Engine_Empire"),
            Some(f) if voice_faction(f) == Some("Rebel") => Some("Unit_Attack_Hardpoint_Engine_Rebel"),
            Some(f) if voice_faction(f) == Some("Underworld") => Some("Unit_Attack_Hardpoint_Engine_Underworld"),
            _ => None,
        },
        Some("WEAPON") => match faction {
            Some(f) if voice_faction(f) == Some("Empire") => Some("Unit_Attack_Hardpoint_Weapon_Empire"),
            Some(f) if voice_faction(f) == Some("Rebel") => Some("Unit_Attack_Hardpoint_Weapon_Rebel"),
            Some(f) if voice_faction(f) == Some("Underworld") => Some("Unit_Attack_Hardpoint_Weapon_Underworld"),
            _ => None,
        },
        Some("BAY") => match faction {
            Some(f) if voice_faction(f) == Some("Empire") => Some("Unit_Attack_Hardpoint_Hangar_Empire"),
            Some(f) if voice_faction(f) == Some("Rebel") => Some("Unit_Attack_Hardpoint_Hangar_Rebel"),
            Some(f) if voice_faction(f) == Some("Underworld") => Some("Unit_Attack_Hardpoint_Hangar_Underworld"),
            _ => None,
        },
        _ => None,
    }
}

/// The `Factions.xml` countdown announcements (`SpeechEvent_Super_Weapon_[Enemy_]Moving_Into_Range`, `..._Moving_Range_60/30/15/05_Seconds`,
/// `..._Moved_Into_Range`) a Death Star approach countdown going from `prev` (`None` = first sight) to `now` seconds triggers.
fn countdown_calls(prev: Option<f32>, now: f32) -> Vec<&'static str> {
    let Some(p) = prev else { return vec!["moving_into_range"] };
    let mut out: Vec<&'static str> = [(60.0, "moving_range_60_seconds"), (30.0, "moving_range_30_seconds"), (15.0, "moving_range_15_seconds"), (5.0, "moving_range_05_seconds")]
        .into_iter()
        .filter(|&(t, _)| p > t && now <= t)
        .map(|(_, tag)| tag)
        .collect();
    if p > 0.0 && now <= 0.0 {
        out.push("moved_into_range");
    }
    out
}

/// Starts the space music scene for the player's faction. Battle music plays while shots are being
/// exchanged and ends after `Music_Space_Battle_To_Ambient_Peace_Seconds` (Audio.xml) without any.
pub fn battle_music(
    settings: Res<BattleViewSettings>,
    status: Res<BattleStatus>,
    rt: Res<BattleRuntime>,
    stats: Res<BattleAudioStats>,
    director: Option<ResMut<MusicDirector>>,
    audio: Option<Res<AlamoAudio>>,
    mut speech: MessageWriter<alamo_audio::PlaySpeech>,
    mut countdown: Local<HashMap<ShipId, f32>>,
) {
    let Some(mut director) = director else { return };
    if !settings.audio {
        return;
    }
    // Once the battle is over the result stinger (set by the flow) must not be replaced by the space scene.
    if status.over {
        director.set_battle_intensity(0.0);
        return;
    }
    if let Some(f) = &rt.player_faction {
        director.set_scene(MusicScene::Space(f.clone()));
    }
    // A Death Star (any ship with a super laser) in the fight switches to the faction's `Music_Event_Space_Battle_Super_Weapon` theme.
    let super_weapon = rt.cur.as_ref().is_some_and(|c| c.ships.iter().any(|s| rt.catalog.ship(s.def).abilities.iter().any(|a| a.super_laser.is_some())));
    director.set_super_weapon(super_weapon);
    if let (Some(cur), Some(audio)) = (rt.cur.as_ref(), audio.as_ref()) {
        for s in &cur.ships {
            let defs = &rt.catalog.ship(s.def).abilities;
            let Some(first) = defs.iter().position(|a| a.super_laser.as_ref().is_some_and(|l| l.initial_cooldown > 0.0)) else { continue };
            let remaining = s.abilities.get(first).map_or(0.0, |&(active, cooldown)| active.max(cooldown));
            let prev = countdown.get(&s.id).copied();
            // Only the approach countdown is announced, not the later recharges.
            if prev == Some(0.0) {
                continue;
            }
            for call in countdown_calls(prev, remaining) {
                // Own Death Star: the faction's own lines; an enemy one: the player's `..._Enemy_...` warnings.
                let tag = format!("speechevent_super_weapon_{}{call}", if s.side == settings.player_side { "" } else { "enemy_" });
                if let Some(n) = rt.player_faction.as_deref().and_then(|f| audio.constants.faction_sfx(f, &tag)) {
                    speech.write(alamo_audio::PlaySpeech { event: n.to_string() });
                }
            }
            countdown.insert(s.id, remaining);
        }
    }
    let fighting = stats.last_combat.is_some_and(|t| rt.clock - t <= COMBAT_WINDOW);
    director.set_battle_intensity(if fighting { 1.0 } else { 0.0 });
}

/// The ears of the battle: on the battle plane at the point the camera looks at, lifted by `Audio_Space_3D_Listener_Z_Pullback_Dist`
/// (Audio.xml, 0 in space: "the higher the Z elevation, the higher above the map the listener is") and turned like the camera for the
/// left / right pan. The distance falloff of every positioned sound is relative to this point, so the zoom does not change what is
/// heard: a far zoom shows more of the battle, and what lies away from the centre of the screen is quieter. The listener used to
/// move 45 % of the way to the camera's eye; zoomed out, every engine and gun was then beyond its falloff and the battle went silent.
/// Does nothing without `AlamoAudioPlugin`.
pub fn battle_listener(
    mut commands: Commands,
    settings: Res<BattleViewSettings>,
    audio: Option<Res<AlamoAudio>>,
    cameras: Query<(&crate::camera::BattleCamera, &Transform), Without<BattleListener>>,
    mut listener: Query<&mut Transform, With<BattleListener>>,
) {
    let Some(audio) = audio.filter(|_| settings.audio) else {
        return;
    };
    let Ok((cam, tf)) = cameras.single() else {
        return;
    };
    let at = Transform { translation: listener_position(cam.focus, audio.constants.space_listener_pullback), rotation: tf.rotation, ..default() };
    match listener.single_mut() {
        Ok(mut l) => *l = at,
        Err(_) => {
            commands.spawn((Name::new("battle sound listener"), crate::BattleEntity, BattleListener, bevy::audio::SpatialListener::new(EAR_GAP), at));
        }
    }
}

/// Where [`battle_listener`] puts the ears for a camera looking at `focus`: `pullback` above the battle plane, whatever the zoom.
pub fn listener_position(focus: Vec3, pullback: f32) -> Vec3 {
    focus + Vec3::Y * pullback
}

/// Sound sources of the battle scene: every ship hums its engine loop: `SFXEvent_Engine_Moving_Loop` while it moves,
/// `SFXEvent_Engine_Idle_Loop` while it stands, `SFXEvent_Ambient_Loop` for units that have no engine loop of the current kind.
/// Loops of dead, retreated or unseen ships are silent. Does nothing without `AlamoAudioPlugin`.
pub fn battle_sources(
    mut commands: Commands,
    settings: Res<BattleViewSettings>,
    rt: Res<BattleRuntime>,
    audio: Option<Res<AlamoAudio>>,
    mut ships: Query<(Entity, &crate::ships::ShipView, Option<&mut alamo_audio::SfxEmitter>)>,
) {
    if !settings.audio || audio.is_none() {
        return;
    }
    let Some(cur) = rt.cur.as_ref() else { return };
    let player = settings.player_side;
    for (entity, view, emitter) in &mut ships {
        let wanted = cur.ship(view.id).and_then(|s| {
            let seen = s.side == player || (s.seen_by >> player.min(7)) & 1 == 1;
            if !seen {
                return None;
            }
            let v = &rt.visuals.ship(s.def).sfx;
            // Estimate: the data names an idle and a moving loop but not the switch point. A ship with a route or a move / attack /
            // retreat order is "moving"; the snapshot only holds living ships that have not retreated yet.
            let moving = !s.path.is_empty() || matches!(s.order, battle_sim::Order::Move(_) | battle_sim::Order::AttackMove(_) | battle_sim::Order::Attack(_) | battle_sim::Order::Retreat);
            let engine = if moving { v.engine_moving.as_ref() } else { v.engine_idle.as_ref() };
            engine.or(v.ambient_loop.as_ref()).cloned()
        });
        match emitter {
            Some(mut e) => {
                if e.event != wanted {
                    e.event = wanted;
                }
            }
            None if wanted.is_some() => {
                commands.entity(entity).insert(alamo_audio::SfxEmitter::new(wanted));
            }
            None => {}
        }
    }
}

/// The emitter that voices the `SFXEvent_Special_Ability_Loop` of one ability of one ship; a child of the ship's entity, so the sound
/// follows the ship ([`battle_ability_loops`]).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbilityLoop {
    /// Simulation id of the ship.
    pub ship: u32,
    /// Index into the ship definition's abilities.
    pub ability: u16,
}

/// The ability loops that sound in `snapshot`, as `((ship id, ability index), event)`: the tractor beam's hum, the proton beam's blast, for
/// as long as the sim says the ability runs (its seconds of effect left), on ships the player sees (own ones and enemy ones in sight, as
/// for the engine loops). The super laser is not here: its shot plays its own loop at the muzzle ([`crate::ships::ShotSounds`]).
pub fn running_ability_loops<'a>(
    snapshot: &Snapshot,
    catalog: &battle_sim::Catalog,
    visuals: &'a crate::visuals::VisualCatalog,
    player: u8,
) -> Vec<((u32, u16), &'a str)> {
    let mut out = Vec::new();
    for s in &snapshot.ships {
        let seen = s.side == player || (s.seen_by >> player.min(7)) & 1 == 1;
        if !seen {
            continue;
        }
        let defs = &catalog.ship(s.def).abilities;
        for (i, &(running, _)) in s.abilities.iter().enumerate() {
            let Some(def) = defs.get(i).filter(|d| running > 0.0 && d.super_laser.is_none()) else { continue };
            let event = visuals.ship(s.def).sfx.abilities.iter().find(|a| a.kind.eq_ignore_ascii_case(&def.kind)).and_then(|a| a.run_loop.as_deref());
            if let Some(event) = event {
                out.push(((s.id, i as u16), event));
            }
        }
    }
    out
}

/// Ability loops: while an ability of a ship runs, the ship makes its `SFXEvent_Special_Ability_Loop` (`Unit_Star_Destroyer_Tractor_Beam` for
/// the 59 objects with a `TRACTOR_BEAM`, `Unit_Star_Destroyer_Energy_Blast` for the proton beam of the Accuser and the Eclipse's
/// `ENERGY_WEAPON`) where it is, through an [`alamo_audio::SfxEmitter`] on a child entity of the ship: the plugin starts the loop
/// when it is audible, gives the event's `Max_Instances` voices to the nearest ships and fades the loop out over its
/// `Loop_Fade_Out_Seconds` when the ability ends or the ship is gone. The emitter stays until its voice has faded out. `SP_AUDIO_LOG` logs
/// each start and end. Does nothing without `AlamoAudioPlugin`.
#[allow(clippy::too_many_arguments)]
pub fn battle_ability_loops(
    mut commands: Commands,
    settings: Res<BattleViewSettings>,
    rt: Res<BattleRuntime>,
    audio: Option<Res<AlamoAudio>>,
    listeners: Query<&GlobalTransform, With<BattleListener>>,
    ships: Query<(), With<crate::ships::ShipView>>,
    mut emitters: Query<(Entity, &AbilityLoop, &mut alamo_audio::SfxEmitter)>,
) {
    if !settings.audio || audio.is_none() {
        return;
    }
    let Some(cur) = rt.cur.as_ref() else { return };
    let wanted = running_ability_loops(cur, &rt.catalog, &rt.visuals, settings.player_side);
    let log = audio_log();
    let mut have: HashSet<(u32, u16)> = HashSet::new();
    for (entity, key, mut emitter) in &mut emitters {
        have.insert((key.ship, key.ability));
        match wanted.iter().find(|(k, _)| *k == (key.ship, key.ability)) {
            Some((_, event)) => {
                if emitter.event.as_deref() != Some(event) {
                    emitter.event = Some((*event).to_string());
                }
            }
            None if emitter.event.is_some() => {
                if log {
                    info!("audio ability loop: ship {} ability {} ends {}", key.ship, key.ability, emitter.event.as_deref().unwrap_or_default());
                }
                emitter.event = None;
            }
            // The plugin has faded the loop out: the emitter is not needed any more.
            None if emitter.playing().is_none() => commands.entity(entity).try_despawn(),
            None => {}
        }
    }
    for ((ship, ability), event) in wanted {
        let Some(&parent) = rt.entities.get(&ship).filter(|e| ships.contains(**e) && !have.contains(&(ship, ability))) else { continue };
        // The position is also given as a global one: the child's own is only worked out from its parent at the end of the frame.
        let at = cur.ship(ship).map_or(Vec3::ZERO, |s| sim_to_bevy(s.pos));
        if log {
            let s = cur.ship(ship);
            info!(
                "audio ability loop: ship {ship} {} ability {ability} {} starts {event} ({:.1} s of effect left, {} units from the listener)",
                s.map_or("?", |s| rt.catalog.ship(s.def).name.as_str()),
                s.and_then(|s| rt.catalog.ship(s.def).abilities.get(ability as usize)).map_or("?", |a| a.kind.as_str()),
                s.and_then(|s| s.abilities.get(ability as usize)).map_or(0.0, |a| a.0),
                listeners.iter().next().map_or("?".to_string(), |l| format!("{:.0}", l.translation().distance(at)))
            );
        }
        commands.entity(parent).with_child((
            Name::new(format!("ability loop {ability} of ship {ship}")),
            AbilityLoop { ship, ability },
            alamo_audio::SfxEmitter::new(Some(event.to_string())),
            Transform::default(),
            GlobalTransform::from_translation(at),
        ));
    }
}

/// Below this speed (units per second) a unit stands still for its `SFXEvent_Ambient_Moving` (estimate: the data does not say; a
/// moving fighter flies hundreds of units per second).
const AMBIENT_MOVING_MIN_SPEED: f32 = 1.0;

/// Battle seconds a flyby request holds a voice of its event before the voice exists: the plugin reads a sample it has not cached on a
/// worker thread first (estimate: a read takes a frame or two).
const FLYBY_REQUEST_GRACE: f64 = 0.5;

/// A flyby that finds every `Max_Instances` voice of its event taken starts only when it is at least this many times as loud as each of
/// them (6 dB); the voice the plugin then cuts, the event's oldest, is quieter by at least that much (estimate: the data does not say).
const FLYBY_TAKEOVER: f32 = 2.0;

/// What the voice rules need of a unit's flyby event: its `Max_Instances` and, for a 3D sound, its saturation distance.
#[derive(Debug, Clone, Copy)]
struct FlybyRules {
    cap: Option<u32>,
    saturation: Option<f32>,
}

/// The `SFXEvent_Ambient_Moving` clock of one unit: the battle time of its next sound (`None` while the unit stands), when it last
/// asked for one, and its own random sequence, seeded by the unit's simulation id so that a battle replays with the same gaps.
#[derive(Component, Debug, Clone)]
pub struct AmbientMoving {
    next: Option<f64>,
    requested: Option<f64>,
    rng: u64,
    /// Looked up once when the unit appears (waiting units are due every frame); `None` keeps the unit silent.
    rules: Option<FlybyRules>,
}

impl AmbientMoving {
    /// The clock of the unit with simulation id `ship` whose flyby is `event`; silent without one and for a loop.
    pub fn new(ship: u32, event: Option<&alamo_audio::SfxEvent>) -> Self {
        let rules = event
            .filter(|e| !e.looping)
            .map(|e| FlybyRules { cap: e.max_instances, saturation: e.saturation_distance.filter(|_| !e.is_2d) });
        Self { next: None, requested: None, rng: u64::from(ship) ^ 0x6A09_E667_F3BC_C909, rules }
    }

    /// A gap uniform in `min..=max` seconds (splitmix64).
    fn gap(&mut self, (min, max): (f32, f32)) -> f64 {
        self.rng = self.rng.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        let u = (z >> 11) as f64 / (1u64 << 53) as f64;
        f64::from(min) + (f64::from(max) - f64::from(min)) * u
    }

    /// Advances the clock to battle time `now`; true while the unit's sound is due, until [`AmbientMoving::played`]. A unit that starts
    /// moving waits one gap first (a fleet that sets off together does not sound at once); one that stops starts over when it moves again.
    pub fn due(&mut self, moving: bool, now: f64, gaps: (f32, f32)) -> bool {
        if !moving {
            self.next = None;
            return false;
        }
        match self.next {
            Some(due) => now >= due,
            None => {
                self.next = Some(now + self.gap(gaps));
                false
            }
        }
    }

    /// The sound was asked for at battle time `now`: the next one is due a gap later.
    pub fn played(&mut self, now: f64, gaps: (f32, f32)) {
        self.next = Some(now + self.gap(gaps));
        self.requested = Some(now);
    }

    /// Battle time of the next sound, if the unit moves.
    pub fn next(&self) -> Option<f64> {
        self.next
    }

    /// True while a request made at battle time `now` may still wait for its voice ([`FLYBY_REQUEST_GRACE`]).
    fn fresh(&self, now: f64) -> bool {
        self.requested.is_some_and(|t| now - t < FLYBY_REQUEST_GRACE)
    }
}

/// How many of the flybys of one event that are due start now. `due`: their loudness (distance attenuation), loudest first;
/// `instances`: the loudness of the event's voices that play and of requests not started yet. While the event has fewer than `cap`
/// (`Max_Instances`) instances the loudest due ones fill them; when all are taken only one at least [`FLYBY_TAKEOVER`] times as loud as
/// each instance starts, so a near flyby is never cut for a far one. The others wait.
fn flybys_to_start(due: &[f32], instances: &[f32], cap: Option<u32>) -> usize {
    let Some(cap) = cap else { return due.len() };
    let free = (cap as usize).saturating_sub(instances.len());
    if free > 0 {
        return free.min(due.len());
    }
    let loudest = instances.iter().copied().fold(0.0, f32::max);
    usize::from(due.first().is_some_and(|&l| l >= FLYBY_TAKEOVER * loudest))
}

/// A unit whose flyby is due (see [`battle_ambient_moving`]).
struct DueFlyby<'a> {
    entity: Entity,
    ship: u32,
    def: ShipId,
    event: &'a str,
    cap: Option<u32>,
    loudness: f32,
    at: Vec3,
    gaps: (f32, f32),
}

/// Flybys: a unit whose data names an `SFXEvent_Ambient_Moving` plays it again and again while it moves, `SFXEvent_Ambient_Moving_Min_Delay_Seconds`
/// to `_Max_Delay_Seconds` of battle time apart ([`AmbientMoving`]); the sound sits on the unit and follows it. Every moving fighter
/// asks for one every few seconds and its event has 2 voices (`Max_Instances` of `Preset_EGB`), so the plugin's rule (the oldest voice
/// gives way) would cut each 5 s whoosh after a fraction of a second: a due flyby waits until a voice of its event is free, the
/// loudest (nearest the listener) first, and takes a voice only from flybys at least 6 dB quieter ([`flybys_to_start`]). Units the
/// player does not see stay silent like their loops, and a looping event is not repeated (it would pile up loops that never end;
/// `Unit_Shuttle_Moving_Engine_Loop` of the shuttles is one). `SP_AUDIO_LOG` logs every flyby with the unit. Does nothing without
/// `AlamoAudioPlugin`.
#[allow(clippy::too_many_arguments)]
pub fn battle_ambient_moving(
    mut commands: Commands,
    settings: Res<BattleViewSettings>,
    rt: Res<BattleRuntime>,
    audio: Option<Res<AlamoAudio>>,
    audio_settings: Option<Res<alamo_audio::AudioSettings>>,
    listeners: Query<&GlobalTransform, With<BattleListener>>,
    voices: Query<&alamo_audio::SfxVoice>,
    mut ships: Query<(Entity, &crate::ships::ShipView, &GlobalTransform, Option<&mut AmbientMoving>)>,
    mut out: MessageWriter<PlaySfx>,
) {
    let Some(audio) = audio.filter(|_| settings.audio) else {
        return;
    };
    let Some(cur) = rt.cur.as_ref() else { return };
    let (player, now) = (settings.player_side, rt.clock);
    let ear = listeners.iter().next().map(GlobalTransform::translation);
    let (scale, rolloff) = audio_settings.map_or((1.0, 1.0), |s| (s.saturation_scale, s.rolloff));
    let mut due: Vec<DueFlyby> = Vec::new();
    // Requests that may still wait for their voice: (event, unit, loudness).
    let mut fresh: Vec<(&str, Entity, f32)> = Vec::new();
    for (entity, view, at, clock) in &mut ships {
        let sfx = &rt.visuals.ship(view.def).sfx;
        let Some(event) = sfx.ambient_moving.as_deref() else { continue };
        let Some(mut clock) = clock else {
            commands.entity(entity).insert(AmbientMoving::new(view.id, audio.sfx.get(event)));
            continue;
        };
        let Some(rules) = clock.rules else { continue };
        let moving = cur.ship(view.id).is_some_and(|s| {
            let seen = s.side == player || (s.seen_by >> player.min(7)) & 1 == 1;
            seen && s.speed > AMBIENT_MOVING_MIN_SPEED
        });
        let (is_due, is_fresh) = (clock.due(moving, now, sfx.ambient_moving_delay), clock.fresh(now));
        if !is_due && !is_fresh {
            continue;
        }
        let loudness = match (ear, rules.saturation) {
            (Some(ear), Some(sat)) => alamo_audio::plugin::distance_attenuation(at.translation().distance(ear), sat * scale, rolloff),
            _ => 1.0,
        };
        if is_fresh {
            fresh.push((event, entity, loudness));
        }
        if is_due {
            let gaps = sfx.ambient_moving_delay;
            due.push(DueFlyby { entity, ship: view.id, def: view.def, event, cap: rules.cap, loudness, at: at.translation(), gaps });
        }
    }
    // Loudest first; the ship id breaks ties so that the choice does not depend on the query order.
    due.sort_by(|a, b| b.loudness.total_cmp(&a.loudness).then(a.ship.cmp(&b.ship)));
    // Unit files spell an event's name in different cases.
    let mut events: Vec<&str> = Vec::new();
    for d in &due {
        if !events.iter().any(|e| e.eq_ignore_ascii_case(d.event)) {
            events.push(d.event);
        }
    }
    for event in events {
        let candidates: Vec<&DueFlyby> = due.iter().filter(|d| d.event.eq_ignore_ascii_case(event)).collect();
        let playing: Vec<&alamo_audio::SfxVoice> = voices.iter().filter(|v| !v.is_stopping() && v.event().eq_ignore_ascii_case(event)).collect();
        let instances: Vec<f32> = playing
            .iter()
            .map(|v| v.attenuation())
            .chain(fresh.iter().filter(|f| f.0.eq_ignore_ascii_case(event) && !playing.iter().any(|v| v.source() == Some(f.1))).map(|f| f.2))
            .collect();
        let cap = candidates[0].cap;
        let loudness: Vec<f32> = candidates.iter().map(|d| d.loudness).collect();
        let start = flybys_to_start(&loudness, &instances, cap);
        for (i, d) in candidates[..start].iter().enumerate() {
            let Ok((.., Some(mut clock))) = ships.get_mut(d.entity) else { continue };
            clock.played(now, d.gaps);
            if audio_log() {
                // `taken` at the cap: the flyby takes the voice of a quieter one.
                info!(
                    "audio ambient moving: ship {} {} plays {} at t={now:.2}, loudness {:.3}, taken {} of {} voices (loudest {:.3}), {} wait, next in {:.2} s",
                    d.ship,
                    rt.catalog.ship(d.def).name,
                    d.event,
                    d.loudness,
                    instances.len() + i,
                    cap.map_or("any".to_string(), |c| c.to_string()),
                    instances.iter().copied().fold(0.0, f32::max),
                    candidates.len() - start,
                    clock.next().map_or(0.0, |n| n - now)
                );
            }
            out.write(PlaySfx { event: d.event.to_string(), position: Some(d.at), source: Some(d.entity) });
        }
    }
}

/// `SP_AUDIO_LOG` is set (see `alamo_audio::plugin`).
fn audio_log() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("SP_AUDIO_LOG").is_some())
}

/// The sound listener of the battle (see [`battle_listener`]).
#[derive(Component)]
pub struct BattleListener;

/// Distance between the listener's ears in world units (the pan depends on the ratio to the distance of a sound).
const EAR_GAP: f32 = 10.0;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::visuals::{HpVisual, ShipVisual};

    #[test]
    fn hardpoint_report_follows_the_hardpoint_type() {
        let mut v = ShipVisual::default();
        v.hardpoints = ["HARD_POINT_WEAPON_LASER", "HARD_POINT_ENGINE", "HARD_POINT_FIGHTER_BAY"]
            .map(|t| HpVisual { hp_type: t.into(), ..Default::default() })
            .to_vec();
        v.sfx.hardpoint_destroyed_by_type = vec![
            ("HARD_POINT_WEAPON_LASER".into(), "Unit_Lost_Laser_Star_Destroyer".into()),
            ("HARD_POINT_ENGINE".into(), "Unit_Lost_Engines_Star_Destroyer".into()),
        ];
        assert_eq!(hardpoint_destroyed_line(&v, 0), Some("Unit_Lost_Laser_Star_Destroyer"));
        assert_eq!(hardpoint_destroyed_line(&v, 1), Some("Unit_Lost_Engines_Star_Destroyer"));
        assert_eq!(hardpoint_destroyed_line(&v, 2), None, "no entry (or SFX_NA) = silent");
        assert_eq!(hardpoint_destroyed_line(&v, 9), None);
    }

    #[test]
    fn test_unit_selection_voice_cooldown_and_fallback() {
        assert_eq!(SELECT_VOICE_COOLDOWN, 2.0);

        let mut mem = AudioMemory::default();
        let clock = 10.0;
        assert!(mem.last_select_voice.is_none_or(|t| clock - t >= SELECT_VOICE_COOLDOWN));
        mem.last_select_voice = Some(clock);

        let clock_quick_click = 10.5;
        assert!(!mem.last_select_voice.is_none_or(|t| clock_quick_click - t >= SELECT_VOICE_COOLDOWN));

        let clock_later = 12.1;
        assert!(mem.last_select_voice.is_none_or(|t| clock_later - t >= SELECT_VOICE_COOLDOWN));

        // Faction fallback lines
        let emp_fallback = match Some("Empire") {
            Some(f) if voice_faction(f) == Some("Empire") => Some("Unit_Select_Empire"),
            _ => None,
        };
        assert_eq!(emp_fallback, Some("Unit_Select_Empire"));
    }

    #[test]
    fn test_order_voice_priority_ranking() {
        let class_priority = |class: TargetClass| match class {
            TargetClass::Super => 7,
            TargetClass::Capital => 6,
            TargetClass::Frigate => 5,
            TargetClass::Corvette => 4,
            TargetClass::Structure => 3,
            TargetClass::Transport => 2,
            TargetClass::Bomber => 1,
            TargetClass::Fighter | TargetClass::Other => 0,
        };
        assert!(class_priority(TargetClass::Super) > class_priority(TargetClass::Capital));
        assert!(class_priority(TargetClass::Capital) > class_priority(TargetClass::Frigate));
        assert!(class_priority(TargetClass::Frigate) > class_priority(TargetClass::Corvette));
        assert!(class_priority(TargetClass::Corvette) > class_priority(TargetClass::Structure));
        assert!(class_priority(TargetClass::Structure) > class_priority(TargetClass::Transport));
        assert!(class_priority(TargetClass::Transport) > class_priority(TargetClass::Bomber));
        assert!(class_priority(TargetClass::Bomber) > class_priority(TargetClass::Fighter));

        // Hero priority over normal capital
        let hero_rank = (true, class_priority(TargetClass::Frigate), 2, 5000i64);
        let normal_capital_rank = (false, class_priority(TargetClass::Capital), 4, 12000i64);
        assert!(hero_rank > normal_capital_rank, "hero outranks any non-hero ship");
    }

    #[test]
    fn test_attack_hardpoint_lines_by_type_and_category_fallback() {
        let mut visuals = crate::visuals::VisualCatalog::default();
        let mut v = ShipVisual::default();
        v.sfx.attack_hardpoint_by_type = vec![
            ("HARD_POINT_WEAPON_LASER".into(), "Unit_Attack_Laser_Star_Destroyer".into()),
            ("HARD_POINT_ENGINE".into(), "Unit_Attack_Engine_Star_Destroyer".into()),
        ];
        visuals.ships.push(v);
        let ranked = vec![0 as ShipId];

        // 1. Exact match
        assert_eq!(
            attack_hardpoint_line(&visuals, &ranked, "HARD_POINT_WEAPON_LASER", Some("Empire")),
            Some("Unit_Attack_Laser_Star_Destroyer")
        );

        // 2. Category match (e.g. sublight engine variation)
        assert_eq!(
            attack_hardpoint_line(&visuals, &ranked, "HARD_POINT_SUBLIGHT_ENGINE", Some("Empire")),
            Some("Unit_Attack_Engine_Star_Destroyer")
        );

        // 3. Faction category fallback when ship has no shield line
        assert_eq!(
            attack_hardpoint_line(&visuals, &ranked, "HARD_POINT_SHIELD_GENERATOR", Some("Empire")),
            Some("Unit_Attack_Hardpoint_Shield_Empire")
        );
        assert_eq!(
            attack_hardpoint_line(&visuals, &ranked, "HARD_POINT_SHIELD_GENERATOR", Some("Rebel")),
            Some("Unit_Attack_Hardpoint_Shield_Rebel")
        );
    }

    #[test]
    fn test_advisor_emergency_and_capital_loss_events() {
        // Verify GalaxyEvent suffixes for emergency events
        assert_eq!(GalaxyEvent::CapitalShipLost.suffixes(), &["Capital_Ship_Destroyed", "Unit_Destroyed"]);
        assert_eq!(GalaxyEvent::StationCriticalDamage.suffixes(), &["Space_Station_Damaged", "Space_Station_Under_Attack", "Base_Under_Attack"]);
        assert_eq!(GalaxyEvent::ShieldsDown.suffixes(), &["Shields_Down", "Shields_Depleted", "Shield_Generator_Destroyed"]);

        // Memory tracking for station critical damage cooldown and shield down warnings
        let mut mem = AudioMemory::default();
        let clock = 50.0;
        assert!(mem.last_station_critical.is_none_or(|t| clock - t >= 30.0));
        mem.last_station_critical = Some(clock);
        assert!(!mem.last_station_critical.is_none_or(|t| 60.0 - t >= 30.0));
        assert!(mem.last_station_critical.is_none_or(|t| 85.0 - t >= 30.0));

        // Shield down deduplication
        let ship_id = 42;
        assert!(mem.shield_down_warned.insert(ship_id), "first shield collapse reports");
        assert!(!mem.shield_down_warned.insert(ship_id), "already warned does not spam");
        mem.shield_down_warned.remove(&ship_id);
        assert!(mem.shield_down_warned.insert(ship_id), "recharged shield can report collapse again");
    }

    /// Game files in memory: just enough for `AlamoAudio::load`.
    struct MemFs(HashMap<String, Vec<u8>>);

    impl alamo_fs::FileSource for MemFs {
        fn read(&self, path: &str) -> std::io::Result<Vec<u8>> {
            self.0.get(&alamo_fs::normalize(path)).cloned().ok_or_else(|| std::io::ErrorKind::NotFound.into())
        }
        fn exists(&self, path: &str) -> bool {
            self.0.contains_key(&alamo_fs::normalize(path))
        }
        fn list(&self, _dir: &str) -> Vec<String> {
            Vec::new()
        }
    }

    fn listener_app(audio_xml: &str) -> App {
        let files = HashMap::from([(alamo_fs::normalize("Xml/Audio.xml"), audio_xml.as_bytes().to_vec())]);
        let mut app = App::new();
        app.insert_resource(BattleViewSettings::default())
            .insert_resource(AlamoAudio::load(std::sync::Arc::new(MemFs(files))))
            .add_systems(Update, battle_listener);
        app
    }

    /// Points the battle camera at `focus` from `distance` (as `apply_camera` does) and returns the camera's and the listener's
    /// transform after one frame.
    fn hear_from(app: &mut App, focus: Vec3, distance: f32) -> (Transform, Transform) {
        let cam = crate::camera::BattleCamera { focus, target_focus: focus, distance, target_distance: distance, ..default() };
        let (translation, rotation) = cam.eye();
        let eye = Transform { translation, rotation, ..default() };
        let world = app.world_mut();
        let existing = world.query_filtered::<Entity, With<crate::camera::BattleCamera>>().iter(world).next();
        match existing {
            Some(e) => {
                world.entity_mut(e).insert((cam, eye));
            }
            None => {
                world.spawn((cam, eye));
            }
        }
        app.update();
        let world = app.world_mut();
        let ears = *world.query_filtered::<&Transform, With<BattleListener>>().single(world).expect("one listener");
        (eye, ears)
    }

    #[test]
    fn the_battle_is_heard_from_the_point_the_camera_looks_at_at_every_zoom() {
        // The Remake's Audio.xml: `Audio_Space_3D_Listener_Z_Pullback_Dist` 0.
        let mut app = listener_app("<Audio><Audio_Space_3D_Listener_Z_Pullback_Dist> 0.0 </Audio_Space_3D_Listener_Z_Pullback_Dist></Audio>");
        let focus = Vec3::new(1200.0, 0.0, -800.0);
        // A turbolaser (`Preset_GUNSTL`: saturation 550 x the space mod 1.5) at the centre of the screen, 2000 and 6000 units to
        // the right of it; zoomed in closest, at the data's default, at the tactical overview (5000 at a 70 degree field of view
        // = 6064 at 60) and out farthest.
        let saturation = 550.0 * 1.5;
        for distance in [80.0, 2000.0, 6064.0, 16000.0] {
            let (eye, ears) = hear_from(&mut app, focus, distance);
            assert!(ears.translation.distance(focus) < 1e-3, "zoom {distance}: listener at {}, not at the focus", ears.translation);
            let gain = |x: f32| alamo_audio::plugin::distance_attenuation((focus + eye.rotation * Vec3::X * x).distance(ears.translation), saturation, 1.0);
            assert_eq!(gain(0.0), 1.0, "zoom {distance}: a gun at the centre of the screen plays at full volume");
            assert!((gain(2000.0) - saturation / 2000.0).abs() < 1e-4, "zoom {distance}: {}", gain(2000.0));
            assert!(gain(6000.0) < gain(2000.0), "zoom {distance}: the distance from the centre of the screen still shapes the mix");
            // The ears point like the camera: what is right on the screen is panned right.
            assert!((ears.rotation * Vec3::X).dot(eye.rotation * Vec3::X) > 0.999, "zoom {distance}");
        }
    }

    /// Battle times at which a unit plays its ambient-moving sound when it moves at the times `moving` says, ticked at 30 frames per second.
    fn flybys(ship: u32, gaps: (f32, f32), moving: impl Fn(f64) -> bool, seconds: f64) -> Vec<f64> {
        let mut clock = AmbientMoving::new(ship, None);
        let mut at = Vec::new();
        for t in (0..(seconds * 30.0) as u32).map(|f| f64::from(f) / 30.0) {
            if clock.due(moving(t), t, gaps) {
                clock.played(t, gaps);
                at.push(t);
            }
        }
        at
    }

    #[test]
    fn a_moving_unit_plays_its_flyby_every_min_to_max_delay_seconds() {
        // An X-wing (`Unit_X_Wing_Fly_By`, delays 5 to 10 s) on the move for ten minutes.
        let at = flybys(7, (5.0, 10.0), |_| true, 600.0);
        let frame = 1.0 / 30.0 + 1e-9;
        assert!((5.0..=10.0 + frame).contains(&at[0]), "the first one waits a gap: {}", at[0]);
        let gaps: Vec<f64> = at.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(gaps.iter().all(|g| (5.0..=10.0 + frame).contains(g)), "{gaps:?}");
        let mean = gaps.iter().sum::<f64>() / gaps.len() as f64;
        assert!((mean - 7.5).abs() < 0.5, "mean gap {mean} s, {} sounds", at.len());
        let (lo, hi) = gaps.iter().fold((f64::MAX, 0.0f64), |(lo, hi), &g| (lo.min(g), hi.max(g)));
        assert!(lo < 5.5 && hi > 9.5, "the whole range is used: {lo:.2}..{hi:.2}");
    }

    #[test]
    fn a_standing_unit_is_silent_and_waits_a_gap_after_it_sets_off() {
        // Stands for 20 s, moves for 20 s, stands again and sets off once more at 60 s; delays 3 to 5 s.
        let moving = |t: f64| (20.0..40.0).contains(&t) || t >= 60.0;
        let at = flybys(3, (3.0, 5.0), moving, 90.0);
        assert!(at.iter().all(|&t| moving(t)), "only while it moves: {at:?}");
        assert!(at.iter().find(|&&t| t >= 20.0).is_some_and(|&t| t >= 23.0), "{at:?}");
        assert!(at.iter().find(|&&t| t >= 60.0).is_some_and(|&t| t >= 63.0), "a new gap after the stop: {at:?}");
    }

    #[test]
    fn each_unit_has_its_own_fixed_flyby_sequence() {
        let a = flybys(11, (3.0, 5.0), |_| true, 120.0);
        assert_eq!(a, flybys(11, (3.0, 5.0), |_| true, 120.0), "a replayed battle hears the same flybys");
        assert_ne!(a, flybys(12, (3.0, 5.0), |_| true, 120.0), "two fighters do not whoosh in step");
    }

    /// The Remake's fighters (skipped without the game files): an X-wing whooshes past every 3 to 5 s, Luke's every 5 to 10 s, the shuttle's
    /// ambient-moving event is a loop; a unit without the delay tags gets the data's common 3 to 5 s.
    #[test]
    fn the_remake_fighters_name_their_flybys() {
        let Some(install) = alamo_fs::GameInstall::detect().filter(|i| i.mod_data("1770851727").is_dir()) else { return };
        let fs = alamo_fs::GameFs::for_mod(&install, Some("1770851727")).expect("game files");
        let (data, _) = alamo_data::load_galaxy(&fs);
        let names = ["X-Wing_Squadron", "TIE_Fighter_Squadron", "Luke_XWing", "Lambda_Shuttle"];
        let catalog = battle_sim::Catalog::build(&data, Some(&fs), &names);
        let visuals = crate::visuals::VisualCatalog::build(&data, &catalog, vec![Color::WHITE]);
        let flyby = |name: &str| {
            let id = catalog.ship_id(name).unwrap_or_else(|| panic!("{name} in the catalog"));
            let sfx = &visuals.ship(id).sfx;
            (sfx.ambient_moving.clone(), sfx.ambient_moving_delay)
        };
        assert_eq!(flyby("X-Wing"), (Some("Unit_X_Wing_Fly_By".into()), (3.0, 5.0)));
        assert_eq!(flyby("TIE_Fighter"), (Some("Unit_TIE_Fighter_Fly_By".into()), (3.0, 5.0)));
        assert_eq!(flyby("Luke_XWing"), (Some("Unit_X_Wing_Fly_By".into()), (5.0, 10.0)));
        assert_eq!(crate::visuals::ambient_moving_delay(None, None), (3.0, 5.0));
        let audio = AlamoAudio::load(std::sync::Arc::new(fs));
        for event in ["Unit_X_Wing_Fly_By", "Unit_TIE_Fighter_Fly_By"] {
            let e = audio.sfx.get(event).expect("flyby event");
            assert!(!e.looping && !e.samples.is_empty() && e.max_instances == Some(2), "{e:?}");
        }
        assert!(audio.sfx.get("Unit_Shuttle_Moving_Engine_Loop").is_some_and(|e| e.looping), "repeating it would pile up loops");
    }

    /// A battle of one fighter per name and side for the flyby tests: the simulation, its catalog and its first snapshot.
    fn fighter_battle(sides: &[&[&str]]) -> (crate::sim::SimHandle, std::sync::Arc<battle_sim::Catalog>, crate::sim::Snapshot) {
        use battle_sim::{Battle, BattleConfig, BattleSetup, Catalog, ProjDef, ShipDef, SideSetup, UnitSetup};
        let fighter = |name: &&str| ShipDef { name: (*name).into(), class: TargetClass::Fighter, hull: 100.0, radius: 10.0, population: 1, ..ShipDef::default() };
        let defs = sides.iter().flat_map(|names| names.iter().map(fighter)).collect();
        let catalog = std::sync::Arc::new(Catalog::from_defs(defs, vec![ProjDef { name: "laser".into(), ..ProjDef::default() }]));
        let side = |units: &&[&str]| SideSetup {
            name: "side".into(),
            units: units.iter().map(|n| UnitSetup { ship: catalog.ship_id(n).expect("def"), count: 1, tag: 1 }).collect(),
            structures: vec![],
            ai: false,
        };
        let setup = BattleSetup { sides: sides.iter().map(side).collect(), catalog: catalog.clone(), config: BattleConfig::default() };
        let sim = crate::sim::SimHandle::spawn(Battle::new(setup));
        sim.request(1);
        let snap = (*sim.fetch(0, true).0.expect("snapshot")).clone();
        (sim, catalog, snap)
    }

    /// The view of a running battle for the flyby tests.
    fn flyby_runtime(
        (sim, catalog, snap): (crate::sim::SimHandle, std::sync::Arc<battle_sim::Catalog>, crate::sim::Snapshot),
        visuals: crate::visuals::VisualCatalog,
    ) -> crate::BattleRuntime {
        crate::BattleRuntime {
            sim,
            catalog,
            visuals: std::sync::Arc::new(visuals),
            dt: 1.0 / 30.0,
            clock: 0.0,
            cur: Some(std::sync::Arc::new(snap)),
            next: None,
            alpha: 0.0,
            fresh_events: vec![],
            entities: HashMap::new(),
            dying: HashSet::new(),
            request: None,
            player_faction: None,
            start_alive: vec![],
            retreat_countdown: 0.0,
            unit_cap: 0,
            no_spawn_radius: 0.0,
            reinforce_clearance: 0.0,
            arena_half: 10_000.0,
            fog_of_war: true,
            speed: 1.0,
            framed: false,
            reported: false,
            minimap_planets: vec![],
            minimap_props: vec![],
            minimap_tint: Color::BLACK,
        }
    }

    /// The view's catalog with `event` as the ambient-moving sound of the named units, `gaps` apart.
    fn flyby_visuals(catalog: &battle_sim::Catalog, flybys: &[(&str, &str, (f32, f32))]) -> crate::visuals::VisualCatalog {
        let mut visuals = crate::visuals::VisualCatalog { ships: vec![ShipVisual::default(); catalog.ships.len()], ..Default::default() };
        for &(name, event, gaps) in flybys {
            let v = &mut visuals.ships[catalog.ship_id(name).expect("def") as usize].sfx;
            (v.ambient_moving, v.ambient_moving_delay) = (Some(event.into()), gaps);
        }
        visuals
    }

    /// `SFXEventFiles.xml` and `SFXEvents.xml` holding `events`.
    fn sfx_files(events: &str) -> HashMap<String, Vec<u8>> {
        HashMap::from([
            (alamo_fs::normalize("Xml/SFXEventFiles.xml"), b"<SFXEvent_Files><File>SFXEvents.xml</File></SFXEvent_Files>".to_vec()),
            (alamo_fs::normalize("Xml/SFXEvents.xml"), format!("<SFXEvents>{events}</SFXEvents>").into_bytes()),
        ])
    }

    #[test]
    fn flybys_fill_free_voices_loudest_first_and_take_one_only_from_much_quieter_ones() {
        assert_eq!(flybys_to_start(&[0.9, 0.5, 0.1], &[], Some(2)), 2, "the two loudest get the two free voices");
        assert_eq!(flybys_to_start(&[0.9, 0.5], &[0.05], Some(2)), 1, "one voice left");
        assert_eq!(flybys_to_start(&[0.3, 0.2], &[0.5, 0.2], Some(2)), 0, "all taken by louder ones: they wait");
        assert_eq!(flybys_to_start(&[0.39], &[0.2, 0.1], Some(2)), 0, "less than twice as loud as the loudest playing one");
        assert_eq!(flybys_to_start(&[0.4, 0.39], &[0.2, 0.1], Some(2)), 1, "twice as loud as each: the cut voice is 6 dB quieter or more");
        assert_eq!(flybys_to_start(&[0.1, 0.1, 0.1], &[0.5, 0.5, 0.5], None), 3, "no Max_Instances: all start");
    }

    /// With the audio plugin's own voice rules (`Max_Instances` 2, the oldest voice gives way): two far flybys take the two voices, a
    /// unit nearer but not twice as loud waits, a near one takes a far one's voice, and the far one that still waits does not cut it.
    #[test]
    fn a_near_flyby_takes_a_voice_from_far_ones_and_is_not_cut_by_them() {
        let names = ["far_a", "far_b", "near", "middle"];
        let battle = fighter_battle(&[&names]);
        let (catalog, mut snap) = (battle.1.clone(), battle.2.clone());
        for s in &mut snap.ships {
            s.speed = 300.0;
        }
        let visuals = flyby_visuals(&catalog, &names.map(|n| (n, "Unit_Z95_Fly_By", (3.0, 5.0))));
        let mut files = sfx_files(
            r#"<SFXEvent Name="Unit_Z95_Fly_By"><Samples>z95.wav</Samples><Is_3D>Yes</Is_3D><Max_Instances>2</Max_Instances>
            <Volume_Saturation_Distance>60</Volume_Saturation_Distance><Min_Volume>100</Min_Volume><Max_Volume>100</Max_Volume></SFXEvent>"#,
        );
        files.insert(alamo_fs::normalize("Audio/SFX/z95.wav"), alamo_audio::Pcm { channels: 1, sample_rate: 8000, samples: vec![0; 800] }.to_wav_bytes());
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<AudioSource>()
            .add_plugins(alamo_audio::AlamoAudioPlugin::new(std::sync::Arc::new(MemFs(files))))
            .insert_resource(BattleViewSettings::default())
            // Before the plugin's systems (`Update`): a request starts its voice in the same frame.
            .add_systems(PreUpdate, battle_ambient_moving);
        app.world_mut().resource_mut::<alamo_audio::AudioSettings>().load_on_worker = false;
        app.world_mut().spawn((BattleListener, bevy::audio::SpatialListener::new(EAR_GAP), GlobalTransform::IDENTITY));
        // On a line from the listener; a clock runs only when the test makes it due.
        let mut unit = HashMap::new();
        let event = app.world().resource::<AlamoAudio>().sfx.get("Unit_Z95_Fly_By").cloned();
        for (name, x) in names.into_iter().zip([2000.0, 2100.0, 100.0, 1500.0]) {
            let s = snap.ships.iter().find(|s| catalog.ship(s.def).name == name).expect("ship");
            let clock = AmbientMoving { next: Some(f64::MAX), ..AmbientMoving::new(s.id, event.as_ref()) };
            let e = app.world_mut().spawn((crate::ships::ShipView::unloaded(s.id, s.def, 0), GlobalTransform::from_translation(Vec3::X * x), clock)).id();
            unit.insert(name, e);
        }
        app.insert_resource(flyby_runtime((battle.0, catalog, snap), visuals));
        let step = |app: &mut App, t: f64, due: &[&str]| {
            for name in due {
                app.world_mut().get_mut::<AmbientMoving>(unit[name]).expect("clock").next = Some(0.0);
            }
            app.world_mut().resource_mut::<crate::BattleRuntime>().clock = t;
            app.update();
            // Units with a voice of the event, and whether it fades out (cut).
            let mut q = app.world_mut().query::<&alamo_audio::SfxVoice>();
            let mut voices: Vec<(&str, bool)> =
                q.iter(app.world()).map(|v| (names.into_iter().find(|n| v.source() == Some(unit[n])).expect("a flyby voice"), v.is_stopping())).collect();
            voices.sort();
            voices
        };
        let waits = |app: &App, name: &str| app.world().get::<AmbientMoving>(unit[name]).expect("clock").next == Some(0.0);
        assert_eq!(step(&mut app, 10.0, &["far_a"]), [("far_a", false)]);
        assert_eq!(step(&mut app, 10.2, &["far_b"]), [("far_a", false), ("far_b", false)]);
        // 1500 from the listener against 2000: louder, but not twice as loud.
        assert_eq!(step(&mut app, 11.0, &["middle"]), [("far_a", false), ("far_b", false)]);
        assert!(waits(&app, "middle"), "it stays due");
        // 100 away: the plugin cuts the oldest voice (far_a) for it; middle still waits.
        assert_eq!(step(&mut app, 12.0, &["near"]), [("far_a", true), ("far_b", false), ("near", false)]);
        assert!(waits(&app, "middle") && !waits(&app, "near"));
        assert_eq!(step(&mut app, 13.0, &[]).into_iter().filter(|v| !v.1).collect::<Vec<_>>(), [("far_b", false), ("near", false)], "nothing cuts the near one");
        assert!(waits(&app, "middle"));
    }

    #[test]
    fn moving_units_the_player_sees_play_their_flyby_where_they_are() {
        use bevy::ecs::system::RunSystemOnce;
        // The player's X-wing (`Unit_X_Wing_Fly_By`), a shuttle whose ambient-moving event loops, a Y-wing that names none and a standing
        // A-wing; the enemy's TIE fighter in the fog of war.
        let names = ["xwing", "shuttle", "ywing", "awing", "tie"];
        let (sim, catalog, mut snap) = fighter_battle(&[&names[..4], &names[4..]]);
        let id = |name: &str| snap.ships.iter().find(|s| catalog.ship(s.def).name == name).expect("ship").id;
        let ids = names.map(id);
        for s in &mut snap.ships {
            s.speed = if catalog.ship(s.def).name == "awing" { 0.0 } else { 300.0 };
            if s.side == 1 {
                s.seen_by = 0b10;
            }
        }
        let flybys = [("xwing", "Unit_X_Wing_Fly_By", (5.0, 10.0)), ("shuttle", "Shuttle_Loop", (3.0, 5.0)), ("awing", "Unit_X_Wing_Fly_By", (3.0, 5.0)), ("tie", "Unit_TIE_Fighter_Fly_By", (3.0, 5.0))];
        let visuals = flyby_visuals(&catalog, &flybys);
        let files = sfx_files(
            r#"<SFXEvent Name="Unit_X_Wing_Fly_By"><Samples>xwing.wav</Samples><Max_Instances>2</Max_Instances></SFXEvent>
            <SFXEvent Name="Unit_TIE_Fighter_Fly_By"><Samples>tie.wav</Samples></SFXEvent>
            <SFXEvent Name="Shuttle_Loop"><Samples>shuttle.wav</Samples><Play_Count>-1</Play_Count></SFXEvent>"#,
        );
        let mut world = World::new();
        world.insert_resource(BattleViewSettings::default());
        world.insert_resource(AlamoAudio::load(std::sync::Arc::new(MemFs(files))));
        world.init_resource::<Messages<PlaySfx>>();
        let mut place = HashMap::new();
        for (i, (&ship, name)) in ids.iter().zip(names).enumerate() {
            let at = Vec3::new(100.0 * i as f32, 0.0, -50.0);
            let def = catalog.ship_id(name).expect("def");
            let e = world.spawn((crate::ships::ShipView::unloaded(ship, def, (name == "tie").into()), GlobalTransform::from_translation(at))).id();
            place.insert(e, (name, at));
        }
        world.insert_resource(flyby_runtime((sim, catalog, snap), visuals));
        // Two minutes of battle time at 10 updates per second.
        let mut heard: Vec<(f64, &str, String, Vec3)> = Vec::new();
        for step in 0..1200 {
            let t = f64::from(step) / 10.0;
            world.resource_mut::<crate::BattleRuntime>().clock = t;
            world.run_system_once(battle_ambient_moving).expect("flybys");
            for m in world.resource_mut::<Messages<PlaySfx>>().drain() {
                let (name, at) = place[&m.source.expect("the sound sits on the unit")];
                assert_eq!(m.position, Some(at), "{name}");
                heard.push((t, name, m.event, at));
            }
        }
        let of = |name: &str| heard.iter().filter(|h| h.1 == name).map(|h| h.0).collect::<Vec<_>>();
        let xwing = of("xwing");
        assert!((12..=24).contains(&xwing.len()), "120 s at one per 5 to 10 s: {xwing:?}");
        assert!(heard.iter().filter(|h| h.1 == "xwing").all(|h| h.2 == "Unit_X_Wing_Fly_By"));
        assert!(xwing.windows(2).all(|w| (5.0..=10.1).contains(&(w[1] - w[0]))), "{xwing:?}");
        assert!(of("shuttle").is_empty() && of("ywing").is_empty() && of("awing").is_empty() && of("tie").is_empty(), "{heard:?}");
        // Out of the fog the TIE fighter is heard too.
        {
            let mut rt = world.resource_mut::<crate::BattleRuntime>();
            for s in &mut std::sync::Arc::make_mut(rt.cur.as_mut().expect("snapshot")).ships {
                s.seen_by = 0b11;
            }
        }
        for step in 1200..1500 {
            world.resource_mut::<crate::BattleRuntime>().clock = f64::from(step) / 10.0;
            world.run_system_once(battle_ambient_moving).expect("flybys");
            heard.extend(world.resource_mut::<Messages<PlaySfx>>().drain().map(|m| (0.0, place[&m.source.expect("source")].0, m.event, Vec3::ZERO)));
        }
        assert!(heard.iter().any(|h| h.1 == "tie" && h.2 == "Unit_TIE_Fighter_Fly_By"), "{heard:?}");
    }

    #[test]
    fn the_listener_height_comes_from_audio_xml() {
        let mut app = listener_app("<Audio><Audio_Space_3D_Listener_Z_Pullback_Dist> 80 </Audio_Space_3D_Listener_Z_Pullback_Dist></Audio>");
        let focus = Vec3::new(-300.0, 0.0, 450.0);
        for distance in [500.0, 16000.0] {
            let (_, ears) = hear_from(&mut app, focus, distance);
            assert!(ears.translation.distance(focus + Vec3::Y * 80.0) < 1e-3, "zoom {distance}: {}", ears.translation);
        }
    }

    // ---- ability loops and start sounds ----

    use crate::sim::ShipSnap;
    use crate::visuals::AbilitySfx;

    /// An own `isd` with a `TURBO` (no loop) and a `TRACTOR_BEAM`, a `laser` with a `SUPER_LASER` (its shot plays its loop) and a `plain` ship;
    /// the visuals name the loops the Remake names.
    fn ability_world() -> (std::sync::Arc<battle_sim::Catalog>, crate::visuals::VisualCatalog) {
        use battle_sim::{AbilityDef, Catalog, ProjDef, ShipDef, SuperLaserDef};
        let ability = |kind: &str| AbilityDef { kind: kind.into(), ..AbilityDef::default() };
        let ship = |name: &str, abilities: Vec<AbilityDef>| ShipDef { name: name.into(), class: TargetClass::Capital, hull: 1000.0, radius: 100.0, population: 1, abilities, ..ShipDef::default() };
        let catalog = Catalog::from_defs(
            vec![
                ship("isd", vec![ability("TURBO"), ability("TRACTOR_BEAM")]),
                ship("laser", vec![AbilityDef { super_laser: Some(SuperLaserDef::default()), ..ability("SUPER_LASER") }]),
                ship("plain", vec![]),
            ],
            vec![ProjDef { name: "laser".into(), ..ProjDef::default() }],
        );
        let sfx = |kind: &str, run_loop: Option<&str>| AbilitySfx { kind: kind.into(), run_loop: run_loop.map(Into::into), ..AbilitySfx::default() };
        let mut visuals = crate::visuals::VisualCatalog { ships: vec![ShipVisual::default(); 3], ..Default::default() };
        visuals.ships[0].sfx.abilities = vec![sfx("TURBO", None), sfx("TRACTOR_BEAM", Some("Unit_Star_Destroyer_Tractor_Beam"))];
        visuals.ships[1].sfx.abilities = vec![sfx("SUPER_LASER", Some("Unit_Death_Star_2_Fire"))];
        (std::sync::Arc::new(catalog), visuals)
    }

    /// A ship of a snapshot at the origin, with the given `(seconds of effect left, seconds until usable)` per ability.
    fn snap_ship(id: u32, def: ShipId, side: u8, seen_by: u8, abilities: &[(f32, f32)]) -> ShipSnap {
        ShipSnap {
            id,
            def,
            side,
            pos: battle_sim::Vec3::new(0.0, 0.0, 0.0),
            yaw: 0.0,
            speed: 0.0,
            hull: 1000.0,
            shield: 0.0,
            squad: u32::MAX,
            group: 0,
            seen_by,
            order: battle_sim::Order::Idle,
            path: vec![],
            hps: vec![],
            abilities: abilities.to_vec(),
            reserve_squadrons: 0,
            autocast: vec![],
            can_retreat: false,
        }
    }

    #[test]
    fn an_ability_loop_sounds_while_the_ability_runs_on_ships_the_player_sees() {
        let (catalog, visuals) = ability_world();
        let tractor = "Unit_Star_Destroyer_Tractor_Beam";
        let snapshot = Snapshot {
            ships: vec![
                snap_ship(0, 0, 0, 0b01, &[(0.0, 0.0), (8.0, 20.0)]),   // own, tractor beam running
                snap_ship(1, 0, 0, 0b01, &[(5.0, 0.0), (0.0, 12.0)]),   // own, only the turbo runs (no loop in the data)
                snap_ship(2, 0, 1, 0b10, &[(0.0, 0.0), (8.0, 20.0)]),   // enemy in the fog of war
                snap_ship(3, 0, 1, 0b11, &[(0.0, 0.0), (3.0, 20.0)]),   // enemy in sight
                snap_ship(4, 1, 0, 0b01, &[(4.0, 0.0)]),                // own super laser: its shot plays the loop, not this
                snap_ship(5, 2, 0, 0b01, &[]),
            ],
            ..Snapshot::default()
        };
        assert_eq!(running_ability_loops(&snapshot, &catalog, &visuals, 0), [((0, 1), tractor), ((3, 1), tractor)]);
        // Seen from the enemy's side the fog hides the first one: its own ships and the ones it sees count.
        assert_eq!(running_ability_loops(&snapshot, &catalog, &visuals, 1), [((2, 1), tractor), ((3, 1), tractor)]);
    }

    /// Plugin, listener, one own `isd` at 30 units from it and a view of a battle whose snapshot the test edits: frames last 0.1 s.
    fn ability_loop_app(files: HashMap<String, Vec<u8>>) -> (App, Entity) {
        use bevy::time::TimeUpdateStrategy;
        let (catalog, visuals) = ability_world();
        let battle = fighter_battle(&[&["x"]]);
        let snap = Snapshot { ships: vec![snap_ship(7, 0, 0, 0b01, &[(0.0, 0.0), (0.0, 0.0)])], ..Snapshot::default() };
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), bevy::transform::TransformPlugin))
            .init_asset::<AudioSource>()
            .add_plugins(alamo_audio::AlamoAudioPlugin::new(std::sync::Arc::new(MemFs(files))))
            .insert_resource(BattleViewSettings::default())
            .insert_resource(TimeUpdateStrategy::ManualDuration(std::time::Duration::from_millis(100)))
            // Before the plugin's systems (`Update`): a new emitter starts its loop in the same frame.
            .add_systems(PreUpdate, battle_ability_loops);
        app.world_mut().resource_mut::<alamo_audio::AudioSettings>().load_on_worker = false;
        app.world_mut().spawn((BattleListener, bevy::audio::SpatialListener::new(EAR_GAP), Transform::IDENTITY));
        let ship = app.world_mut().spawn((crate::ships::ShipView::unloaded(7, 0, 0), Transform::from_xyz(30.0, 0.0, 0.0))).id();
        let mut rt = flyby_runtime((battle.0, catalog, snap), visuals);
        rt.entities.insert(7, ship);
        app.insert_resource(rt);
        (app, ship)
    }

    /// The ship's ability state as the displayed snapshot says it.
    fn set_abilities(app: &mut App, abilities: &[(f32, f32)]) {
        let mut rt = app.world_mut().resource_mut::<crate::BattleRuntime>();
        std::sync::Arc::make_mut(rt.cur.as_mut().expect("snapshot")).ships[0].abilities = abilities.to_vec();
    }

    /// `(event, fading out)` of the voices that play (the tests play nothing else).
    fn ability_voices(app: &mut App) -> Vec<(String, bool)> {
        let mut q = app.world_mut().query::<&alamo_audio::SfxVoice>();
        q.iter(app.world()).map(|v| (v.event().to_string(), v.is_stopping())).collect()
    }

    fn ability_emitters(app: &mut App) -> usize {
        app.world_mut().query_filtered::<Entity, With<AbilityLoop>>().iter(app.world()).count()
    }

    #[test]
    fn a_tractor_beam_hums_from_its_ship_while_it_runs_and_fades_out_when_it_ends() {
        let mut files = sfx_files(
            r#"<SFXEvent Name="Unit_Star_Destroyer_Tractor_Beam"><Samples>tractor.wav</Samples><Is_3D>Yes</Is_3D><Play_Count>-1</Play_Count><Max_Instances>2</Max_Instances>
            <Volume_Saturation_Distance>100</Volume_Saturation_Distance><Loop_Fade_Out_Seconds>1.0</Loop_Fade_Out_Seconds><Min_Volume>100</Min_Volume><Max_Volume>100</Max_Volume></SFXEvent>"#,
        );
        files.insert(alamo_fs::normalize("Audio/SFX/tractor.wav"), alamo_audio::Pcm { channels: 1, sample_rate: 8000, samples: vec![0; 800] }.to_wav_bytes());
        let (mut app, ship) = ability_loop_app(files);
        let hum = "unit_star_destroyer_tractor_beam".to_string();
        // Only the turbo runs: nothing hums.
        set_abilities(&mut app, &[(5.0, 0.0), (0.0, 0.0)]);
        for _ in 0..3 {
            app.update();
        }
        assert_eq!((ability_emitters(&mut app), ability_voices(&mut app)), (0, vec![]));
        // The beam starts: an emitter on the ship, a voice at the ship.
        set_abilities(&mut app, &[(0.0, 0.0), (12.0, 20.0)]);
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(ability_emitters(&mut app), 1);
        assert_eq!(ability_voices(&mut app), [(hum.clone(), false)], "one loop for the beam");
        let emitter = app.world_mut().query_filtered::<Entity, With<AbilityLoop>>().single(app.world()).expect("emitter");
        assert_eq!(app.world().get::<ChildOf>(emitter).map(ChildOf::parent), Some(ship), "the emitter hangs on the ship");
        let at = app.world().get::<GlobalTransform>(emitter).expect("position").translation();
        assert!(at.distance(Vec3::new(30.0, 0.0, 0.0)) < 1e-3, "the sound sits on the ship: {at}");
        // It keeps humming for as long as the sim says the ability runs.
        for _ in 0..30 {
            app.update();
        }
        assert_eq!(ability_voices(&mut app), [(hum.clone(), false)]);
        // The ability ends: the loop fades out (the event's `Loop_Fade_Out_Seconds` 1.0 s), and the emitter goes with it.
        set_abilities(&mut app, &[(0.0, 0.0), (0.0, 20.0)]);
        app.update();
        app.update();
        assert_eq!(ability_voices(&mut app), [(hum, true)], "fading out");
        for _ in 0..15 {
            app.update();
        }
        assert_eq!((ability_emitters(&mut app), ability_voices(&mut app)), (0, vec![]), "faded out and gone");
        // It can start again.
        set_abilities(&mut app, &[(0.0, 0.0), (12.0, 20.0)]);
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(ability_voices(&mut app).iter().filter(|v| !v.1).count(), 1);
    }

    #[test]
    fn a_ship_that_is_gone_or_hidden_stops_its_ability_loop() {
        let mut files = sfx_files(
            r#"<SFXEvent Name="Unit_Star_Destroyer_Tractor_Beam"><Samples>tractor.wav</Samples><Is_3D>Yes</Is_3D><Play_Count>-1</Play_Count>
            <Volume_Saturation_Distance>100</Volume_Saturation_Distance><Loop_Fade_Out_Seconds>0.5</Loop_Fade_Out_Seconds><Min_Volume>100</Min_Volume><Max_Volume>100</Max_Volume></SFXEvent>"#,
        );
        files.insert(alamo_fs::normalize("Audio/SFX/tractor.wav"), alamo_audio::Pcm { channels: 1, sample_rate: 8000, samples: vec![0; 800] }.to_wav_bytes());
        let (mut app, ship) = ability_loop_app(files);
        set_abilities(&mut app, &[(0.0, 0.0), (12.0, 20.0)]);
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(ability_voices(&mut app).len(), 1);
        // The ship leaves the snapshot (destroyed, retreated): the loop fades out although its ability was still running.
        std::sync::Arc::make_mut(app.world_mut().resource_mut::<crate::BattleRuntime>().cur.as_mut().expect("snapshot")).ships.clear();
        for _ in 0..12 {
            app.update();
        }
        assert_eq!((ability_emitters(&mut app), ability_voices(&mut app)), (0, vec![]));
        // An own ship is always heard; the loop of a ship the player cannot see is silent.
        let mut rt = app.world_mut().resource_mut::<crate::BattleRuntime>();
        std::sync::Arc::make_mut(rt.cur.as_mut().expect("snapshot")).ships.push(snap_ship(7, 0, 1, 0b10, &[(0.0, 0.0), (12.0, 20.0)]));
        for _ in 0..3 {
            app.update();
        }
        assert_eq!((ability_emitters(&mut app), ability_voices(&mut app)), (0, vec![]), "an enemy ship in the fog of war is silent");
        let _ = ship;
    }

    /// The events a ship's ability starts, as `battle_audio` sends them: `(event, position)`.
    fn ability_start_sounds(player_side: u8, ship: ShipSnap, ability: u16) -> Vec<(String, Option<Vec3>)> {
        use bevy::ecs::system::RunSystemOnce;
        let (catalog, mut visuals) = ability_world();
        let sfx = |kind: &str, activated: Option<&str>, start: Option<&str>| AbilitySfx {
            kind: kind.into(),
            activated: activated.map(Into::into),
            start: start.map(Into::into),
            ..AbilitySfx::default()
        };
        // Ability 0 is a `TURBO` with a 3D start sound, ability 1 a `TRACTOR_BEAM` whose start sound is 2D (a GUI line); a third one names the
        // same 3D event as the unit's activated line.
        visuals.ships[0].sfx.abilities =
            vec![sfx("TURBO", None, Some("Start_3D")), sfx("TRACTOR_BEAM", None, Some("Start_2D")), sfx("SPRINT", Some("Start_3D"), Some("Start_3D"))];
        let mut cat = (*catalog).clone();
        cat.ships[0].abilities.push(battle_sim::AbilityDef { kind: "SPRINT".into(), ..Default::default() });
        let files = sfx_files(
            r#"<SFXEvent Name="Start_3D"><Samples>a.wav</Samples><Is_3D>Yes</Is_3D><Volume_Saturation_Distance>100</Volume_Saturation_Distance></SFXEvent>
            <SFXEvent Name="Start_2D"><Samples>a.wav</Samples><Is_2D>Yes</Is_2D></SFXEvent>"#,
        );
        let battle = fighter_battle(&[&["x"]]);
        let snap = Snapshot { ships: vec![ship.clone()], ..Snapshot::default() };
        let mut rt = flyby_runtime((battle.0, std::sync::Arc::new(cat), snap), visuals);
        rt.fresh_events = vec![Event::AbilityActivated { ship: ship.id, ability }];
        let mut world = World::new();
        world.insert_resource(BattleViewSettings { player_side, ..BattleViewSettings::default() });
        world.insert_resource(AlamoAudio::load(std::sync::Arc::new(MemFs(files))));
        world.insert_resource(rt);
        world.init_resource::<Selection>();
        world.init_resource::<BattleAudioStats>();
        world.init_resource::<Messages<PlaySfx>>();
        world.init_resource::<Messages<alamo_audio::plugin::GalaxyAudio>>();
        world.init_resource::<Messages<OrderIssued>>();
        world.init_resource::<Messages<crate::control::HardpointOrderIssued>>();
        world.run_system_once(battle_audio).expect("battle audio");
        world.resource_mut::<Messages<PlaySfx>>().drain().map(|m| (m.event, m.position)).collect()
    }

    /// The Remake's ability sounds through the whole chain (skipped without the game files): the units' data names the loops and start sounds the view
    /// plays, and for the Star Destroyer the real simulation runs the tractor beam for its duration while the loops planned from the snapshots begin with
    /// the ability and end with it.
    #[test]
    fn the_remake_ability_sounds_come_from_the_data_and_the_tractor_beam_hums_exactly_as_long_as_the_sim_runs_it() {
        use battle_sim::{Battle, BattleConfig, BattleSetup, Catalog, Command, SideSetup, UnitSetup};
        let Some(install) = alamo_fs::GameInstall::detect().filter(|i| i.mod_data("1770851727").is_dir()) else { return };
        let fs = alamo_fs::GameFs::for_mod(&install, Some("1770851727")).expect("game files");
        let (data, _) = alamo_data::load_galaxy(&fs);
        let catalog = std::sync::Arc::new(Catalog::build(&data, Some(&fs), &["Star_Destroyer", "Nebulon_B_Frigate", "Accuser_Star_Destroyer", "IG-2000", "Eclipse_Star_Dreadnought"]));
        let visuals = crate::visuals::VisualCatalog::build(&data, &catalog, vec![Color::WHITE; 2]);
        // What the data of other ability types names: the proton beam's loop, IG-88's corrupt systems start sound, the Eclipse's super laser (its shot plays
        // the loop, the planner leaves it out).
        let sound = |ship: &str, kind: &str| {
            let id = catalog.ship_id(ship).unwrap_or_else(|| panic!("{ship} in the catalog"));
            let a = visuals.ship(id).sfx.abilities.iter().find(|a| a.kind.eq_ignore_ascii_case(kind)).unwrap_or_else(|| panic!("{ship} has {kind}"));
            (a.run_loop.clone(), a.start.clone())
        };
        assert_eq!(sound("Accuser_Star_Destroyer", "ENERGY_WEAPON").0.as_deref(), Some("Unit_Star_Destroyer_Energy_Blast"));
        assert_eq!(sound("IG-2000", "CORRUPT_SYSTEMS").1.as_deref(), Some("Unit_IG88_Corrupt_Systems"));
        let eclipse = catalog.ship_id("Eclipse_Star_Dreadnought").expect("def");
        assert!(catalog.ship(eclipse).abilities.iter().any(|a| a.super_laser.is_some()) && sound("Eclipse_Star_Dreadnought", "SUPER_LASER").0.as_deref() == Some("Unit_Death_Star_2_Fire"));
        let isd = catalog.ship_id("Star_Destroyer").expect("Star_Destroyer in the catalog");
        let beam = catalog.ship(isd).abilities.iter().position(|a| a.kind.eq_ignore_ascii_case("TRACTOR_BEAM")).expect("the Star Destroyer has a tractor beam");
        let named = visuals.ship(isd).sfx.abilities.iter().find(|a| a.kind.eq_ignore_ascii_case("TRACTOR_BEAM")).and_then(|a| a.run_loop.as_deref());
        assert_eq!(named, Some("Unit_Star_Destroyer_Tractor_Beam"), "the loop comes from the unit's `Unit_Ability`");
        let side = |name: &str, ship: &str| SideSetup {
            name: name.into(),
            units: vec![UnitSetup { ship: catalog.ship_id(ship).expect("def"), count: 1, tag: 1 }],
            structures: vec![],
            ai: false,
        };
        let config = BattleConfig { record_events: true, fog_of_war: false, ..BattleConfig::default() };
        let mut battle = Battle::new(BattleSetup { catalog: catalog.clone(), sides: vec![side("Empire", "Star_Destroyer"), side("Rebel", "Nebulon_B_Frigate")], config });
        // The frigate 2000 units from the Star Destroyer: inside the beam's reach (`Activation_Max_Range` 3000).
        let ship = battle.ships.iter().find(|s| s.def == isd).expect("the Star Destroyer").id;
        for s in &mut battle.ships {
            s.pos = if s.id == ship { battle_sim::Vec3::new(0.0, 0.0, 0.0) } else { battle_sim::Vec3::new(2000.0, 0.0, 0.0) };
        }
        let sim = crate::sim::SimHandle::spawn(battle);
        sim.command(0, Command::UseAbility { ship, ability: beam as u16 });
        let ticks = (20.0 / sim.dt) as u64;
        sim.request(ticks);
        let mut humming = Vec::new();
        for tick in 1..=ticks {
            let snapshot = sim.fetch(tick, true).0.expect("snapshot");
            let loops = running_ability_loops(&snapshot, &catalog, &visuals, 0);
            assert!(loops.is_empty() || loops == [((ship, beam as u16), "Unit_Star_Destroyer_Tractor_Beam")], "tick {tick}: {loops:?}");
            humming.push(!loops.is_empty());
        }
        let seconds = humming.iter().filter(|h| **h).count() as f32 * sim.dt;
        let duration = catalog.ship(isd).abilities[beam].duration;
        assert!((seconds - duration).abs() < 0.1, "the loop sounds {seconds:.2} s, the sim runs the ability {duration} s");
        assert!(humming[..(duration / sim.dt) as usize - 2].iter().all(|h| *h) && humming[(duration / sim.dt) as usize + 2..].iter().all(|h| !*h), "one run from the start to the end");
        let activated = sim.drain_events(ticks).into_iter().filter(|e| matches!(e, Event::AbilityActivated { ship: s, ability } if *s == ship && *ability == beam as u16)).count();
        assert_eq!(activated, 1, "the sim reports the start");
    }

    #[test]
    fn an_ability_starts_with_its_own_3d_sound_on_the_unit_for_everyone_who_sees_it() {
        let at = |s: ShipSnap| Some(crate::convert::sim_to_bevy(s.pos));
        let own = snap_ship(3, 0, 0, 0b01, &[]);
        let sounds = ability_start_sounds(0, own.clone(), 0);
        assert_eq!(sounds, [("Start_3D".to_string(), at(own))], "own ship: the 3D start sound at the unit");
        let seen_enemy = snap_ship(3, 0, 1, 0b11, &[]);
        assert_eq!(ability_start_sounds(0, seen_enemy.clone(), 0), [("Start_3D".to_string(), at(seen_enemy))], "an enemy the player sees is heard too");
        assert!(ability_start_sounds(0, snap_ship(3, 0, 1, 0b10, &[]), 0).is_empty(), "an enemy in the fog of war is silent");
        assert!(ability_start_sounds(0, snap_ship(3, 0, 0, 0b01, &[]), 1).is_empty(), "a 2D start sound is a GUI or voice line: the unit's own hooks cover it");
        // The unit's activated line is the same event: it plays once (the line, as before), not twice.
        let sounds = ability_start_sounds(0, snap_ship(3, 0, 0, 0b01, &[]), 2);
        assert_eq!(sounds.iter().filter(|s| s.0 == "Start_3D").count(), 1, "{sounds:?}");
    }
}

/// The classic voice set (`Empire`, `Rebel`, `Underworld`) a faction speaks with. The Clone Wars factions have no own `Unit_Select_*` /
/// `Unit_Move_*` sets in the unit-response data, so they use the set of the faction icon their `FACTIONS.XML` entry names:
/// Republic `i_icon_rebel.tga` (Rebel), CIS `i_icon_pirate.tga` (Underworld).
fn voice_faction(name: &str) -> Option<&'static str> {
    match name.trim().to_ascii_lowercase().as_str() {
        "empire" => Some("Empire"),
        "rebel" | "republic" => Some("Rebel"),
        "underworld" | "cis" => Some("Underworld"),
        _ => None,
    }
}

#[cfg(test)]
mod countdown_tests {
    use super::*;

    #[test]
    fn countdown_announcements() {
        assert_eq!(countdown_calls(None, 600.0), ["moving_into_range"]);
        assert!(countdown_calls(Some(600.0), 599.0).is_empty());
        assert_eq!(countdown_calls(Some(61.0), 59.9), ["moving_range_60_seconds"]);
        assert_eq!(countdown_calls(Some(5.5), 4.0), ["moving_range_05_seconds"]);
        assert_eq!(countdown_calls(Some(0.02), 0.0), ["moved_into_range"]);
        assert!(countdown_calls(Some(0.0), 0.0).is_empty());
    }
}
