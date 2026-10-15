//! Demo: Empire fleet (Imperial / Victory Star Destroyers, TIE fighters) against a Rebel fleet
//! (MC80, Nebulon-B, CR90, X-wings) on a Remake 4.0 space map.
//!
//! ```text
//! battle_demo [--offscreen out.png] [--times 5,30,90] [--seconds N] [--size WxH]
//!             [--map _SPACE_PLANET_KUAT_01] [--speed S] [--seed N] [--scale k]
//!             [--cam yaw_deg,pitch_deg,distance[,x,z]]   (repeatable, one per capture time)
//!             [--station Rebel_Star_Base_2]   give the Rebels a star base
//!             [--overlay]   select the player's capital ships and hover an enemy (offscreen)
//!             [--msaa N] [--no-bloom] [--small]
//! ```
//! Without `--offscreen` the demo opens a window. With it, no window is created: frames are
//! rendered into a texture at fixed 16 ms steps and saved as `out_t<ms>.png` for every capture time.

use std::sync::Arc;
use std::time::{Duration, Instant};

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use alamo_model::image::{Rgba8, encode_png};
use alamo_render::{AlamoRenderPlugin, ParticleStats};
use battle_sim::{BattleConfig, Catalog, SideSetup, UnitSetup};
use battle_view::{
    BattleCamera, BattleSession, BattleStatus, BattleViewPlugin, BattleViewSettings, Hover,
    Pointer, RenderedShips, Selection, SpaceScene, VisualCatalog,
};
use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::time::TimeUpdateStrategy;
use bevy::window::ExitCondition;

const STEP_MS: u64 = 16;

fn parse_vec(s: &str) -> Vec<f32> {
    s.split(',').filter_map(|v| v.trim().parse().ok()).collect()
}

#[derive(Resource)]
struct Capture {
    out: String,
    times: Vec<f32>,
    cams: Vec<Vec<f32>>,
    next: usize,
    pending: usize,
    overlay: bool,
    target: Handle<Image>,
    frame_ms: Vec<f64>,
    last: Instant,
    wall_start: Instant,
    warm: u32,
    /// Frames to leave out of the statistics after a screenshot (the readback stalls the frame).
    skip: u32,
    /// Wall milliseconds of every simulation step seen (one entry per tick).
    step_ms: Vec<f32>,
    last_tick: u64,
    /// Frame times before the statistics start (entering the battle: model and texture loading).
    early_ms: Vec<f64>,
    frames: u32,
    cam_applied: bool,
    auto: bool,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut offscreen: Option<String> = None;
    let mut times = vec![5.0f32, 30.0, 90.0];
    let mut seconds: Option<f32> = None;
    let mut size = vec![1920.0, 1080.0];
    let mut map = "_SPACE_PLANET_KUAT_01".to_string();
    let (mut speed, mut seed, mut msaa) = (1.0f32, 1u64, 4u32);
    let (mut cams, mut overlay, mut bloom, mut scale) = (Vec::<Vec<f32>>::new(), false, true, 1.0f32);
    let mut small = false;
    let mut gpu_profile = false;
    let (mut shadows, mut quality) = (false, None::<battle_view::QualityPreset>);
    let mut auto = false;
    let mut station: Option<String> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--offscreen" => offscreen = it.next().cloned(),
            "--times" => times = parse_vec(it.next().unwrap()),
            "--seconds" => seconds = it.next().and_then(|v| v.parse().ok()),
            "--size" => size = it.next().unwrap().split('x').filter_map(|v| v.parse().ok()).collect(),
            "--map" => map = it.next().unwrap().clone(),
            "--speed" => speed = it.next().unwrap().parse().unwrap(),
            "--seed" => seed = it.next().unwrap().parse().unwrap(),
            "--msaa" => msaa = it.next().unwrap().parse().unwrap(),
            "--cam" => cams.push(parse_vec(it.next().unwrap())),
            "--overlay" => overlay = true,
            "--no-bloom" => bloom = false,
            "--shadows" => shadows = true,
            "--gpu-profile" => gpu_profile = true,
            "--quality" => quality = Some(match it.next().unwrap().as_str() {
                "low" => battle_view::QualityPreset::Low,
                "medium" => battle_view::QualityPreset::Medium,
                _ => battle_view::QualityPreset::High,
            }),
            "--scale" => scale = it.next().unwrap().parse().unwrap(),
            "--small" => small = true,
            "--auto" => auto = true,
            "--station" => station = it.next().cloned(),
            other => eprintln!("ignoring argument {other}"),
        }
    }
    if let Some(s) = seconds {
        times = vec![s];
    }

    let install = GameInstall::detect().expect("Empire at War install");
    let fs = Arc::new(GameFs::for_mod(&install, Some("1770851727")).expect("game files"));
    let (data, _) = load_galaxy(&*fs);
    let map_path = format!("Art/Maps/{map}.TED");
    let mut scene = SpaceScene::load(&*fs, &data, &map_path).expect("space map");
    if std::env::var("NOPROPS").is_ok() {
        scene.props.clear();
    }
    println!(
        "map {} bounds {:?}..{:?}, {} props, skydome {:?}, nebula {:?}",
        scene.name, scene.bounds_min, scene.bounds_max, scene.props.len(), scene.skydome, scene.nebula
    );

    if std::env::var("PROPS").is_ok() {
        for p in &scene.props {
            println!("prop {} {} scale {} pos {:?} backdrop {}", p.name, p.model, p.scale, p.position, p.backdrop);
        }
    }
    let scene_rocks: Vec<Vec3> = scene.props.iter().filter(|p| !p.backdrop).map(|p| p.position).collect();
    // Fleets.
    let empire: &[(&str, u32)] = if small {
        &[("Star_Destroyer", 1), ("TIE_Fighter_Squadron", 2)]
    } else {
        &[
            ("Star_Destroyer", 3),
            ("Victory_Destroyer", 3),
            ("TIE_Fighter_Squadron", 4),
        ]
    };
    let rebel: &[(&str, u32)] = if std::env::var("STATION_ONLY").is_ok() {
        &[]
    } else if small {
        &[("MC-80_Star_Cruiser", 1), ("X-Wing_Squadron", 2)]
    } else {
        &[
            ("MC-80_Star_Cruiser", 3),
            ("Nebulon_B_Frigate", 4),
            ("CR-90_Corvette", 4),
            ("X-Wing_Squadron", 4),
        ]
    };
    let mut names: Vec<&str> = empire.iter().chain(rebel).map(|x| x.0).collect();
    if let Some(st) = station.as_deref() {
        names.push(st);
    }
    let catalog = Arc::new(Catalog::build(&data, Some(&*fs), &names));
    let side = |name: &str, fleet: &[(&str, u32)], structures: Vec<battle_sim::ShipId>| SideSetup {
        name: name.into(),
        units: fleet
            .iter()
            .map(|&(n, c)| UnitSetup {
                ship: catalog.ship_id(n).unwrap_or_else(|| panic!("unknown ship {n}")),
                count: c,
                tag: 0,
            })
            .collect(),
        structures,
        ai: true,
    };
    let station_ids: Vec<battle_sim::ShipId> = station.as_deref().and_then(|n| catalog.ship_id(n)).into_iter().collect();
    let arena = battle_sim::Arena::load(&data, &*fs, &format!("{map}.TED")).map(Arc::new);
    let half = arena.as_ref().map_or(7000.0, |a| a.half_size());
    let config = BattleConfig {
        half_size: half,
        arena,
        seed,
        fog_of_war: std::env::var_os("SP_DEMO_NOFOG").is_none(),
        ..BattleConfig::default()
    };
    let setup = battle_sim::setup(
        catalog.clone(),
        vec![side("Empire", empire, vec![]), side("Rebel_Alliance", rebel, station_ids)],
        config,
    );
    let faction_color = |name: &str, fallback: Color| {
        data.faction(name)
            .and_then(|f| f.color)
            .map_or(fallback, |c| Color::srgb_u8(c[0], c[1], c[2]))
    };
    let colors = vec![
        faction_color("Empire", Color::srgb(0.2, 0.7, 0.4)),
        faction_color("Rebel_Alliance", Color::srgb(0.9, 0.2, 0.15)),
    ];
    let visuals = VisualCatalog::build(&data, &catalog, colors);

    let mut app = App::new();
    let mut settings = BattleViewSettings {
        speed,
        msaa_samples: msaa,
        bloom,
        ui_scale: scale,
        shadows,
        ..default()
    };
    if let Some(p) = std::env::var("SP_DEMO_PLAYER").ok().and_then(|v| v.parse::<u8>().ok()) {
        settings.player_side = p;
    }
    if let Some(q) = quality {
        settings.apply_quality(q);
    }
    match &offscreen {
        Some(out) => {
            let mut plugins = DefaultPlugins.set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            });
            if gpu_profile {
                plugins = plugins.set(battle_view::gpu_profile::timestamp_render_plugin());
            }
            app.add_plugins(plugins)
            .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::from_millis(1)))
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(STEP_MS)));
            let (w, h) = (size[0] as u32, size[1] as u32);
            let image = Image::new_target_texture(w, h, TextureFormat::Rgba8UnormSrgb, None);
            let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
            settings.render_target = Some(target.clone());
            settings.blocking = true;
            settings.synthetic_pointer = true;
            app.insert_resource(Capture {
                out: out.clone(),
                times: times.clone(),
                cams,
                next: 0,
                pending: 0,
                overlay,
                target,
                frame_ms: Vec::new(),
                last: Instant::now(),
                wall_start: Instant::now(),
                warm: 0,
                skip: 0,
                step_ms: Vec::new(),
                last_tick: 0,
                early_ms: Vec::new(),
                frames: 0,
                cam_applied: false,
                auto,
            })
            .add_systems(Last, capture_driver);
        }
        None => {
            app.add_plugins(DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Space Peace - battle view demo".into(),
                    resolution: (1920, 1080).into(),
                    ..default()
                }),
                ..default()
            }));
        }
    }
    if gpu_profile {
        app.add_plugins(battle_view::gpu_profile::GpuProfilePlugin);
    }
    let rocks: Vec<Vec3> = scene_rocks;
    app.insert_resource(DemoRocks(rocks))
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(settings)
        .add_plugins(AlamoRenderPlugin::new(fs.clone()))
        .add_plugins(BattleViewPlugin)
        .insert_resource(BattleSession::new(setup, visuals, scene));
    app.run();
}

/// Positions of the map's rocks and other props (for `SP_DEMO_MOVE`).
#[derive(Resource)]
struct DemoRocks(Vec<Vec3>);

/// Applies the camera pose, selects ships for the overlay test, takes screenshots and measures
/// frame times.
#[allow(clippy::too_many_arguments)]
fn capture_driver(
    mut commands: Commands,
    mut cap: ResMut<Capture>,
    status: Res<BattleStatus>,
    stats: Res<ParticleStats>,
    ships: Res<RenderedShips>,
    rt: Option<Res<battle_view::BattleRuntime>>,
    settings: Res<BattleViewSettings>,
    mut selection: ResMut<Selection>,
    mut pointer: ResMut<Pointer>,
    hover: Res<Hover>,
    cam: Option<Single<(&mut BattleCamera, &Camera, &GlobalTransform)>>,
    mesh_entities: Query<&ViewVisibility, With<Mesh3d>>,
    mut reinforced: Local<bool>,
    mut moved: Local<bool>,
    rocks: Res<DemoRocks>,
    named: Query<(&Name, &GlobalTransform)>,
) {
    let now = Instant::now();
    let dt = now.duration_since(cap.last).as_secs_f64() * 1000.0;
    cap.last = now;
    cap.frames += 1;
    if cap.frames <= 400 {
        cap.early_ms.push(dt);
    }
    let Some(rt) = rt else { return };
    if !*reinforced
        && let Some(at) = std::env::var("SP_DEMO_REINFORCE").ok().and_then(|v| v.parse::<f32>().ok())
        && let Some(cur) = rt.cur.as_ref()
        && cur.time >= at
        && let Some(s) = cur.ships.iter().find(|s| s.side == 0)
    {
        rt.command(0, battle_sim::Command::Reinforce { ship: s.def, count: 3 });
        *reinforced = true;
    }
    if !*moved
        && std::env::var_os("SP_DEMO_MOVE").is_some()
        && let Some(cur) = rt.cur.as_ref()
        && cur.time >= 2.0
    {
        // Send the player's capital ships through the nearest rock to see the move line bend around it.
        let player = settings.player_side;
        let own: Vec<_> = ships.0.values().filter(|s| s.side == player && rt.catalog.ship(s.def).radius > 200.0).collect();
        if !own.is_empty() {
            let c = own.iter().map(|s| s.pos).sum::<Vec3>() / own.len() as f32;
            let rock = rocks.0.iter().copied().filter(|r| r.distance(c) > 1500.0).min_by(|a, b| a.distance(c).total_cmp(&b.distance(c)));
            if let Some(r) = rock {
                let dest = c + (r - c) * 2.0;
                println!("demo move: fleet {c:?} rock {r:?} destination {dest:?}");
                let ids: Vec<u32> = own.iter().map(|s| s.id).collect();
                rt.command(player, battle_sim::Command::Move { ships: ids, to: battle_view::convert::bevy_to_sim(dest) });
            }
            *moved = true;
        }
    }
    if status.tick == 0 || ships.0.is_empty() {
        return;
    }
    let Some(mut cam) = cam else { return };
    let (bc, camera, gt) = &mut *cam;
    let idx = cap.next.min(cap.cams.len().saturating_sub(1));
    if let Some(c) = cap.cams.get(idx) {
        // yaw, pitch (deg), distance, [x, z]
        bc.snap = true;
        bc.target_yaw = c[0].to_radians();
        bc.pitch = c.get(1).copied().unwrap_or(50.0).to_radians();
        bc.target_distance = c.get(2).copied().unwrap_or(5000.0);
        if c.len() >= 5 {
            bc.target_focus = Vec3::new(c[3], 0.0, c[4]);
        }
        bc.min_distance = 100.0;
        cap.cam_applied = true;
    }
    if std::env::var_os("SP_DEMO_CLOSE").is_some() {
        // Close view of a damaged capital ship: one with a destroyed hardpoint, else the most worn shield.
        let pick = ships
            .0
            .values()
            .filter(|s| s.hp_alive.iter().any(|a| !a))
            .max_by(|a, b| a.radius.total_cmp(&b.radius))
            .or_else(|| ships.0.values().filter(|s| s.radius > 150.0).min_by(|a, b| a.shield.total_cmp(&b.shield)));
        if let Some(s) = pick {
            bc.snap = true;
            bc.target_focus = s.pos;
            bc.target_distance = (s.radius * 4.0).clamp(900.0, 3500.0);
        }
    }
    if std::env::var_os("SP_DEMO_FIGHT").is_some() {
        // Frame the line between the two fleets (a close view of the engagement).
        let centre = |side: u8| {
            let v: Vec<Vec3> = ships.0.values().filter(|s| s.side == side).map(|s| s.pos).collect();
            (!v.is_empty()).then(|| v.iter().sum::<Vec3>() / v.len() as f32)
        };
        if let (Some(a), Some(b)) = (centre(0), centre(1)) {
            bc.snap = true;
            bc.target_focus = (a + b) * 0.5;
            bc.target_distance = ((a - b).length() * 0.9).clamp(1800.0, 5000.0);
        }
    }
    if cap.auto {
        // Frame all living ships (a stand-in for the player scrolling to the action).
        let n = ships.0.len().max(1) as f32;
        let center = ships.0.values().map(|s| s.pos).sum::<Vec3>() / n;
        let radius = ships
            .0
            .values()
            .map(|s| (s.pos - center).length())
            .fold(0.0f32, f32::max);
        bc.snap = true;
        bc.target_focus = center;
        bc.target_distance = (radius * 1.7).clamp(1500.0, 16000.0);
    }
    // `SP_DEMO_SUPER`: the Death Star dish bone; `SP_DEMO_WRECK`: the first debris chunk of a dead ship.
    for (var, bone, dist) in [("SP_DEMO_SUPER", "p_planet_deathstar_laser_large", 3000.0), ("SP_DEMO_WRECK", "ship_debris_chunk", 1500.0)] {
        if std::env::var_os(var).is_some()
            && let Some((_, g)) = named.iter().find(|(n, _)| n.as_str().eq_ignore_ascii_case(bone))
        {
            bc.snap = true;
            bc.target_focus = g.translation();
            bc.bounds_min = Vec2::splat(-200_000.0);
            bc.bounds_max = Vec2::splat(200_000.0);
            bc.target_distance = dist;
        }
    }
    if let Some(want) = std::env::var("SP_DEMO_PROJ").ok()
        && let Some(cur) = rt.cur.as_ref()
        && let Some(p) = cur.projectiles.iter().find(|p| rt.catalog.projectiles.get(p.def as usize).is_some_and(|d| d.category.eq_ignore_ascii_case(&want)))
    {
        // Close view of the first projectile of the category (`missile`, `laser`, `rocket`).
        bc.snap = true;
        bc.target_focus = battle_view::convert::sim_to_bevy(p.pos);
        bc.target_distance = 300.0;
    }
    if std::env::var_os("SP_DEMO_DOGFIGHT").is_some() {
        // Closest pair of opposing fighters (radius under 100).
        let f: Vec<_> = ships.0.values().filter(|s| s.radius < 100.0).collect();
        let pair = f
            .iter()
            .flat_map(|a| f.iter().filter(|b| b.side != a.side).map(move |b| (a, b)))
            .min_by(|x, y| x.0.pos.distance(x.1.pos).total_cmp(&y.0.pos.distance(y.1.pos)));
        let lone = std::env::var_os("SP_DEMO_SQUAD").and_then(|_| f.iter().min_by_key(|s| s.id));
        if let Some(a) = lone {
            bc.snap = true;
            bc.target_focus = a.pos;
            bc.target_distance = 700.0;
        } else
        if let Some((a, b)) = pair {
            bc.snap = true;
            bc.target_focus = (a.pos + b.pos) * 0.5;
            bc.target_distance = 700.0;
        }
    }
    if let Some(pitch) = std::env::var("SP_DEMO_PITCH").ok().and_then(|v| v.parse::<f32>().ok()) {
        bc.pitch = pitch.to_radians();
    }
    if let Some(d) = std::env::var("SP_DEMO_DIST").ok().and_then(|v| v.parse::<f32>().ok()) {
        bc.target_distance = d;
    }
    if cap.overlay {
        // Select the player's capital ships and hover the biggest enemy ship.
        let player = settings.player_side;
        selection.ships = ships
            .0
            .iter()
            .filter(|(_, s)| s.side == player && rt.catalog.ship(s.def).radius > 200.0)
            .map(|(id, _)| *id)
            .collect();
        selection.ships.sort_unstable();
        let target = ships
            .0
            .values()
            .filter(|s| s.side != player)
            .max_by(|a, b| a.radius.total_cmp(&b.radius));
        if let Some(t) = target {
            println!("overlay target #{} hull {:.2} shield {:.2} radius {}", t.id, t.hull, t.shield, t.radius);
        }
        if let Some(t) = target
            && let Ok(p) = camera.world_to_viewport(gt, t.pos)
        {
            // Offset the cursor a little so a hardpoint is picked rather than the ship centre.
            pointer.cursor = Some(p + Vec2::new(30.0, 6.0));
        }
        if let Some(t) = target
            && let Some(d) = std::env::var("SP_DEMO_FOCUS_TARGET").ok().and_then(|v| v.parse::<f32>().ok())
        {
            // Close view of the hovered enemy (hardpoint marker checks).
            bc.snap = true;
            bc.target_focus = t.pos;
            bc.target_distance = d;
        }
        let _ = &hover;
    }

    // Frame time statistics start once the first step has been reached.
    cap.warm += 1;
    if status.tick != cap.last_tick {
        cap.last_tick = status.tick;
        cap.step_ms.push(status.step_ms);
    }
    if cap.skip > 0 {
        cap.skip -= 1;
    } else if cap.warm > 40 {
        cap.frame_ms.push(dt);
        if dt > 20.0 {
            println!(
                "hitch {dt:.1} ms at t={:.2}s tick {} alive {:?} particles {} batches {} step {:.2} ms",
                status.time, status.tick, status.alive, stats.live, stats.batches, status.step_ms
            );
        }
    }
    if cap.next < cap.times.len() && status.time >= cap.times[cap.next] && cap.warm > 30 {
        let t = cap.times[cap.next];
        let path = if cap.times.len() > 1 {
            format!("{}_t{:04}.png", cap.out.trim_end_matches(".png"), (t * 1000.0) as u32)
        } else {
            cap.out.clone()
        };
        cap.next += 1;
        cap.pending += 1;
        cap.skip = 4;
        println!(
            "capture t={:.1}s tick {} alive {:?} step {:.2} ms; {} particles in {} batches ({:.2} ms cpu); {} mesh entities, {} visible",
            status.time, status.tick, status.alive, status.step_ms, stats.live, stats.batches, stats.cpu_ms,
            mesh_entities.iter().count(),
            mesh_entities.iter().filter(|v| v.get()).count()
        );
        commands
            .spawn(Screenshot::image(cap.target.clone()))
            .observe(
                move |cap_ev: On<ScreenshotCaptured>,
                      mut exit: MessageWriter<AppExit>,
                      mut cap: ResMut<Capture>,
                      prof: Option<Res<battle_view::gpu_profile::GpuProfile>>| {
                    let img = &cap_ev.image;
                    let (w, h) = (img.width() as usize, img.height() as usize);
                    let mut px = img.data.clone().unwrap_or_default();
                    if img.texture_descriptor.format == TextureFormat::Bgra8UnormSrgb {
                        for p in px.as_chunks_mut::<4>().0.iter_mut() {
                            p.swap(0, 2);
                        }
                    }
                    std::fs::write(&path, encode_png(&Rgba8 { width: w, height: h, pixels: px }))
                        .expect("write png");
                    println!("wrote {path}");
                    cap.pending -= 1;
                    if cap.pending == 0 && cap.next >= cap.times.len() {
                        let mut ms = cap.frame_ms.clone();
                        ms.sort_by(f64::total_cmp);
                        if !ms.is_empty() {
                            let pct = |p: f64| ms[((ms.len() - 1) as f64 * p) as usize];
                            let mean = ms.iter().sum::<f64>() / ms.len() as f64;
                            println!(
                                "frame time over {} frames at {w}x{h}: mean {mean:.2} ms, p50 {:.2}, p95 {:.2}, p99 {:.2}, max {:.2} (excluding screenshot frames)",
                                ms.len(),
                                pct(0.5),
                                pct(0.95),
                                pct(0.99),
                                ms[ms.len() - 1]
                            );
                        }
                        if let Some(p) = prof {
                            print!("{}", p.summary());
                        }
                        {
                            let mut v: Vec<(usize, f64)> = cap.early_ms.iter().copied().enumerate().skip(1).collect();
                            v.sort_by(|a, b| b.1.total_cmp(&a.1));
                            let top: Vec<String> = v.iter().take(4).map(|(i, m)| format!("{m:.0} ms (frame {i})")).collect();
                            println!("longest of the first {} frames: {top:?}", cap.early_ms.len());
                        }
                        let mut st = cap.step_ms.clone();
                        st.sort_by(f32::total_cmp);
                        if !st.is_empty() {
                            let p = |q: f32| st[((st.len() - 1) as f32 * q) as usize];
                            println!(
                                "sim step over {} ticks: mean {:.2} ms, p50 {:.2}, p95 {:.2}, p99 {:.2}, max {:.2}",
                                st.len(),
                                st.iter().sum::<f32>() / st.len() as f32,
                                p(0.5),
                                p(0.95),
                                p(0.99),
                                st[st.len() - 1]
                            );
                        }
                        println!("wall time {:.1} s", cap.wall_start.elapsed().as_secs_f32());
                        exit.write(AppExit::Success);
                    }
                },
            );
    }
}





