//! Renders a scene offscreen (no window) to PNG files.
//!
//! ```text
//! render_scene <model.alo|-> <out.png> [options]
//!   --team r,g,b          team color (0..1 sRGB)
//!   --sun x,y,z           direction the sunlight travels
//!   --cam yaw,pitch,dist  orbit camera, dist in model radii
//!   --size WxH            image size
//!   --lod n               fixed LOD level
//!   --bow                 ship bow along -Z
//!   --many N              N extra copies of the model in a grid
//!   --fx name[@x,y,z]     standalone particle system (repeatable)
//!   --enable a,b          enable model proxy particle systems by name (event effects)
//!   --times t1,t2,...     capture at these simulation times in seconds (out_t<ms>.png)
//!   --radius r            framing radius when no model is given
//!   --look x,y,z          camera target offset
//! ```
//! Time advances in fixed 16 ms steps, so particle runs are reproducible.

use std::sync::Arc;
use std::time::Duration;

use alamo_fs::{GameFs, GameInstall};
use alamo_model::image::{Rgba8, encode_png};
use alamo_render::{
    AlamoModel, AlamoModelLoaded, AlamoParticles, AlamoRenderPlugin, LodSelect, ParticleStats,
    spawn_sun,
};
use bevy::app::ScheduleRunnerPlugin;
use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::time::TimeUpdateStrategy;
use bevy::window::ExitCondition;

const STEP_MS: u64 = 16;

/// Moves an entity at a constant velocity (world units per second).
#[derive(Component)]
struct Mover(Vec3);

fn mover(mut q: Query<(&Mover, &mut Transform)>) {
    for (m, mut t) in &mut q {
        t.translation += m.0 * (STEP_MS as f32 / 1000.0);
    }
}

#[derive(Resource)]
struct Shot {
    target: Handle<Image>,
    out: String,
    yaw: f32,
    pitch: f32,
    dist: f32,
    radius: f32,
    look: Vec3,
    times: Vec<f32>,
    next: usize,
    frame: u32,
    pending: usize,
    root: Option<Entity>,
    enable: Vec<String>,
    enabled_done: bool,
    /// Wall-clock moment after which simulation time starts (pipelines compile before that).
    ready_at: Option<std::time::Instant>,
    bench_start: Option<(std::time::Instant, u32)>,
}

fn parse_vec(s: &str) -> Vec<f32> {
    s.split(',').filter_map(|v| v.trim().parse().ok()).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mut model, mut out) = (None, None);
    let mut team = vec![1.0, 1.0, 1.0];
    let mut sun = vec![-0.5, -0.4, -0.6];
    let mut cam = vec![0.4, 0.25, 2.6];
    let (mut size, mut lod, mut bow) = (vec![1024.0, 640.0], None, false);
    let (mut many, mut radius) = (0usize, 100.0f32);
    let (mut fx, mut enable, mut times) = (Vec::new(), Vec::new(), vec![0.0f32]);
    let mut look = vec![0.0, 0.0, 0.0];
    let mut no_alpha = false;
    let mut hdr = false;
    let mut teams = 6usize;
    let mut fxn = 1usize;
    let mut mv: Vec<f32> = vec![0.0; 3];
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--team" => team = parse_vec(it.next().unwrap()),
            "--sun" => sun = parse_vec(it.next().unwrap()),
            "--cam" => cam = parse_vec(it.next().unwrap()),
            "--size" => {
                size = it
                    .next()
                    .unwrap()
                    .split('x')
                    .filter_map(|v| v.parse().ok())
                    .collect()
            }
            "--lod" => lod = it.next().unwrap().parse::<u32>().ok(),
            "--bow" => bow = true,
            "--many" => many = it.next().unwrap().parse().unwrap_or(0),
            "--fx" => fx.push(it.next().unwrap().clone()),
            "--enable" => {
                enable = it
                    .next()
                    .unwrap()
                    .split(',')
                    .map(str::to_lowercase)
                    .collect()
            }
            "--times" => times = parse_vec(it.next().unwrap()),
            "--radius" => radius = it.next().unwrap().parse().unwrap_or(100.0),
            "--addalpha" => no_alpha = true,
            "--hdr" => hdr = true,
            "--teams" => teams = it.next().unwrap().parse().unwrap_or(6),
            "--fxn" => fxn = it.next().unwrap().parse().unwrap_or(1),
            "--move" => mv = parse_vec(it.next().unwrap()),
            "--look" => look = parse_vec(it.next().unwrap()),
            _ if model.is_none() => model = Some(a.clone()),
            _ => out = Some(a.clone()),
        }
    }
    let (model, out) = (model.expect("model or -"), out.expect("out.png"));
    let install = GameInstall::detect().expect("Empire at War install");
    let fs = Arc::new(GameFs::for_mod(&install, Some("1770851727")).expect("fs"));

    let (w, h) = (size[0] as u32, size[1] as u32);
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: None,
        exit_condition: ExitCondition::DontExit,
        ..default()
    }))
    .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::from_millis(1)))
    .add_plugins(AlamoRenderPlugin::new(fs))
    .insert_resource(alamo_render::ParticleSettings {
        additive_alpha: no_alpha,
        ..default()
    })
    .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        STEP_MS,
    )))
    .insert_resource(ClearColor(Color::srgb(0.0, 0.0, 0.02)))
    .insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 0.0,
        ..default()
    });

    let mut image = Image::new_uninit(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        TextureFormat::Rgba8UnormSrgb,
        default(),
    );
    image.texture_descriptor.usage =
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC | TextureUsages::RENDER_ATTACHMENT;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);

    let mut root = None;
    if model != "-" {
        let mut spec =
            AlamoModel::new(model.clone()).with_team_color(Color::srgb(team[0], team[1], team[2]));
        if let Some(l) = lod {
            spec = spec.with_lod(LodSelect::Level(l));
        }
        spec.bow_forward = bow;
        root = Some(app.world_mut().spawn((spec, Transform::default())).id());
        for i in 0..many {
            let (gx, gz) = ((i % 15) as f32, (i / 15) as f32);
            let mut m = AlamoModel::new(model.clone()).with_team_color(Color::hsl(
                ((i % teams) * 360 / teams) as f32,
                0.8,
                0.5,
            ));
            m.bow_forward = bow;
            app.world_mut().spawn((
                m,
                Transform::from_xyz(gx * 900.0, 0.0, -gz * 1500.0 - 1500.0),
            ));
        }
    }
    for f in &fx {
        let (name, pos) = f.split_once('@').unwrap_or((f, "0,0,0"));
        let p = parse_vec(pos);
        for i in 0..fxn {
            let (gx, gz) = ((i % 12) as f32, (i / 12) as f32);
            app.world_mut().spawn((
                AlamoParticles::new(name),
                Transform::from_xyz(p[0] + gx * 150.0, p[1], p[2] - gz * 150.0),
                Mover(Vec3::new(mv[0], mv[1], mv[2])),
            ));
        }
    }
    let hdr_camera = hdr;
    {
        let mut commands = app.world_mut().commands();
        spawn_sun(
            &mut commands,
            Vec3::new(sun[0], sun[1], sun[2]),
            1.0,
            Color::WHITE,
        );
        let mut camera = commands.spawn((
            Camera3d::default(),
            Camera::default(),
            RenderTarget::Image(target.clone().into()),
            Tonemapping::None,
            Msaa::Sample4,
            Projection::Perspective(PerspectiveProjection {
                near: 0.05,
                far: 100000.0,
                ..default()
            }),
            Transform::default(),
        ));
        if hdr_camera {
            camera.insert((bevy::camera::Hdr, bevy::post_process::bloom::Bloom::NATURAL));
        }
    }
    app.world_mut().flush();
    app.insert_resource(Shot {
        target,
        out,
        yaw: cam[0],
        pitch: cam[1],
        dist: cam[2],
        radius,
        look: Vec3::new(look[0], look[1], look[2]),
        times,
        next: 0,
        frame: 0,
        pending: 0,
        root,
        enable,
        enabled_done: false,
        ready_at: None,
        bench_start: None,
    });
    app.add_systems(Update, (mover, drive));
    app.run();
}

#[allow(clippy::too_many_arguments)]
fn drive(
    mut commands: Commands,
    mut shot: ResMut<Shot>,
    loaded: Query<&AlamoModelLoaded>,
    mut cam: Query<&mut Transform, With<Camera3d>>,
    mut particles: Query<(&mut AlamoParticles, Option<&Name>)>,
    stats: Res<ParticleStats>,
    mut strategy: ResMut<TimeUpdateStrategy>,
    drawables: Query<(
        &Mesh3d,
        &MeshMaterial3d<alamo_render::material::AlamoMaterial>,
    )>,
) {
    let (center, radius) = match shot.root {
        Some(r) => match loaded.get(r) {
            Ok(l) => (l.center, l.radius),
            Err(_) => return,
        },
        None => (Vec3::ZERO, shot.radius),
    };
    let dir = Quat::from_euler(EulerRot::YXZ, shot.yaw, -shot.pitch, 0.0) * Vec3::Z;
    let eye = center + shot.look + dir * radius * shot.dist;
    if let Ok(mut t) = cam.single_mut() {
        *t = Transform::from_translation(eye).looking_at(center + shot.look, Vec3::Y);
    }
    // Hold simulation time until shaders and textures had a moment to get ready.
    let ready_at = *shot
        .ready_at
        .get_or_insert_with(|| std::time::Instant::now() + Duration::from_millis(3000));
    if std::time::Instant::now() < ready_at {
        *strategy = TimeUpdateStrategy::ManualDuration(Duration::ZERO);
        return;
    }
    *strategy = TimeUpdateStrategy::ManualDuration(Duration::from_millis(STEP_MS));
    if !shot.enabled_done {
        shot.enabled_done = true;
        // Entries are `name` (all proxies of that name) or `name#k` (k-th proxy of that name).
        let wanted = shot.enable.clone();
        let mut seen: std::collections::HashMap<String, usize> = Default::default();
        let proxies: Vec<Entity> = shot
            .root
            .and_then(|r| loaded.get(r).ok())
            .map(|l| l.proxies.clone())
            .unwrap_or_default();
        for entity in proxies {
            let Ok((mut p, _)) = particles.get_mut(entity) else {
                continue;
            };
            let n = p.path.trim_end_matches(".alo").to_lowercase();
            let k = seen.entry(n.clone()).or_default();
            if wanted.contains(&n) || wanted.contains(&format!("{n}#{k}")) {
                p.enabled = true;
            }
            *k += 1;
        }
        shot.frame = 0; // simulation time starts when everything is configured
        return;
    }
    shot.frame += 1;
    if shot.frame == 5 {
        shot.bench_start = Some((std::time::Instant::now(), shot.frame));
    }
    // Capture 3 frames after the target time so the screenshot shows that moment.
    let now = shot.frame as f32 * STEP_MS as f32 / 1000.0;
    let warmup = 20.0 * STEP_MS as f32 / 1000.0;
    if shot.next < shot.times.len() && now >= shot.times[shot.next] + warmup {
        let t = shot.times[shot.next];
        shot.next += 1;
        shot.pending += 1;
        let path = if shot.times.len() > 1 {
            format!(
                "{}_t{:04}.png",
                shot.out.trim_end_matches(".png"),
                (t * 1000.0) as u32
            )
        } else {
            shot.out.clone()
        };
        if let Some((t0, f0)) = shot.bench_start {
            let frames = (shot.frame - f0).max(1);
            println!(
                "average frame time {:.1} ms over {frames} frames (wall clock, includes readback)",
                t0.elapsed().as_secs_f32() * 1000.0 / frames as f32
            );
        }
        let pairs: std::collections::HashSet<_> = drawables
            .iter()
            .map(|(m, k)| (m.0.id(), k.0.id()))
            .collect();
        println!(
            "mesh entities {}, distinct (mesh, material) pairs = draw calls after batching {}",
            drawables.iter().count(),
            pairs.len()
        );
        println!(
            "capture t={t:.2}s: {} live particles, {} batches, {:.2} ms cpu",
            stats.live, stats.batches, stats.cpu_ms
        );
        commands
            .spawn(Screenshot::image(shot.target.clone()))
            .observe(
                move |cap: On<ScreenshotCaptured>,
                      mut exit: MessageWriter<AppExit>,
                      mut shot: ResMut<Shot>| {
                    let img = &cap.image;
                    let (w, h) = (img.width() as usize, img.height() as usize);
                    let mut px = img.data.clone().unwrap_or_default();
                    if img.texture_descriptor.format == TextureFormat::Bgra8UnormSrgb {
                        for p in px.as_chunks_mut::<4>().0.iter_mut() {
                            p.swap(0, 2);
                        }
                    }
                    let png = encode_png(&Rgba8 {
                        width: w,
                        height: h,
                        pixels: px,
                    });
                    std::fs::write(&path, png).expect("write png");
                    println!("wrote {path}");
                    shot.pending -= 1;
                    if shot.pending == 0 && shot.next >= shot.times.len() {
                        exit.write(AppExit::Success);
                    }
                },
            );
    }
}
