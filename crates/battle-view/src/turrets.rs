//! Turrets turn toward what their hardpoint is shooting at (the Remake 4.0 hallmark: turbolaser turrets track their target).
//!
//! The ship model carries the unit's hardpoints (`AlamoModel::with_unit`): every hardpoint with a `Model_To_Attach` is a turret model hung on the hull
//! bone, with a [`TurretAim`] built from the hardpoint XML (`Turret_Bone_Name`, `Barrel_Bone_Name`, `Turret_Rotate_Speed`, `Turret_Rotate_Extent_Degrees`,
//! `Turret_Elevate_Extent_Degrees`). `alamo-render` yaws the turret bone and pitches the barrel bone inside those limits at that speed; this system only
//! tells each turret where its target is, from the simulation's per-hardpoint target ([`crate::sim::HpSnap::target`]).

use alamo_render::{AlamoHardpoint, AlamoModelLoaded, TurretAim};
use bevy::prelude::*;

use crate::BattleRuntime;
use crate::ships::{RenderedShips, ShipView};

/// World point a hardpoint of a ship aims at: the hardpoint of the target being aimed at if known, else the target ship's centre.
pub(crate) fn aim_point(target: &crate::ships::ShipRender, target_hp: u16) -> Vec3 {
    target.hp_pos.get(target_hp as usize).copied().flatten().unwrap_or(target.pos)
}

/// Points every turret at the target of its hardpoint.
pub(crate) fn aim_ship_turrets(
    rt: Res<BattleRuntime>,
    rendered: Res<RenderedShips>,
    ships: Query<(&ShipView, &AlamoModelLoaded)>,
    hardpoints: Query<&AlamoHardpoint>,
    mut turrets: Query<(&mut TurretAim, Option<&AlamoModelLoaded>)>,
) {
    let Some(cur) = rt.cur.as_ref() else { return };
    // `SP_TURRET_LOG=1`: a line per second for the first hardpoints of the first ships (offscreen checks).
    let log = std::env::var_os("SP_TURRET_LOG").is_some() && (rt.clock.fract() < 0.03);
    let (mut with_target, mut matched, mut turret_models) = (0, 0, 0);
    let mut sample = 0;
    for (view, loaded) in &ships {
        let Some(snap) = cur.ship(view.id) else { continue };
        if log && view.id < 3 {
            info!("turret-log ship {} hps {} alive {} first target {:?}", view.id, snap.hps.len(), snap.hps.iter().filter(|h| h.alive).count(), snap.hps.first().map(|h| h.target));
        }
        let def = rt.catalog.ship(view.def);
        for (i, hp) in snap.hps.iter().enumerate() {
            if !hp.alive || hp.target == battle_sim::NONE {
                continue;
            }
            with_target += 1;
            let Some(target) = rendered.0.get(&hp.target) else { continue };
            let Some(name) = def.hardpoints.get(i).map(|d| d.name.as_str()) else { continue };
            // The sim's hardpoint and the model's hardpoint entity are matched by object name.
            let Some(turret) = loaded
                .hardpoints
                .iter()
                .filter_map(|&e| hardpoints.get(e).ok())
                .find(|h| h.name.eq_ignore_ascii_case(name) && !h.destroyed)
                .and_then(|h| h.turret)
            else {
                continue;
            };
            matched += 1;
            if let Ok((mut aim, tl)) = turrets.get_mut(turret) {
                aim.active = true;
                aim.target = aim_point(target, hp.target_hp);
                let yaw_key = aim.yaw_bone.trim().to_ascii_lowercase();
                let has_bones = tl.is_some_and(|l| l.bone_by_name.contains_key(&yaw_key) || ["turret", "b_turret", "barrel"].iter().any(|k| l.bone_by_name.contains_key(*k)));
                if log && has_bones && view.id < 3 && sample < 2 {
                    sample += 1;
                    info!("turret-log t={:.1} ship {} at {:?} {} yaw {:.2} pitch {:.2} -> ship {} at {:?}", rt.clock, view.id, rendered.0.get(&view.id).map(|r| r.pos), name, aim.yaw, aim.pitch, hp.target, target.pos);
                }
            }
        }
    }
    if log {
        turret_models += turrets.iter().count();
        info!("turret-log t={:.1}: {} hardpoints with a target, {} matched a turret model, {} turret models", rt.clock, with_target, matched, turret_models);
    }
}

pub(crate) fn plugin(app: &mut App) {
    app.add_systems(Update, aim_ship_turrets.run_if(resource_exists::<BattleRuntime>));
}
