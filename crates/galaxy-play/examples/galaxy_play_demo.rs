//! Headless demo of the playable galactic map.
//!
//! ```text
//! galaxy_play_demo --offscreen <out.png> [options]
//!   --weeks N          simulate N galactic weeks at high speed before the capture (default 20)
//!   --human            the campaign's active player is human: scripted orders (build + move) are
//!                      given at the start, everybody else is AI (default: all factions are AI)
//!   --stop-on-battle   with --human: capture as soon as a battle waits for the player
//!   --campaign NAME    campaign (default GC_Full_Web_Way_Rebel)
//!   --focus PLANET     frame this planet (object or display name) closely
//!   --zoom F           camera distance multiplier (default 1)
//!   --size WxH         image size (default 1920x1080)
//!   --seed N           simulation seed
//! ```
//! A minimal map (planet models, lanes, owner rings) is drawn here only so the fleets have
//! something to sit on; the real map belongs to the `space-peace` app.

use std::f32::consts::FRAC_PI_2;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use alamo_model::image::{Rgba8, encode_png};
use alamo_render::{AlamoModel, AlamoModelLoaded, AlamoRenderPlugin, spawn_sun};
use bevy::app::{AppExit, ScheduleRunnerPlugin};
use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::time::TimeUpdateStrategy;
use bevy::window::ExitCondition;
use galaxy_play::{
    BuildCategory, BuildMenu, BuildTarget, GalaxyLayout, GalaxyPlayPlugin, GalaxyPlayUiPlugin, PlayAction, PlayConfig, PlayInit, PlayUiModel,
    PlayerFaction, PlaySet, SimState,
};
use galaxy_sim::{PlanetId, World};

const FRAME_MS: u64 = 50;

struct Args {
    out: String,
    weeks: u64,
    human: bool,
    stop_on_battle: bool,
    campaign: String,
    focus: Option<String>,
    zoom: f32,
    size: (u32, u32),
    seed: u64,
}

fn parse_args() -> Args {
    let mut a = Args {
        out: String::new(),
        weeks: 20,
        human: false,
        stop_on_battle: false,
        campaign: "GC_Full_Web_Way_Rebel".into(),
        focus: None,
        zoom: 1.0,
        size: (1920, 1080),
        seed: 1,
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut it = args.iter();
    while let Some(k) = it.next() {
        let mut val = || it.next().cloned().unwrap_or_default();
        match k.as_str() {
            "--offscreen" => a.out = val(),
            "--weeks" => a.weeks = val().parse().unwrap_or(20),
            "--human" => a.human = true,
            "--stop-on-battle" => a.stop_on_battle = true,
            "--campaign" => a.campaign = val(),
            "--focus" => a.focus = Some(val()),
            "--zoom" => a.zoom = val().parse().unwrap_or(1.0),
            "--seed" => a.seed = val().parse().unwrap_or(1),
            "--size" => {
                let v = val();
                if let Some((w, h)) = v.split_once('x') {
                    a.size = (w.parse().unwrap_or(1920), h.parse().unwrap_or(1080));
                }
            }
            other => eprintln!("ignoring unknown argument {other}"),
        }
    }
    if a.out.is_empty() {
        eprintln!("usage: galaxy_play_demo --offscreen <out.png> [--weeks N] [--human] [--focus PLANET] [--zoom F]");
        std::process::exit(2);
    }
    a
}

#[derive(Resource)]
struct Demo {
    args: Args,
    target: Handle<Image>,
    /// Frame at which the capture phase started.
    settle_from: Option<u32>,
    settle_started: Option<Instant>,
    frame: u32,
    scripted: bool,
    noted_hold: bool,
    shot_requested: bool,
    started: Instant,
    max_work_micros: u64,
    last_seq: u64,
    snapshots: u64,
    focus: Option<PlanetId>,
    planet_models: Vec<Option<(String, f32)>>,
}

#[derive(Component)]
struct PlanetTag {
    id: PlanetId,
    owner: Option<u32>,
}

#[derive(Component)]
struct PlanetLabel(PlanetId);

fn main() {
    let args = parse_args();
    let install = GameInstall::detect().expect("Empire at War install not found");
    let fs = Arc::new(GameFs::for_mod(&install, Some("1770851727")).expect("cannot open game files"));
    let t = Instant::now();
    let (data, _) = load_galaxy(fs.as_ref());
    let campaign = data.campaign(&args.campaign).expect("campaign not found");
    println!("data loaded in {:.2?}", t.elapsed());

    let config = PlayConfig {
        human_is_ai: !args.human,
        initial_speed_level: 0,
        sim: galaxy_sim::SimConfig { seed: args.seed, victory_enabled: false, ..Default::default() },
        ..Default::default()
    };
    let t = Instant::now();
    let init = PlayInit::from_data(&data, campaign, config).expect("cannot build simulation");
    let world = init.sim.world().clone();
    println!(
        "sim built in {:.2?}: {} planets, {} routes, {} factions, {} unit types",
        t.elapsed(),
        world.planets.len(),
        world.routes.len(),
        world.factions.len(),
        world.units.len()
    );
    let planet_models: Vec<Option<(String, f32)>> = world
        .planets
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let d = data.planet(&p.name)?;
            let m = d.galactic_model.clone().filter(|m| !m.to_ascii_lowercase().contains("invisible"))?;
            (world.real[i] && !p.name.to_ascii_lowercase().contains("dummy")).then(|| (m, if d.scale_factor > 0.0 { d.scale_factor } else { 1.0 }))
        })
        .collect();
    let focus = args.focus.as_ref().and_then(|f| {
        world
            .planets
            .iter()
            .position(|p| p.name.eq_ignore_ascii_case(f) || p.display_name.eq_ignore_ascii_case(f))
            .map(|i| i as PlanetId)
    });

    let (w, h) = args.size;
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() }))
        .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::ZERO))
        .add_plugins(AlamoRenderPlugin::new(fs))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(FRAME_MS)))
        .insert_resource(ClearColor(Color::srgb(0.0, 0.0, 0.015)))
        .insert_resource(GlobalAmbientLight { color: Color::srgb(0.55, 0.62, 0.85), brightness: 220.0, ..default() });
    let mut image = Image::new_uninit(Extent3d { width: w, height: h, depth_or_array_layers: 1 }, TextureDimension::D2, TextureFormat::Rgba8UnormSrgb, default());
    image.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC | TextureUsages::RENDER_ATTACHMENT;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    app.add_plugins((GalaxyPlayPlugin::new(init), GalaxyPlayUiPlugin))
        .insert_resource(Demo {
            args,
            target,
            settle_from: None,
            settle_started: None,
            frame: 0,
            scripted: false,
            noted_hold: false,
            shot_requested: false,
            started: Instant::now(),
            max_work_micros: 0,
            last_seq: 0,
            snapshots: 0,
            focus,
            planet_models,
        })
        .add_systems(Startup, setup)
        .add_systems(Update, (drive.after(PlaySet::Ui), planets, labels).chain());
    app.run();
}

fn setup(mut commands: Commands, demo: Res<Demo>, state: Res<SimState>, mut layout: ResMut<GalaxyLayout>) {
    let world = &state.world;
    spawn_sun(&mut commands, Vec3::new(-0.45, -0.7, -0.55), 0.75, Color::WHITE);
    for (i, m) in demo.planet_models.iter().enumerate() {
        if let Some((model, scale)) = m {
            commands.spawn((
                AlamoModel::new(model.clone()),
                Transform::from_translation(layout.positions[i]).with_scale(Vec3::splat(*scale)),
                PlanetTag { id: i as PlanetId, owner: None },
            ));
            commands.spawn((
                Text::new(world.planets[i].display_name.clone()),
                TextFont { font_size: FontSize::Px(13.0), ..default() },
                TextColor(Color::srgb(0.85, 0.9, 1.0)),
                Node { position_type: PositionType::Absolute, ..default() },
                Visibility::Hidden,
                PlanetLabel(i as PlanetId),
            ));
        }
    }
    // Camera framing the map (or the focused planet).
    let visible: Vec<Vec3> = demo.planet_models.iter().enumerate().filter(|(_, m)| m.is_some()).map(|(i, _)| layout.positions[i]).collect();
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for p in &visible {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    let center = (lo + hi) * 0.5;
    let extent = (hi - lo).max_element().max(layout.unit * 10.0);
    let (focus, dist) = match demo.focus {
        Some(p) => (layout.positions[p as usize], layout.unit * 22.0 * demo.args.zoom),
        None => (Vec3::new(center.x, 0.0, center.z), extent * 1.5 * demo.args.zoom),
    };
    let rot = Quat::from_rotation_x(-58f32.to_radians());
    commands.spawn((
        Camera3d::default(),
        IsDefaultUiCamera,
        bevy::camera::Hdr,
        RenderTarget::Image(demo.target.clone().into()),
        Tonemapping::None,
        Projection::Perspective(PerspectiveProjection { fov: 40f32.to_radians(), near: 1.0, far: 20_000.0, ..default() }),
        Transform { rotation: rot, translation: focus + rot * Vec3::Z * dist, ..default() },
    ));
    let _ = &mut layout;
}

/// Planet colours, radii, lanes and owner rings.
#[allow(clippy::too_many_arguments)]
fn planets(
    mut gizmos: Gizmos,
    state: Res<SimState>,
    mut layout: ResMut<GalaxyLayout>,
    mut q: ParamSet<(Query<(&mut PlanetTag, &mut AlamoModel)>, Query<(&PlanetTag, &AlamoModelLoaded, &Transform), Added<AlamoModelLoaded>>)>,
) {
    let world: &World = &state.world;
    let g = state.galaxy();
    for (tag, m, tf) in &q.p1() {
        layout.radii[tag.id as usize] = m.radius * tf.scale.x;
    }
    for (mut tag, mut model) in &mut q.p0() {
        let owner = g.planets[tag.id as usize].owner;
        if tag.owner != owner || (owner.is_none() && model.team_color == Color::WHITE) {
            tag.owner = owner;
            model.team_color = owner.map_or(Color::srgb(0.6, 0.62, 0.68), |o| color(world, o));
        }
    }
    for r in &world.routes {
        let c = match r.hs_factor {
            f if f >= 2.5 => Color::srgb(0.15, 0.55, 0.7),
            f if f >= 1.5 => Color::srgb(0.1, 0.38, 0.5),
            _ => Color::srgb(0.07, 0.24, 0.32),
        };
        gizmos.line(layout.positions[r.a as usize], layout.positions[r.b as usize], c);
    }
    for (i, ps) in g.planets.iter().enumerate() {
        if let Some(o) = ps.owner
            && state_visible(&layout, i)
        {
            gizmos.circle(Isometry3d::new(layout.positions[i], Quat::from_rotation_x(-FRAC_PI_2)), layout.radii[i] * 1.2, color(world, o));
        }
    }
}

fn state_visible(layout: &GalaxyLayout, i: usize) -> bool {
    layout.positions[i].length_squared() < 1.0e12
}

fn color(world: &World, f: u32) -> Color {
    let c = world.factions[f as usize].color;
    Color::srgb_u8(c[0], c[1], c[2])
}

/// Names of the richest owned planets (and the focus) so the map is readable.
fn labels(
    state: Res<SimState>,
    demo: Res<Demo>,
    camera: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    layout: Res<GalaxyLayout>,
    mut labels: Query<(&PlanetLabel, &mut Node, &mut Visibility)>,
) {
    let Some((camera, cam_tf)) = camera.iter().next() else { return };
    let world = &state.world;
    let g = state.galaxy();
    let mut rich: Vec<(PlanetId, f32)> = world.planets.iter().enumerate().map(|(i, p)| (i as PlanetId, p.credit_value)).collect();
    rich.sort_by(|a, b| b.1.total_cmp(&a.1));
    let top: Vec<PlanetId> = rich.iter().take(if demo.focus.is_some() { 60 } else { 22 }).map(|x| x.0).collect();
    for (l, mut node, mut vis) in &mut labels {
        let show = top.contains(&l.0) && g.planets[l.0 as usize].owner.is_some();
        match camera.world_to_viewport(cam_tf, layout.positions[l.0 as usize] + Vec3::new(0.0, 0.0, layout.radii[l.0 as usize] * 2.2)) {
            Ok(p) if show => {
                node.left = Val::Px(p.x - 20.0);
                node.top = Val::Px(p.y);
                *vis = Visibility::Inherited;
            }
            _ => *vis = Visibility::Hidden,
        }
    }
}

/// Runs the scenario: scripted orders, fast-forward, settle, capture.
#[allow(clippy::too_many_arguments)]
fn drive(
    mut commands: Commands,
    mut demo: ResMut<Demo>,
    state: Res<SimState>,
    player: Res<PlayerFaction>,
    model: Res<PlayUiModel>,
    models: Query<(Has<AlamoModelLoaded>, Has<alamo_render::model::AlamoLoadFailed>), With<AlamoModel>>,
    mut actions: MessageWriter<PlayAction>,
    mut exit: MessageWriter<AppExit>,
) {
    demo.frame += 1;
    if state.current.seq != demo.last_seq {
        demo.last_seq = state.current.seq;
        demo.snapshots += 1;
        demo.max_work_micros = demo.max_work_micros.max(state.current.work_micros);
    }
    let world = &state.world;
    let g = state.galaxy();

    if !demo.scripted {
        demo.scripted = true;
        if let Some(h) = player.0 {
            script_orders(&state, h, &mut actions);
        }
        // One galactic week per 50 ms frame: the worker is the bottleneck, not the frame rate.
        if demo.args.weeks > 0 {
            actions.write(PlayAction::SetSpeedMultiplier(world.config.week_seconds / (FRAME_MS as f32 / 1000.0)));
        }
        return;
    }

    let held = model.battle.is_some();
    let stalled = demo.started.elapsed() > Duration::from_secs(40);
    if held && !demo.args.stop_on_battle && demo.settle_from.is_none() && !demo.noted_hold {
        demo.noted_hold = true;
        println!("note: the game is held by a battle prompt at week {}", state.current.week);
    }
    let done_sim = state.current.week >= demo.args.weeks || g.winner.is_some() || held && (demo.args.stop_on_battle || stalled) || stalled;
    if demo.settle_from.is_none() && done_sim {
        demo.settle_from = Some(demo.frame);
        demo.settle_started = Some(Instant::now());
        actions.write(PlayAction::SetSpeedMultiplier(0.0));
        println!(
            "simulated {} weeks ({:.0} galactic s) in {:.2?} wall time, {} snapshots, worst batch {:.1} ms",
            state.current.week,
            state.current.time,
            demo.started.elapsed(),
            demo.snapshots,
            demo.max_work_micros as f64 / 1000.0
        );
        summary(&state);
    }
    let Some(from) = demo.settle_from else { return };
    let all_loaded = models.iter().all(|(l, f)| l || f);
    if demo.frame == from + 40 {
        println!("models: {} total, {} loaded, {} failed", models.iter().count(), models.iter().filter(|m| m.0).count(), models.iter().filter(|m| m.1).count());
    }
    // Pipelines compile asynchronously: give the renderer a few real seconds before the capture.
    let waited = demo.settle_started.is_some_and(|s| s.elapsed() > Duration::from_secs(5));
    if !demo.shot_requested && demo.frame > from + 40 && waited && all_loaded {
        demo.shot_requested = true;
        let out = demo.args.out.clone();
        commands.spawn(Screenshot::image(demo.target.clone())).observe(move |cap: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
            let img = &cap.image;
            let mut px = img.data.clone().unwrap_or_default();
            if img.texture_descriptor.format == TextureFormat::Bgra8UnormSrgb {
                for p in px.as_chunks_mut::<4>().0.iter_mut() {
                    p.swap(0, 2);
                }
            }
            let png = encode_png(&Rgba8 { width: img.width() as usize, height: img.height() as usize, pixels: px });
            std::fs::write(&out, png).expect("write png");
            println!("wrote {out}");
            exit.write(AppExit::Success);
        });
    }
    if demo.settle_started.is_some_and(|s| s.elapsed() > Duration::from_secs(60)) {
        eprintln!("capture never arrived");
        exit.write(AppExit::error());
    }
}

/// Builds a few things at the home planet and sends the biggest fleet two hops away.
fn script_orders(state: &SimState, human: u32, actions: &mut MessageWriter<PlayAction>) {
    let world = &state.world;
    let g = state.galaxy();
    let names = galaxy_play::PlayCatalog::plain(world);
    let home = world.factions[human as usize].home.or_else(|| g.owned_planets(human).next());
    let Some(home) = home else { return };
    if let Some(menu) = BuildMenu::for_planet(world, g, &names, human, home) {
        let mut credits = g.factions[human as usize].credits;
        let mut picked = 0;
        for o in menu.options.iter().filter(|o| o.availability.is_available()) {
            let wanted = matches!(o.target, BuildTarget::Unit(_)) && matches!(o.category, BuildCategory::Frigates | BuildCategory::Capitals | BuildCategory::Corvettes);
            if wanted && o.cost as f64 <= credits && picked < 4 {
                credits -= o.cost as f64;
                picked += 1;
                actions.write(PlayAction::Build { planet: home, target: o.target });
            }
        }
        if let Some(o) = menu.options.iter().find(|o| o.category == BuildCategory::Starbase && o.availability.is_available()) {
            actions.write(PlayAction::Build { planet: home, target: o.target });
        }
    }
    actions.write(PlayAction::SelectPlanet(Some(home)));
    // Biggest idle fleet moves two lanes away.
    let fleet = g.fleets.values().filter(|f| f.owner == human && f.planet().is_some()).max_by_key(|f| f.unit_count());
    if let Some(f) = fleet {
        let at = f.planet().expect("filtered");
        // A quiet planet (own or unowned, no base) within a few lanes.
        let quiet = |p: u32| {
            let ps = &g.planets[p as usize];
            world.real[p as usize] && ps.starbase.is_none() && ps.owner.is_none_or(|o| o == human) && g.fleets_at(p).all(|f| f.owner == human)
        };
        let dest = (0..world.planets.len() as u32)
            .filter(|&p| p != at && quiet(p))
            .filter_map(|p| Some((p, world.find_path(at, p)?.len())))
            .filter(|&(_, hops)| (3..=5).contains(&hops))
            .map(|(p, _)| p)
            .next();
        if let Some(d) = dest {
            actions.write(PlayAction::SelectFleets(vec![f.id]));
            actions.write(PlayAction::MoveSelected(d));
        }
    }
}

fn summary(state: &SimState) {
    let w = &state.world;
    let g = state.galaxy();
    let in_transit = g.fleets.values().filter(|f| f.planet().is_none()).count();
    println!("fleets {} ({} in hyperspace), ships {}", g.fleets.len(), in_transit, g.fleets.values().map(|f| f.unit_count()).sum::<u32>());
    for (i, f) in w.factions.iter().enumerate() {
        let s = &g.factions[i];
        if s.planets_owned > 0 {
            println!("  {:<14} planets {:>3} credits {:>9.0} income/wk {:>7.0} pop {}/{}", f.name, s.planets_owned, s.credits, s.income_per_week, s.population_used, s.population_cap);
        }
    }
}









