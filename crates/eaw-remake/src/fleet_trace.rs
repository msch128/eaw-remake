//! `SP_FLEET_TRACE=<speed level>`: a diagnostic of how smoothly fleets fly on the galaxy map. Once the scripted orders are done (e.g.
//! `--move`), the galaxy runs at that speed level (1 = 1x .. 4 = 8x) for `SP_FLEET_TRACE_SECONDS` real seconds (default 6) while the
//! drawn position of the player's moving fleets is measured every frame; the capture waits for it. Prints one `fleet-trace:` line:
//! frames that did not move, the largest per-frame speed against the mean, and how often snapshots and galactic time arrive
//! (`SP_FLEET_TRACE_FRAMES=1` logs every frame too).

use std::collections::HashMap;

use bevy::prelude::*;
use galaxy_play::{FleetVisual, GalaxyLayout, PlayAction, PlayerFaction, SimState};
use galaxy_sim::FleetLocation;

use crate::play::ScriptHold;

/// Measures fleet motion when `SP_FLEET_TRACE` is set.
pub fn plugin(app: &mut App) {
    let Some(level) = std::env::var("SP_FLEET_TRACE").ok().and_then(|v| v.parse::<usize>().ok()) else { return };
    let seconds = std::env::var("SP_FLEET_TRACE_SECONDS").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(6.0);
    app.insert_resource(Trace { level, seconds, frames: std::env::var_os("SP_FLEET_TRACE_FRAMES").is_some(), ..default() })
        .add_systems(PostUpdate, trace.run_if(resource_exists::<SimState>).run_if(resource_exists::<GalaxyLayout>).before(bevy::transform::TransformSystems::Propagate));
}

#[derive(Resource, Default)]
struct Trace {
    level: usize,
    seconds: f64,
    /// `SP_FLEET_TRACE_FRAMES`: also log every measured frame.
    frames: bool,
    started: Option<f64>,
    done: bool,
    last: HashMap<u32, Vec3>,
    /// Map units per real second of every measured frame.
    speeds: Vec<f32>,
    zero_frames: u32,
    run: u32,
    longest_run: u32,
    seq: u64,
    snaps: Vec<f64>,
    galaxy_time: f64,
    time_steps: Vec<f64>,
}

fn trace(
    time: Res<Time>,
    mut tr: ResMut<Trace>,
    mut hold: ResMut<ScriptHold>,
    state: Res<SimState>,
    player: Res<PlayerFaction>,
    mut actions: MessageWriter<PlayAction>,
    fleets: Query<(&FleetVisual, &Transform)>,
    layout: Res<GalaxyLayout>,
) {
    if tr.done {
        return;
    }
    let now = time.elapsed_secs_f64();
    let Some(started) = tr.started else {
        // The scripted orders are done: run the clock and keep the capture waiting.
        if hold.done_at.is_some() && !hold.active {
            actions.write(PlayAction::SetSpeedLevel(tr.level));
            hold.active = true;
            tr.started = Some(now);
            tr.seq = state.current.seq;
            tr.galaxy_time = state.current.time;
        }
        return;
    };
    if state.current.seq != tr.seq {
        tr.seq = state.current.seq;
        tr.snaps.push(now);
    }
    if state.current.time != tr.galaxy_time {
        tr.galaxy_time = state.current.time;
        tr.time_steps.push(now);
    }
    let dt = time.delta_secs();
    let g = state.galaxy();
    let mut moved_any = None;
    for (v, tf) in &fleets {
        let pos = Vec3::new(tf.translation.x, 0.0, tf.translation.z);
        // On the way: the simulation has it moving and it is not drawn at its destination yet (the map gets there up to a tick
        // before the simulation does).
        let moving = g.fleets.get(&v.id).filter(|f| Some(f.owner) == player.0).is_some_and(|f| match &f.location {
            FleetLocation::Moving { to, rest, .. } => {
                let end = layout.positions[*rest.last().unwrap_or(to) as usize];
                Vec3::new(end.x, 0.0, end.z).distance(pos) > 0.01 * layout.unit
            }
            FleetLocation::AtPlanet(_) => false,
        });
        let prev = tr.last.insert(v.id, pos);
        if !moving || dt <= 0.0 {
            continue;
        }
        let Some(prev) = prev else { continue };
        let step = pos.distance(prev);
        if tr.frames {
            info!("fleet-trace frame: t {:.3} dt {:.4} fleet {} step {step:.4} speed {:.3} at ({:.2}, {:.2}) snapshot {:.0}", now - started, dt, v.id, step / dt, pos.x, pos.z, g.time);
        }
        tr.speeds.push(step / dt);
        moved_any = Some(moved_any.unwrap_or(false) || step > 1e-5);
    }
    match moved_any {
        Some(false) => {
            tr.zero_frames += 1;
            tr.run += 1;
            tr.longest_run = tr.longest_run.max(tr.run);
        }
        Some(true) => tr.run = 0,
        None => {}
    }
    if now - started < tr.seconds {
        return;
    }
    tr.done = true;
    hold.active = false;
    hold.done_at = Some(std::time::Instant::now());
    let n = tr.speeds.len().max(1) as f32;
    let mean = tr.speeds.iter().sum::<f32>() / n;
    let max = tr.speeds.iter().copied().fold(0.0, f32::max);
    let gaps = |v: &[f64]| {
        let d: Vec<f64> = v.windows(2).map(|w| (w[1] - w[0]) * 1000.0).collect();
        let mean = d.iter().sum::<f64>() / d.len().max(1) as f64;
        (mean, d.iter().copied().fold(0.0, f64::max))
    };
    let (snap_mean, snap_max) = gaps(&tr.snaps);
    let (step_mean, step_max) = gaps(&tr.time_steps);
    info!(
        "fleet-trace: level {} over {:.1} s: {} fleet frames, zero-move frames {} (longest run {}), speed mean {mean:.3} max {max:.3} u/s (max/mean {:.2}); snapshots every {snap_mean:.1} ms (max {snap_max:.1}), galactic time changes every {step_mean:.1} ms (max {step_max:.1})",
        tr.level,
        now - started,
        tr.speeds.len(),
        tr.zero_frames,
        tr.longest_run,
        if mean > 0.0 { max / mean } else { 0.0 },
    );
}
