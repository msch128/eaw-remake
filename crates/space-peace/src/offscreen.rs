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

static TARGET_SIZE: std::sync::OnceLock<(u32, u32)> = std::sync::OnceLock::new();

/// Render target size in physical pixels (`--size`, default 4K).
pub fn size() -> (u32, u32) {
    TARGET_SIZE.get().copied().unwrap_or((3840, 2160))
}

/// OS display scaling of the target: 150 % on 4K like a typical monitor, 100 % below.
fn scale_factor(height: u32) -> f32 {
    if height >= 2000 { 1.5 } else { 1.0 }
}

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
    /// Planet whose view to open once the scenario has settled, and the frame it was opened at.
    planet_view: Option<String>,
    planet_view_opened: Option<u32>,
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
    /// `--burst`: frames still to save one by one, and frames saved so far.
    burst_left: u32,
    burst_saved: u32,
}

/// `<out>-007.png` next to `<out>.png`: frame 7 of a `--burst`.
fn burst_path(path: &std::path::Path, k: u32) -> PathBuf {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "png".into());
    path.with_file_name(format!("{stem}-{k:03}.{ext}"))
}

/// Registers the offscreen target and the driver system on `app` (call before `run`).
pub fn configure(app: &mut App, args: &Args, path: PathBuf) {
    let _ = TARGET_SIZE.set(args.size.unwrap_or((3840, 2160)));
    let image = Image::new_target_texture(size().0, size().1, TextureFormat::Rgba8UnormSrgb, None);
    let handle = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    let frames = args.frames.max(2);
    app.insert_resource(OffscreenTarget { image: handle, scale_factor: scale_factor(size().1) })
        .insert_resource(OffscreenState {
            path,
            frames,
            warmup: 30.min(frames / 2),
            select: args.select.clone(),
            planet_view: args.planet_view.clone(),
            planet_view_opened: None,
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
            burst_left: args.burst,
            burst_saved: 0,
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
    mut planet_view: ResMut<crate::planet_view::PlanetViewState>,
    mut cam: Single<&mut GalaxyCamera>,
    screen: Res<State<AppScreen>>,
    mut exit: MessageWriter<AppExit>,
) {
    let now = Instant::now();
    let dt = now.duration_since(st.last).as_secs_f64() * 1000.0;
    st.last = now;
    if dt > 40.0 && st.frame > 3 {
        info!("slow frame: {dt:.0} ms (frame {}, screen {:?})", st.frame + 1, *screen.get());
    }
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

    // `--planet-view`: open the view once the scenario flags have run, and give it time to fly in before the capture.
    if let Some(name) = st.planet_view.clone()
        && st.planet_view_opened.is_none()
        && st.galaxy_frames > 3
        && hold.settled()
    {
        match view.planets.iter().position(|p| p.id.eq_ignore_ascii_case(&name) || p.display_name.eq_ignore_ascii_case(&name)) {
            Some(i) => {
                selection.0 = Some(i);
                planet_view.active = true;
                planet_view.planet = Some(i);
                st.planet_view_opened = Some(frame);
            }
            None => {
                warn!("offscreen: planet `{name}` not found");
                st.planet_view = None;
            }
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
    let view_ready = st.planet_view.is_none() || st.planet_view_opened.is_some_and(|f| frame >= f + 150);
    // `--burst n`: the n frames from the scripted orders on (or before the final capture if nothing was ordered), each saved on its
    // own, to look for single-frame flicker.
    if st.burst_left > 0 && (hold.acted || (frame >= st.frames && settled)) && !st.capture_requested {
        let path = burst_path(&st.path, st.burst_saved);
        info!("offscreen: burst frame {} is frame {frame}", st.burst_saved);
        commands
            .spawn(Screenshot(RenderTarget::Image(ImageRenderTarget {
                handle: target.image.clone(),
                scale_factor: target.scale_factor,
            })))
            .observe(save_to_disk(path));
        st.burst_left -= 1;
        st.burst_saved += 1;
    }
    if frame >= st.frames && st.frame_ms.len() as u32 >= st.frames / 2 && settled && view_ready && !st.capture_requested && st.burst_left == 0 {
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
            size().0,
            size().1,
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
