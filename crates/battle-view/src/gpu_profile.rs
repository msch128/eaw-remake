//! GPU / CPU time per render pass: wgpu timestamp queries through Bevy's `RenderDiagnosticsPlugin`.
//!
//! Wall-clock frame times are noisy on a shared machine; per-pass GPU times are not. Build the app
//! with [`timestamp_render_plugin`] in place of the default `RenderPlugin`
//! (`DefaultPlugins.set(timestamp_render_plugin())`), add [`GpuProfilePlugin`], and print
//! [`GpuProfile::summary`] at the end of the run. Timestamp queries need Vulkan or DX12; elsewhere
//! only CPU times are recorded.

use std::collections::HashMap;

use bevy::diagnostic::DiagnosticsStore;
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::diagnostic::RenderDiagnosticsPlugin;
use bevy::render::settings::{RenderCreation, WgpuFeatures, WgpuSettings};

/// A `RenderPlugin` that asks for the timestamp query features (ignored by adapters without them).
pub fn timestamp_render_plugin() -> RenderPlugin {
    RenderPlugin {
        render_creation: RenderCreation::Automatic(WgpuSettings {
            features: WgpuFeatures::TIMESTAMP_QUERY
                | WgpuFeatures::TIMESTAMP_QUERY_INSIDE_ENCODERS
                | WgpuFeatures::TIMESTAMP_QUERY_INSIDE_PASSES,
            ..default()
        }),
        ..default()
    }
}

/// Per-pass samples (milliseconds), one per frame after the warm-up.
#[derive(Resource, Default)]
pub struct GpuProfile {
    samples: HashMap<String, Vec<f64>>,
    frames: u32,
}

/// Frames skipped before recording (shader compilation, asset loading).
const WARMUP_FRAMES: u32 = 200;

impl GpuProfile {
    /// Median of every recorded diagnostic, GPU rows first (largest first), as a printable table.
    pub fn summary(&self) -> String {
        let mut rows: Vec<(&str, f64, f64)> = self
            .samples
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| {
                let mut s = v.clone();
                s.sort_by(f64::total_cmp);
                (k.as_str(), s[s.len() / 2], s[(s.len() - 1) * 95 / 100])
            })
            .collect();
        rows.sort_by(|a, b| {
            let gpu = |r: &(&str, f64, f64)| r.0.ends_with("elapsed_gpu");
            gpu(b).cmp(&gpu(a)).then(b.1.total_cmp(&a.1))
        });
        let total_gpu: f64 = rows.iter().filter(|r| r.0.ends_with("elapsed_gpu")).map(|r| r.1).sum();
        let mut out = format!("per-pass medians over {} frames (ms, p95); sum of GPU medians {total_gpu:.2} ms\n", self.frames.saturating_sub(WARMUP_FRAMES));
        for (name, med, p95) in rows.iter().take(40) {
            out += &format!("  {med:7.3} {p95:7.3}  {}\n", name.trim_start_matches("render/"));
        }
        out
    }
}

fn collect(store: Res<DiagnosticsStore>, mut profile: ResMut<GpuProfile>) {
    profile.frames += 1;
    if profile.frames <= WARMUP_FRAMES {
        return;
    }
    for d in store.iter() {
        let path = d.path().as_str();
        if path.starts_with("render/") && (path.ends_with("elapsed_gpu") || path.ends_with("elapsed_cpu")) {
            if let Some(v) = d.value() {
                profile.samples.entry(path.to_string()).or_default().push(v);
            }
        }
    }
}

/// Collects the render diagnostics into [`GpuProfile`].
pub struct GpuProfilePlugin;

impl Plugin for GpuProfilePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(RenderDiagnosticsPlugin)
            .init_resource::<GpuProfile>()
            .add_systems(Last, collect);
    }
}
