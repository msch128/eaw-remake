//! AI tech levels in "From the Ground Up" games on the real Remake data (docs/RULES.md 1.12, "How AI tech works").
//!
//! This is the data's rubber band, not a defect: `Research/Story/Tech_Levels_<F>_WW.xml` (part of the human's plot
//! `Story_Plots_<F>_Web_Way_Full.xml`) gives the other factions the levels it names whenever the human player reaches a level
//! (`STORY_TECH_LEVEL` of the human -> `STORY_TRIGGER` events with `SET_TECH_LEVEL`), and `Story_SandBox_*_Ai_Locks.xml`
//! (`Lock_Research_Institute`) locks the research institute, which every `*_Tech-N` item needs, for the AI. So the AI factions
//! never research on their own: an idle human keeps them at tech 1, a researching human takes them along.
//! Skipped without the game and the mod.

use std::collections::BTreeMap;
use std::sync::Arc;

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_lua::story::Plot;
use galaxy_script::{DifficultyTier, ScriptedConfig, ScriptedSim};
use galaxy_sim::{Command, Event, FactionId, ProductionKind, SimConfig};

struct Ctx {
    fs: Arc<dyn FileSource>,
    data: Arc<GameData>,
}

fn ctx() -> Option<Ctx> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).ok()?);
    let (data, _) = load_galaxy(&*fs);
    Some(Ctx { fs, data: Arc::new(data) })
}

/// The Ground Up campaign of each playable major faction, the faction and its tech plot.
const CASES: [(&str, &str, &str); 3] = [
    ("GC_Full_Web_Way_Rebel", "Rebel", "Tech_Levels_R_WW.xml"),
    ("GC_Full_Web_Way_Imperial", "Empire", "Tech_Levels_E_WW.xml"),
    ("GC_Full_Web_Way_Underworld", "Underworld", "Tech_Levels_U_WW.xml"),
];

fn game(c: &Ctx, campaign: &str, human: &str, seed: u64) -> ScriptedSim {
    let campaign = c.data.campaign(campaign).expect("campaign").clone();
    ScriptedSim::new(
        c.fs.clone(),
        c.data.clone(),
        &campaign,
        SimConfig { victory_enabled: false, ground_up: true, seed, ..SimConfig::default() },
        ScriptedConfig { remake_ai: true, human: Some(human.into()), game_scoring: false, strategy: true, difficulty: Some(DifficultyTier::Normal), ..ScriptedConfig::default() },
    )
    .expect("sim")
}

/// Tech level of every faction, by lower-cased name.
fn all_tech(sim: &ScriptedSim) -> BTreeMap<String, i32> {
    sim.with_sim(|s| s.world().factions.iter().enumerate().map(|(i, f)| (f.name.to_ascii_lowercase(), s.galaxy().factions[i].tech)).collect())
}

/// What the plot hands out: `(human level, faction (lower case), level)` for every `SET_TECH_LEVEL` reward whose event waits for the
/// human's `<Human>_Tech_N` (a `STORY_TECH_LEVEL` event of level N).
fn plot_rewards(c: &Ctx, file: &str) -> Vec<(i32, String, i32)> {
    let bytes = c.fs.read(&format!("Xml/Research/Story/{file}")).expect("tech plot");
    let plot = Plot::parse(file, &bytes, &mut Vec::new()).expect("plot parses");
    let levels: BTreeMap<String, i32> =
        plot.events.iter().filter(|e| e.kind == "STORY_TECH_LEVEL").filter_map(|e| Some((e.name.to_ascii_lowercase(), e.params.first()?.trim().parse().ok()?))).collect();
    let mut out = Vec::new();
    for e in &plot.events {
        for r in e.rewards.iter().filter(|r| r.kind == "SET_TECH_LEVEL") {
            let at = e.prereqs.iter().flatten().filter_map(|p| levels.get(&p.to_ascii_lowercase())).max().copied().expect("the reward waits for a level");
            out.push((at, r.params[0].trim().to_ascii_lowercase(), r.params[1].trim().parse().expect("level")));
        }
    }
    out
}

/// Every faction's level once the human holds `level`: what it started with, raised to the highest reward the plot gives it up to
/// that level, within its `Max_Tech_Level`.
fn expected(start: &BTreeMap<String, i32>, max: &BTreeMap<String, i32>, rewards: &[(i32, String, i32)], human: &str, level: i32) -> BTreeMap<String, i32> {
    start
        .iter()
        .map(|(f, &t)| {
            let given = rewards.iter().filter(|(at, rf, _)| *at <= level && rf == f).map(|r| r.2).max().unwrap_or(t);
            let v = if f == human { level } else { t.max(given.min(max[f])) };
            (f.clone(), v)
        })
        .collect()
}

/// The human researches levels 2 to 5 the way a player does: the star base its research institute asks for, the institute, then
/// each `<F>_Tech-N` item, all through the production queues (credits are handed over, the time is not). In the step each level
/// completes, every other faction holds exactly the level the plot names for it, and no faction gets more.
#[test]
fn a_researching_human_takes_every_faction_to_the_levels_its_tech_plot_names() {
    let Some(c) = ctx() else { return };
    for (campaign, human, file) in CASES {
        let rewards = plot_rewards(&c, file);
        assert!(rewards.iter().any(|r| r.1 != human.to_ascii_lowercase() && r.2 == 5), "{file}: levels up to 5 for the other majors: {rewards:?}");
        let mut sim = game(&c, campaign, human, 1);
        let h: FactionId = sim.with_sim(|s| s.world().faction_index[&human.to_ascii_lowercase()]);
        let start = all_tech(&sim);
        let max: BTreeMap<String, i32> = sim.with_sim(|s| s.world().factions.iter().map(|f| (f.name.to_ascii_lowercase(), f.max_tech)).collect());
        let (home, institute) = sim.with_sim(|s| {
            (s.galaxy().owned_planets(h).next().expect("home"), s.world().structure_index[&format!("{}_research_institute", &human[..1]).to_ascii_lowercase()])
        });
        let mut reached = Vec::new();
        let mut week = 0;
        while week < 70 && reached.len() < 4 {
            sim.with_sim_mut(|s| {
                s.add_credits(h, 50_000.0 - s.galaxy().factions[h as usize].credits.min(50_000.0));
                let ps = &s.galaxy().planets[home as usize];
                let has_institute = ps.structures.contains(&institute) || ps.queues.iter().flatten().any(|i| i.kind == ProductionKind::Structure(institute));
                if !has_institute {
                    if s.apply(Command::BuildStructure { faction: h, planet: home, structure: institute }).is_err() {
                        let _ = s.apply(Command::UpgradeStarbase { faction: h, planet: home });
                    }
                } else {
                    let _ = s.apply(Command::UpgradeTech { faction: h, planet: home });
                }
            });
            let events = sim.step(1.0).sim_events;
            for e in &events {
                match e {
                    Event::WeekEnded { week: w } => week = *w,
                    Event::TechAdvanced { faction, level } if *faction == h => {
                        let now = all_tech(&sim);
                        let want = expected(&start, &max, &rewards, &human.to_ascii_lowercase(), *level);
                        println!("{campaign}: the human {human} reaches tech {level} in week {week}: {now:?}");
                        assert_eq!(now, want, "{campaign}: every faction's level in the step the human reaches {level}");
                        reached.push((*level, week));
                    }
                    _ => {}
                }
            }
        }
        assert_eq!(reached.iter().map(|r| r.0).collect::<Vec<_>>(), vec![2, 3, 4, 5], "{campaign}: the human reaches levels 2 to 5 within 70 weeks: {reached:?}");
    }
}

/// The week each major faction reaches each level in 60 weeks of `GC_Full_Web_Way_Rebel` Ground Up with no human, seed 1.
fn all_ai_curve(c: &Ctx) -> BTreeMap<String, Vec<(i32, u64)>> {
    let mut sim = game(c, "GC_Full_Web_Way_Rebel", "none", 1);
    let (majors, names): (Vec<FactionId>, Vec<String>) = sim.with_sim(|s| (s.world().main_factions.clone(), s.world().factions.iter().map(|f| f.name.clone()).collect()));
    let mut curve: BTreeMap<String, Vec<(i32, u64)>> = majors.iter().map(|&f| (names[f as usize].clone(), Vec::new())).collect();
    let mut week = 0;
    while week < 60 {
        let events = sim.step(1.0).sim_events;
        for e in &events {
            match e {
                Event::WeekEnded { week: w } => week = *w,
                Event::TechAdvanced { faction, level } if majors.contains(faction) => curve.get_mut(&names[*faction as usize]).expect("major").push((*level, week)),
                _ => {}
            }
        }
    }
    curve
}

/// With no human nothing in the data would ever raise a level (the AI cannot research, the tech plot waits for the human's levels),
/// so the engine's estimate buys them (`galaxy-ai` engine, docs/RULES.md 1.12): every 4 weeks, with more than 20000 credits, level
/// L + 1 only from week 10 x L on. The Rebel plot (`Tech_Levels_R_WW.xml`, owned by the AI Rebel here) still hands the Rebel's levels
/// to the others, but only the Rebel's own: before `STORY_TECH_LEVEL` was limited to the plot's faction, the Empire's level fired
/// the Rebel plot too and the Empire and the Underworld rose together (levels 2/3/4/5 in weeks 13/18/33/41, level 3 before week 20).
/// Pinned: the weeks of seed 1, and no level L before week 10 x (L - 1).
#[test]
fn without_a_human_the_levels_come_from_the_engines_timer_estimate() {
    let Some(c) = ctx() else { return };
    let curve = all_ai_curve(&c);
    println!("all-AI levels (level, week) per major: {curve:?}");
    for (f, levels) in &curve {
        for &(level, week) in levels {
            assert!(week >= 10 * u64::try_from(level - 1).unwrap(), "{f}: level {level} in week {week}, before the estimate allows it");
        }
    }
    let pinned: BTreeMap<String, Vec<(i32, u64)>> = [
        ("Rebel", vec![(2, 20), (3, 34), (4, 49)]),
        ("Empire", vec![(2, 18), (3, 34), (4, 49)]),
        ("Underworld", vec![(2, 20), (3, 34), (4, 49), (5, 57)]),
    ]
    .into_iter()
    .map(|(f, v)| (f.to_string(), v))
    .collect();
    assert_eq!(curve, pinned, "all-AI tech curve, seed 1");
}

/// A human who has lost (eliminated; the "Stay on the map" continuation) is still the game's human: the engine's estimate stays off,
/// and with no one left to reach a level the plot hands out nothing more, so the AI factions keep the levels they had. Here they had
/// tech 1 and keep it for 30 weeks.
#[test]
fn after_the_human_is_eliminated_the_ai_factions_keep_their_levels() {
    let Some(c) = ctx() else { return };
    let mut sim = game(&c, "GC_Full_Web_Way_Rebel", "Rebel", 1);
    sim.with_sim_mut(|s| {
        let h = s.world().faction_index["rebel"];
        let pirates = s.world().passive_faction().expect("pirates");
        let g = s.galaxy_mut();
        for p in g.planets.iter_mut().filter(|p| p.owner == Some(h)) {
            p.owner = Some(pirates);
        }
        g.fleets.retain(|_, f| f.owner != h);
        g.sync_ships();
        g.factions[h as usize].eliminated = true;
    });
    let start = all_tech(&sim);
    let mut week = 0;
    while week < 30 {
        for e in sim.step(1.0).sim_events {
            if let Event::WeekEnded { week: w } = e {
                week = w;
            }
        }
    }
    assert_eq!(all_tech(&sim), start, "30 weeks after the human's defeat");
}

/// An idle human (no orders) keeps every faction at its start level for 60 weeks: no AI faction researches on its own while a human
/// plays (the data's rubber band, see the file comment). Pinned: every major at tech 1 every week, in all three Ground Up campaigns.
#[test]
fn an_idle_human_keeps_every_faction_at_its_start_level_for_sixty_weeks() {
    let Some(c) = ctx() else { return };
    for (campaign, human, _) in CASES {
        let mut sim = game(&c, campaign, human, 1);
        let start = all_tech(&sim);
        let majors: Vec<String> = sim.with_sim(|s| s.world().main_factions.iter().map(|&f| s.world().factions[f as usize].name.to_ascii_lowercase()).collect());
        assert!(majors.iter().all(|m| start[m] == 1), "{campaign}: the majors start at tech 1: {start:?}");
        let mut week = 0;
        while week < 60 {
            for e in sim.step(1.0).sim_events {
                if let Event::WeekEnded { week: w } = e {
                    week = w;
                    assert_eq!(all_tech(&sim), start, "{campaign}: week {week}");
                }
            }
        }
        println!("{campaign}: idle human {human}, week {week}: {:?}", all_tech(&sim));
    }
}
