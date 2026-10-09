//! Opt-in self test of the real pointer path of the galaxy screens (`SP_UI_TEST=1`, or `=exit` to quit afterwards).
//!
//! Needs a real window (`space-peace --hidden ...`) and a scenario in which a battle is pending for the player
//! (`--select <planet> --weeks 8 --move <planet>`). Like `battle-view`'s input test it writes what the window system
//! would write (`CursorMoved`, `MouseButtonInput`, `KeyboardInput`, the window's cursor position) and lets Bevy's input
//! and UI focus systems do the rest. Checks: a click on the battle banner's Auto-Resolve button decides the battle;
//! Esc opens the pause menu; Save Game and a slot click write the save file; Options opens and Esc closes it; Main Menu
//! leaves the game. The verdict is one log line `ui-test: ... PASS|FAIL`. `SP_SAVES_DIR` should point at a scratch
//! folder so that the player's saves stay untouched.

use bevy::input::ButtonState;
use bevy::input::InputSystems;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::mouse::{MouseButtonInput, MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::window::{CursorMoved, PrimaryWindow};
use eaw_ui::EawButton;

use crate::flow::{AppScreen, Paused};
use crate::menu::{MenuAction, MenuButton};
use crate::newgame::{SlotMode, SlotScreen};
use crate::options::OptionsOpen;
use crate::play_ui::BattleDialogState;

pub(crate) fn build(app: &mut App) {
    if std::env::var_os("SP_UI_TEST").is_some() {
        app.init_resource::<TestState>().add_systems(PreUpdate, drive.before(InputSystems));
    }
}

#[derive(Resource, Default)]
struct TestState {
    phase: u32,
    t: u32,
    started: Option<std::time::SystemTime>,
    results: Vec<(&'static str, bool)>,
    done: bool,
    /// Phase clock value at which the phase's target was first on screen.
    found_at: Option<u32>,
    /// Value of the filter under test before its click.
    before: bool,
    /// Length of the production queue before the pad click.
    queue_before: usize,
    /// Pad slot (`b_create_NN`) under test and the entry it must show.
    pad: Option<(usize, usize)>,
    /// What that entry orders, read before the click.
    pad_target: Option<galaxy_play::BuildTarget>,
    /// Fleet used by the fleet-ring click phases.
    ring_fleet: Option<galaxy_sim::FleetId>,
    /// Phase clock value at which a multi-step phase saw its target (tooltip hover, objectives window).
    sub: Option<u32>,
    /// Step of the cheat menu phase and the credits read before the credit click.
    cheat_step: u32,
    credits_before: f64,
}

#[derive(Clone)]
enum Act {
    Move(Vec2),
    Down,
    Up,
    Key(KeyCode, Key),
    Wheel(f32),
}

/// Presses a button: move, press, release at fixed frame offsets of the phase clock.
fn click(st: &mut TestState, target: Option<Vec2>) -> Option<Act> {
    if target.is_some() && st.found_at.is_none() {
        st.found_at = Some(st.t.saturating_sub(1));
    }
    let t = st.found_at.map_or(u32::MAX, |f| (st.t - 1).saturating_sub(f));
    match (t, target) {
        (0, Some(p)) => {
            info!("ui-test: moving the pointer to {p:?}");
            Some(Act::Move(p))
        }
        (2, _) => Some(Act::Down),
        (4, _) => Some(Act::Up),
        _ => None,
    }
}

fn menu_button(nodes: &Query<(&MenuButton, &ComputedNode, &UiGlobalTransform, &InheritedVisibility)>, wanted: &MenuAction) -> Option<Vec2> {
    nodes
        .iter()
        .find(|(b, n, _, v)| &b.0 == wanted && v.get() && n.size().x > 4.0)
        .map(|(_, _, g, _)| g.translation)
}

#[allow(clippy::too_many_arguments)]
fn drive(
    mut st: ResMut<TestState>,
    screen: Res<State<AppScreen>>,
    dialog: Option<Res<BattleDialogState>>,
    (mut paused, mut tipctx): (ResMut<Paused>, TipCtx),
    slots: Res<SlotScreen>,
    (options, toast, mut ring): (Res<OptionsOpen>, Res<crate::play_ui::Toast>, RingCtx),
    (filters, bar, frames): (Res<crate::galaxy_map::MapFilters>, Res<crate::play_ui::BarState>, Res<bevy::diagnostic::FrameCount>),
    (mut pv, mut selection, bar_data, remap, model, (player, bridge)): (ResMut<crate::planet_view::PlanetViewState>, ResMut<crate::galaxy_map::Selection>, Res<eaw_ui::CommandBarData>, Res<eaw_ui::PadRemap>, Option<Res<galaxy_play::PlayUiModel>>, (Option<Res<galaxy_play::PlayerFaction>>, Option<Res<crate::play::PlayBridge>>)),
    sim: Option<Res<galaxy_play::SimState>>,
    (banner, menu): (
        Query<(&EawButton, &ComputedNode, &UiGlobalTransform, &InheritedVisibility)>,
        Query<(&MenuButton, &ComputedNode, &UiGlobalTransform, &InheritedVisibility)>,
    ),
    diag: Query<(Entity, Option<&Name>, &ComputedNode, &UiGlobalTransform, &InheritedVisibility, Option<&Interaction>, Option<&GlobalZIndex>)>,
    window: Option<Single<(Entity, &mut Window), With<PrimaryWindow>>>,
    mut moved: MessageWriter<CursorMoved>,
    mut buttons: MessageWriter<MouseButtonInput>,
    mut keys: MessageWriter<KeyboardInput>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(mut window) = window else { return };
    if st.done {
        return;
    }
    let t = st.t;
    st.t += 1;
    let mut act: Option<Act> = None;
    let finish = |st: &mut TestState, name: &'static str, ok: bool| {
        if !ok {
            for (b, n, g, v) in &menu {
                info!("ui-test:   menu button {:?} visible {} size {:?} at {:?}", b.0, v.get(), n.size(), g.translation);
            }
            if let Some(c) = window.1.physical_cursor_position() {
                for (e, n, node, g, v, i, z) in &diag {
                    if v.get() && node.size().x > 0.0 && node.contains_point(*g, c) {
                        info!("ui-test:   under the cursor: {e:?} {:?} interaction {i:?} z {:?}", n.map(|n| n.as_str()), z.map(|z| z.0));
                    }
                }
            }
        }
        info!("ui-test: {name} {}", if ok { "ok" } else { "FAILED" });
        st.results.push((name, ok));
        st.phase += 1;
        st.t = 0;
        st.found_at = None;
    };
    if t > 1500 && st.phase > 0 && (st.phase < 10 || (100..115).contains(&st.phase) || (120..122).contains(&st.phase) || (130..132).contains(&st.phase) || (140..145).contains(&st.phase)) {
        finish(&mut st, "phase timed out", false);
        return;
    }
    match st.phase {
        // 1: the battle banner's Auto-Resolve button.
        0 => {
            let open = dialog.as_ref().is_some_and(|d| d.open.is_some());
            if st.started.is_none() && open {
                st.started = Some(std::time::SystemTime::now());
                st.t = 0;
                return;
            }
            let target = banner
                .iter()
                .find(|(b, n, _, v)| b.name == eaw_ui::BANNER_AUTO && v.get() && n.size().x > 4.0)
                .map(|(_, _, g, _)| g.translation);
            if st.started.is_some() && target.is_some() {
                act = click(&mut st, target);
            }
            if st.started.is_some() && !open && dialog.as_ref().is_some_and(|d| d.decided.is_some()) {
                finish(&mut st, "banner-auto-resolve-click", true);
                st.phase = 100;
            } else if t > 3000 {
                finish(&mut st, "banner-auto-resolve-click (no banner, or the click did not arrive)", false);
            }
        }
        // Filter toggles next to the minimap: each click flips its map filter (credits, base level, ability, heroes).
        100..=103 => {
            let i = st.phase - 100;
            let name = format!("b_filters{i}");
            let read = |f: &crate::galaxy_map::MapFilters| [f.credits, f.base_level, f.weather, f.heroes][i as usize];
            if st.found_at.is_none() {
                st.before = read(&filters);
            }
            let target = banner.iter().find(|(b, n, _, v)| b.name == name && v.get() && n.size().x > 4.0).map(|(_, _, g, _)| g.translation);
            act = click(&mut st, target);
            if st.found_at.is_some_and(|f| t == f + 20) {
                let ok = read(&filters) != st.before;
                let label: &'static str = ["filter-credits-click", "filter-base-level-click", "filter-ability-click", "filter-heroes-click"][i as usize];
                finish(&mut st, label, ok);
                if i == 3 {
                    st.phase = 110;
                }
            }
        }
        // Class buttons of the left grid: each click sets the build-bar class filter, a second click on the same one clears it.
        110..=114 => {
            use crate::play_ui::BarClass::{Capital, Corvette, Frigate, Station};
            let (name, want, label): (&str, Option<crate::play_ui::BarClass>, &'static str) = match st.phase {
                110 => ("b_filters_corvette", Some(Corvette), "class-corvette-click"),
                111 => ("b_filters_frigate", Some(Frigate), "class-frigate-click"),
                112 => ("b_filters_capital", Some(Capital), "class-capital-click"),
                113 => ("b_filters_station", Some(Station), "class-station-click"),
                _ => ("b_filters_station", None, "class-station-second-click-clears"),
            };
            let target = banner.iter().find(|(b, n, _, v)| b.name == name && v.get() && n.size().x > 4.0).map(|(_, _, g, _)| g.translation);
            act = click(&mut st, target);
            if st.found_at.is_some_and(|f| t == f + 20) {
                let ok = bar.class == want;
                finish(&mut st, label, ok);
                if st.phase == 115 {
                    st.phase = 130;
                }
            }
        }
        // Fleet ring click at two zoom levels: a click on the drawn ring (not its centre) must select the fleet.
        130 | 131 => {
            let near = st.phase == 130;
            let mut verdict = None;
            let target = fleet_ring_phase(&mut st, t, near, &mut ring, window.1.scale_factor(), &mut verdict);
            act = click(&mut st, target);
            if let Some(v) = verdict {
                finish(&mut st, if near { "fleet-ring-click-near (setup)" } else { "fleet-ring-click-far (setup)" }, v);
            } else if st.found_at.is_some_and(|f| t == f + 20) {
                let id = st.ring_fleet;
                let ok = id.is_some_and(|id| ring.selection.as_deref().is_some_and(|s| s.fleets.contains(&id)));
                finish(&mut st, if near { "fleet-ring-click-near" } else { "fleet-ring-click-far" }, ok);
                if st.phase == 132 {
                    st.phase = 120;
                }
            }
        }
        // Planet view (the wheel path is covered elsewhere; here it is opened directly): the structure list fills the bottom
        // pad row left to right, and a real click on a pad of that row queues exactly the structure the entry shows.
        120 => {
            if selection.0.is_none() && let (Some(me), Some(sim), Some(bridge)) = (player.as_ref().and_then(|p| p.0), sim.as_ref(), bridge.as_ref()) {
                // The pending battle left no planet selected: take the first planet of the player.
                selection.0 = bridge.sim_to_planet.iter().enumerate().find_map(|(s, v)| (sim.galaxy().planets.get(s)?.owner == Some(me)).then_some((*v)?));
            }
            if !pv.active && selection.0.is_some() {
                pv.active = true;
                pv.planet = selection.0;
            }
            if t % 300 == 100 {
                info!("ui-test: planet view wait: active {} planet {:?} selection {:?} flag {} remap {} pads {}", pv.active, pv.planet, selection.0, bar_data.pads_bottom_row, remap.0.len(), bar_data.pads.len());
            }
            if t > 60 && bar_data.pads_bottom_row && !remap.0.is_empty() {
                let mut row: Vec<(usize, usize)> = remap.0.iter().map(|(&s, &e)| (s, e)).collect();
                row.sort_by_key(|r| r.1);
                let ordered = row.iter().enumerate().all(|(i, r)| r.1 == i);
                let pick = row.iter().find(|r| bar_data.pads.get(r.1).is_some_and(|p| !p.disabled)).copied();
                info!("ui-test: planet view bottom row {} slots, {} entries, remap contiguous {ordered}, pick {pick:?}", row.len(), bar_data.pads.len());
                if let (true, Some(p)) = (ordered, pick) {
                    st.pad = Some(p);
                    st.pad_target = bar.pads.get(p.1).copied().flatten();
                    st.queue_before = bar.queue.iter().flatten().count();
                    st.phase = 121;
                    st.t = 0;
                    st.found_at = None;
                } else {
                    finish(&mut st, "planet-view-pad-click (no affordable structure or remap not contiguous)", false);
                    st.phase = 1;
                }
            }
        }
        121 => {
            let Some((slot, entry)) = st.pad else { return };
            let name = format!("b_create_{slot:02}");
            let target = banner.iter().find(|(b, n, _, v)| b.name == name && v.get() && n.size().x > 4.0).map(|(_, _, g, _)| g.translation);
            act = click(&mut st, target);
            if st.found_at.is_some_and(|f| t == f + 40) {
                let queued = bar.queue.iter().flatten().count() > st.queue_before;
                let want = st.pad_target;
                let matches = want.is_some() && model.as_ref().and_then(|m| m.planet.as_ref()).is_some_and(|m| m.queue.iter().any(|q| Some(q.target) == want));
                info!("ui-test: pad {name} -> entry {entry}, queue {} -> {}", st.queue_before, bar.queue.iter().flatten().count());
                info!("ui-test: wanted {want:?}, model queue {:?}", model.as_ref().and_then(|m| m.planet.as_ref()).map(|m| m.queue.iter().map(|q| q.target).collect::<Vec<_>>()));
                let ground_lane = bar.queue.iter().enumerate().any(|(i, q)| q.is_some() && i >= 5);
                info!("ui-test: structure sits in the ground lane (right slots 5..10): {ground_lane}");
                finish(&mut st, "planet-view-pad-click-queues-structure", queued && matches && ground_lane);
                pv.active = false;
                st.phase = 140;
                st.sub = None;
            }
        }
        // Hover tooltip of a ship pad: the encyclopedia text of the unit, anchored right of the advisor droid above the info panel.
        140 => {
            let pick = bar.pads.iter().enumerate().find_map(|(i, p)| match p {
                Some(galaxy_play::BuildTarget::Unit(u)) if bar_data.pads.get(i).is_some_and(|e| !e.tooltip.is_empty()) => Some((i, *u)),
                _ => None,
            });
            let Some((entry, unit)) = pick else {
                if t > 900 {
                    finish(&mut st, "pad-tooltip-shows-lore (no ship pad on the bar)", false);
                    st.phase = 141;
                    st.sub = None;
                }
                return;
            };
            let name = format!("b_create_{entry:02}");
            let target = tipctx.buttons.iter().find(|(_, b, n, _, v)| b.name == name && v.get() && n.size().x > 4.0).map(|(_, _, _, g, _)| g.translation);
            let Some(target) = target else { return };
            let t0 = *st.sub.get_or_insert(t);
            match t - t0 {
                0 => act = Some(Act::Move(target)),
                20 => {
                    let game = tipctx.assets.as_ref().and_then(|a| a.game.clone());
                    let object = sim.as_ref().map(|s| s.world.units[unit as usize].name.clone());
                    let expected = object.and_then(|o| crate::play_ui::lore(game.as_deref(), &o));
                    let body = tipctx.parts.iter().find(|p| *p.0 == eaw_ui::TipPart::Body).map(|p| p.1.0.clone()).unwrap_or_default();
                    let title = tipctx.parts.iter().find(|p| *p.0 == eaw_ui::TipPart::Title).map(|p| p.1.0.clone()).unwrap_or_default();
                    let shown = tipctx.root.iter().next().is_some_and(|r| r.0.display != Display::None && *r.1 != Visibility::Hidden);
                    let want_title = bar_data.pads[entry].tooltip.lines().next().unwrap_or("").to_owned();
                    info!("ui-test: pad tooltip of {name}: shown {shown}, title {title:?}, body {} chars, lore from data {:?}", body.len(), expected.as_deref().map(|e| e.chars().take(60).collect::<String>()));
                    let lore_ok = expected.as_deref().is_some_and(|e| !e.is_empty() && body.trim() == e.trim());
                    // Anchor: the bubble sits above the droid's question-mark button, right of its left edge.
                    let droid = tipctx.buttons.iter().find(|b| b.1.name == "b_droid_help").map(|b| (b.2.size(), b.3.translation));
                    let anchored = match (tipctx.root.iter().next(), droid) {
                        (Some(r), Some((dsize, dpos))) => {
                            let (size, c) = (r.2.size(), r.3.translation);
                            let (left, bottom) = (c.x - size.x * 0.5, c.y + size.y * 0.5);
                            info!("ui-test: tooltip box left {left:.0} bottom {bottom:.0}; droid button left {:.0} top {:.0}", dpos.x - dsize.x * 0.5, dpos.y - dsize.y * 0.5);
                            left >= dpos.x - dsize.x * 0.5 && bottom <= dpos.y - dsize.y * 0.5 + 2.0 && left < dpos.x + dsize.x * 8.0
                        }
                        _ => false,
                    };
                    finish(&mut st, "pad-tooltip-shows-lore-text", shown && lore_ok && title == want_title);
                    st.results.push(("pad-tooltip-anchored-at-droid", shown && anchored));
                    info!("ui-test: pad-tooltip-anchored-at-droid {}", if shown && anchored { "ok" } else { "FAILED" });
                    // The tech upgrade entry explains what the next level unlocks.
                    let tech = bar.pads.iter().enumerate().find_map(|(i, p)| matches!(p, Some(galaxy_play::BuildTarget::Tech(_))).then_some(i));
                    match tech.and_then(|i| bar_data.pads.get(i)) {
                        Some(p) => {
                            info!("ui-test: tech pad tooltip: {:?}", p.tooltip);
                            let ok = p.tooltip.contains("Raises your tech level") && p.tooltip.contains("Unlocks ");
                            st.results.push(("tech-tooltip-lists-unlocks", ok));
                            info!("ui-test: tech-tooltip-lists-unlocks {}", if ok { "ok" } else { "FAILED" });
                        }
                        None => info!("ui-test: no tech upgrade pad on the bar, tech tooltip not exercised"),
                    }
                    st.phase = 141;
                    st.t = 0;
                    st.sub = None;
                }
                _ => {}
            }
        }
        // The small upkeep/income marker explains itself on hover.
        141 => {
            let pick = bar.pads.iter().enumerate().find_map(|(i, p)| match p {
                Some(galaxy_play::BuildTarget::Unit(_)) if bar_data.pads.get(i).is_some_and(|e| e.upkeep.abs() > 0.5 && !e.marker_tooltip.is_empty()) => Some(i),
                _ => None,
            });
            let Some(entry) = pick else {
                info!("ui-test: no pad with an upkeep marker on the bar ({} pads, upkeeps {:?}), marker tooltip not exercised", bar_data.pads.len(), bar_data.pads.iter().map(|p| p.upkeep as i32).collect::<Vec<_>>());
                st.phase = 142;
                st.t = 0;
                st.sub = None;
                return;
            };
            let name = format!("b_create_{entry:02}");
            let button = tipctx.buttons.iter().find(|(_, b, n, _, v)| b.name == name && v.get() && n.size().x > 4.0).map(|(e, ..)| e);
            let marker = button.and_then(|b| tipctx.markers.iter().find(|m| *m.0 == eaw_ui::widgets::SlotPart::UpkeepIcon && m.3.parent() == b && m.1.size().x > 1.0).map(|m| m.2.translation));
            let Some(marker) = marker else { return };
            let t0 = *st.sub.get_or_insert(t);
            match t - t0 {
                0 => act = Some(Act::Move(marker)),
                20 => {
                    let title = tipctx.parts.iter().find(|p| *p.0 == eaw_ui::TipPart::Title).map(|p| p.1.0.clone()).unwrap_or_default();
                    let body = tipctx.parts.iter().find(|p| *p.0 == eaw_ui::TipPart::Body).map(|p| p.1.0.clone()).unwrap_or_default();
                    info!("ui-test: marker tooltip: {title:?} / {body:?}");
                    let ok = (title == "Upkeep" || title == "Income bonus") && body.contains("credits per week");
                    finish(&mut st, "upkeep-marker-tooltip-explains", ok);
                    st.phase = 142;
                    st.t = 0;
                    st.sub = None;
                }
                _ => {}
            }
        }
        // Active Story Objectives: the upper list scrolls with the wheel.
        142 => {
            let list = tipctx.scroll.iter().find(|s| s.0.0 == 0).map(|s| (s.1.y, s.2.content_size().y > s.2.size().y + 1.0, s.3.translation));
            match (list, st.sub) {
                (None, _) => {
                    // The test campaign has a short list: pad it so that the list overflows its box and can scroll.
                    if tipctx.objectives.list.as_ref().is_none_or(|l| l.len() < 25) {
                        let rows = (0..25).map(|i| crate::objectives::Objective { title: format!("Test objective {i}"), color: [255, 255, 255], chapter: i, text: Vec::new() }).collect();
                        tipctx.objectives.list = Some(rows);
                    }
                    let target = tipctx.buttons.iter().find(|(_, b, n, _, v)| b.name == "b_story_arc_g" && v.get() && n.size().x > 4.0).map(|(_, _, _, g, _)| g.translation);
                    act = click(&mut st, target);
                }
                (Some((_, _, center)), None) => {
                    st.sub = Some(t);
                    act = Some(Act::Move(center));
                }
                (Some((y, scrollable, _)), Some(t0)) => match t - t0 {
                    8 => act = Some(Act::Wheel(-3.0)),
                    16 => {
                        info!("ui-test: objectives list scroll {y:.0} px, scrollable {scrollable}");
                        // The padded list overflows its box: the wheel must have moved it.
                        finish(&mut st, "objectives-list-scrolls-with-wheel", scrollable && y > 0.5);
                        act = Some(Act::Key(KeyCode::Escape, Key::Escape));
                        st.phase = 143;
                        st.t = 0;
                        st.found_at = None;
                    }
                    _ => {}
                },
            }
        }
        // The F2 cheat menu: opens, +5000 credits, AI pause toggles, closes.
        144 => {
            use crate::cheat_menu::CheatAction;
            let centre = |a: CheatAction| tipctx.cheats.iter().find(|c| *c.0 == a && c.1.size().x > 4.0).map(|c| c.2.translation);
            let credits = sim.as_ref().zip(player.as_ref().and_then(|p| p.0)).map_or(0.0, |(s, f)| s.galaxy().factions[f as usize].credits);
            let paused_ai = sim.as_ref().is_some_and(|s| s.galaxy().ai_paused);
            let t0 = *st.sub.get_or_insert(t);
            let dt = t - t0;
            match st.cheat_step {
                0 if dt == 0 => act = Some(Act::Key(KeyCode::F2, Key::F2)),
                0 if dt == 10 => {
                    let open = centre(CheatAction::Credits).is_some();
                    finish(&mut st, "f2-opens-cheat-menu", open);
                    st.phase = 144;
                    st.cheat_step = 1;
                    st.sub = None;
                    st.found_at = None;
                    st.credits_before = credits;
                }
                1 => {
                    act = click(&mut st, centre(CheatAction::Credits));
                    if st.found_at.is_some_and(|f| t == f + 20) {
                        // Income only adds on top of the 5000.
                        let ok = credits >= st.credits_before + 5000.0 - 0.5;
                        finish(&mut st, "cheat-credits-click-adds-5000", ok);
                        st.phase = 144;
                        st.cheat_step = 2;
                        st.found_at = None;
                    }
                }
                2 => {
                    act = click(&mut st, centre(CheatAction::AiPause));
                    if st.found_at.is_some_and(|f| t == f + 20) {
                        finish(&mut st, "cheat-ai-pause-toggles-on", paused_ai);
                        st.phase = 144;
                        st.cheat_step = 3;
                        st.found_at = None;
                    }
                }
                3 => {
                    act = click(&mut st, centre(CheatAction::AiPause));
                    if st.found_at.is_some_and(|f| t == f + 20) {
                        finish(&mut st, "cheat-ai-pause-toggles-off", !paused_ai);
                        st.phase = 144;
                        st.cheat_step = 4;
                        st.sub = None;
                        st.t = 0;
                    }
                }
                4 if dt == 1 => act = Some(Act::Key(KeyCode::F2, Key::F2)),
                4 if dt == 12 => {
                    let closed = centre(CheatAction::Credits).is_none();
                    finish(&mut st, "f2-closes-cheat-menu", closed);
                    st.phase = 1;
                    st.sub = None;
                }
                _ => {}
            }
        }
        // The "Main Menu" button of the left column opens the pause menu like Esc.
        143 => {
            if t < 20 {
                return;
            }
            let target = tipctx.buttons.iter().find(|(_, b, n, _, v)| b.name == "b_option_g" && v.get() && n.size().x > 4.0).map(|(_, _, _, g, _)| g.translation);
            act = click(&mut st, target);
            if st.found_at.is_some_and(|f| t == f + 20) {
                let ok = paused.0;
                finish(&mut st, "main-menu-button-opens-menu", ok);
                paused.0 = false;
                st.phase = 144;
                st.cheat_step = 0;
                st.sub = None;
                st.t = 0;
            }
        }
        // 2: Esc opens the pause menu.
        1 => match t {
            0 => act = Some(Act::Key(KeyCode::Escape, Key::Escape)),
            10 => {
                let ok = paused.0;
                finish(&mut st, "escape-opens-pause-menu", ok);
            }
            _ => {}
        },
        // 3: Save Game opens the slot list.
        2 => {
            let target = menu_button(&menu, &MenuAction::OpenSlots(SlotMode::Save));
            act = click(&mut st, target);
            if st.found_at.is_some_and(|f| t == f + 20) {
                let ok = slots.0 == Some(SlotMode::Save);
                finish(&mut st, "save-game-opens-slots", ok);
            }
        }
        // 4: a slot click writes the file.
        3 => {
            let target = menu_button(&menu, &MenuAction::Slot(SlotMode::Save, 1));
            act = click(&mut st, target);
            if t > 4 && t.is_multiple_of(20) {
                let fresh = crate::newgame::slot_path(1)
                    .and_then(|f| std::fs::metadata(f).ok())
                    .and_then(|m| m.modified().ok())
                    .is_some_and(|m| st.started.is_some_and(|s| m >= s));
                if fresh || t > 1200 {
                    finish(&mut st, "slot-click-writes-save", fresh);
                }
            }
        }
        // 5: back out of the slot list with its Back button.
        4 => {
            if slots.0.is_none() {
                let ok = toast.0 == "Game saved";
                finish(&mut st, "slot-list-closes-and-toasts-saved", ok);
            } else {
                let target = menu_button(&menu, &MenuAction::CloseSlots);
                act = click(&mut st, target);
                if st.found_at.is_some_and(|f| t == f + 20) {
                    let ok = slots.0.is_none();
                    finish(&mut st, "slot-list-back", ok);
                }
            }
        }
        // 6: Options opens.
        5 => {
            // Saving resumes the game: bring the pause menu back first.
            let target = if paused.0 { menu_button(&menu, &MenuAction::Options) } else { None };
            if !paused.0 && t.is_multiple_of(30) {
                act = Some(Act::Key(KeyCode::Escape, Key::Escape));
            }
            if paused.0 {
                act = click(&mut st, target);
            }
            if st.found_at.is_some_and(|f| t == f + 20) {
                let ok = options.0;
                finish(&mut st, "options-opens", ok);
            }
        }
        // 7: Esc closes it.
        6 => match t {
            0 => act = Some(Act::Key(KeyCode::Escape, Key::Escape)),
            10 => {
                let ok = !options.0;
                finish(&mut st, "escape-closes-options", ok);
            }
            _ => {}
        },
        // 8: Load Game opens the slot list.
        7 => {
            let target = menu_button(&menu, &MenuAction::OpenSlots(SlotMode::Load));
            act = click(&mut st, target);
            if st.found_at.is_some_and(|f| t == f + 20) {
                let ok = slots.0 == Some(SlotMode::Load);
                finish(&mut st, "load-game-opens-slots", ok);
            }
        }
        // 9: a slot click loads the save and closes the menu.
        8 => {
            let target = menu_button(&menu, &MenuAction::Slot(SlotMode::Load, 1));
            act = click(&mut st, target);
            if st.found_at.is_some_and(|f| t == f + 40) {
                let ok = slots.0.is_none() && !paused.0;
                finish(&mut st, "slot-click-loads-save", ok);
            }
        }
        // 10: Main Menu leaves the game.
        9 => {
            // The load restarts the galaxy behind its loading overlay; wait for the map.
            if *screen.get() == AppScreen::Galaxy && (sim.as_ref().is_none_or(|s| s.current.loading) || t < 200) {
                st.found_at = None;
                return;
            }
            let target = if paused.0 { menu_button(&menu, &MenuAction::MainMenu) } else { None };
            if !paused.0 && t.is_multiple_of(30) {
                act = Some(Act::Key(KeyCode::Escape, Key::Escape));
            }
            if paused.0 {
                act = click(&mut st, target);
            }
            if st.found_at.is_some_and(|f| t == f + 60) {
                let ok = *screen.get() == AppScreen::MainMenu;
                finish(&mut st, "main-menu-button", ok);
            }
        }
        _ => {
            let pass = st.results.iter().all(|r| r.1);
            info!("ui-test: {} checks {} (frame {})", st.results.len(), if pass { "PASS" } else { "FAIL" }, frames.0);
            st.done = true;
            if std::env::var("SP_UI_TEST").is_ok_and(|v| v == "exit") {
                exit.write(AppExit::Success);
            }
        }
    }
    let (entity, win) = &mut *window;
    match act {
        Some(Act::Move(p)) => {
            // Without change detection: bevy_winit would copy the position to the OS cursor and move the real mouse.
            win.bypass_change_detection().set_physical_cursor_position(Some(p.as_dvec2()));
            moved.write(CursorMoved { window: *entity, position: p / win.scale_factor(), delta: None });
        }
        Some(Act::Down) => {
            buttons.write(MouseButtonInput { button: MouseButton::Left, state: ButtonState::Pressed, window: *entity });
        }
        Some(Act::Up) => {
            buttons.write(MouseButtonInput { button: MouseButton::Left, state: ButtonState::Released, window: *entity });
        }
        Some(Act::Wheel(y)) => {
            tipctx.wheel.write(MouseWheel { unit: MouseScrollUnit::Line, x: 0.0, y, window: *entity, phase: bevy::input::touch::TouchPhase::Moved });
        }
        Some(Act::Key(code, key)) => {
            keys.write(KeyboardInput { key_code: code, logical_key: key.clone(), state: ButtonState::Pressed, text: None, repeat: false, window: *entity });
            keys.write(KeyboardInput { key_code: code, logical_key: key, state: ButtonState::Released, text: None, repeat: false, window: *entity });
        }
        None => {}
    }
}

/// Registers the test when `SP_UI_TEST` is set.
pub(crate) fn plugin(app: &mut App) {
    build(app);
}

/// Everything the fleet-ring click phases need.
#[derive(bevy::ecs::system::SystemParam)]
struct RingCtx<'w, 's> {
    camera: Query<'w, 's, (&'static Camera, &'static GlobalTransform, &'static mut crate::camera::GalaxyCamera)>,
    bounds: Option<Res<'w, crate::camera::CameraBounds>>,
    fleets: Option<Res<'w, galaxy_play::FleetEntities>>,
    visuals: Query<'w, 's, &'static Transform, With<galaxy_play::FleetVisual>>,
    layout: Option<Res<'w, galaxy_play::GalaxyLayout>>,
    cfg: Option<Res<'w, galaxy_play::PlayConfig>>,
    selection: Option<ResMut<'w, galaxy_play::PlayerSelection>>,
    sim: Option<Res<'w, galaxy_play::SimState>>,
    player: Option<Res<'w, galaxy_play::PlayerFaction>>,
}

/// One fleet-ring click: zooms the galaxy camera onto a parked fleet, then clicks on its owner ring (not on its centre)
/// and checks that the fleet got selected. Returns the pointer position to click (physical px), or `None` while waiting.
/// `ok` is set once the verdict is known.
fn fleet_ring_phase(st: &mut TestState, t: u32, near: bool, ctx: &mut RingCtx, scale: f32, ok: &mut Option<bool>) -> Option<Vec2> {
    let (Some(bounds), Some(fleets), Some(layout), Some(cfg), Some(sim)) = (ctx.bounds.as_deref(), ctx.fleets.as_deref(), ctx.layout.as_deref(), ctx.cfg.as_deref(), ctx.sim.as_deref()) else {
        *ok = Some(false);
        return None;
    };
    let me = ctx.player.as_deref().and_then(|p| p.0);
    if t == 0 {
        // Prefer an own parked fleet.
        let g = sim.galaxy();
        let parked = |id: &&galaxy_sim::FleetId| g.fleets.get(*id).is_some_and(|f| f.planet().is_some());
        let pick = fleets.0.keys().filter(parked).find(|id| g.fleets.get(*id).is_some_and(|f| Some(f.owner) == me)).or_else(|| fleets.0.keys().find(parked)).copied();
        st.ring_fleet = pick;
        if let Some(sel) = ctx.selection.as_deref_mut() {
            sel.fleets.clear();
        }
    }
    let Some(id) = st.ring_fleet else {
        info!("ui-test: fleet-ring click: no parked fleet on the map");
        *ok = Some(false);
        return None;
    };
    let Some(pos) = fleets.0.get(&id).and_then(|&e| ctx.visuals.get(e).ok()).map(|t| t.translation) else {
        *ok = Some(false);
        return None;
    };
    let Ok((camera, cam_tf, mut gc)) = ctx.camera.single_mut() else {
        *ok = Some(false);
        return None;
    };
    if t <= 1 || (t < 400 && (gc.distance - gc.target_distance).abs() > 0.5) || gc.focus.distance(gc.target_focus) > 0.5 {
        gc.target_distance = if near { bounds.min_distance * 1.5 } else { bounds.max_distance * 0.6 };
        gc.target_focus = Vec3::new(pos.x, 0.0, pos.z);
        return None;
    }
    // Aim at the ring's right edge, as a player clicking the ring would.
    let ring = layout.unit * cfg.fleet_icon_scale * 1.5;
    let center = Vec3::new(pos.x, pos.y - layout.unit * 0.1, pos.z);
    let right = cam_tf.right();
    let (Ok(c), Ok(e)) = (camera.world_to_viewport(cam_tf, center), camera.world_to_viewport(cam_tf, center + right * ring * 0.95)) else {
        *ok = Some(false);
        return None;
    };
    info!("ui-test: fleet-ring click ({}): camera distance {:.0}, ring radius {:.1} px, aiming {:.1} px from the centre", if near { "near" } else { "far" }, gc.distance, c.distance(e), c.distance(e));
    Some(e * scale)
}

/// Queries of the tooltip, objectives and main menu button phases.
#[derive(bevy::ecs::system::SystemParam)]
struct TipCtx<'w, 's> {
    root: Query<'w, 's, (&'static Node, &'static Visibility, &'static ComputedNode, &'static UiGlobalTransform), With<eaw_ui::Tooltip>>,
    parts: Query<'w, 's, (&'static eaw_ui::TipPart, &'static Text)>,
    markers: Query<'w, 's, (&'static eaw_ui::widgets::SlotPart, &'static ComputedNode, &'static UiGlobalTransform, &'static ChildOf)>,
    buttons: Query<'w, 's, (Entity, &'static EawButton, &'static ComputedNode, &'static UiGlobalTransform, &'static InheritedVisibility)>,
    scroll: Query<'w, 's, (&'static crate::play_ui::ObjectivesScroll, &'static ScrollPosition, &'static ComputedNode, &'static UiGlobalTransform)>,
    assets: Option<Res<'w, eaw_ui::EawUiAssets>>,
    cheats: Query<'w, 's, (&'static crate::cheat_menu::CheatAction, &'static ComputedNode, &'static UiGlobalTransform)>,
    wheel: MessageWriter<'w, MouseWheel>,
    objectives: ResMut<'w, crate::play_ui::ObjectivesWindow>,
}
