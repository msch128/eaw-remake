//! Battle sound hooks: weapon fire, impacts, explosions, ability and unit-response events are
//! sent as [`alamo_audio::PlaySfx`] messages.
//!
//! The view never plays anything itself: without `AlamoAudioPlugin` the messages are consumed
//! by Bevy's message cleanup and nothing is audible, which keeps tests and offscreen runs silent.
//! [`BattleViewSettings::audio`] switches the hooks off completely.

use std::collections::HashMap;

use alamo_audio::{MusicDirector, MusicScene, PlaySfx};
use battle_sim::{Event, HitKind};
use bevy::prelude::*;

use crate::control::{OrderIssued, OrderKind, Selection};
use crate::convert::sim_to_bevy;
use crate::{BattleRuntime, BattleStatus, BattleViewSettings};

/// Most sounds of one event name per frame; the rest would only pile up volume.
const PER_NAME_PER_FRAME: u32 = 2;
/// Most battle sounds per frame.
const PER_FRAME: u32 = 24;

/// Counters of what the hooks sent (for tests and diagnostics).
#[derive(Resource, Debug, Default, Clone)]
pub struct BattleAudioStats {
    /// Messages written so far.
    pub sent: u64,
    /// Last event names, newest last (at most 32).
    pub recent: Vec<String>,
}

/// Translates the events of the displayed steps and the player's input into sound requests.
#[allow(clippy::too_many_arguments)]
pub fn battle_audio(
    settings: Res<BattleViewSettings>,
    rt: Res<BattleRuntime>,
    sel: Res<Selection>,
    mut orders: MessageReader<OrderIssued>,
    mut out: MessageWriter<PlaySfx>,
    mut stats: ResMut<BattleAudioStats>,
    mut voice: MessageWriter<alamo_audio::plugin::GalaxyAudio>,
    mut last_selection: Local<Vec<u32>>,
    mut defs: Local<HashMap<u32, battle_sim::ShipId>>,
) {
    if !settings.audio {
        orders.clear();
        return;
    }
    let Some(cur) = rt.cur.clone() else { return };
    let visuals = &rt.visuals;
    defs.extend(cur.ships.iter().map(|s| (s.id, s.def)));
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

    for ev in &rt.fresh_events {
        match ev {
            Event::ProjectileFired { ship, hardpoint, from, .. } => {
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
            Event::Hit { projectile, pos, kind, .. } => {
                // Shield hits are a separate effect in the game; only hull and hardpoint hits detonate audibly.
                if matches!(kind, HitKind::Shield) {
                    continue;
                }
                if let Some(Some(n)) = visuals.proj_detonate.get(*projectile as usize) {
                    send(n, Some(sim_to_bevy(*pos)), &mut stats, &mut out);
                }
            }
            Event::ShipDestroyed { ship, pos } => {
                if let Some(n) = defs.get(ship).and_then(|d| visuals.ship(*d).sfx.death.as_deref()) {
                    send(n, Some(sim_to_bevy(*pos)), &mut stats, &mut out);
                }
            }
            Event::HardpointDestroyed { ship, .. } => {
                if let Some(s) = cur.ship(*ship)
                    && let Some(n) = visuals.ship(s.def).sfx.hardpoint_destroyed.as_deref()
                {
                    send(n, Some(sim_to_bevy(s.pos)), &mut stats, &mut out);
                }
            }
            Event::ReinforcementsCalled { side, .. } if *side == settings.player_side => {
                if let Some(f) = &rt.player_faction {
                    voice.write(alamo_audio::plugin::GalaxyAudio { faction: f.clone(), event: alamo_audio::GalaxyEvent::ReinforcementsEnRoute });
                }
            }
            Event::AbilityActivated { ship, .. } => {
                if let Some(s) = cur.ship(*ship)
                    && s.side == settings.player_side
                    && let Some(n) = visuals.ship(s.def).sfx.ability.as_deref()
                {
                    send(n, None, &mut stats, &mut out);
                }
            }
            _ => {}
        }
    }

    // Unit responses of the player's own units.
    if sel.is_changed() && sel.ships != *last_selection {
        *last_selection = sel.ships.clone();
        if let Some(n) = sel.ships.first().and_then(|id| cur.ship(*id)).and_then(|s| visuals.ship(s.def).sfx.select.as_deref()) {
            send(n, None, &mut stats, &mut out);
        }
    }
    for o in orders.read() {
        let Some(s) = cur.ship(o.lead) else { continue };
        let v = &visuals.ship(s.def).sfx;
        let name = match o.kind {
            OrderKind::Move => v.mv.as_deref(),
            OrderKind::Attack => v.attack.as_deref(),
            OrderKind::Stop => None,
        };
        if let Some(n) = name {
            send(n, None, &mut stats, &mut out);
        }
    }
}

/// Starts the space music scene for the player's faction and scales the intensity with the share of ships still fighting.
pub fn battle_music(
    settings: Res<BattleViewSettings>,
    status: Res<BattleStatus>,
    rt: Res<BattleRuntime>,
    director: Option<ResMut<MusicDirector>>,
) {
    let Some(mut director) = director else { return };
    if !settings.audio {
        return;
    }
    if let Some(f) = &rt.player_faction {
        director.set_scene(MusicScene::Space(f.clone()));
    }
    let total: u32 = status.alive.iter().sum();
    let start = rt.start_alive.iter().sum::<u32>().max(1);
    // Everything alive and shooting is intense; it eases when ships die off.
    director.set_battle_intensity(if status.over || total == 0 { 0.0 } else { (total as f32 / start as f32).clamp(0.0, 1.0) });
}
