//! The Droid Advisor menu (opened by the "?" button next to R2 on the galactic command bar or key 'H' / F1):
//! A full modal dialog with 4 tabs:
//! 1. DROID LOG: Droid advisor status, "Enable hints" toggle, operational advice, and galactic icon legend.
//! 2. HISTORY: Faction statistics and trend charts (Economy, Military, Planets) over galactic weeks.
//! 3. TECH TREE: Technology tiers 1..5 for the player's faction, showing unlocked ships, stations and facilities.
//! 4. SUMMARY: Galactic overview tables (Active Production, Fleets & Heroes, Economy).

use bevy::prelude::*;
use bevy::text::FontSource;
use galaxy_play::{PlayerFaction, SimState};
use galaxy_sim::{FactionId, Lane, ProductionKind};

use crate::flow::AppScreen;

/// Registers the droid menu plugin.
pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<DroidMenuState>()
        .init_resource::<HistoryTracker>()
        .init_resource::<AdvisorLog>()
        .add_systems(
            Update,
            (
                toggle_menu,
                track_history,
                record_advisor_log,
                update_window,
                handle_clicks,
                handle_scroll,
            )
                .chain()
                .run_if(in_state(AppScreen::Galaxy)),
        )
        .add_systems(OnExit(AppScreen::Galaxy), close_on_exit);
}

/// Dynamic state of the Droid Advisor menu.
#[derive(Resource, Debug, Clone)]
pub(crate) struct DroidMenuState {
    pub open: bool,
    pub tab: DroidMenuTab,
    pub enable_hints: bool,
    pub history_metric: HistoryMetric,
    pub summary_view: SummaryView,
    pub scroll: f32,
    pub selected_planet: Option<usize>,
}

impl Default for DroidMenuState {
    fn default() -> Self {
        Self {
            open: false,
            // SP_DROID_TAB=history|tech|summary opens another tab first (offscreen checks).
            tab: match std::env::var("SP_DROID_TAB").as_deref() {
                Ok("history") => DroidMenuTab::History,
                Ok("tech") => DroidMenuTab::TechTree,
                Ok("summary") => DroidMenuTab::Summary,
                _ => DroidMenuTab::Log,
            },
            enable_hints: true,
            history_metric: HistoryMetric::All,
            summary_view: SummaryView::Production,
            scroll: 0.0,
            selected_planet: None,
        }
    }
}

/// The four primary tabs of the Droid Menu.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DroidMenuTab {
    #[default]
    Log,
    History,
    TechTree,
    Summary,
}

/// Metrics available on the History tab.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HistoryMetric {
    #[default]
    All,
    Economy,
    Military,
    Planets,
}

/// Sub-tables on the Summary tab.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SummaryView {
    #[default]
    Production,
    Fleets,
    Economy,
}

/// Component tagging the root node of the Droid Menu.
#[derive(Component)]
struct DroidMenuRoot;

/// Component tagging the scrollable content area.
#[derive(Component)]
struct DroidMenuScrollBox;

/// Click actions dispatched from buttons inside the Droid Menu.
#[derive(Component, Clone, PartialEq, Debug)]
pub(crate) enum DroidMenuClick {
    Tab(DroidMenuTab),
    ToggleHints,
    Metric(HistoryMetric),
    Summary(SummaryView),
    Close,
}

/// Historical record of galactic faction progress sampled over time.
#[derive(Resource, Default, Debug, Clone)]
pub(crate) struct HistoryTracker {
    pub samples: Vec<HistorySample>,
    pub last_week: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct HistorySample {
    #[allow(dead_code)]
    pub week: f64,
    pub factions: Vec<FactionRecord>,
}

#[derive(Debug, Clone)]
pub(crate) struct FactionRecord {
    #[allow(dead_code)]
    pub id: FactionId,
    pub name: String,
    pub color: Color,
    pub credits: f64,
    #[allow(dead_code)]
    pub income: f64,
    pub planets: u32,
    pub military: u32,
}

fn close_on_exit(mut state: ResMut<DroidMenuState>) {
    state.open = false;
}

/// Toggles or opens the Droid Menu via UI clicks, button shortcuts, or keyboard.
fn toggle_menu(
    mut clicks: MessageReader<eaw_ui::EawButtonClicked>,
    keys: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<DroidMenuState>,
    mut paused: Option<ResMut<crate::flow::Paused>>,
) {
    for c in clicks.read() {
        if matches!(
            c.name.as_str(),
            "b_droid_help" | "b_droid_help_g" | "b_droid_help_tactical" | "movie_galactic"
        ) {
            state.open = !state.open;
            state.scroll = 0.0;
        }
    }

    if keys.just_pressed(KeyCode::KeyH) || keys.just_pressed(KeyCode::F1) {
        state.open = !state.open;
        state.scroll = 0.0;
    }

    if keys.just_pressed(KeyCode::Escape) && state.open {
        state.open = false;
        // Suppress opening the pause menu simultaneously when closing the modal
        if let Some(ref mut p) = paused {
            p.0 = false;
        }
    }
}

/// Periodically records faction statistics for the History tab.
fn track_history(
    sim_opt: Option<Res<SimState>>,
    mut history: ResMut<HistoryTracker>,
) {
    let Some(sim) = sim_opt else { return };
    let current_week = sim.current.galaxy.week() as f64;

    // Sample once per galactic week (or initial sample)
    if history.samples.is_empty() || (current_week - history.last_week) >= 1.0 {
        history.last_week = current_week;
        let mut records = Vec::new();
        for (i, f_state) in sim.current.galaxy.factions.iter().enumerate() {
            let f_id = i as FactionId;
            let name = sim
                .world
                .factions
                .get(i)
                .map(|f| f.name.clone())
                .unwrap_or_else(|| format!("Faction {i}"));

            let color = match name.to_uppercase().as_str() {
                "EMPIRE" => Color::srgb(0.95, 0.2, 0.2),
                "REBEL" | "REBELLION" => Color::srgb(0.2, 0.9, 0.3),
                "UNDERWORLD" => Color::srgb(0.95, 0.85, 0.2),
                "PIRATES" | "INDEPENDENT" => Color::srgb(0.95, 0.55, 0.15),
                _ => Color::srgb(0.6, 0.7, 0.9),
            };

            // Ships and squadrons in fleets (not population points: a capital ship counts for several of those).
            let military: u32 = sim.current.galaxy.fleets.values().filter(|fl| fl.owner == f_id).map(|fl| fl.unit_count()).sum();

            records.push(FactionRecord {
                id: f_id,
                name,
                color,
                credits: f_state.credits,
                income: f_state.income_per_week,
                planets: f_state.planets_owned,
                military,
            });
        }

        history.samples.push(HistorySample {
            week: current_week,
            factions: records,
        });

        // Retain up to 60 historical samples
        if history.samples.len() > 60 {
            history.samples.remove(0);
        }
    }
}

/// Handles button clicks within the Droid Menu.
fn handle_clicks(
    mut state: ResMut<DroidMenuState>,
    clicks: Query<(&Interaction, &DroidMenuClick), Changed<Interaction>>,
) {
    for (interaction, action) in &clicks {
        if *interaction == Interaction::Pressed {
            match action {
                DroidMenuClick::Tab(t) => {
                    state.tab = *t;
                    state.scroll = 0.0;
                }
                DroidMenuClick::ToggleHints => {
                    state.enable_hints = !state.enable_hints;
                }
                DroidMenuClick::Metric(m) => {
                    state.history_metric = *m;
                }
                DroidMenuClick::Summary(s) => {
                    state.summary_view = *s;
                    state.scroll = 0.0;
                }
                DroidMenuClick::Close => {
                    state.open = false;
                }
            }
        }
    }
}

/// Handles mouse wheel scrolling inside the Droid Menu content area.
fn handle_scroll(
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
    mut state: ResMut<DroidMenuState>,
    mut query: Query<&mut ScrollPosition, With<DroidMenuScrollBox>>,
) {
    let delta: f32 = wheel
        .read()
        .map(|w| {
            if matches!(w.unit, bevy::input::mouse::MouseScrollUnit::Line) {
                w.y * 36.0
            } else {
                w.y
            }
        })
        .sum();

    if delta != 0.0 {
        state.scroll = (state.scroll - delta).max(0.0);
        for mut pos in &mut query {
            pos.y = state.scroll;
        }
    }
}

/// Rebuilds the Droid Menu modal UI when open or when state changes.
fn update_window(
    mut commands: Commands,
    state: Res<DroidMenuState>,
    history: Res<HistoryTracker>,
    advisor_log: Res<AdvisorLog>,
    existing: Query<Entity, With<DroidMenuRoot>>,
    sim_opt: Option<Res<SimState>>,
    player: Res<PlayerFaction>,
    mut last_rendered: Local<Option<(bool, DroidMenuTab, bool, HistoryMetric, SummaryView, Option<usize>)>>,
) {
    let current_key = (
        state.open,
        state.tab,
        state.enable_hints,
        state.history_metric,
        state.summary_view,
        state.selected_planet,
    );

    if *last_rendered == Some(current_key) {
        return;
    }
    *last_rendered = Some(current_key);

    for e in &existing {
        commands.entity(e).despawn();
    }

    if !state.open {
        return;
    }

    let Some(sim) = sim_opt else { return };
    let player_faction_id = player.0.unwrap_or(0);

    let font = |size: f32| TextFont {
        font: FontSource::Family("Bahnschrift".into()),
        font_size: FontSize::Px(size),
        ..default()
    };
    let blue = Color::srgb(0.32, 0.52, 0.93);
    let cyan = Color::srgb(0.35, 0.85, 0.95);
    let amber = Color::srgb(0.95, 0.75, 0.25);

    // Modal Frame
    commands
        .spawn((
            DroidMenuRoot,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(14.0),
                top: Val::Percent(7.0),
                width: Val::Percent(72.0),
                height: Val::Percent(86.0),
                padding: UiRect::all(Val::Px(16.0)),
                border: UiRect::all(Val::Px(3.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(10.0),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.03, 0.08, 0.96)),
            BorderColor::all(Color::srgb(0.45, 0.6, 0.95)),
            BoxShadow::new(
                Color::srgba(0.3, 0.5, 1.0, 0.5),
                Val::Px(0.0),
                Val::Px(0.0),
                Val::Px(4.0),
                Val::Px(24.0),
            ),
            GlobalZIndex(650),
        ))
        .with_children(|root| {
            // Header Bar
            root.spawn(Node {
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                margin: UiRect::bottom(Val::Px(4.0)),
                ..default()
            })
            .with_children(|hdr| {
                hdr.spawn((
                    Text::new("DROID ADVISOR & GALACTIC ARCHIVES"),
                    font(22.0),
                    TextColor(Color::WHITE),
                    bevy::text::LetterSpacing::Rem(0.08),
                ));
                hdr.spawn((
                    Text::new(format!("WEEK {:.0}", sim.current.galaxy.week())),
                    font(15.0),
                    TextColor(cyan),
                    bevy::text::LetterSpacing::Rem(0.05),
                ));
            });

            // Tab Navigation Bar
            root.spawn(Node {
                column_gap: Val::Px(8.0),
                margin: UiRect::bottom(Val::Px(8.0)),
                ..default()
            })
            .with_children(|tabs| {
                let tab_items = [
                    ("DROID LOG", DroidMenuTab::Log),
                    ("HISTORY", DroidMenuTab::History),
                    ("TECH TREE", DroidMenuTab::TechTree),
                    ("SUMMARY", DroidMenuTab::Summary),
                ];

                for (label, tab_val) in tab_items {
                    let is_active = state.tab == tab_val;
                    tabs.spawn((
                        Button,
                        DroidMenuClick::Tab(tab_val),
                        Node {
                            width: Val::Percent(24.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            padding: UiRect::vertical(Val::Px(6.0)),
                            border: UiRect::all(Val::Px(1.5)),
                            ..default()
                        },
                        BackgroundColor(if is_active {
                            Color::srgb(0.58, 0.72, 0.92)
                        } else {
                            Color::srgb(0.18, 0.22, 0.32)
                        }),
                        BorderColor::all(if is_active {
                            Color::srgb(0.85, 0.93, 1.0)
                        } else {
                            Color::srgb(0.35, 0.42, 0.55)
                        }),
                    ))
                    .with_child((
                        Text::new(label),
                        font(14.0),
                        TextColor(if is_active {
                            Color::srgb(0.02, 0.03, 0.06)
                        } else {
                            Color::srgb(0.8, 0.85, 0.95)
                        }),
                        bevy::text::LetterSpacing::Rem(0.08),
                    ));
                }
            });

            // Content Area Container
            root.spawn(Node {
                flex_grow: 1.0,
                min_height: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                border: UiRect::all(Val::Px(1.0)),
                padding: UiRect::all(Val::Px(12.0)),
                overflow: Overflow::clip(),
                ..default()
            })
            .insert((
                BackgroundColor(Color::srgba(0.01, 0.01, 0.04, 0.9)),
                BorderColor::all(Color::srgb(0.25, 0.35, 0.55)),
            ))
            .with_children(|content| {
                match state.tab {
                    DroidMenuTab::Log => render_log_tab(content, &state, &advisor_log, &font, blue, cyan),
                    DroidMenuTab::History => {
                        render_history_tab(content, &state, &history, &font, blue, cyan)
                    }
                    DroidMenuTab::TechTree => {
                        render_techtree_tab(content, &sim, player_faction_id, &font, blue, cyan, amber)
                    }
                    DroidMenuTab::Summary => {
                        render_summary_tab(content, &state, &sim, player_faction_id, &font, blue, cyan, amber)
                    }
                }
            });

            // Footer Bar
            root.spawn(Node {
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                margin: UiRect::top(Val::Px(6.0)),
                ..default()
            })
            .with_children(|footer| {
                footer.spawn((
                    Text::new("HINT: THE DROID ADVISOR PROVIDES SYSTEM STATUS AND HISTORICAL ARCHIVES."),
                    font(13.0),
                    TextColor(blue),
                    bevy::text::LetterSpacing::Rem(0.05),
                ));

                footer
                    .spawn((
                        Button,
                        DroidMenuClick::Close,
                        Node {
                            width: Val::Px(180.0),
                            padding: UiRect::vertical(Val::Px(7.0)),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            border: UiRect::all(Val::Px(1.5)),
                            ..default()
                        },
                        BackgroundColor(Color::BLACK),
                        BorderColor::all(Color::srgb(0.65, 0.72, 0.88)),
                    ))
                    .with_child((
                        Text::new("CLOSE"),
                        font(15.0),
                        TextColor(Color::WHITE),
                        bevy::text::LetterSpacing::Rem(0.1),
                    ));
            });
        });
}

/// Tab 1: Droid Log (Advisor status, hints toggle, guidance, and icon legend).
fn render_log_tab(
    content: &mut ChildSpawnerCommands,
    state: &DroidMenuState,
    advisor_log: &AdvisorLog,
    font: &dyn Fn(f32) -> TextFont,
    blue: Color,
    cyan: Color,
) {
    // Top Row: Enable hints checkbox & advisor status
    content
        .spawn(Node {
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::Center,
            margin: UiRect::bottom(Val::Px(12.0)),
            padding: UiRect::all(Val::Px(8.0)),
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        })
        .insert((
            BackgroundColor(Color::srgba(0.05, 0.1, 0.2, 0.5)),
            BorderColor::all(Color::srgb(0.3, 0.45, 0.7)),
        ))
        .with_children(|row| {
            row.spawn((
                Button,
                DroidMenuClick::ToggleHints,
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(10.0),
                    padding: UiRect::all(Val::Px(6.0)),
                    ..default()
                },
            ))
            .with_children(|cb| {
                cb.spawn((
                    Node {
                        width: Val::Px(18.0),
                        height: Val::Px(18.0),
                        border: UiRect::all(Val::Px(1.5)),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    BackgroundColor(if state.enable_hints {
                        Color::srgb(0.2, 0.6, 0.9)
                    } else {
                        Color::NONE
                    }),
                    BorderColor::all(Color::WHITE),
                ))
                .with_child((
                    Text::new(if state.enable_hints { "X" } else { "" }),
                    font(12.0),
                    TextColor(Color::WHITE),
                ));

                cb.spawn((
                    Text::new("ENABLE ADVISOR HINTS"),
                    font(14.0),
                    TextColor(Color::WHITE),
                ));
            });

            row.spawn((
                Text::new("ADVISOR UPLINK: CONNECTED TO SECTOR COMMAND"),
                font(13.0),
                TextColor(cyan),
            ));
        });

    // Two-Column Split (Left: Advisor Advice, Right: Legend & History)
    content
        .spawn(Node {
            flex_grow: 1.0,
            column_gap: Val::Px(16.0),
            overflow: Overflow::clip(),
            ..default()
        })
        .with_children(|split| {
            // Left Column: Droid Profile & Strategic Directives
            split
                .spawn(Node {
                    width: Val::Percent(45.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(10.0),
                    padding: UiRect::all(Val::Px(10.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                })
                .insert((
                    BackgroundColor(Color::srgba(0.03, 0.05, 0.1, 0.6)),
                    BorderColor::all(Color::srgb(0.25, 0.35, 0.55)),
                ))
                .with_children(|col| {
                    col.spawn((
                        Text::new("ASTROMECH / TACTICAL ADVISOR"),
                        font(16.0),
                        TextColor(cyan),
                        bevy::text::LetterSpacing::Rem(0.06),
                    ));

                    col.spawn((
                        Text::new(
                            "Autonomous droid advisory module actively monitoring military\n\
                            engagements, industrial quotas, and territorial defense.",
                        ),
                        font(13.0),
                        TextColor(Color::srgb(0.75, 0.82, 0.95)),
                    ));

                    col.spawn((
                        Text::new("STRATEGIC DIRECTIVES:"),
                        font(14.0),
                        TextColor(blue),
                        bevy::text::LetterSpacing::Rem(0.04),
                    ));

                    let hints = [
                        "1. ECONOMY: Build mining facilities on high-yield planets to ensure sustainable credit inflow each galactic week.",
                        "2. INFRASTRUCTURE: Upgrade orbital starbases to unlock heavier capital cruisers and station defense batteries.",
                        "3. ORBITAL DEFENSE: Position fleets onto planetary circles to instantly intercept invading hostile battle groups.",
                        "4. REINFORCEMENTS: During space combat, call in reserve squadrons dynamically into flanking sensor positions.",
                    ];

                    for h in hints {
                        col.spawn((
                            Text::new(h),
                            font(12.5),
                            TextColor(Color::WHITE),
                            Node {
                                margin: UiRect::bottom(Val::Px(4.0)),
                                ..default()
                            },
                        ));
                    }
                });

            // Right Column: Galactic Map Icon Legend & Event Log
            split
                .spawn(Node {
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(10.0),
                    padding: UiRect::all(Val::Px(10.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                })
                .insert((
                    BackgroundColor(Color::srgba(0.03, 0.05, 0.1, 0.6)),
                    BorderColor::all(Color::srgb(0.25, 0.35, 0.55)),
                ))
                .with_children(|col| {
                    col.spawn((
                        Text::new("GALACTIC MAP ICON LEGEND"),
                        font(16.0),
                        TextColor(cyan),
                        bevy::text::LetterSpacing::Rem(0.06),
                    ));

                    let legend = [
                        ("GREEN DISC", Color::srgb(0.2, 0.9, 0.3), "Allied Star System / Player Fleet"),
                        ("RED DISC", Color::srgb(0.95, 0.2, 0.2), "Hostile Enemy Sector / Fleet"),
                        ("ORANGE DISC", Color::srgb(0.95, 0.6, 0.1), "Neutral / Independent Sector"),
                        ("CYAN CONDUIT", Color::srgb(0.3, 0.7, 0.95), "Hyperspace Hyperlane Network"),
                        ("STATION ICON", Color::srgb(0.9, 0.9, 0.4), "Orbital Starbase Tier (Level 1 - 5)"),
                        ("CREDIT PIP", Color::srgb(0.4, 0.85, 0.4), "Planetary Mining & Trade Income"),
                    ];

                    for (symbol, sym_col, desc) in legend {
                        col.spawn(Node {
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(10.0),
                            margin: UiRect::vertical(Val::Px(2.0)),
                            ..default()
                        })
                        .with_children(|item| {
                            item.spawn((
                                Node {
                                    width: Val::Px(14.0),
                                    height: Val::Px(14.0),
                                    ..default()
                                },
                                BackgroundColor(sym_col),
                            ));
                            item.spawn((Text::new(format!("{symbol}: {desc}")), font(13.0), TextColor(Color::WHITE)));
                        });
                    }

                    col.spawn((
                        Text::new("ADVISOR LOG ARCHIVE:"),
                        font(14.0),
                        TextColor(blue),
                        Node {
                            margin: UiRect::top(Val::Px(8.0)),
                            ..default()
                        },
                    ));

                    // The recorded advisor messages, newest first, grouped by month.
                    if advisor_log.entries.is_empty() {
                        col.spawn((Text::new("No advisor reports yet."), font(12.0), TextColor(Color::srgb(0.7, 0.8, 0.9))));
                    }
                    let mut month = u64::MAX;
                    for (week, text) in advisor_log.entries.iter().rev().take(14) {
                        if week / 4 != month {
                            month = week / 4;
                            col.spawn((Text::new(format!("MONTH {}", month + 1)), font(12.0), TextColor(blue)));
                        }
                        col.spawn((Text::new(format!("[WEEK {:02}] {text}", week + 1)), font(12.0), TextColor(Color::srgb(0.7, 0.8, 0.9))));
                    }
                });
        });
}

/// Tab 2: History (Line charts / graphs tracking Economy, Military, Planets over galactic weeks).
fn render_history_tab(
    content: &mut ChildSpawnerCommands,
    state: &DroidMenuState,
    history: &HistoryTracker,
    font: &dyn Fn(f32) -> TextFont,
    blue: Color,
    cyan: Color,
) {
    // Metric Selector Row
    content
        .spawn(Node {
            column_gap: Val::Px(8.0),
            margin: UiRect::bottom(Val::Px(10.0)),
            ..default()
        })
        .with_children(|row| {
            let metrics = [
                ("ALL METRICS", HistoryMetric::All),
                ("ECONOMY (CREDITS)", HistoryMetric::Economy),
                ("MILITARY (SHIPS)", HistoryMetric::Military),
                ("TERRITORY (PLANETS)", HistoryMetric::Planets),
            ];

            for (lbl, m) in metrics {
                let active = state.history_metric == m;
                row.spawn((
                    Button,
                    DroidMenuClick::Metric(m),
                    Node {
                        padding: UiRect::axes(Val::Px(12.0), Val::Px(5.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(if active {
                        Color::srgb(0.35, 0.55, 0.85)
                    } else {
                        Color::srgb(0.15, 0.18, 0.25)
                    }),
                    BorderColor::all(if active {
                        Color::WHITE
                    } else {
                        Color::srgb(0.35, 0.45, 0.6)
                    }),
                ))
                .with_child((
                    Text::new(lbl),
                    font(13.0),
                    TextColor(if active { Color::WHITE } else { Color::srgb(0.75, 0.8, 0.9) }),
                ));
            }
        });

    // Chart Display Box
    content
        .spawn(Node {
            flex_grow: 1.0,
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(Val::Px(12.0)),
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        })
        .insert((
            BackgroundColor(Color::srgba(0.02, 0.04, 0.08, 0.8)),
            BorderColor::all(Color::srgb(0.3, 0.4, 0.6)),
        ))
        .with_children(|chart| {
            // Faction Legend
            chart
                .spawn(Node {
                    justify_content: JustifyContent::FlexEnd,
                    column_gap: Val::Px(16.0),
                    margin: UiRect::bottom(Val::Px(8.0)),
                    ..default()
                })
                .with_children(|leg| {
                    if let Some(last) = history.samples.last() {
                        for f in &last.factions {
                            leg.spawn(Node {
                                align_items: AlignItems::Center,
                                column_gap: Val::Px(6.0),
                                ..default()
                            })
                            .with_children(|item| {
                                item.spawn((
                                    Node {
                                        width: Val::Px(12.0),
                                        height: Val::Px(12.0),
                                        ..default()
                                    },
                                    BackgroundColor(f.color),
                                ));
                                item.spawn((
                                    Text::new(&f.name),
                                    font(13.0),
                                    TextColor(Color::WHITE),
                                ));
                            });
                        }
                    }
                });

            // Line charts of the recorded weekly samples: one chart per metric ("all" shows the three side by side).
            chart
                .spawn(Node { flex_grow: 1.0, column_gap: Val::Px(14.0), align_items: AlignItems::Center, justify_content: JustifyContent::Center, ..default() })
                .with_children(|charts| {
                    if history.samples.len() < 2 {
                        charts.spawn((Text::new("COLLECTING GALACTIC HISTORICAL DATA..."), font(14.0), TextColor(cyan)));
                        return;
                    }
                    let all = state.history_metric == HistoryMetric::All;
                    let kinds: &[HistoryMetric] = if all {
                        &[HistoryMetric::Economy, HistoryMetric::Military, HistoryMetric::Planets]
                    } else {
                        std::slice::from_ref(&state.history_metric)
                    };
                    let (w, h) = if all { (295.0, 250.0) } else { (860.0, 300.0) };
                    for &kind in kinds {
                        draw_line_chart(charts, history, kind, w, h, font, blue);
                    }
                });

            chart.spawn((
                Text::new("RECORDED ACROSS RECENT GALACTIC WEEKS FROM ORBITAL SENSORS"),
                font(12.0),
                TextColor(blue),
                Node {
                    margin: UiRect::top(Val::Px(6.0)),
                    ..default()
                },
            ));
        });
}

/// Tab 3: Tech Tree (Tiers 1..5 with unlocked ships, structures and costs).
fn render_techtree_tab(
    content: &mut ChildSpawnerCommands,
    sim: &SimState,
    player_id: FactionId,
    font: &dyn Fn(f32) -> TextFont,
    blue: Color,
    cyan: Color,
    amber: Color,
) {
    let current_tech = sim
        .current
        .galaxy
        .factions
        .get(player_id as usize)
        .map(|f| f.tech)
        .unwrap_or(1);

    content
        .spawn(Node {
            justify_content: JustifyContent::SpaceBetween,
            margin: UiRect::bottom(Val::Px(10.0)),
            ..default()
        })
        .with_children(|hdr| {
            hdr.spawn((
                Text::new(format!("CURRENT RESEARCH LEVEL: TIER {current_tech}")),
                font(17.0),
                TextColor(cyan),
                bevy::text::LetterSpacing::Rem(0.06),
            ));
            hdr.spawn((
                Text::new("STATION UPGRADES EXPAND SHIPYARD CAPACITY & UNLOCK HIGHER TIERS"),
                font(13.0),
                TextColor(blue),
            ));
        });

    // 5 Tech Tiers Container
    content
        .spawn(Node {
            flex_grow: 1.0,
            column_gap: Val::Px(10.0),
            ..default()
        })
        .with_children(|tiers| {
            let tech_data = [
                (
                    1,
                    "CORVETTE & FRIGATE FOUNDATION",
                    0,
                    &[
                        "CR90 Corvette / Tartan Patrol",
                        "Nebulon-B / Acclamator Frigate",
                        "Level 1 Starbase",
                        "Mining Facility",
                    ][..],
                ),
                (
                    2,
                    "FLEET CRUISERS & COMMERCE",
                    5000,
                    &[
                        "Dreadnaught Heavy Cruiser",
                        "Broadside Missile Cruiser",
                        "Level 2 Starbase",
                        "Advanced Research Station",
                    ][..],
                ),
                (
                    3,
                    "HEAVY CAPITAL SHIPYARDS",
                    10000,
                    &[
                        "Victory-class Star Destroyer",
                        "MC80 Command Cruiser",
                        "Major Construction Yard I",
                        "Level 3 Starbase",
                    ][..],
                ),
                (
                    4,
                    "IMPERIAL CLASS BATTLECRUISERS",
                    20000,
                    &[
                        "Imperial-class Star Destroyer",
                        "MC80 Home One Flagship",
                        "Level 4 Starbase",
                        "Planetary Shield Generator",
                    ][..],
                ),
                (
                    5,
                    "SUPER CAPITAL DREADNOUGHTS",
                    5000,
                    &[
                        "Executor Super Star Destroyer",
                        "Viscount Super Carrier",
                        "Level 5 Starbase",
                        "Hypervelocity Ground Cannon",
                    ][..],
                ),
            ];

            for (lvl, title, cost, unlocks) in tech_data {
                let is_unlocked = current_tech >= lvl;
                let is_current = current_tech == lvl;

                tiers
                    .spawn(Node {
                        width: Val::Percent(19.2),
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(6.0),
                        padding: UiRect::all(Val::Px(8.0)),
                        border: UiRect::all(Val::Px(1.5)),
                        ..default()
                    })
                    .insert((
                        BackgroundColor(if is_current {
                            Color::srgba(0.05, 0.15, 0.25, 0.9)
                        } else if is_unlocked {
                            Color::srgba(0.03, 0.08, 0.12, 0.8)
                        } else {
                            Color::srgba(0.02, 0.03, 0.06, 0.7)
                        }),
                        BorderColor::all(if is_current {
                            cyan
                        } else if is_unlocked {
                            Color::srgb(0.3, 0.6, 0.8)
                        } else {
                            Color::srgb(0.2, 0.25, 0.35)
                        }),
                    ))
                    .with_children(|card| {
                        card.spawn((
                            Text::new(format!("TIER {lvl}")),
                            font(16.0),
                            TextColor(if is_current {
                                cyan
                            } else if is_unlocked {
                                Color::WHITE
                            } else {
                                Color::srgb(0.5, 0.6, 0.7)
                            }),
                        ));

                        card.spawn((
                            Text::new(title),
                            font(11.5),
                            TextColor(blue),
                            Node {
                                margin: UiRect::bottom(Val::Px(4.0)),
                                ..default()
                            },
                        ));

                        // Status Badge
                        let badge_txt = if is_current {
                            "CURRENT TIER"
                        } else if is_unlocked {
                            "RESEARCHED"
                        } else {
                            "LOCKED"
                        };

                        let badge_col = if is_current {
                            cyan
                        } else if is_unlocked {
                            Color::srgb(0.3, 0.9, 0.4)
                        } else {
                            amber
                        };

                        card.spawn((
                            Text::new(badge_txt),
                            font(12.0),
                            TextColor(badge_col),
                            Node {
                                margin: UiRect::bottom(Val::Px(4.0)),
                                ..default()
                            },
                        ));

                        if cost > 0 && !is_unlocked {
                            card.spawn((
                                Text::new(format!("Cost: {cost} cr")),
                                font(11.0),
                                TextColor(amber),
                            ));
                        }

                        card.spawn((
                            Text::new("UNLOCKED:"),
                            font(11.5),
                            TextColor(Color::WHITE),
                            Node {
                                margin: UiRect::top(Val::Px(4.0)),
                                ..default()
                            },
                        ));

                        for u in unlocks {
                            card.spawn((
                                Text::new(format!("- {u}")),
                                font(11.0),
                                TextColor(if is_unlocked {
                                    Color::srgb(0.85, 0.92, 1.0)
                                } else {
                                    Color::srgb(0.45, 0.52, 0.62)
                                }),
                            ));
                        }
                    });
            }
        });
}

/// Tab 4: Summary (Active Production, Fleets & Heroes, and Economy overview).
fn render_summary_tab(
    content: &mut ChildSpawnerCommands,
    state: &DroidMenuState,
    sim: &SimState,
    player_id: FactionId,
    font: &dyn Fn(f32) -> TextFont,
    _blue: Color,
    cyan: Color,
    amber: Color,
) {
    // Sub-view Toggle Row
    content
        .spawn(Node {
            column_gap: Val::Px(8.0),
            margin: UiRect::bottom(Val::Px(10.0)),
            ..default()
        })
        .with_children(|row| {
            let views = [
                ("PRODUCTION SUMMARY", SummaryView::Production),
                ("FLEETS & HEROES", SummaryView::Fleets),
                ("ECONOMIC REPORT", SummaryView::Economy),
            ];

            for (lbl, v) in views {
                let active = state.summary_view == v;
                row.spawn((
                    Button,
                    DroidMenuClick::Summary(v),
                    Node {
                        padding: UiRect::axes(Val::Px(12.0), Val::Px(5.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(if active {
                        Color::srgb(0.35, 0.55, 0.85)
                    } else {
                        Color::srgb(0.15, 0.18, 0.25)
                    }),
                    BorderColor::all(if active {
                        Color::WHITE
                    } else {
                        Color::srgb(0.35, 0.45, 0.6)
                    }),
                ))
                .with_child((
                    Text::new(lbl),
                    font(13.0),
                    TextColor(if active { Color::WHITE } else { Color::srgb(0.75, 0.8, 0.9) }),
                ));
            }
        });

    // Overview Table Box
    content
        .spawn(Node {
            flex_grow: 1.0,
            flex_direction: FlexDirection::Column,
            border: UiRect::all(Val::Px(1.0)),
            overflow: Overflow::clip(),
            ..default()
        })
        .insert((
            BackgroundColor(Color::srgba(0.01, 0.02, 0.05, 0.9)),
            BorderColor::all(Color::srgb(0.25, 0.35, 0.55)),
        ))
        .with_children(|tbl| {
            // Table Header
            tbl.spawn(Node {
                padding: UiRect::all(Val::Px(8.0)),
                border: UiRect::bottom(Val::Px(1.5)),
                ..default()
            })
            .insert((
                BackgroundColor(Color::srgba(0.05, 0.1, 0.2, 0.7)),
                BorderColor::all(Color::srgb(0.35, 0.45, 0.65)),
            ))
            .with_children(|hdr| {
                match state.summary_view {
                    SummaryView::Production => {
                        hdr.spawn((Text::new("PLANET"), font(13.5), TextColor(cyan), Node { width: Val::Percent(25.0), ..default() }));
                        hdr.spawn((Text::new("SPACE PRODUCTION"), font(13.5), TextColor(cyan), Node { width: Val::Percent(35.0), ..default() }));
                        hdr.spawn((Text::new("GROUND PRODUCTION"), font(13.5), TextColor(cyan), Node { width: Val::Percent(35.0), ..default() }));
                    }
                    SummaryView::Fleets => {
                        hdr.spawn((Text::new("PLANET"), font(13.5), TextColor(cyan), Node { width: Val::Percent(25.0), ..default() }));
                        hdr.spawn((Text::new("FLEETS IN ORBIT"), font(13.5), TextColor(cyan), Node { width: Val::Percent(25.0), ..default() }));
                        hdr.spawn((Text::new("UNITS PRESENT"), font(13.5), TextColor(cyan), Node { width: Val::Percent(25.0), ..default() }));
                        hdr.spawn((Text::new("ORBITAL GARRISON"), font(13.5), TextColor(cyan), Node { width: Val::Percent(25.0), ..default() }));
                    }
                    SummaryView::Economy => {
                        hdr.spawn((Text::new("PLANET"), font(13.5), TextColor(cyan), Node { width: Val::Percent(25.0), ..default() }));
                        hdr.spawn((Text::new("STARBASE LEVEL"), font(13.5), TextColor(cyan), Node { width: Val::Percent(25.0), ..default() }));
                        hdr.spawn((Text::new("WEEKLY REVENUE"), font(13.5), TextColor(cyan), Node { width: Val::Percent(25.0), ..default() }));
                        hdr.spawn((Text::new("STRUCTURES"), font(13.5), TextColor(cyan), Node { width: Val::Percent(25.0), ..default() }));
                    }
                }
            });

            // Table Rows (Scrollable)
            tbl.spawn((
                DroidMenuScrollBox,
                ScrollPosition(Vec2::new(0.0, state.scroll)),
                Node {
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Column,
                    overflow: Overflow::scroll_y(),
                    padding: UiRect::all(Val::Px(6.0)),
                    ..default()
                },
            ))
            .with_children(|rows| {
                let mut owned_count = 0;
                for (p_id, p_state) in sim.current.galaxy.planets.iter().enumerate() {
                    if p_state.owner == Some(player_id) {
                        owned_count += 1;
                        let p_def = sim.world.planets.get(p_id);
                        let p_name = p_def.map(|p| p.name.clone()).unwrap_or_else(|| format!("Planet {p_id}"));

                        rows.spawn(Node {
                            padding: UiRect::vertical(Val::Px(5.0)),
                            border: UiRect::bottom(Val::Px(1.0)),
                            align_items: AlignItems::Center,
                            ..default()
                        })
                        .insert(BorderColor::all(Color::srgba(0.2, 0.25, 0.35, 0.4)))
                        .with_children(|row| {
                            match state.summary_view {
                                SummaryView::Production => {
                                    row.spawn((Text::new(p_name), font(13.0), TextColor(Color::WHITE), Node { width: Val::Percent(25.0), ..default() }));

                                    let space_prod = p_state.queues[Lane::Units as usize]
                                        .front()
                                        .map(|item| match item.kind {
                                            ProductionKind::Unit(u) => sim.world.units.get(u as usize).map(|d| d.name.clone()).unwrap_or_else(|| format!("Unit {u}")),
                                            ProductionKind::Structure(s) => sim.world.structures.get(s as usize).map(|d| d.name.clone()).unwrap_or_else(|| format!("Structure {s}")),
                                            ProductionKind::Starbase(b) => format!("Starbase Level {b}"),
                                            ProductionKind::Tech(t) => format!("Tech Level {t}"),
                                        })
                                        .unwrap_or_else(|| "Idle".into());

                                    let ground_prod = p_state.queues[Lane::Structures as usize]
                                        .front()
                                        .map(|item| match item.kind {
                                            ProductionKind::Structure(s) => sim.world.structures.get(s as usize).map(|d| d.name.clone()).unwrap_or_else(|| format!("Structure {s}")),
                                            _ => "Constructing".into(),
                                        })
                                        .unwrap_or_else(|| "Idle".into());

                                    row.spawn((Text::new(space_prod), font(12.5), TextColor(Color::srgb(0.8, 0.9, 1.0)), Node { width: Val::Percent(35.0), ..default() }));
                                    row.spawn((Text::new(ground_prod), font(12.5), TextColor(Color::srgb(0.8, 0.9, 1.0)), Node { width: Val::Percent(35.0), ..default() }));
                                }
                                SummaryView::Fleets => {
                                    row.spawn((Text::new(p_name), font(13.0), TextColor(Color::WHITE), Node { width: Val::Percent(25.0), ..default() }));

                                    let fleets_here = sim
                                        .current
                                        .galaxy
                                        .fleets
                                        .values()
                                        .filter(|f| f.owner == player_id && f.planet() == Some(p_id as u32))
                                        .count();

                                    let units_here: u32 = sim
                                        .current
                                        .galaxy
                                        .fleets
                                        .values()
                                        .filter(|f| f.owner == player_id && f.planet() == Some(p_id as u32))
                                        .map(|f| f.unit_count())
                                        .sum();

                                    let base_tier = p_state.starbase.map(|s| format!("Tier {s}")).unwrap_or_else(|| "None".into());

                                    row.spawn((Text::new(format!("{fleets_here} Fleets")), font(12.5), TextColor(Color::srgb(0.8, 0.9, 1.0)), Node { width: Val::Percent(25.0), ..default() }));
                                    row.spawn((Text::new(format!("{units_here} Ships")), font(12.5), TextColor(Color::srgb(0.8, 0.9, 1.0)), Node { width: Val::Percent(25.0), ..default() }));
                                    row.spawn((Text::new(base_tier), font(12.5), TextColor(cyan), Node { width: Val::Percent(25.0), ..default() }));
                                }
                                SummaryView::Economy => {
                                    row.spawn((Text::new(p_name), font(13.0), TextColor(Color::WHITE), Node { width: Val::Percent(25.0), ..default() }));

                                    let base_tier = p_state.starbase.map(|s| format!("Level {s}")).unwrap_or_else(|| "None".into());
                                    let income = p_def.map(|p| p.credit_value as f64).unwrap_or(100.0);
                                    let struct_count = p_state.structures.len();

                                    row.spawn((Text::new(base_tier), font(12.5), TextColor(cyan), Node { width: Val::Percent(25.0), ..default() }));
                                    row.spawn((Text::new(format!("+{income:.0} cr")), font(12.5), TextColor(Color::srgb(0.4, 0.9, 0.4)), Node { width: Val::Percent(25.0), ..default() }));
                                    row.spawn((Text::new(format!("{struct_count} facilities")), font(12.5), TextColor(Color::srgb(0.8, 0.9, 1.0)), Node { width: Val::Percent(25.0), ..default() }));
                                }
                            }
                        });
                    }
                }

                if owned_count == 0 {
                    rows.spawn((
                        Text::new("NO CONTROLLED STAR SYSTEMS FOUND."),
                        font(13.0),
                        TextColor(amber),
                    ));
                }
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn droid_menu_initial_state_defaults() {
        let state = DroidMenuState::default();
        assert!(!state.open);
        assert_eq!(state.tab, DroidMenuTab::Log);
        assert!(state.enable_hints);
        assert_eq!(state.history_metric, HistoryMetric::All);
        assert_eq!(state.summary_view, SummaryView::Production);
        assert_eq!(state.scroll, 0.0);
    }

    #[test]
    fn droid_menu_tab_switching_and_clicks() {
        let mut app = App::new();
        app.init_resource::<DroidMenuState>();
        app.add_systems(Update, handle_clicks);

        // Open menu and switch tabs
        app.world_mut().resource_mut::<DroidMenuState>().open = true;

        // Spawn interaction to switch to History tab
        let btn = app.world_mut().spawn((
            Interaction::Pressed,
            DroidMenuClick::Tab(DroidMenuTab::History),
        )).id();

        app.update();
        assert_eq!(app.world().resource::<DroidMenuState>().tab, DroidMenuTab::History);

        // Despawn and switch to TechTree
        app.world_mut().entity_mut(btn).despawn();
        let btn2 = app.world_mut().spawn((
            Interaction::Pressed,
            DroidMenuClick::Tab(DroidMenuTab::TechTree),
        )).id();
        app.update();
        assert_eq!(app.world().resource::<DroidMenuState>().tab, DroidMenuTab::TechTree);

        // Toggle hints
        app.world_mut().entity_mut(btn2).despawn();
        let btn_toggle = app.world_mut().spawn((
            Interaction::Pressed,
            DroidMenuClick::ToggleHints,
        )).id();
        app.update();
        assert!(!app.world().resource::<DroidMenuState>().enable_hints);

        // Close button
        app.world_mut().entity_mut(btn_toggle).despawn();
        app.world_mut().spawn((
            Interaction::Pressed,
            DroidMenuClick::Close,
        ));
        app.update();
        assert!(!app.world().resource::<DroidMenuState>().open);
    }

    #[test]
    fn history_tracker_records_samples() {
        let mut tracker = HistoryTracker::default();
        assert!(tracker.samples.is_empty());

        tracker.samples.push(HistorySample {
            week: 1.0,
            factions: vec![
                FactionRecord {
                    id: 0,
                    name: "Empire".into(),
                    color: Color::srgb(1.0, 0.0, 0.0),
                    credits: 50000.0,
                    income: 1200.0,
                    planets: 10,
                    military: 15,
                },
            ],
        });

        assert_eq!(tracker.samples.len(), 1);
        assert_eq!(tracker.samples[0].factions[0].name, "Empire");
        assert_eq!(tracker.samples[0].factions[0].planets, 10);
    }
}


/// Advisor messages of the campaign (the ticker lines), kept for the Droid Log archive.
#[derive(Resource, Default, Debug)]
pub(crate) struct AdvisorLog {
    /// Highest notification id recorded so far.
    last_id: u64,
    /// (galactic week, text), oldest first.
    pub entries: Vec<(u64, String)>,
}

/// Copies new ticker notifications into the [`AdvisorLog`].
fn record_advisor_log(model: Option<Res<galaxy_play::PlayUiModel>>, mut log: ResMut<AdvisorLog>) {
    let Some(model) = model else { return };
    let log = &mut *log;
    let seen = log.last_id;
    for n in model.notifications.iter().filter(|n| n.id > seen) {
        log.last_id = n.id;
        // Weekly income lines would fill the archive without telling anything new.
        if n.kind != galaxy_play::NotificationKind::Income {
            log.entries.push((model.top.week, n.text.clone()));
        }
    }
    let excess = log.entries.len().saturating_sub(200);
    log.entries.drain(..excess);
}

/// One line chart of a history metric: a polyline per faction (rotated thin nodes), the maximum on the y axis and the
/// recorded week range below.
fn draw_line_chart(
    parent: &mut ChildSpawnerCommands,
    history: &HistoryTracker,
    metric: HistoryMetric,
    w: f32,
    h: f32,
    font: &dyn Fn(f32) -> TextFont,
    blue: Color,
) {
    let value = |r: &FactionRecord| -> f32 {
        match metric {
            HistoryMetric::Planets => r.planets as f32,
            HistoryMetric::Military => r.military as f32,
            _ => r.credits as f32,
        }
    };
    let (title, unit) = match metric {
        HistoryMetric::Planets => ("TERRITORY (PLANETS)", ""),
        HistoryMetric::Military => ("MILITARY (SHIPS)", ""),
        _ => ("ECONOMY (CREDITS)", ""),
    };
    let samples = &history.samples;
    let max = samples.iter().flat_map(|s| s.factions.iter()).map(value).fold(1.0f32, f32::max);
    let n = samples.len();
    let point = |i: usize, v: f32| Vec2::new(i as f32 / (n - 1) as f32 * w, h - (v / max).clamp(0.0, 1.0) * h);
    parent
        .spawn(Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(4.0), ..default() })
        .with_children(|col| {
            col.spawn((Text::new(format!("{title}  MAX {max:.0}{unit}")), font(12.0), TextColor(blue)));
            col.spawn((
                Node { width: Val::Px(w), height: Val::Px(h), border: UiRect::left(Val::Px(1.0)).with_bottom(Val::Px(1.0)), ..default() },
                BackgroundColor(Color::srgba(0.01, 0.02, 0.05, 0.9)),
                BorderColor::all(Color::srgb(0.3, 0.4, 0.6)),
            ))
            .with_children(|plot| {
                // Horizontal grid.
                for k in 1..4 {
                    plot.spawn((
                        Node { position_type: PositionType::Absolute, left: Val::Px(0.0), top: Val::Px(h * k as f32 / 4.0), width: Val::Px(w), height: Val::Px(1.0), ..default() },
                        BackgroundColor(Color::srgba(0.3, 0.4, 0.6, 0.25)),
                    ));
                }
                let Some(last) = samples.last() else { return };
                for (fi, rec) in last.factions.iter().enumerate() {
                    for i in 1..n {
                        let (Some(a), Some(b)) = (samples[i - 1].factions.get(fi), samples[i].factions.get(fi)) else { continue };
                        let (pa, pb) = (point(i - 1, value(a)), point(i, value(b)));
                        let d = pb - pa;
                        let len = d.length() + 1.0;
                        let c = (pa + pb) * 0.5;
                        plot.spawn((
                            Node { position_type: PositionType::Absolute, left: Val::Px(c.x - len * 0.5), top: Val::Px(c.y - 1.0), width: Val::Px(len), height: Val::Px(2.0), ..default() },
                            UiTransform::from_rotation(Rot2::radians(d.y.atan2(d.x))),
                            BackgroundColor(rec.color),
                        ));
                    }
                }
            });
            col.spawn((
                Text::new(format!("WEEK {} - WEEK {}", samples[0].week as u32, samples[n - 1].week as u32)),
                font(11.0),
                TextColor(Color::srgb(0.6, 0.7, 0.85)),
            ));
        });
}
