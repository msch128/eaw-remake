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
}

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
                        if def.class == TargetClass::Structure {
                            // `Delay_Between_Space_Base_Attack_Announcement_Seconds` (Audio.xml).
                            if mem.last_station_alarm.is_none_or(|t| clock - t >= f64::from(space_base_attack_announcement_delay)) {
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
                // Shield hits are a separate effect in the game; only hull and hardpoint hits detonate audibly.
                if matches!(kind, HitKind::Shield) {
                    continue;
                }
                if let Some(Some(n)) = visuals.proj_detonate.get(*projectile as usize) {
                    send(n, Some(sim_to_bevy(*pos)), &mut stats, &mut out);
                }
            }
            Event::ShipDestroyed { ship, pos } => {
                mem.health_flags.remove(ship);
                let Some(&(def, side)) = mem.ships.get(ship) else { continue };
                let vis = visuals.ship(def);
                if let Some(n) = vis.sfx.death.as_deref() {
                    send(n, Some(sim_to_bevy(*pos)), &mut stats, &mut out);
                }
                let class = rt.catalog.ship(def).class;
                if is_small(class) {
                    continue;
                }
                let structure = class == TargetClass::Structure;
                if side == player {
                    if let Some(n) = vis.sfx.unit_lost.as_deref() {
                        send(n, None, &mut stats, &mut out);
                    }
                    say(
                        match (vis.hero, structure) {
                            (true, _) => GalaxyEvent::HeroLost,
                            (_, true) => GalaxyEvent::StructureDestroyed,
                            _ => GalaxyEvent::UnitDestroyed,
                        },
                        &mut stats,
                    );
                } else {
                    say(
                        match (vis.hero, structure) {
                            (true, _) => GalaxyEvent::HeroEliminated,
                            (_, true) => GalaxyEvent::EnemyStructureDestroyed,
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
            Event::AbilityActivated { ship, .. } => {
                if let Some(s) = cur.ship(*ship)
                    && s.side == player
                    && let Some(n) = visuals.ship(s.def).sfx.ability.as_deref()
                {
                    send(n, None, &mut stats, &mut out);
                }
            }
            _ => {}
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

    // Unit responses of the player's own units.
    if sel.is_changed() && sel.ships != mem.last_selection {
        mem.last_selection = sel.ships.clone();
        if let Some(n) = sel.ships.first().and_then(|id| cur.ship(*id)).and_then(|s| visuals.ship(s.def).sfx.select.as_deref()) {
            send(n, None, &mut stats, &mut out);
        }
    }
    // An attack on one hardpoint: the unit's `SFXEvent_Attack_Hardpoint` line of that hardpoint's type ("Instructions on taking out those laser cannons!"),
    // instead of its plain attack line.
    let mut hardpoint_line = false;
    for o in hardpoint_orders.read() {
        let (Some(me), Some(target)) = (cur.ship(o.lead), cur.ship(o.target)) else { continue };
        let ty = visuals.ship(target.def).hardpoints.get(o.hardpoint as usize).map_or("", |h| h.hp_type.as_str());
        let line = visuals.ship(me.def).sfx.attack_hardpoint_by_type.iter().find(|(t, _)| t == ty).map(|(_, e)| e.as_str());
        if let Some(n) = line {
            send(n, None, &mut stats, &mut out);
            hardpoint_line = true;
        }
    }
    for o in orders.read() {
        let Some(s) = cur.ship(o.lead) else { continue };
        let v = &visuals.ship(s.def).sfx;
        // `SFXEvent_Fleet_Move` answers a move order given to a group (estimate: the data does not say when the
        // engine prefers it; units without a fleet line use their single-unit move line).
        let group = sel.ships.len() > 1;
        let name = match o.kind {
            OrderKind::Move if group => v.fleet_move.as_deref().or(v.mv.as_deref()),
            OrderKind::Move => v.mv.as_deref().or(v.fleet_move.as_deref()),
            OrderKind::Attack if hardpoint_line => None,
            OrderKind::Attack => v.attack.as_deref(),
            OrderKind::Guard => v.guard.as_deref().or(v.stop.as_deref()),
            OrderKind::Stop => v.stop.as_deref(),
        };
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
    // `Audio_Space_3D_Listener_Z_Pullback_Dist` = 0 (Audio.xml): the listener is not lifted above the map, it stands on the
    // battle plane at the point the camera looks at (not at the camera's eye), turned like the camera for the left / right pan.
    if let Ok((cam, tf)) = cameras.single() {
        match listener.single_mut() {
            Ok(mut l) => {
                l.translation = cam.focus;
                l.rotation = tf.rotation;
            }
            Err(_) => {
                commands.spawn((
                    Name::new("battle sound listener"),
                    crate::BattleEntity,
                    BattleListener,
                    bevy::audio::SpatialListener::new(EAR_GAP),
                    Transform { translation: cam.focus, rotation: tf.rotation, ..default() },
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
}
