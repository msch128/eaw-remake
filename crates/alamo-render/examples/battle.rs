//! Scripted offscreen ship battle: an Imperial Star Destroyer and Home One exchange turbolaser fire
//! (static ships). Bolts are batched geometry, hits on the shield bubble flash it locally, later
//! hull hits spawn impact particles and one hardpoint-style explosion goes off.
//!
//! `battle <out.png> [--times t1,t2,...] [--cam yaw,pitch,dist] [--size WxH] [--rate bolts_per_s]
//! [--look x,y,z]` (writes `out_t<ms>.png` for several times).

use std::sync::Arc;
use std::time::Duration;

use alamo_fs::{GameFs, GameInstall};
use alamo_model::image::{Rgba8, encode_png};
use alamo_render::{
    AimTurrets, AlamoHardpoint, AlamoModel, AlamoModelLoaded, AlamoRenderPlugin,
    HardpointDestroyed, ImpactKind, ParticleStats, ProjectileImpact, ProjectileVisual, ShieldHit,
    SpawnParticleEffect, XmlDefs, spawn_sun,
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
struct Scene {
    target: Handle<Image>,
    out: String,
    times: Vec<f32>,
    next: usize,
    pending: usize,
    frame: u32,
    isd: Entity,
    mc: Entity,
    rate: f32,
    ready_at: Option<std::time::Instant>,
    spawn_acc: f32,
    rng: u32,
    hull_phase: bool,
    exploded: bool,
    units: bool,
    destroyed: bool,
    cam: Vec<f32>,
    look: Vec3,
}

/// A scripted bolt flying to a point on the other ship.
#[derive(Component)]
struct Flight {
    velocity: Vec3,
    arrive_at: f32,
    victim: Entity,
    projectile: &'static str,
    /// Point where the bolt ends (on the shield bubble or the hull).
    end: Vec3,
    shield: bool,
}

fn rand(s: &mut u32) -> f32 {
    *s ^= *s << 13;
    *s ^= *s >> 17;
    *s ^= *s << 5;
    (*s >> 8) as f32 / (1u32 << 24) as f32
}

fn parse_vec(s: &str) -> Vec<f32> {
    s.split(',').filter_map(|v| v.trim().parse().ok()).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut out = None;
    let (mut times, mut cam, mut size) = (vec![4.0f32], vec![0.0, 0.12, 1.0], vec![1280.0, 720.0]);
    let (mut rate, mut look) = (6.0f32, vec![0.0, 0.0, 0.0]);
    let mut units = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--times" => times = parse_vec(it.next().unwrap()),
            "--cam" => cam = parse_vec(it.next().unwrap()),
            "--size" => {
                size = it
                    .next()
                    .unwrap()
                    .split('x')
                    .filter_map(|v| v.parse().ok())
                    .collect()
            }
            "--rate" => rate = it.next().unwrap().parse().unwrap(),
            "--units" => units = true,
            "--look" => look = parse_vec(it.next().unwrap()),
            _ => out = Some(a.clone()),
        }
    }
    let out = out.expect("out.png");
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

    // Bow along +X for the Star Destroyer, along -X for Home One.
    let isd_spec = if units {
        AlamoModel::new("EMPIRE_ALLEGIANCE_STAR_DESTROYER.ALO")
            .with_bow_forward()
            .with_unit("Allegiance_Star_Destroyer")
    } else {
        AlamoModel::new("EMPIRE_IMPERIAL_STAR_DESTROYER.ALO").with_bow_forward()
    };
    let mc_spec = if units {
        AlamoModel::new("REBEL_MON_CALAMARI_MC_80.ALO")
            .with_bow_forward()
            .with_unit("MC-80_Star_Cruiser")
    } else {
        AlamoModel::new("REBEL_MON_CALAMARI_HOME_ONE.ALO").with_bow_forward()
    };
    let isd = app
        .world_mut()
        .spawn((
            isd_spec,
            Transform::from_xyz(-1000.0, 0.0, 0.0)
                .with_rotation(Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2)),
        ))
        .id();
    let mc = app
        .world_mut()
        .spawn((
            mc_spec.with_team_color(Color::srgb(0.9, 0.2, 0.2)),
            Transform::from_xyz(1150.0, 0.0, 0.0)
                .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)),
        ))
        .id();
    {
        let mut commands = app.world_mut().commands();
        spawn_sun(&mut commands, Vec3::new(0.3, -0.5, -0.8), 1.0, Color::WHITE);
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
                far: 100000.0,
                ..default()
            }),
            Transform::default(),
        ));
    }
    app.world_mut().flush();
    app.insert_resource(Scene {
        target,
        out,
        times,
        next: 0,
        pending: 0,
        frame: 0,
        isd,
        mc,
        rate,
        ready_at: None,
        spawn_acc: 0.0,
        rng: 0x1234_5678,
        hull_phase: false,
        exploded: false,
        units,
        destroyed: false,
        cam,
        look: Vec3::new(look[0], look[1], look[2]),
    });
    app.add_systems(Update, (script, fly, camera_and_capture).chain());
    app.run();
}

fn script(
    mut commands: Commands,
    mut sc: ResMut<Scene>,
    time: Res<Time>,
    roots: Query<(&AlamoModelLoaded, &GlobalTransform)>,
    mut strategy: ResMut<TimeUpdateStrategy>,
    hardpoints: Query<&AlamoHardpoint>,
) {
    let (Ok((li, gi)), Ok((lm, gm))) = (roots.get(sc.isd), roots.get(sc.mc)) else {
        return;
    };
    // Hold simulation time until shaders and textures had a moment to get ready.
    let ready_at = *sc
        .ready_at
        .get_or_insert_with(|| std::time::Instant::now() + Duration::from_millis(3000));
    if std::time::Instant::now() < ready_at {
        *strategy = TimeUpdateStrategy::ManualDuration(Duration::ZERO);
        return;
    }
    *strategy = TimeUpdateStrategy::ManualDuration(Duration::from_millis(STEP_MS));
    let t = time.elapsed_secs();
    sc.frame += 1;
    let (ci, ri) = (gi.transform_point(li.center), li.radius);
    let (cm, rm) = (gm.transform_point(lm.center), lm.radius);
    if sc.units {
        // Turrets of both ships track the other ship.
        commands.trigger(AimTurrets {
            model: sc.isd,
            target: cm,
        });
        commands.trigger(AimTurrets {
            model: sc.mc,
            target: ci,
        });
        if t > 4.0 && !sc.destroyed {
            sc.destroyed = true;
            for (loaded, ship) in [(li, sc.isd), (lm, sc.mc)] {
                let names: Vec<String> = loaded
                    .hardpoints
                    .iter()
                    .filter_map(|&h| hardpoints.get(h).ok())
                    .filter(|h| h.turret.is_some())
                    .map(|h| h.name.clone())
                    .collect();
                println!(
                    "{} turret hardpoints on a ship; destroying a few",
                    names.len()
                );
                for n in names.iter().step_by((names.len() / 3).max(1)).take(3) {
                    commands.trigger(HardpointDestroyed {
                        model: ship,
                        hardpoint: n.clone(),
                    });
                }
            }
        }
    }
    let hull_phase = t > 5.0;
    sc.hull_phase = hull_phase;
    sc.spawn_acc += sc.rate * time.delta_secs();
    while sc.spawn_acc >= 1.0 {
        sc.spawn_acc -= 1.0;
        let mut rng = sc.rng;
        // Alternate shooters; start on the near hull side (facing the camera is +Z).
        let from_isd = rand(&mut rng) < 0.5;
        let (shooter_c, shooter_r, victim_c, victim_r, victim, proj, speed) = if from_isd {
            (
                ci,
                ri,
                cm,
                rm,
                sc.mc,
                "Proj_Ship_Turbolaser_Green",
                45.0 * 30.0,
            )
        } else {
            (
                cm,
                rm,
                ci,
                ri,
                sc.isd,
                "Proj_Ship_Turbolaser_Red",
                45.0 * 30.0,
            )
        };
        let jitter = |r: &mut u32, s: f32| {
            Vec3::new(rand(r) - 0.5, (rand(r) - 0.5) * 0.4, (rand(r) - 0.5) * 0.4) * s
        };
        let start = shooter_c
            + (victim_c - shooter_c).normalize() * shooter_r * 0.45
            + jitter(&mut rng, shooter_r);
        // Aim at a point near the victim, on its shield bubble first, on the hull later.
        let aim = victim_c + jitter(&mut rng, victim_r * 0.7);
        let dir = (aim - start).normalize();
        let radius = if hull_phase {
            victim_r * 0.32
        } else {
            victim_r * 0.5
        };
        // Intersect the ray with a sphere around the victim to find the end point.
        let oc = start - victim_c;
        let b = oc.dot(dir);
        let c = oc.length_squared() - radius * radius;
        let disc = b * b - c;
        let end = if disc > 0.0 {
            start + dir * (-b - disc.sqrt())
        } else {
            aim
        };
        let dist = (end - start).length();
        sc.rng = rng;
        commands.spawn((
            ProjectileVisual::new(proj),
            Transform::from_translation(start).looking_to(dir, Vec3::Y),
            Flight {
                velocity: dir * speed,
                arrive_at: t + dist / speed,
                victim,
                projectile: proj,
                end,
                shield: !hull_phase,
            },
        ));
    }
    if t > 7.0 && !sc.exploded {
        sc.exploded = true;
        // A hardpoint-style explosion on the Star Destroyer's flank.
        commands.trigger(SpawnParticleEffect {
            particle: "Large_Explosion_Space_Empire_pd_medium_3_300".into(),
            position: ci + Vec3::new(40.0, 20.0, 60.0),
            normal: Vec3::Z,
        });
    }
}

fn fly(
    mut commands: Commands,
    time: Res<Time>,
    mut bolts: Query<(Entity, &Flight, &mut Transform)>,
    defs: Res<XmlDefs>,
) {
    let t = time.elapsed_secs();
    let dt = time.delta_secs();
    for (e, f, mut tf) in &mut bolts {
        tf.translation += f.velocity * dt;
        if t >= f.arrive_at {
            let normal = (f.end - tf.translation + Vec3::Y * 0.001).normalize();
            let _ = normal;
            if f.shield {
                commands.trigger(ShieldHit {
                    model: f.victim,
                    position: f.end,
                    intensity: 1.6,
                });
                commands.trigger(ProjectileImpact {
                    projectile: f.projectile.into(),
                    kind: ImpactKind::Shield,
                    position: f.end,
                    normal: -f.velocity.normalize(),
                });
            } else {
                commands.trigger(ProjectileImpact {
                    projectile: f.projectile.into(),
                    kind: ImpactKind::Hull,
                    position: f.end,
                    normal: -f.velocity.normalize(),
                });
            }
            commands.entity(e).despawn();
        }
    }
    let _ = &defs;
}

fn camera_and_capture(
    mut commands: Commands,
    mut sc: ResMut<Scene>,
    time: Res<Time>,
    mut cam: Query<&mut Transform, With<Camera3d>>,
    stats: Res<ParticleStats>,
    bolts: Query<(), With<Flight>>,
) {
    let dist = 2900.0 * sc.cam[2];
    let dir = Quat::from_euler(EulerRot::YXZ, sc.cam[0], -sc.cam[1], 0.0) * Vec3::Z;
    if let Ok(mut t) = cam.single_mut() {
        *t = Transform::from_translation(sc.look + dir * dist).looking_at(sc.look, Vec3::Y);
    }
    let now = time.elapsed_secs();
    if sc.next < sc.times.len() && now >= sc.times[sc.next] + 0.4 {
        let t = sc.times[sc.next];
        sc.next += 1;
        sc.pending += 1;
        let path = format!(
            "{}_t{:05}.png",
            sc.out.trim_end_matches(".png"),
            (t * 1000.0) as u32
        );
        println!(
            "capture t={t:.2}s: {} live particles, {} bolts, {} batches, {:.2} ms cpu",
            stats.live,
            bolts.iter().count(),
            stats.batches,
            stats.cpu_ms
        );
        commands
            .spawn(Screenshot::image(sc.target.clone()))
            .observe(
                move |cap: On<ScreenshotCaptured>,
                      mut exit: MessageWriter<AppExit>,
                      mut sc: ResMut<Scene>| {
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
                    sc.pending -= 1;
                    if sc.pending == 0 && sc.next >= sc.times.len() {
                        exit.write(AppExit::Success);
                    }
                },
            );
    }
}
