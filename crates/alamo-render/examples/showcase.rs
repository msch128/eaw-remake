//! Offscreen showcase of animation playback, turret aiming, hyperspace arrival and death
//! sequences: `showcase <scene> <out.png> [--times t1,t2,...] [--size WxH] [--cam yaw,pitch,dist]`
//! with scene one of `idle`, `xwing`, `turret`, `hyper`, `death`. Writes `out_t<ms>.png`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use alamo_fs::{GameFs, GameInstall};
use alamo_model::image::{Rgba8, encode_png};
use alamo_render::{
    AlamoAnimation, AlamoModel, AlamoModelLoaded, AlamoRenderPlugin, HyperspaceArrival, PlayDeath,
    TurretAim, spawn_sun,
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

#[derive(Resource)]
struct Rig {
    scene: String,
    target: Handle<Image>,
    out: String,
    times: Vec<f32>,
    next: usize,
    pending: usize,
    cam: Vec<f32>,
    root: Entity,
    ready_at: Option<Instant>,
    scripted: bool,
}

fn parse_vec(s: &str) -> Vec<f32> {
    s.split(',').filter_map(|v| v.trim().parse().ok()).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let scene = args[0].clone();
    let out = args[1].clone();
    let (mut times, mut size, mut cam) = (vec![1.0f32], vec![1280.0, 720.0], None);
    let mut it = args.iter().skip(2);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--times" => times = parse_vec(it.next().unwrap()),
            "--size" => {
                size = it
                    .next()
                    .unwrap()
                    .split('x')
                    .filter_map(|v| v.parse().ok())
                    .collect()
            }
            "--cam" => cam = Some(parse_vec(it.next().unwrap())),
            _ => {}
        }
    }
    let install = GameInstall::detect().expect("install");
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
    .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO))
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

    let (root, default_cam) = match scene.as_str() {
        "idle" => (
            app.world_mut()
                .spawn((
                    AlamoModel::new("EMPIRE_EXECUTOR_SUPER_STAR_DESTROYER.ALO").with_bow_forward(),
                    Transform::default(),
                ))
                .id(),
            vec![0.6, 0.35, 1.5],
        ),
        "xwing" => (
            app.world_mut()
                .spawn((
                    AlamoModel::new("REBEL_X_WING.ALO"),
                    Transform::default(),
                    AlamoAnimation::once("UNDEPLOY").with_speed(0.0),
                ))
                .id(),
            vec![0.5, 0.45, 2.2],
        ),
        "turret" => (
            app.world_mut()
                .spawn((
                    AlamoModel::new("EMPIRE_ALLEGIANCE_STAR_DESTROYER_LASER_TURRET_01.ALO"),
                    Transform::default(),
                    TurretAim::new(Vec3::new(200.0, 30.0, 0.0)),
                ))
                .id(),
            vec![0.6, 0.6, 2.4],
        ),
        "hyper" => (
            app.world_mut()
                .spawn((
                    AlamoModel::new("REBEL_MON_CALAMARI_HOME_ONE.ALO")
                        .with_bow_forward()
                        .with_team_color(Color::srgb(0.9, 0.2, 0.2)),
                    Transform::default(),
                ))
                .id(),
            vec![2.2, 0.15, 2.4],
        ),
        "death_mc" => (
            app.world_mut()
                .spawn((
                    AlamoModel::new("Rebel_Mon_Calamari_MC_80.alo").with_bow_forward(),
                    Transform::default(),
                ))
                .id(),
            vec![0.5, 0.3, 2.2],
        ),
        _ => (
            app.world_mut()
                .spawn((
                    AlamoModel::new("EMPIRE_IMPERIAL_STAR_DESTROYER.ALO").with_bow_forward(),
                    Transform::default(),
                ))
                .id(),
            vec![0.5, 0.3, 2.2],
        ),
    };
    {
        let mut commands = app.world_mut().commands();
        spawn_sun(
            &mut commands,
            Vec3::new(-0.5, -0.4, -0.6),
            1.0,
            Color::WHITE,
        );
        commands.spawn((
            Camera3d::default(),
            Camera::default(),
            bevy::camera::Hdr,
            bevy::post_process::bloom::Bloom::NATURAL,
            RenderTarget::Image(target.clone().into()),
            Tonemapping::None,
            Msaa::Sample4,
            Projection::Perspective(PerspectiveProjection {
                near: 1.0,
                far: 200000.0,
                ..default()
            }),
            Transform::default(),
        ));
    }
    app.world_mut().flush();
    app.insert_resource(Rig {
        scene,
        target,
        out,
        times,
        next: 0,
        pending: 0,
        cam: cam.unwrap_or(default_cam),
        root,
        ready_at: None,
        scripted: false,
    });
    app.add_systems(Update, drive);
    app.run();
}

#[allow(clippy::too_many_arguments)]
fn drive(
    mut commands: Commands,
    mut rig: ResMut<Rig>,
    time: Res<Time>,
    loaded: Query<&AlamoModelLoaded>,
    mut cam: Query<&mut Transform, With<Camera3d>>,
    mut strategy: ResMut<TimeUpdateStrategy>,
    mut anims: Query<&mut AlamoAnimation>,
    mut turrets: Query<&mut TurretAim>,
) {
    let Ok(l) = loaded.get(rig.root) else {
        return;
    };
    let dist = l.radius * rig.cam[2];
    let dir = Quat::from_euler(EulerRot::YXZ, rig.cam[0], -rig.cam[1], 0.0) * Vec3::Z;
    if let Ok(mut t) = cam.single_mut() {
        *t = Transform::from_translation(l.center + dir * dist).looking_at(l.center, Vec3::Y);
    }
    let ready_at = *rig
        .ready_at
        .get_or_insert_with(|| Instant::now() + Duration::from_millis(3000));
    if Instant::now() < ready_at {
        *strategy = TimeUpdateStrategy::ManualDuration(Duration::ZERO);
        return;
    }
    *strategy = TimeUpdateStrategy::ManualDuration(Duration::from_millis(STEP_MS));
    let now = time.elapsed_secs();

    // Scene scripts.
    match rig.scene.as_str() {
        "xwing" => {
            // Scrub the S-foil animation: speed 0, time follows the script.
            if let Ok(mut a) = anims.get_mut(rig.root) {
                a.time = now;
                a.speed = 1.0;
                a.looping = false;
                a.playing = false;
                let _ = &a;
            }
        }
        "turret" => {
            if let Ok(mut t) = turrets.get_mut(rig.root) {
                let ang = now * 0.9;
                t.target = Vec3::new(
                    ang.cos() * 250.0,
                    20.0 + 60.0 * (now * 0.5).sin().abs(),
                    ang.sin() * 250.0,
                );
            }
        }
        "hyper" if !rig.scripted => {
            rig.scripted = true;
            commands.entity(rig.root).insert(HyperspaceArrival {
                distance_radii: 14.0,
                ..default()
            });
        }
        "death" | "death_mc" if !rig.scripted && now > 0.5 => {
            rig.scripted = true;
            commands.trigger(PlayDeath {
                entity: rig.root,
                unit: None,
            });
        }
        _ => {}
    }

    if rig.next < rig.times.len() && now >= rig.times[rig.next] {
        let t = rig.times[rig.next];
        rig.next += 1;
        rig.pending += 1;
        let path = format!(
            "{}_t{:05}.png",
            rig.out.trim_end_matches(".png"),
            (t * 1000.0) as u32
        );
        commands
            .spawn(Screenshot::image(rig.target.clone()))
            .observe(
                move |cap: On<ScreenshotCaptured>,
                      mut exit: MessageWriter<AppExit>,
                      mut rig: ResMut<Rig>| {
                    let img = &cap.image;
                    let (w, h) = (img.width() as usize, img.height() as usize);
                    let mut px = img.data.clone().unwrap_or_default();
                    if img.texture_descriptor.format == TextureFormat::Bgra8UnormSrgb {
                        for p in px.as_chunks_mut::<4>().0.iter_mut() {
                            p.swap(0, 2);
                        }
                    }
                    std::fs::write(
                        &path,
                        encode_png(&Rgba8 {
                            width: w,
                            height: h,
                            pixels: px,
                        }),
                    )
                    .expect("write png");
                    println!("wrote {path}");
                    rig.pending -= 1;
                    if rig.pending == 0 && rig.next >= rig.times.len() {
                        exit.write(AppExit::Success);
                    }
                },
            );
    }
}
