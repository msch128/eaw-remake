//! Simple built-in Bevy UI for the galactic game.
//!
//! This is a placeholder look behind a clean seam: it only reads [`PlayUiModel`] and sends
//! [`PlayAction`]s. A different UI (the `eaw-ui` crate) replaces it by not adding
//! [`GalaxyPlayUiPlugin`] and doing the same.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use bevy::prelude::*;
use galaxy_sim::Lane;

use crate::build_menu::{Availability, BuildCategory, BuildMenu};
use crate::geometry::format_duration;
use crate::input::PlayUiBlock;
use crate::notify::NotificationKind;
use crate::state::*;

/// Simple panels: top bar with speed buttons, planet build menu, fleet panel, notification
/// ticker and the battle prompt.
pub struct GalaxyPlayUiPlugin;

impl Plugin for GalaxyPlayUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UiState>()
            .add_systems(Startup, spawn_panels)
            .add_systems(Update, (buttons, refresh_top_bar, refresh_planet_panel, refresh_fleet_panel, refresh_notifications, refresh_battle, update_dynamic));
    }
}

const PANEL_BG: Color = Color::srgba(0.02, 0.05, 0.10, 0.85);
const PANEL_BORDER: Color = Color::srgba(0.35, 0.65, 1.0, 0.8);
const BTN: Color = Color::srgba(0.12, 0.25, 0.42, 0.95);
const BTN_ACTIVE: Color = Color::srgba(0.20, 0.45, 0.75, 1.0);
const ROWS_PER_PAGE: usize = 14;

/// UI-only state.
#[derive(Resource, Default)]
struct UiState {
    page: usize,
    signatures: [u64; 5],
}

#[derive(Component)]
struct TopBar;
#[derive(Component)]
struct PlanetPanel;
#[derive(Component)]
struct FleetPanel;
#[derive(Component)]
struct Ticker;
#[derive(Component)]
struct BattlePanel;
#[derive(Component)]
struct RouteHint;

/// A button that sends an action when pressed.
#[derive(Component, Clone)]
struct UiButton(Action);

#[derive(Clone)]
enum Action {
    Play(PlayAction),
    Page(i32),
}

/// Text that follows the model every frame.
#[derive(Component, Clone, Copy)]
enum Dynamic {
    Credits,
    Pop,
    Planets,
    Week,
    QueueBar(usize),
    Hint,
}

fn panel(position: Node) -> impl Bundle {
    (
        Node { padding: UiRect::all(Val::Px(10.0)), border: UiRect::all(Val::Px(1.0)), row_gap: Val::Px(3.0), ..position },
        BackgroundColor(PANEL_BG),
        BorderColor::all(PANEL_BORDER),
        PlayUiBlock,
    )
}

fn spawn_panels(mut commands: Commands) {
    let abs = Node { position_type: PositionType::Absolute, ..default() };
    commands.spawn((panel(Node { top: Val::Px(8.0), left: Val::Px(8.0), flex_direction: FlexDirection::Row, column_gap: Val::Px(18.0), align_items: AlignItems::Center, ..abs.clone() }), TopBar));
    commands.spawn((panel(Node { top: Val::Px(60.0), right: Val::Px(8.0), width: Val::Px(360.0), flex_direction: FlexDirection::Column, ..abs.clone() }), PlanetPanel, Visibility::Hidden));
    commands.spawn((panel(Node { bottom: Val::Px(40.0), left: Val::Percent(30.0), width: Val::Percent(40.0), flex_direction: FlexDirection::Column, ..abs.clone() }), FleetPanel, Visibility::Hidden));
    commands.spawn((
        Node { top: Val::Px(70.0), left: Val::Px(8.0), width: Val::Px(420.0), flex_direction: FlexDirection::Column, row_gap: Val::Px(2.0), ..abs.clone() },
        Ticker,
    ));
    commands.spawn((
        panel(Node { top: Val::Percent(30.0), left: Val::Percent(35.0), width: Val::Percent(30.0), row_gap: Val::Px(8.0), flex_direction: FlexDirection::Column, ..abs.clone() }),
        GlobalZIndex(20),
        BattlePanel,
        Visibility::Hidden,
    ));
    commands.spawn((
        Text::new(""),
        TextFont { font_size: FontSize::Px(14.0), ..default() },
        TextColor(Color::srgb(0.7, 1.0, 0.8)),
        Node { position_type: PositionType::Absolute, bottom: Val::Px(14.0), left: Val::Percent(30.0), ..default() },
        Dynamic::Hint,
        RouteHint,
    ));
}

fn text(s: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    (Text::new(s.into()), TextFont { font_size: FontSize::Px(size), ..default() }, TextColor(color))
}

fn button(label: impl Into<String>, action: Action, active: bool) -> impl Bundle {
    (
        Button,
        Node { padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)), ..default() },
        BackgroundColor(if active { BTN_ACTIVE } else { BTN }),
        UiButton(action),
        children![text(label, 14.0, Color::WHITE)],
    )
}

fn thousands(n: i64) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

fn sig<T: Hash>(v: &T) -> u64 {
    let mut h = DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

#[allow(clippy::type_complexity)]
fn buttons(
    q: Query<(&Interaction, &UiButton), Changed<Interaction>>,
    mut actions: MessageWriter<PlayAction>,
    mut ui: ResMut<UiState>,
) {
    for (i, b) in &q {
        if *i == Interaction::Pressed {
            match &b.0 {
                Action::Play(a) => {
                    actions.write(a.clone());
                }
                Action::Page(d) => {
                    ui.page = (ui.page as i32 + d).max(0) as usize;
                    ui.signatures[1] = 0;
                }
            }
        }
    }
}

fn refresh_top_bar(model: Res<PlayUiModel>, mut ui: ResMut<UiState>, bar: Single<Entity, With<TopBar>>, mut commands: Commands) {
    let s = sig(&(model.speed.level, model.speed.speeds.iter().map(|x| x.to_bits()).collect::<Vec<_>>(), model.top.faction.clone(), model.speed.held_by_battle));
    if ui.signatures[0] == s {
        return;
    }
    ui.signatures[0] = s;
    let c = model.top.color;
    commands.entity(*bar).despawn_related::<Children>().with_children(|p| {
        p.spawn(text(model.top.faction.clone(), 17.0, Color::srgb(c[0], c[1], c[2])));
        p.spawn((text("", 16.0, Color::srgb(1.0, 0.9, 0.5)), Dynamic::Credits));
        p.spawn((text("", 16.0, Color::srgb(0.8, 0.9, 1.0)), Dynamic::Pop));
        p.spawn((text("", 16.0, Color::srgb(0.8, 0.9, 1.0)), Dynamic::Planets));
        p.spawn((text("", 16.0, Color::WHITE), Dynamic::Week));
        p.spawn(button("||", Action::Play(PlayAction::SetSpeedLevel(0)), model.speed.level == 0));
        for (i, s) in model.speed.speeds.iter().enumerate() {
            p.spawn(button(format!("{s:.0}x"), Action::Play(PlayAction::SetSpeedLevel(i + 1)), model.speed.level == i + 1));
        }
        if model.speed.held_by_battle {
            p.spawn(text("(decision pending)", 14.0, Color::srgb(1.0, 0.5, 0.4)));
        }
    });
}

fn update_dynamic(model: Res<PlayUiModel>, mut q: Query<(&Dynamic, Option<&mut Text>, Option<&mut Node>)>) {
    for (d, text, node) in &mut q {
        let t = &model.top;
        match (d, text, node) {
            (Dynamic::Credits, Some(mut x), _) => x.0 = format!("Credits {} ({:+}/wk)", thousands(t.credits as i64), t.income_per_week as i64),
            (Dynamic::Pop, Some(mut x), _) => x.0 = format!("Pop {}/{}", t.population_used, t.population_cap),
            (Dynamic::Planets, Some(mut x), _) => x.0 = format!("Planets {}", t.planets),
            (Dynamic::Week, Some(mut x), _) => {
                x.0 = match &t.winner {
                    Some(w) => format!("{w} wins!"),
                    None => format!("Week {}", t.week + 1),
                }
            }
            (Dynamic::Hint, Some(mut x), _) => x.0 = model.route_hint.clone().unwrap_or_default(),
            (Dynamic::QueueBar(i), _, Some(mut n)) => {
                if let Some(e) = model.planet.as_ref().and_then(|m| m.queue.get(*i)) {
                    n.width = Val::Percent(e.progress * 100.0);
                }
            }
            _ => {}
        }
    }
}

enum Row<'a> {
    Header(BuildCategory),
    Item(&'a crate::build_menu::BuildOption),
}

fn rows(menu: &BuildMenu) -> Vec<Row<'_>> {
    let mut v = Vec::new();
    let mut last = None;
    for o in &menu.options {
        if last != Some(o.category) {
            v.push(Row::Header(o.category));
            last = Some(o.category);
        }
        v.push(Row::Item(o));
    }
    v
}

fn refresh_planet_panel(
    model: Res<PlayUiModel>,
    mut ui: ResMut<UiState>,
    panel: Single<(Entity, &mut Visibility), With<PlanetPanel>>,
    mut commands: Commands,
) {
    let (root, mut vis) = panel.into_inner();
    let Some(menu) = &model.planet else {
        match &model.other_planet {
            Some((name, owner)) => {
                *vis = Visibility::Inherited;
                let s = sig(&(name.clone(), owner.clone()));
                if ui.signatures[1] != s {
                    ui.signatures[1] = s;
                    commands.entity(root).despawn_related::<Children>().with_children(|p| {
                        p.spawn(text(name.clone(), 20.0, Color::WHITE));
                        p.spawn(text(format!("Owner: {}", owner.as_deref().unwrap_or("Neutral")), 14.0, Color::srgb(0.8, 0.85, 0.9)));
                    });
                }
            }
            None => {
                *vis = Visibility::Hidden;
                ui.signatures[1] = 0;
            }
        }
        return;
    };
    *vis = Visibility::Inherited;
    let all = rows(menu);
    let pages = all.len().div_ceil(ROWS_PER_PAGE).max(1);
    ui.page = ui.page.min(pages - 1);
    let page = ui.page;
    let s = sig(&(
        menu.planet,
        page,
        menu.options.iter().map(|o| (o.target, o.availability.clone())).collect::<Vec<_>>(),
        menu.queue.iter().map(|q| (q.target, q.lane as u8)).collect::<Vec<_>>(),
        menu.starbase_level,
        menu.structures.len(),
    ));
    if ui.signatures[1] == s {
        return;
    }
    ui.signatures[1] = s;
    let week = 60.0;
    commands.entity(root).despawn_related::<Children>().with_children(|p| {
        p.spawn(text(menu.planet_name.clone(), 20.0, Color::WHITE));
        p.spawn(text(
            format!(
                "Income {}/wk | Base: {} | Orbit {}/{} | Ground {}/{}",
                menu.income as i64,
                menu.starbase_name.as_deref().unwrap_or("none"),
                menu.space_slots.0,
                menu.space_slots.1,
                menu.ground_slots.0,
                menu.ground_slots.1
            ),
            13.0,
            Color::srgb(0.75, 0.85, 1.0),
        ));
        // Queue.
        if !menu.queue.is_empty() {
            p.spawn(text("Production", 15.0, Color::srgb(1.0, 0.9, 0.5)));
            for (i, q) in menu.queue.iter().enumerate() {
                let lane = match q.lane {
                    Lane::Units => "U",
                    Lane::Structures => "S",
                    Lane::Base => "B",
                };
                p.spawn((Node { flex_direction: FlexDirection::Column, ..default() }, children![
                    text(format!("[{lane}] {}", q.name), 13.0, if q.active { Color::WHITE } else { Color::srgb(0.6, 0.65, 0.7) }),
                    (
                        Node { height: Val::Px(4.0), width: Val::Percent(0.0), ..default() },
                        BackgroundColor(Color::srgb(0.3, 0.8, 0.5)),
                        Dynamic::QueueBar(i),
                    ),
                ]));
            }
        }
        // Menu page.
        let end = ((page + 1) * ROWS_PER_PAGE).min(all.len());
        for row in &all[page * ROWS_PER_PAGE..end] {
            match row {
                Row::Header(c) => {
                    p.spawn(text(c.title(), 15.0, Color::srgb(0.55, 0.85, 1.0)));
                }
                Row::Item(o) => {
                    let label = format!("{}   {}  {}", o.name, thousands(o.cost as i64), format_duration(o.build_seconds, week));
                    if o.availability.is_available() {
                        p.spawn(button(label, Action::Play(PlayAction::Build { planet: menu.planet, target: o.target }), false));
                    } else {
                        let dim = if matches!(o.availability, Availability::NeedCredits) { Color::srgb(0.75, 0.7, 0.5) } else { Color::srgb(0.5, 0.52, 0.55) };
                        p.spawn(text(format!("{label}  ({})", o.availability.reason()), 13.0, dim));
                    }
                }
            }
        }
        if pages > 1 {
            p.spawn(Node { column_gap: Val::Px(8.0), align_items: AlignItems::Center, ..default() }).with_children(|r| {
                r.spawn(button("<", Action::Page(-1), false));
                r.spawn(text(format!("{}/{}", page + 1, pages), 13.0, Color::WHITE));
                r.spawn(button(">", Action::Page(1), false));
            });
        }
    });
}

fn refresh_fleet_panel(
    model: Res<PlayUiModel>,
    mut ui: ResMut<UiState>,
    panel: Single<(Entity, &mut Visibility), With<FleetPanel>>,
    mut commands: Commands,
) {
    let (root, mut vis) = panel.into_inner();
    if model.fleets.is_empty() {
        *vis = Visibility::Hidden;
        ui.signatures[2] = 0;
        return;
    }
    *vis = Visibility::Inherited;
    let s = sig(&model.fleets.iter().map(|f| (f.id, f.count, f.at.clone(), f.heading.as_ref().map(|h| (h.0.clone(), (h.1 / 6.0) as i64)))).collect::<Vec<_>>());
    if ui.signatures[2] == s {
        return;
    }
    ui.signatures[2] = s;
    commands.entity(root).despawn_related::<Children>().with_children(|p| {
        for f in model.fleets.iter().take(5) {
            let place = match (&f.at, &f.heading) {
                (Some(a), _) => format!("at {a}"),
                (_, Some((d, eta))) => format!("to {d}, ETA {}", format_duration(*eta, 60.0)),
                _ => String::new(),
            };
            let ships: Vec<String> = f.ships.iter().take(4).map(|(n, c)| if *c > 1 { format!("{c}x {n}") } else { n.clone() }).collect();
            p.spawn(text(format!("{} ({} ships) {place}", f.flagship, f.count), 14.0, if f.mine { Color::WHITE } else { Color::srgb(1.0, 0.6, 0.5) }));
            p.spawn(text(ships.join(", "), 12.0, Color::srgb(0.7, 0.75, 0.8)));
        }
        if model.fleets.len() > 5 {
            p.spawn(text(format!("+{} more fleets", model.fleets.len() - 5), 12.0, Color::srgb(0.7, 0.75, 0.8)));
        }
        if model.fleets.iter().any(|f| f.mine) {
            p.spawn(Node { column_gap: Val::Px(8.0), ..default() }).with_children(|r| {
                r.spawn(button("Merge (M)", Action::Play(PlayAction::MergeSelected), false));
                r.spawn(button("Split (V)", Action::Play(PlayAction::SplitSelected), false));
                r.spawn(button("Retreat (R)", Action::Play(PlayAction::RetreatSelected), false));
            });
            p.spawn(text("Drag a fleet onto a planet, or right-click a planet, to move.", 12.0, Color::srgb(0.6, 0.7, 0.8)));
        }
    });
}

fn kind_color(k: NotificationKind) -> Color {
    match k {
        NotificationKind::Construction => Color::srgb(0.6, 1.0, 0.7),
        NotificationKind::BattlePending | NotificationKind::PlanetLost | NotificationKind::Error => Color::srgb(1.0, 0.5, 0.4),
        NotificationKind::BattleResolved => Color::srgb(1.0, 0.8, 0.5),
        NotificationKind::PlanetCaptured => Color::srgb(0.5, 1.0, 0.9),
        NotificationKind::FleetArrived => Color::srgb(0.7, 0.85, 1.0),
        NotificationKind::Income => Color::srgb(1.0, 0.92, 0.55),
        NotificationKind::Campaign => Color::WHITE,
    }
}

fn refresh_notifications(model: Res<PlayUiModel>, mut ui: ResMut<UiState>, ticker: Single<Entity, With<Ticker>>, mut commands: Commands) {
    let s = sig(&model.notifications.iter().map(|n| n.id).collect::<Vec<_>>());
    if ui.signatures[3] == s {
        return;
    }
    ui.signatures[3] = s;
    commands.entity(*ticker).despawn_related::<Children>().with_children(|p| {
        for n in model.notifications.iter().rev().take(7).collect::<Vec<_>>().into_iter().rev() {
            p.spawn((
                text(n.text.clone(), 14.0, kind_color(n.kind)),
                TextShadow { offset: Vec2::splat(1.5), color: Color::srgba(0.0, 0.0, 0.0, 0.9) },
            ));
        }
    });
}

fn refresh_battle(model: Res<PlayUiModel>, mut ui: ResMut<UiState>, panel: Single<(Entity, &mut Visibility), With<BattlePanel>>, mut commands: Commands) {
    let (root, mut vis) = panel.into_inner();
    let Some(b) = &model.battle else {
        *vis = Visibility::Hidden;
        ui.signatures[4] = 0;
        return;
    };
    *vis = Visibility::Inherited;
    let s = sig(&(b.id, b.sides.iter().map(|x| x.ships).collect::<Vec<_>>()));
    if ui.signatures[4] == s {
        return;
    }
    ui.signatures[4] = s;
    commands.entity(root).despawn_related::<Children>().with_children(|p| {
        p.spawn(text(format!("Battle at {}", b.planet_name), 22.0, Color::srgb(1.0, 0.6, 0.4)));
        for side in &b.sides {
            let c = side.color;
            p.spawn(text(
                format!("{}{}: {} ships{}", side.faction, if side.mine { " (you)" } else { "" }, side.ships, if side.defender { ", defending the planet" } else { "" }),
                15.0,
                Color::srgb(c[0], c[1], c[2]),
            ));
        }
        p.spawn(Node { column_gap: Val::Px(12.0), ..default() }).with_children(|r| {
            r.spawn(button("Fight (tactical)", Action::Play(PlayAction::ResolveBattle { id: b.id, choice: BattleChoice::Fight }), false));
            r.spawn(button("Auto-resolve", Action::Play(PlayAction::ResolveBattle { id: b.id, choice: BattleChoice::AutoResolve }), false));
        });
    });
}

