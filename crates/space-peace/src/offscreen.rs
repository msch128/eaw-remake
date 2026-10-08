//! Headless rendering (`--offscreen <out.png>`): renders the normal scene into a 3840x2160
//! texture without creating a window, logs frame timings and saves the last frame as PNG.

use std::{collections::HashMap, path::PathBuf, time::Instant};

use bevy::{
    app::AppExit,
    camera::{ImageRenderTarget, RenderTarget},
    diagnostic::DiagnosticsStore,
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    render::render_resource::TextureFormat,
};

use crate::camera::{CameraBounds, GalaxyCamera};
use crate::cli::Args;
use crate::flow::AppScreen;
use crate::galaxy_map::{Selection, galaxy_to_world};
use crate::galaxy_view::GalaxyView;

/// Render target size in physical pixels.
pub const SIZE: (u32, u32) = (3840, 2160);
/// UI scale of the target (a typical Windows setting for 4K monitors).
const SCALE_FACTOR: f32 = 1.5;

/// The texture the camera renders into.
#[derive(Resource)]
pub struct OffscreenTarget {
    /// Target image handle.
    pub image: Handle<Image>,
    /// UI scale factor of the target.
    pub scale_factor: f32,
}

/// Progress and measurements of an offscreen run.
#[derive(Resource)]
struct OffscreenState {
    path: PathBuf,
    frames: u32,
    warmup: u32,
    select: Option<String>,
    zoom: Option<f32>,
    frame: u32,
    created: Instant,
    last: Instant,
    started: Option<Instant>,
    frame_ms: Vec<f64>,
    gpu_ms: HashMap<String, (f64, u32)>,
    capture_requested: bool,
    capture_time: Option<Instant>,
    /// Frames spent on the galaxy map (the requested view is applied after a few).
    galaxy_frames: u32,
    /// First moment the loading screen was left.
    ready_at: Option<Instant>,
    /// Capture the loading screen itself.
    loading_shot: bool,
}

/// Registers the offscreen target and the driver system on `app` (call before `run`).
pub fn configure(app: &mut App, args: &Args, path: PathBuf) {
    let image = Image::new_target_texture(SIZE.0, SIZE.1, TextureFormat::Rgba8UnormSrgb, None);
    let handle = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    let frames = args.frames.max(2);
    app.insert_resource(OffscreenTarget { image: handle, scale_factor: SCALE_FACTOR })
        .insert_resource(OffscreenState {
            path,
            frames,
            warmup: 30.min(frames / 2),
            select: args.select.clone(),
            zoom: args.zoom,
            frame: 0,
            created: Instant::now(),
            last: Instant::now(),
            started: None,
            frame_ms: Vec::new(),
            gpu_ms: HashMap::new(),
            capture_requested: false,
            capture_time: None,
            galaxy_frames: 0,
            ready_at: None,
            loading_shot: args.screen.as_deref() == Some("loading"),
        })
        .add_systems(Last, drive);
}

#[allow(clippy::too_many_arguments)]
fn drive(
    mut commands: Commands,
    mut st: ResMut<OffscreenState>,
    target: Res<OffscreenTarget>,
    view: Res<GalaxyView>,
    bounds: Res<CameraBounds>,
    diagnostics: Res<DiagnosticsStore>,
    hold: Res<crate::play::ScriptHold>,
    mut selection: ResMut<Selection>,
    mut cam: Single<&mut GalaxyCamera>,
    screen: Res<State<AppScreen>>,
    mut exit: MessageWriter<AppExit>,
) {
    let now = Instant::now();
    let dt = now.duration_since(st.last).as_secs_f64() * 1000.0;
    st.last = now;
    st.frame += 1;
    let frame = st.frame;
    let screen = *screen.get();
    if screen != AppScreen::Loading && st.ready_at.is_none() {
        st.ready_at = Some(now);
    }
    if screen == AppScreen::Galaxy {
        st.galaxy_frames += 1;
    }
    // Apply the requested view once the camera has framed the galaxy.
    if st.galaxy_frames == 3 {
        if let Some(sel) = st.select.clone() {
            let idx = view.planets.iter().position(|p| {
                p.id.eq_ignore_ascii_case(&sel) || p.display_name.eq_ignore_ascii_case(&sel)
            });
            match idx {
                Some(i) => {
                    selection.0 = Some(i);
                    let w = galaxy_to_world(view.planets[i].position);
                    cam.focus = Vec3::new(w.x, 0.0, w.z);
                    cam.target_focus = cam.focus;
                    if st.zoom.is_none() {
                        st.zoom = Some(0.12);
                    }
                }
                None => warn!("offscreen: planet `{sel}` not found"),
            }
        }
        if let Some(z) = st.zoom {
            let d = bounds.min_distance + (bounds.max_distance - bounds.min_distance) * z.clamp(0.0, 1.0);
            cam.distance = d;
            cam.target_distance = d;
        }
    }

    if frame == st.warmup {
        st.started = Some(now);
    }
    if frame > st.warmup && !st.capture_requested && hold.settled() && (st.loading_shot || screen != AppScreen::Loading) && screen != AppScreen::SpaceBattle {
        st.frame_ms.push(dt);
        for (path, d) in diagnostics.iter().map(|d| (d.path().to_string(), d)) {
            if path.ends_with("elapsed_gpu")
                && let Some(v) = d.value()
            {
                let e = st.gpu_ms.entry(path).or_insert((0.0, 0));
                e.0 += v;
                e.1 += 1;
            }
        }
    }

    // Wait for the asynchronous asset uploads (sky texture, meshes) as well as the frame count.
    let settled = if st.loading_shot {
        now.duration_since(st.created).as_secs_f32() > 3.0
    } else {
        st.ready_at.is_some_and(|t| now.duration_since(t).as_secs_f32() > 3.0) && hold.settled()
    };
    if frame >= st.frames && st.frame_ms.len() as u32 >= st.frames / 2 && settled && !st.capture_requested {
        st.capture_requested = true;
        st.capture_time = Some(now);
        let path = st.path.clone();
        commands
            // The target must equal the camera's (including scale factor) for the capture to match.
            .spawn(Screenshot(RenderTarget::Image(ImageRenderTarget {
                handle: target.image.clone(),
                scale_factor: target.scale_factor,
            })))
            .observe(save_to_disk(path))
            .observe(finish);
    }
    // Safety net: never hang if the capture does not arrive.
    if st.capture_time.is_some_and(|t| now.duration_since(t).as_secs() > 20) {
        error!("offscreen: screenshot never arrived");
        exit.write(AppExit::error());
    }
}

/// Reports timings once the screenshot has been read back, then exits.
fn finish(_: On<ScreenshotCaptured>, st: Res<OffscreenState>, mut exit: MessageWriter<AppExit>) {
    if let Some(started) = st.started {
        let n = st.frame.saturating_sub(st.warmup).max(1);
        let secs = started.elapsed().as_secs_f64();
        info!(
            "offscreen: {n} frames at {}x{} in {:.2} s = {:.2} ms/frame throughput (includes finishing queued GPU work)",
            SIZE.0,
            SIZE.1,
            secs,
            secs * 1000.0 / n as f64
        );
    }
    let mut ms = st.frame_ms.clone();
    if !ms.is_empty() {
        ms.sort_by(f64::total_cmp);
        let pct = |p: f64| ms[((ms.len() - 1) as f64 * p) as usize];
        let mean = ms.iter().sum::<f64>() / ms.len() as f64;
        info!(
            "offscreen: loop frame time over {} frames: mean {mean:.2} ms, p50 {:.2}, p95 {:.2}, p99 {:.2}, max {:.2}",
            ms.len(),
            pct(0.5),
            pct(0.95),
            pct(0.99),
            ms[ms.len() - 1]
        );
    }
    let mut gpu: Vec<(&String, f64)> = st
        .gpu_ms
        .iter()
        .map(|(k, (sum, n))| (k, sum / f64::from(*n)))
        .collect();
    gpu.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (k, v) in gpu.iter().take(12) {
        info!("offscreen: gpu {k} avg {v:.3} ms");
    }
    exit.write(AppExit::Success);
}
