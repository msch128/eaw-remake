//! Galaxy -> tactical -> galaxy round trip with the real renderer, offscreen (no window, no audio).
//!
//! A pending battle is created in `galaxy-sim`, turned into a `BattleSession` with
//! `BattleSession::from_request`, played at high speed with the AI in control of both sides, and the
//! `BattleFinished` result is applied back with `Sim::apply_tactical_result`. One screenshot (with the EaW
//! tactical bar when `--ui` is given) is taken at `--at` seconds of battle time.
//!
//! ```text
//! tactical_roundtrip --offscreen out.png [--at 25] [--speed 30] [--size 1280x720] [--ui]
//! ```

use std::sync::{Arc, Mutex};
use std::time::Duration;

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use alamo_model::image::{Rgba8, encode_png};
use alamo_render::AlamoRenderPlugin;
use battle_view::tactical_ui::TacticalUiPlugin;
use battle_view::{BattleCamera, BattleFinished, BattleSession, BattleStatus, BattleViewPlugin, BattleViewSettings, RenderedShips, Selection};
use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::time::TimeUpdateStrategy;
use bevy::window::ExitCondition;
use eaw_ui::{EawUiPlugin, EawUiSettings, UiScaleMode};
use galaxy_sim::{Event, FleetLocation, Sim, SimConfig};

#[derive(Resource)]
struct Galaxy(Mutex<Sim>);

#[derive(Resource)]
struct Shot {
    out: String,
    at: f32,
    target: Handle<Image>,
    requested: bool,
    written: bool,
    finished: bool,
    selected: bool,
    frames: u32,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let out = arg("--offscreen").expect("--offscreen <png>");
    let at: f32 = arg("--at").and_then(|v| v.parse().ok()).unwrap_or(25.0);
    let speed: f32 = arg("--speed").and_then(|v| v.parse().ok()).unwrap_or(30.0);
    let size: Vec<u32> = arg("--size").unwrap_or("1280x720".into()).split('x').filter_map(|v| v.parse().ok()).collect();
    let ui = args.iter().any(|a| a == "--ui");

    let install = GameInstall::detect().expect("Empire at War install");
    let fs = Arc::new(GameFs::for_mod(&install, Some("1770851727")).expect("game files"));
    let data = Arc::new(load_galaxy(&*fs).0);
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    let mut sim = Sim::new(&data, campaign, SimConfig { auto_resolve_battles: false, victory_enabled: false, ..SimConfig::default() }).expect("sim");

    // Two fleets meet over an unowned planet.
    let w = sim.world().clone();
    let (empire, rebel) = (w.faction_id("Empire").unwrap(), w.faction_id("Rebel").unwrap());
    let planet = (0..w.planets.len() as u32)
        .find(|&p| w.real[p as usize] && w.planets[p as usize].space_map.is_some() && sim.galaxy().planets[p as usize].owner.is_none() && sim.galaxy().fleets_at(p).next().is_none())
        .expect("empty planet");
    for (owner, unit, n) in [(empire, "Star_Destroyer", 2), (empire, "TIE_Fighter_Squadron", 2), (rebel, "MC-80_Star_Cruiser", 2), (rebel, "X-Wing_Squadron", 2)] {
        sim.spawn_units(owner, planet, w.unit_id(unit).unwrap_or_else(|| panic!("unit {unit}")), n);
    }
    let battle = sim
        .step(1.0)
        .into_iter()
        .find_map(|e| if let Event::BattlePending(b) = e { Some(b) } else { None })
        .expect("pending battle");
    let request = sim.tactical_setup(battle.id).expect("request");
    println!("battle {} at {} on map {:?}, sides: {:?}", request.battle_id, request.planet_name, request.map, request.sides.iter().map(|s| (&s.faction_name, s.groups.len())).collect::<Vec<_>>());

    let session = BattleSession::from_request(&data, &*fs, &request, None).expect("session");

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() }))
        .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::from_millis(1)))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(16)));
    let image = Image::new_target_texture(size[0], size[1], TextureFormat::Rgba8UnormSrgb, None);
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    let settings = BattleViewSettings {
        speed,
        render_target: Some(target.clone()),
        blocking: true,
        synthetic_pointer: true,
        audio: true, // hooks run, but no audio plugin is present: silent
        ..default()
    };
    app.insert_resource(ClearColor(Color::BLACK))
        .insert_resource(settings)
        .add_plugins(AlamoRenderPlugin::new(fs.clone()))
        .add_plugins(BattleViewPlugin);
    if ui {
        app.add_plugins(EawUiPlugin::new(fs.clone()).with_game_data(data.clone()).with_settings(EawUiSettings {
            scale: UiScaleMode::Fixed(size[1] as f32 / 768.0),
            ..default()
        }))
        .add_plugins(TacticalUiPlugin);
    }
    app.insert_resource(Galaxy(Mutex::new(sim)))
        .insert_resource(Shot { out, at, target, requested: false, written: false, finished: false, selected: false, frames: 0 })
        .insert_resource(session)
        .add_systems(Update, (ui_camera, drive))
        .run();
}

fn ui_camera(cams: Query<Entity, Added<BattleCamera>>, settings: Option<ResMut<EawUiSettings>>) {
    if let (Some(mut s), Some(e)) = (settings, cams.iter().next()) {
        s.camera = Some(e);
    }
}

#[allow(clippy::too_many_arguments)]
fn drive(
    mut commands: Commands,
    mut shot: ResMut<Shot>,
    status: Res<BattleStatus>,
    ships: Res<RenderedShips>,
    mut sel: ResMut<Selection>,
    mut cam: Option<Single<&mut BattleCamera>>,
    mut finished: MessageReader<BattleFinished>,
    galaxy: Res<Galaxy>,
    mut exit: MessageWriter<AppExit>,
) {
    if status.tick > 0 && !ships.0.is_empty() {
        if let Some(cam) = cam.as_mut() {
            let n = ships.0.len() as f32;
            let center = ships.0.values().map(|s| s.pos).sum::<Vec3>() / n;
            let radius = ships.0.values().map(|s| (s.pos - center).length()).fold(0.0f32, f32::max);
            cam.snap = true;
            cam.target_focus = center;
            cam.target_distance = (radius * 1.7).clamp(1500.0, 16000.0);
        }
        if !shot.selected {
            shot.selected = true;
            sel.ships = ships.0.iter().filter(|(_, s)| s.side == 0).map(|(id, _)| *id).collect();
            sel.ships.sort_unstable();
        }
    }
    shot.frames += 1;
    // Models and textures load asynchronously: wait a couple of hundred frames before the picture.
    if !shot.requested && status.time >= shot.at && shot.frames > 200 {
        shot.requested = true;
        println!("screenshot at t={:.1}s alive {:?}", status.time, status.alive);
        commands.spawn(Screenshot::image(shot.target.clone())).observe(|ev: On<ScreenshotCaptured>, mut shot: ResMut<Shot>| {
            let img = &ev.image;
            let mut px = img.data.clone().unwrap_or_default();
            if img.texture_descriptor.format == TextureFormat::Bgra8UnormSrgb {
                for p in px.as_chunks_mut::<4>().0.iter_mut() {
                    p.swap(0, 2);
                }
            }
            std::fs::write(&shot.out, encode_png(&Rgba8 { width: img.width() as usize, height: img.height() as usize, pixels: px })).expect("write png");
            println!("wrote {}", shot.out);
            shot.written = true;
        });
    }
    for f in finished.read() {
        let mut sim = galaxy.0.lock().unwrap();
        let events = sim.apply_tactical_result(f.battle_id, &f.result).expect("result applies");
        println!("battle over after {:.0}s, winner side {:?}, survivors {:?}", f.result.duration, f.result.winner, f.result.survived);
        for e in &events {
            println!("  {e:?}");
        }
        println!("pending battles left: {}, fleets now: {:?}", sim.galaxy().battles.len(), sim.galaxy().fleets.values().map(|fl| (fl.owner, fl.units.clone(), matches!(fl.location, FleetLocation::AtPlanet(_)))).collect::<Vec<_>>());
        shot.finished = true;
    }
    if shot.finished && (shot.written || !shot.requested) {
        exit.write(AppExit::Success);
    }
}
