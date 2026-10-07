//! Space Peace: Not at War - Empire at War galactic conquest shell.

mod camera;
mod cli;
mod flow;
mod galaxy_map;
mod galaxy_view;
mod game;
mod menu;
mod offscreen;
mod planet_visual;
mod ui;

use std::{sync::Arc, time::Duration};

use alamo_audio::{AlamoAudioPlugin, AudioBuses};
use alamo_fs::FileSource;
use alamo_render::AlamoRenderPlugin;
use bevy::{
    app::ScheduleRunnerPlugin,
    audio::AudioPlugin,
    prelude::*,
    render::diagnostic::RenderDiagnosticsPlugin,
    window::{ExitCondition, MonitorSelection, PresentMode, WindowMode},
    winit::WinitPlugin,
};

use camera::CameraPlugin;
use cli::Args;
use flow::{AppScreen, CurrentCampaign, FlowPlugin};
use galaxy_map::GalaxyMapPlugin;
use galaxy_view::GalaxyView;
use game::{DEFAULT_CAMPAIGN, DEMO_CAMPAIGN, Game, StartupLog};
use menu::{Campaigns, MenuPlugin};
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
    let mut log = StartupLog::default();
    let assets = if args.demo {
        log.info("using the built-in demo galaxy (--demo)".into());
        None
    } else {
        match game::load_assets(&args, &mut log) {
            Ok(a) => Some(a),
            Err(e) => {
                log.warn(format!("FALLING BACK TO DEMO GALAXY: {e}"));
                None
            }
        }
    };
    let mut campaigns = assets
        .as_ref()
        .map(|a| game::campaign_entries(&a.data))
        .unwrap_or_default();
    if campaigns.is_empty() {
        campaigns.push(game::demo_entry());
    }

    // Start screen: the menu, unless a campaign (or a screen) was requested explicitly.
    let start = match args.screen.as_deref() {
        Some("menu") => AppScreen::MainMenu,
        Some("campaigns") => AppScreen::CampaignSelect,
        Some("galaxy" | "pause") => AppScreen::Galaxy,
        _ if args.campaign.is_some() || args.demo || args.offscreen.is_some() => AppScreen::Galaxy,
        _ => AppScreen::MainMenu,
    };
    // The galaxy to show when starting directly on the map.
    let mut current = CurrentCampaign::default();
    let mut view = GalaxyView::default();
    if start == AppScreen::Galaxy {
        let name = args.campaign.clone().unwrap_or_else(|| {
            if assets.is_some() { DEFAULT_CAMPAIGN.to_owned() } else { DEMO_CAMPAIGN.to_owned() }
        });
        let built = match &assets {
            Some(a) => game::build_campaign(a, &name, &mut log),
            None => Ok((GalaxyView::demo(), Some("Rebel".to_owned()))),
        };
        match built {
            Ok((v, faction)) => {
                view = v;
                current = CurrentCampaign { name: Some(name), faction };
            }
            Err(e) => {
                log.warn(format!("FALLING BACK TO DEMO GALAXY: {e}"));
                view = GalaxyView::demo();
                current = CurrentCampaign {
                    name: Some(DEMO_CAMPAIGN.into()),
                    faction: Some("Rebel".into()),
                };
            }
        }
    }

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

    let mut app = App::new();
    let offscreen = args.offscreen.clone();
    if let Some(path) = &offscreen {
        // Headless: no window, no winit event loop, no audio device; a schedule runner drives
        // the app.
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .disable::<WinitPlugin>()
                .disable::<AudioPlugin>(),
        )
        .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::ZERO))
        .add_plugins(RenderDiagnosticsPlugin);
        offscreen::configure(&mut app, &args, path.clone());
    } else {
        app.add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Space Peace: Not at War".into(),
                present_mode,
                mode,
                resolution: (1600, 900).into(),
                ..default()
            }),
            ..default()
        }));
    }
    if let Some(a) = &assets {
        app.add_plugins(AlamoRenderPlugin::new(a.fs.clone()));
        if offscreen.is_none() {
            app.add_plugins(AlamoAudioPlugin::new(a.fs.clone() as Arc<dyn FileSource>));
            app.insert_resource(AudioBuses { master: args.music_volume, ..default() });
        }
    }
    app.insert_resource(flow::Paused(args.screen.as_deref() == Some("pause")))
        .insert_state(start)
        .insert_resource(args)
        .insert_resource(log)
        .insert_resource(Game(assets))
        .insert_resource(Campaigns(campaigns))
        .insert_resource(view)
        .insert_resource(current)
        .add_systems(Startup, game::replay_startup_log)
        .add_plugins((FlowPlugin, MenuPlugin, CameraPlugin, GalaxyMapPlugin, UiPlugin))
        .run();
}
