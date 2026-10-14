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
                if let Some(s) = cur.ship(*ship)
                    && s.side == player
                {
                    let vis = visuals.ship(s.def);
                    let kind = rt.catalog.ship(s.def).abilities.get(*ability as usize).map(|a| a.kind.to_ascii_uppercase()).unwrap_or_default();
                    let own = vis.sfx.abilities.iter().find(|a| a.kind == kind);
                    let line = own.and_then(|a| a.activated.as_deref()).or(vis.sfx.ability.as_deref())
                        .or_else(|| if vis.hero { vis.sfx.attack.as_deref() } else { None });
                    if let Some(n) = line {
                        send(n, None, &mut stats, &mut out);
                    }
                    // A non-hero ability switched on also plays the faction's GUI toggle sound (`SFXEvent_GUI_Toggle_Non_Hero_Ability_On`).
                    if !vis.hero && let Some(n) = faction.as_deref().and_then(|f| constants.toggle_sfx(f, &kind, true)) {
                        send(n, None, &mut stats, &mut out);
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

/// Starts the space music scene for the player's faction. Battle music plays while shots are being
/// exchanged and ends after `Music_Space_Battle_To_Ambient_Peace_Seconds` (Audio.xml) without any.
pub fn battle_music(
    settings: Res<BattleViewSettings>,
    status: Res<BattleStatus>,
    rt: Res<BattleRuntime>,
    stats: Res<BattleAudioStats>,
    director: Option<ResMut<MusicDirector>>,
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
    let fighting = stats.last_combat.is_some_and(|t| rt.clock - t <= COMBAT_WINDOW);
    director.set_battle_intensity(if fighting { 1.0 } else { 0.0 });
}

/// Sound sources of the battle scene: the listener sits where the battle camera looks (distance falloff and left / right
/// pan of every positioned sound are relative to it) and every ship hums its engine loop: `SFXEvent_Engine_Moving_Loop`
/// while it moves, `SFXEvent_Engine_Idle_Loop` while it stands, `SFXEvent_Ambient_Loop` for units that have no engine
/// loop of the current kind. Loops of dead, retreated or unseen ships are silent. Does nothing without `AlamoAudioPlugin`.
#[allow(clippy::type_complexity)]
pub fn battle_sources(
    mut commands: Commands,
    settings: Res<BattleViewSettings>,
    rt: Res<BattleRuntime>,
    audio: Option<Res<AlamoAudio>>,
    cameras: Query<(&crate::camera::BattleCamera, &Transform), Without<BattleListener>>,
    mut listener: Query<&mut Transform, With<BattleListener>>,
    mut ships: Query<(Entity, &crate::ships::ShipView, Option<&mut alamo_audio::SfxEmitter>)>,
) {
    if !settings.audio || audio.is_none() {
        return;
    }
    if let Ok((cam, tf)) = cameras.single() {
        // Acoustic 3D perspective:
        // Close camera (< 500m) places the listener right at the focus point on the battle plane
        // for full bass and direct proximity; pulling back to high altitude (> 2000m) lifts and
        // pulls the listener towards the camera eye to attenuate distant battle events realistically.
        let pullback_frac = ((cam.distance - 500.0) / 4000.0).clamp(0.0, 0.45);
        let listener_pos = cam.focus.lerp(tf.translation, pullback_frac);
        match listener.single_mut() {
            Ok(mut l) => {
                l.translation = listener_pos;
                l.rotation = tf.rotation;
            }
            Err(_) => {
                commands.spawn((
                    Name::new("battle sound listener"),
                    crate::BattleEntity,
                    BattleListener,
                    bevy::audio::SpatialListener::new(EAR_GAP),
                    Transform { translation: listener_pos, rotation: tf.rotation, ..default() },
                ));
            }
        }
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

/// The sound listener of the battle (see [`battle_sources`]).
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

    #[test]
    fn test_spatial_listener_acoustic_perspective() {
        let focus = Vec3::new(100.0, 0.0, 200.0);
        let cam_eye = Vec3::new(100.0, 1500.0, 3000.0);

        // Near camera zoom (< 500m): listener stays exactly at focus for full bass
        let dist_near: f32 = 400.0;
        let pullback_near = ((dist_near - 500.0) / 4000.0).clamp(0.0, 0.45);
        assert_eq!(pullback_near, 0.0);
        let pos_near = focus.lerp(cam_eye, pullback_near);
        assert_eq!(pos_near, focus);

        // Mid-distance zoom (2500m)
        let dist_mid: f32 = 2500.0;
        let pullback_mid = ((dist_mid - 500.0) / 4000.0).clamp(0.0, 0.45);
        assert!(pullback_mid > 0.4 && pullback_mid < 0.55);
        let pos_mid = focus.lerp(cam_eye, pullback_mid);
        assert!(pos_mid.y > 0.0, "listener is lifted and pulled back towards eye");

        // Distant camera zoom (> 4000m): clamped to maximum pullback of 0.45
        let dist_far: f32 = 8000.0;
        let pullback_far = ((dist_far - 500.0) / 4000.0).clamp(0.0, 0.45);
        assert_eq!(pullback_far, 0.45);
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
