//! Renders the Empire at War command bar offscreen (no window) to a PNG.
//!
//! ```text
//! ui_demo --offscreen <out.png> [--faction Rebel|Empire|Underworld] [--mode galactic|space]
//!         [--size WxH] [--hover button_name] [--tooltip button_name] [--select button_name]
//! ```
//! Without `--offscreen` this opens a normal window with the same content.

use std::sync::Arc;
use std::time::Duration;

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use alamo_model::image::{Rgba8, encode_png};
use bevy::app::ScheduleRunnerPlugin;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::time::TimeUpdateStrategy;
use bevy::window::ExitCondition;
use eaw_ui::*;

#[derive(Resource)]
struct Shot {
    out: String,
    target: Handle<Image>,
    frame: u32,
    loaded: bool,
    spec: CommandBarSpec,
    hover: Vec<String>,
    select: Vec<String>,
    data: CommandBarData,
    panels: Vec<String>,
    panels_done: bool,
    game: Arc<alamo_data::GameData>,
    faction: UiFaction,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let all = |k: &str| -> Vec<String> {
        args.iter()
            .enumerate()
            .filter(|(_, a)| *a == k)
            .filter_map(|(i, _)| args.get(i + 1).cloned())
            .collect()
    };
    let out = arg("--offscreen");
    let faction = UiFaction::from_name(&arg("--faction").unwrap_or_else(|| "Empire".into()));
    let mode = match arg("--mode").as_deref() {
        Some("space" | "tactical") => CommandBarMode::SpaceTactical,
        _ => CommandBarMode::Galactic,
    };
    let (w, h) = arg("--size")
        .and_then(|s| s.split_once('x').map(|(a, b)| (a.parse().ok(), b.parse().ok())))
        .and_then(|(a, b)| Some((a?, b?)))
        .unwrap_or((1920u32, 1080u32));

    let install = GameInstall::detect().expect("Empire at War install not found");
    let fs = Arc::new(GameFs::for_mod(&install, Some("1770851727")).expect("file system"));
    let (game, _) = load_galaxy(&*fs);
    let game = Arc::new(game);

    let settings = EawUiSettings {
        scale: if out.is_some() {
            UiScaleMode::Fixed(h as f32 / 768.0)
        } else {
            UiScaleMode::Auto
        },
        faceplate_oversample: (h as f32 / 768.0).ceil().max(2.0),
        font_family: arg("--font").map_or(Some("Arial".into()), |f| (f != "none").then_some(f)),
        ..default()
    };
    let data = sample_data(&game, faction);

    let mut app = App::new();
    if out.is_some() {
        app.add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: None,
            exit_condition: ExitCondition::DontExit,
            ..default()
        }))
        .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::from_millis(1)))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(16)));
    } else {
        app.add_plugins(DefaultPlugins);
    }
    app.add_plugins(EawUiPlugin::new(fs).with_game_data(game.clone()).with_settings(settings))
        .insert_resource(ClearColor(Color::srgb(0.02, 0.025, 0.04)));

    let spec = CommandBarSpec { mode, faction };
    if let Some(out) = out {
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
        image.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING
            | TextureUsages::COPY_SRC
            | TextureUsages::RENDER_ATTACHMENT;
        let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
        let cam = app
            .world_mut()
            .spawn((Camera2d, RenderTarget::Image(target.clone().into())))
            .id();
        app.world_mut().resource_mut::<EawUiSettings>().camera = Some(cam);
        app.insert_resource(Shot {
            out,
            target,
            frame: 0,
            loaded: false,
            spec,
            hover: all("--hover"),
            select: all("--select"),
            data,
            panels: all("--panel"),
            panels_done: false,
            game: game.clone(),
            faction,
        });
        if let Some(t) = arg("--tooltip") {
            app.insert_resource(ForcedTooltip(Some(t)));
        }
        app.add_systems(Update, drive);
    } else {
        app.insert_resource(data);
        app.add_systems(Startup, move |mut c: Commands| {
            c.spawn(Camera2d);
            c.spawn_eaw_command_bar(spec);
        });
        app.add_systems(Update, |mut ev: MessageReader<EawButtonClicked>| {
            for e in ev.read() {
                println!("clicked {}", e.name);
            }
        });
    }
    app.run();
}

fn sample_data(game: &alamo_data::GameData, faction: UiFaction) -> CommandBarData {
    let want = faction.name().to_lowercase();
    let mut icons: Vec<String> = game
        .objects
        .iter()
        .filter(|o| {
            ["Squadron", "SpaceUnit", "UniqueUnit"].iter().any(|k| o.kind.eq_ignore_ascii_case(k))
                && !o.name.to_lowercase().contains("template")
        })
        .filter(|o| {
            game.objects
                .resolve(&o.name)
                .and_then(|r| r.get("affiliation").map(|a| a.to_lowercase()))
                .is_some_and(|a| a.contains(&want) || (want == "underworld" && a.contains("underworld")))
        })
        .filter_map(|o| unit_icon(game, &o.name))
        .filter(|i| !i.is_empty())
        .collect();
    icons.dedup();
    println!("{} sample icons, first: {:?}", icons.len(), icons.first());
    let pick = |i: usize| icons.get(i % icons.len().max(1)).cloned().unwrap_or_default();
    CommandBarData {
        credits: 12_450,
        planet_income: 350,
        planet_name: "Coruscant".into(),
        planet_owner: Some(faction),
        day: 2,
        month_income: 4_634,
        month_progress: 0.4,
        planet_color: Some(Color::srgb(0.3, 1.0, 0.4)),
        tech_level: 3,
        pop_current: 42,
        pop_cap: 100,
        space_tab: true,
        queue: (0..4)
            .map(|i| QueueEntry {
                icon: pick(i * 3),
                progress: if i == 0 { 0.6 } else { 0.0 },
                count: if i == 1 { 3 } else { 1 },
                tooltip: String::new(),
            })
            .collect(),
        pads: (0..16)
            .map(|i| BuildPadEntry {
                icon: pick(i),
                price: Some(150 + 25 * i as u32),
                disabled: i % 5 == 4,
                building: false,
                tooltip: String::new(),
            })
            .collect(),
        selection: (0..6)
            .map(|i| SelectionEntry {
                icon: pick(i + 1),
                health: 1.0 - i as f32 * 0.15,
                shield: (i % 2 == 0).then_some(0.8 - i as f32 * 0.1),
                count: 1 + i as u32 % 3,
            })
            .collect(),
        abilities: Vec::new(),
    }
}

fn drive(
    mut commands: Commands,
    mut shot: ResMut<Shot>,
    mut buttons: Query<(&EawButton, &mut EawButtonState)>,
    mut exit: MessageWriter<AppExit>,
    mut loaded: MessageReader<UiTexturesLoaded>,
) {
    if loaded.read().next().is_some() {
        shot.loaded = true;
    }
    if shot.frame == 0 {
        // Requested immediately: the bar waits for the async texture decode by itself.
        let spec = shot.spec;
        commands.insert_resource(shot.data.clone());
        if shot.panels.is_empty() || shot.panels.iter().any(|p| p.starts_with("minimap") || p == "tactical") {
            commands.spawn_eaw_command_bar(spec);
        }
        shot.frame = 1;
        return;
    }
    if !shot.loaded {
        return;
    }
    if !shot.panels_done {
        shot.panels_done = true;
        let (panels, game, faction) = (shot.panels.clone(), shot.game.clone(), shot.faction);
        commands.queue(move |world: &mut World| {
            for p in &panels {
                spawn_panel(world, p, faction, &game);
            }
        });
    }
    shot.frame += 1;
    if shot.frame == 6 {
        for (b, mut st) in &mut buttons {
            st.force_hover = shot.hover.iter().any(|h| h.eq_ignore_ascii_case(&b.name));
            st.selected |= shot.select.iter().any(|h| h.eq_ignore_ascii_case(&b.name));
        }
    }
    if shot.frame == 40 {
        let path = shot.out.clone();
        commands
            .spawn(Screenshot::image(shot.target.clone()))
            .observe(move |cap: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                let img = &cap.image;
                let mut px = img.data.clone().unwrap_or_default();
                if img.texture_descriptor.format == TextureFormat::Bgra8UnormSrgb {
                    for p in px.as_chunks_mut::<4>().0.iter_mut() {
                        p.swap(0, 2);
                    }
                }
                let png = encode_png(&Rgba8 {
                    width: img.width() as usize,
                    height: img.height() as usize,
                    pixels: px,
                });
                std::fs::write(&path, png).expect("write png");
                println!("wrote {path}");
                exit.write(AppExit::Success);
            });
    }
    let _ = &mut exit;
}






#[allow(clippy::drop_non_drop)]
/// Sample content for each panel of `--panel <name>`.
fn spawn_panel(world: &mut World, name: &str, faction: UiFaction, game: &alamo_data::GameData) {
    let icons: Vec<String> = {
        let want = faction.name().to_lowercase();
        let mut v: Vec<String> = game
            .objects
            .iter()
            .filter(|o| ["Squadron", "UniqueUnit", "SpaceUnit"].iter().any(|k| o.kind.eq_ignore_ascii_case(k)))
            .filter(|o| !o.name.to_lowercase().contains("template"))
            .filter(|o| {
                game.objects
                    .resolve(&o.name)
                    .and_then(|r| r.get("affiliation").map(str::to_lowercase))
                    .is_some_and(|a| a.contains(&want))
            })
            .filter_map(|o| unit_icon(game, &o.name))
            .collect();
        v.dedup();
        v
    };
    let icon = |i: usize| icons.get(i % icons.len().max(1)).cloned().unwrap_or_default();
    let structures: Vec<String> = game
        .objects
        .of_kind("GroundBuildable")
        .filter_map(|o| unit_icon(game, &o.name))
        .collect();
    let mut data = world.resource_mut::<PanelData>();
    match name {
        "planet" => {
            let p = game.planet("Coruscant").or_else(|| game.planets.first());
            let (income, structs) = p.map_or((0, 0), |p| (p.credit_value, p.max_ground_base));
            data.set_text("planet_name_text", "Coruscant");
            data.set_text("planetary_income_number", income.to_string());
            data.set_text("unit_cap_text", game.text("TEXT_UNIT_CAP").unwrap_or("Unit Cap"));
            data.set_text("unit_cap_number", "12/40");
            data.set_text("building_cap_text", game.text("TEXT_BUILDING_CAP").unwrap_or("Building Cap"));
            data.set_text("building_cap_number", format!("3/{structs}"));
            data.set_text("day_counter_text", "Day 37");
                        drop(data);
            spawn_shell(world, &ShellSpec::new("I_Planetary_Mode.ALO", faction));
        }
        "fleets" => {
            for fleet in 1..=3u32 {
                for k in 0..(5 * fleet as usize) {
                    let (row, col) = (k as u32 / 7, k as u32 % 7);
                    data.set_slot(
                        &fleet_slot_name(fleet, row, col),
                        PanelSlot {
                            icon: icon(k + fleet as usize * 3),
                            label: if k % 3 == 0 { "x4".into() } else { String::new() },
                            selected: fleet == 2 && k == 1,
                            ..default()
                        },
                    );
                }
            }
            for i in 0..6 {
                data.set_slot(
                    &format!("planet_structure_{i:02}"),
                    PanelSlot { icon: structures.get(i).cloned().unwrap_or_default(), ..default() },
                );
            }
            data.set_text("text_fleet1", "Fleet 1");
            data.set_text("text_fleet2", "Fleet 2");
            data.set_text("text_fleet3", "Fleet 3");
            drop(data);
            spawn_shell(world, &ShellSpec::new("I_Galactic_Organize_New.ALO", faction));
        }
        "reinforce" => {
            for k in 0..9u32 {
                data.set_slot(
                    &format!("r_{:02}{:02}", k / 4, k % 4),
                    PanelSlot { icon: icon(k as usize), label: (100 + k * 25).to_string(), ..default() },
                );
            }
            data.set_text("r_pop_text", "42/100");
            drop(data);
            let mut spec = ShellSpec::new("I_Main_Reinforce.ALO", faction);
            // `reinforcement_window_ALTn` is the frame for n+1 rows.
            spec.art_alt = Some(2);
            spec.anchor = ShellAnchor::TopLeft;
            spec.offset = Vec2::new(30.0, -60.0);
            spawn_shell(world, &spec);
        }
        "hero" => {
            let side = if faction == UiFaction::Rebel { "good" } else { "evil" };
            for k in 0..5usize {
                data.set_slot(
                    &format!("hero_slot_{side}{k:02}"),
                    PanelSlot { icon: icon(k * 5), ..default() },
                );
                data.bars.insert(format!("hero_hb_{side}{k:02}"), 1.0 - k as f32 * 0.2);
            }
            drop(data);
            let model = if faction == UiFaction::Rebel {
                "I_Main_Hero_Frame_Rebel.ALO"
            } else {
                "I_Main_Hero_Frame_Empire.ALO"
            };
            let mut spec = ShellSpec::new(model, faction);
            // `rebel_hero_frame_ALTn` is the frame for n+1 hero slots.
            spec.art_alt = Some(4);
            spec.anchor = if faction == UiFaction::Rebel { ShellAnchor::TopRight } else { ShellAnchor::TopLeft };
            spec.offset = Vec2::new(0.0, -20.0);
            spawn_shell(world, &spec);
        }
        "battle" => {
            drop(data);
            let side = |stacks, tint| eaw_ui::BannerSide { stacks, icon: Some(icon(0)), tint };
            spawn_battle_banner(
                world,
                &eaw_ui::BattleBannerSpec {
                    title: "BESTINE: SPACE BATTLE IMMINENT!".into(),
                    faction,
                    begin: "BEGIN".into(),
                    auto: "AUTO-RESOLVE".into(),
                    left: side(2, Color::srgb(0.3, 1.0, 0.4)),
                    right: side(3, Color::srgb(1.0, 0.25, 0.25)),
                },
            );
        }
        "dialog" => {
            drop(data);
            spawn_dialog(
                world,
                &DialogSpec {
                    title: "Advisor".into(),
                    text: "Commander, the enemy fleet has been sighted above Kuat. We should reinforce the orbital defenses before they strike.".into(),
                    portrait: None,
                    faction,
                    buttons: vec![],
                },
            );
        }
        "minimap-galactic" | "minimap-tactical" => {
            let mut m = MinimapData::default();
            if name == "minimap-galactic" {
                let owners = game
                    .campaigns
                    .iter()
                    .find(|c| c.has_galaxy())
                    .map(|c| game.campaign_planet_owners(c))
                    .unwrap_or_default();
                let planets = game.planets.iter().filter(|p| owners.contains_key(&p.name.to_lowercase()) || owners.contains_key(&p.name)).collect::<Vec<_>>();
                let planets = if planets.is_empty() { game.planets.iter().collect() } else { planets };
                let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
                for p in &planets {
                    lo = lo.min(Vec2::new(p.position[0], p.position[1]));
                    hi = hi.max(Vec2::new(p.position[0], p.position[1]));
                }
                m.bounds = Some((lo - Vec2::splat(30.0), hi + Vec2::splat(30.0)));
                for p in planets {
                    let owner = owners.get(&p.name.to_lowercase()).or_else(|| owners.get(&p.name)).map(|s| s.to_lowercase());
                    let color = match owner.as_deref() {
                        Some("rebel") => Color::srgb(1.0, 0.25, 0.2),
                        Some("empire") => Color::srgb(0.3, 0.55, 1.0),
                        Some("underworld") => Color::srgb(1.0, 0.75, 0.2),
                        _ => Color::srgb(0.6, 0.6, 0.6),
                    };
                    m.planets.push(MinimapDot {
                        pos: Vec2::new(p.position[0], p.position[1]),
                        color,
                        selected: p.name.starts_with("Coruscant"),
                    });
                }
                let c = (lo + hi) * 0.5;
                m.camera = Some((c - Vec2::new(120.0, 70.0), c + Vec2::new(120.0, 70.0)));
            } else {
                m.bounds = Some((Vec2::new(-3000.0, -2000.0), Vec2::new(3000.0, 2000.0)));
                for i in 0..60 {
                    let a = i as f32 * 0.7;
                    let side = i % 2 == 0;
                    m.units.push(MinimapDot {
                        pos: Vec2::new(if side { -1800.0 } else { 1800.0 } + a.cos() * 700.0, a.sin() * 900.0 + i as f32 * 6.0 - 180.0),
                        color: if side { Color::srgb(0.3, 0.6, 1.0) } else { Color::srgb(1.0, 0.3, 0.25) },
                        selected: false,
                    });
                }
                m.camera = Some((Vec2::new(-900.0, -500.0), Vec2::new(300.0, 400.0)));
            }
            drop(data);
            world.insert_resource(m);
        }
        "tactical" => {
            drop(data);
            let want = faction.name().to_lowercase();
            let mut units: Vec<TacticalUnit> = {
                let assets = world.resource::<EawUiAssets>();
                let tex = world.resource::<textures::UiTextures>();
                let mut v: Vec<TacticalUnit> = game
                    .objects
                    .space_units()
                    .iter()
                    .filter(|u| u.affiliation.iter().any(|a| a.to_lowercase().contains(&want)))
                    .filter(|u| !u.name.to_lowercase().contains("template") && u.icon.is_some())
                    .filter_map(|u| TacticalUnit::from_game(game, &assets.text, tex, &u.name))
                    .collect();
                v.sort_by_key(|u| std::cmp::Reverse(u.abilities.len()));
                v.dedup_by(|a, b| a.icon == b.icon);
                v.truncate(7);
                v
            };
            for (i, u) in units.iter_mut().enumerate() {
                u.health = 1.0 - i as f32 * 0.11;
                u.shield = u.shield.map(|_| 0.9 - i as f32 * 0.12);
                u.count = 1 + (i as u32 * 2) % 5;
                for (k, a) in u.abilities.iter_mut().enumerate() {
                    if i == 0 {
                        a.cooldown = [0.0, 0.6, 0.0, 0.25][k % 4];
                        a.active = k == 2;
                        a.disabled = k == 3;
                    }
                }
            }
            let hardpoints = (0..6)
                .map(|i| HardpointMarker {
                    id: format!("HP_{i:02}"),
                    kind: ["WEAPONS_TURBO", "WEAPONS_ION", "ENGINES", "SHIELD_GEN", "DOCKING_BAY", "GENERIC"][i].into(),
                    tracked: i == 1,
                    screen_pos: Vec2::new(1100.0 + (i % 3) as f32 * 70.0, 330.0 + (i / 3) as f32 * 60.0),
                    health: 1.0 - i as f32 * 0.17,
                    targetable: i != 2,
                    destroyed: i == 5,
                })
                .collect();
            let names: Vec<String> = units.iter().map(|u| u.name.clone()).collect();
            let ship = |id: u64, x: f32, y: f32, hs: f32, h: f32, sel: bool, enemy: bool| ShipOverlay {
                name: Some(if enemy { "Enemy Capital Ship".to_string() } else { names[(id as usize - 1) % names.len().max(1)].clone() }),
                id,
                screen_pos: Vec2::new(x, y),
                half_size: hs,
                health: h,
                shield: Some(h * 0.8),
                selected: sel,
                hovered: false,
                enemy,
                always: false,
            };
            world.insert_resource(TacticalData {
                active: true,
                selection: units,
                focus: 0,
                reinforcement_points: Some(3),
                pop: Some((42, 100)),
                retreat: RetreatState::Countdown(45.0),
            });
            world.insert_resource(TacticalOverlays {
                ships: vec![
                    ship(1, 520.0, 400.0, 46.0, 0.9, true, false),
                    ship(2, 700.0, 250.0, 30.0, 0.5, true, false),
                    ship(3, 300.0, 300.0, 22.0, 0.25, true, false),
                    ship(4, 1170.0, 360.0, 90.0, 0.7, false, true),
                ],
                hardpoints,
            });
            world.resource_mut::<ForcedTooltip>().0 = Some("special_button_00".into());
        }
        other => eprintln!("unknown panel {other}"),
    }
}







