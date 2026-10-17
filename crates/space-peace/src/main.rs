//! EAW:Remake - Empire at War galactic conquest shell.

// The installed game (`dist` feature, built by tools/package.ps1) is a windowed app: no console window opens behind it. Normal
// builds stay console apps, the smoke tests read their output.
#![cfg_attr(feature = "dist", windows_subsystem = "windows")]
#![allow(clippy::type_complexity, clippy::too_many_arguments)]

mod battle_play;
mod battle_seam;
mod campaign_select;
mod cheat_menu;
mod droid_menu;
mod drag_demo;
mod label_probe;
mod camera;
mod cli;
mod flow;
mod galaxy_map;
mod hero_heads;
mod ui_sounds;
mod planet_fleets;
mod planet_view;
mod galaxy_models;
mod input_test;
mod install_check;
mod galaxy_view;
mod game;
mod loading;
mod logfile;
mod menu;
mod menu_scene;
mod newgame;
mod objectives;
mod offscreen;
mod options;
mod planet_visual;
mod play;
mod play_ui;
mod settings;
mod ui;

use std::{sync::Arc, time::Duration};

use alamo_audio::AlamoAudioPlugin;
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

use battle_play::BattlePlayPlugin;
use battle_seam::BattleSeamPlugin;
use camera::CameraPlugin;
use cli::Args;
use flow::{AppScreen, FlowPlugin};
use galaxy_map::GalaxyMapPlugin;
use galaxy_view::GalaxyView;
use game::{Game, StartupLog};
use loading::{AfterLoading, GameFiles, InitialCampaign, LoadDelay, LoadingPlugin};
use menu::MenuPlugin;
use newgame::NewGamePlugin;
use options::OptionsPlugin;
use play::PlayGluePlugin;
use play_ui::PlayUiPlugin;
use settings::{Settings, SettingsPlugin};
use ui::UiPlugin;

fn main() {
    let args = Args::from_env();
    if args.check_install {
        // First of all: no saves folder, settings, window or GPU is touched; the installer reads the exit code.
        std::process::exit(install_check::run(&args));
    }
    if args.offscreen.is_some() {
        newgame::isolate_offscreen_saves();
    }

    if args.list_campaigns {
        if let Err(e) = game::list_campaigns(&args) {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return;
    }

    // Opening the file system is fast; the data itself is parsed in the background while the
    // loading screen is shown. Without an install the demo galaxy is used.
    let mut log = StartupLog::default();
    let mut problem = game::StartupProblem::default();
    let mut content = game::ContentChoice { active: game::Content::from_mod_arg(args.mod_id.as_deref()), ..default() };
    let fs = if args.demo {
        log.info("using the built-in demo galaxy (--demo)".into());
        None
    } else {
        match game::open_fs(&args, &mut log) {
            Ok((install, fs)) => {
                content.clone_wars_available = game::clone_wars_installed(&install);
                Some(fs)
            }
            Err(e) => {
                log.warn(format!("FALLING BACK TO DEMO GALAXY: {e}"));
                problem.0 = Some(game::describe_problem(&e, args.mod_id.as_deref().unwrap_or(game::DEFAULT_MOD_ID)));
                None
            }
        }
    };

    // Screen shown once loading is done: the menu, unless a campaign (or a screen) was requested.
    let after = match args.screen.as_deref() {
        _ if args.via_menu => AppScreen::MainMenu,
        Some("campaigns") => AppScreen::CampaignSelect,
        Some("galaxy" | "pause") => AppScreen::Galaxy,
        Some("menu" | "loading" | "options") => AppScreen::MainMenu,
        _ if args.campaign.is_some() || args.demo || args.offscreen.is_some() => AppScreen::Galaxy,
        _ => AppScreen::MainMenu,
    };
    let mut loading_delay = args.loading_delay;
    if args.screen.as_deref() == Some("loading") && loading_delay <= 0.0 {
        loading_delay = 8.0;
    }

    let settings = Settings::load().with_cli(&args);
    let mode = if settings.borderless {
        WindowMode::BorderlessFullscreen(MonitorSelection::Primary)
    } else {
        WindowMode::Windowed
    };
    let present_mode = if settings.vsync { PresentMode::AutoVsync } else { PresentMode::AutoNoVsync };

    let mut app = App::new();
    // A system whose parameter is missing must never take the whole game down: log it and carry on.
    app.set_error_handler(bevy::ecs::error::warn);
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
        let mut plugins = DefaultPlugins.set(bevy::log::LogPlugin { custom_layer: logfile::file_layer, ..default() }).set(WindowPlugin {
            primary_window: Some(Window {
                title: "EAW:Remake".into(),
                visible: !args.hidden,
                present_mode,
                mode,
                resolution: settings.resolution.into(),
                ..default()
            }),
            ..default()
        });
        if args.audio_off() {
            // No output stream is opened (it can crackle under CPU load); the audio logic runs against an empty device.
            plugins = plugins.disable::<AudioPlugin>();
        }
        app.add_plugins(plugins);
        if args.audio_off() {
            app.init_asset::<bevy::audio::AudioSource>().insert_resource(alamo_audio::NoAudioDevice);
            info!("audio: device not opened (--no-audio / --volume 0)");
        }
    }
    if let Some(fs) = &fs {
        app.add_plugins(AlamoRenderPlugin::new(fs.clone()));
        app.add_plugins(eaw_ui::EawUiPlugin::new(fs.clone()));
        app.add_plugins((battle_view::BattleViewPlugin, battle_view::tactical_ui::TacticalUiPlugin));
        // An offscreen run has no audio device; with `SP_AUDIO_LEDGER` / `SP_AUDIO_LOG` it still runs the audio logic against an
        // empty device so every sound event of the scripted run is logged and checked against the file system.
        let audio_audit = std::env::var_os("SP_AUDIO_LEDGER").is_some() || std::env::var_os("SP_AUDIO_LOG").is_some();
        if offscreen.is_some() && audio_audit {
            app.init_asset::<bevy::audio::AudioSource>().insert_resource(alamo_audio::NoAudioDevice);
        }
        if offscreen.is_none() || audio_audit {
            app.add_plugins(AlamoAudioPlugin::new(fs.clone() as Arc<dyn FileSource>));
        }
    }
    if (args.hidden || args.exit_after.is_some()) && offscreen.is_none() {
        let frames = args.exit_after.unwrap_or(args.frames);
        app.add_systems(Last, move |mut n: Local<u32>, mut exit: MessageWriter<bevy::app::AppExit>| {
            *n += 1;
            if *n >= frames {
                info!("exit-after: {frames} frames reached");
                exit.write(bevy::app::AppExit::Success);
            }
        });
    }
    app.insert_state(AppScreen::Loading)
        .insert_resource(AfterLoading(after))
        .insert_resource(GameFiles(fs))
        .insert_resource(InitialCampaign(args.campaign.clone()))
        .insert_resource(LoadDelay(loading_delay))
        .insert_resource(flow::Paused(args.screen.as_deref() == Some("pause")))
        .insert_resource(options::OptionsOpen(args.screen.as_deref() == Some("options")))
        .insert_resource(settings)
        .insert_resource(content)
        .insert_resource(newgame::NewGameOptions::from_args(&args))
        .insert_resource(args)
        .insert_resource(log)
        .insert_resource(Game(None))
        .insert_resource(problem)
        .insert_resource(GalaxyView::default())
        .add_systems(Startup, game::replay_startup_log)
        .add_plugins(input_test::plugin)
        .add_plugins(hero_heads::HeroHeadsPlugin)
        .add_plugins(ui_sounds::UiSoundsPlugin)
        .add_plugins(planet_view::PlanetViewPlugin)
        .add_plugins(planet_fleets::plugin)
        .add_plugins(cheat_menu::plugin)
        .add_plugins(droid_menu::plugin)
        .add_plugins(galaxy_models::GalaxyModelsPlugin)
        .add_plugins(drag_demo::DragDemoPlugin)
        .add_plugins(label_probe::LabelProbePlugin)
        .add_plugins((
            FlowPlugin,
            LoadingPlugin,
            MenuPlugin,
            campaign_select::CampaignSelectPlugin,
            menu_scene::MenuScenePlugin,
            NewGamePlugin,
            OptionsPlugin,
            SettingsPlugin,
            BattleSeamPlugin,
            BattlePlayPlugin,
            CameraPlugin,
            GalaxyMapPlugin,
            UiPlugin,
            PlayGluePlugin,
            PlayUiPlugin,
        ))
        .run();
}
