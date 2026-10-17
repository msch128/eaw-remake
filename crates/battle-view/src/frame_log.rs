//! Debug aid (`SP_FRAME_LOG=<ms>`, default 33): logs every battle frame whose main-thread work or whole duration exceeds
//! the limit, with what the frame did (new ships, models and particle systems, the main-thread loads noted in
//! [`alamo_render::frame_trace`]) and what the render thread did meanwhile (its longest frame, render pipelines created).

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::time::Instant;

use alamo_render::{AlamoModel, AlamoParticles};
use bevy::prelude::*;
use bevy::render::render_resource::PipelineCache;
use bevy::render::{Render, RenderApp, RenderSystems};

use crate::ships::ShipView;
use crate::{BattleRuntime, BattleStatus};

/// Numbers the render world publishes for the main world.
#[derive(Resource, Clone, Default)]
struct RenderStats {
    /// Longest render-world frame (CPU, microseconds) since the main world last read it.
    worst_us: Arc<AtomicU32>,
    /// Render pipelines known to the pipeline cache.
    pipelines: Arc<AtomicUsize>,
    /// Pipelines queued or still compiling.
    waiting: Arc<AtomicUsize>,
}

#[derive(Resource)]
struct RenderFrameStart(Instant);

#[derive(Resource)]
struct FrameLog {
    limit: f32,
    started: Instant,
    pipelines: usize,
}

/// Adds the logger when `SP_FRAME_LOG` is set.
pub(crate) fn plugin(app: &mut App) {
    let Some(limit) = std::env::var("SP_FRAME_LOG").ok().map(|v| v.parse::<f32>().unwrap_or(33.0)) else { return };
    let stats = RenderStats::default();
    app.insert_resource(FrameLog { limit, started: Instant::now(), pipelines: 0 })
        .insert_resource(stats.clone())
        .add_systems(First, frame_start.after(bevy::time::TimeSystems))
        .add_systems(Last, log_frame_spikes);
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render
            .insert_resource(stats)
            .insert_resource(RenderFrameStart(Instant::now()))
            .add_systems(Render, (render_frame_start.in_set(RenderSystems::ExtractCommands), render_frame_end.in_set(RenderSystems::PostCleanup)));
    }
}

fn frame_start(mut log: ResMut<FrameLog>) {
    log.started = Instant::now();
}

fn render_frame_start(mut start: ResMut<RenderFrameStart>) {
    start.0 = Instant::now();
}

fn render_frame_end(start: Res<RenderFrameStart>, stats: Res<RenderStats>, cache: Res<PipelineCache>) {
    let us = start.0.elapsed().as_micros().min(u32::MAX as u128) as u32;
    stats.worst_us.fetch_max(us, Ordering::Relaxed);
    stats.pipelines.store(cache.pipelines().count(), Ordering::Relaxed);
    stats.waiting.store(cache.waiting_pipelines().count(), Ordering::Relaxed);
}

#[allow(clippy::too_many_arguments)]
fn log_frame_spikes(
    time: Res<Time<Real>>,
    status: Res<BattleStatus>,
    rt: Option<Res<BattleRuntime>>,
    stats: Res<RenderStats>,
    mut log: ResMut<FrameLog>,
    ships: Query<(), Added<ShipView>>,
    models: Query<(), Added<AlamoModel>>,
    particles: Query<(), Added<AlamoParticles>>,
) {
    let notes = alamo_render::frame_trace::take();
    let render_ms = stats.worst_us.swap(0, Ordering::Relaxed) as f32 / 1000.0;
    let pipelines = stats.pipelines.load(Ordering::Relaxed);
    let new_pipelines = pipelines.saturating_sub(log.pipelines);
    log.pipelines = pipelines;
    if rt.is_none() {
        return;
    }
    let work = log.started.elapsed().as_secs_f32() * 1000.0;
    let frame = time.delta_secs() * 1000.0;
    if work < log.limit && frame < log.limit {
        return;
    }
    info!(
        "frame spike: {frame:.0} ms since the last frame, main thread {work:.0} ms in this one, render thread up to {render_ms:.0} ms | battle t={:.1}, last sim step {:.1} ms | {} new ships, {} new models, {} new particle systems, {new_pipelines} new render pipelines ({} compiling) | {}",
        status.time,
        status.step_ms,
        ships.iter().count(),
        models.iter().count(),
        particles.iter().count(),
        stats.waiting.load(Ordering::Relaxed),
        notes.join("; ")
    );
}
