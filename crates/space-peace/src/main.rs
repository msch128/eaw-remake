//! Space Peace: Not at War - Empire at War galactic conquest shell.

mod camera;
mod cli;
mod galaxy_map;
mod galaxy_view;
mod game;
mod music;
mod ui;

use bevy::{
    prelude::*,
    window::{MonitorSelection, PresentMode, WindowMode},
};

use camera::CameraPlugin;
use cli::Args;
use galaxy_map::GalaxyMapPlugin;
use galaxy_view::GalaxyView;
use music::{MusicPlugin, MusicSourceSlot};
use ui::UiPlugin;

fn main() {
    let args = Args::from_env();

    if args.list_campaigns {
        if let Err(e) = game::list_campaigns(&args) {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return;
    }

    // Real game data when an install is found, demo galaxy otherwise.
    let mut log = game::StartupLog::default();
    let (view, music) = if args.demo {
        log.info("using the built-in demo galaxy (--demo)".into());
        (GalaxyView::demo(), MusicSourceSlot::default())
    } else {
        match game::load(&args, &mut log) {
            Ok(g) => (g.view, MusicSourceSlot::new(g.music)),
            Err(e) => {
                log.warn(format!("FALLING BACK TO DEMO GALAXY: {e}"));
                (GalaxyView::demo(), MusicSourceSlot::default())
            }
        }
    };

    let present_mode = if args.no_vsync {
        PresentMode::AutoNoVsync
    } else {
        PresentMode::AutoVsync
    };
    let mode = if args.fullscreen {
        WindowMode::BorderlessFullscreen(MonitorSelection::Primary)
    } else {
        WindowMode::Windowed
    };

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Space Peace: Not at War".into(),
                present_mode,
                mode,
                resolution: (1600, 900).into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(args)
        .insert_resource(log)
        .add_systems(Startup, game::replay_startup_log)
        .insert_resource(view)
        .insert_resource(music)
        .add_plugins((CameraPlugin, GalaxyMapPlugin, MusicPlugin, UiPlugin))
        .run();
}
