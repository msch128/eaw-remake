//! `STORY_TRIGGER` over the Remake's active Web Way plots. Skips when the mod is not installed.

use std::collections::BTreeSet;

use alamo_data::xml;
use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_lua::story::{Fired, Plot, StoryEngine};

const REMAKE: &str = "1770851727";

fn fs() -> Option<GameFs> {
    let install = GameInstall::detect()?;
    if !install.mod_data(REMAKE).is_dir() {
        eprintln!("skipped: Remake mod not installed");
        return None;
    }
    GameFs::for_mod(&install, Some(REMAKE)).ok()
}

/// Plot files named by the Web Way `Story_Plots_*` lists (the 52 the galaxy audit counted).
fn active_plots(fs: &GameFs) -> Vec<Plot> {
    let mut indexes: Vec<String> = fs
        .list("Xml")
        .into_iter()
        .filter(|p| {
            let name = p.rsplit(['/', '\\']).next().unwrap_or(p);
            let upper = name.to_ascii_uppercase();
            upper.starts_with("STORY_PLOTS")
                && (upper.contains("WEB_WAY") || upper.contains("_WW."))
        })
        .collect();
    indexes.sort();
    let mut files = BTreeSet::new();
    for index in &indexes {
        let bytes = fs.read(index).unwrap_or_else(|e| panic!("{index}: {e}"));
        let mut warnings = Vec::new();
        let Some(root) = xml::parse(&bytes, index, &mut warnings) else {
            continue;
        };
        for node in root.children_named("active_plot") {
            let file = node.text_trimmed().trim();
            if !file.is_empty() {
                files.insert(file.to_string());
            }
        }
    }
    let mut plots = Vec::new();
    for file in files {
        let bytes = fs
            .read(&format!("Xml/{file}"))
            .unwrap_or_else(|e| panic!("{file}: {e}"));
        let mut warnings = Vec::new();
        let plot =
            Plot::parse(&file, &bytes, &mut warnings).unwrap_or_else(|| panic!("unparsed {file}"));
        plots.push(plot);
    }
    plots
}

fn is_trigger(plot: &Plot, name: &str) -> bool {
    plot.events
        .iter()
        .any(|e| e.name == name && e.kind == "STORY_TRIGGER")
}

/// Fires every chain that is due at `now`, without advancing the clock.
fn drain_at(engine: &mut StoryEngine, now: f64) -> Vec<Fired> {
    let mut out = Vec::new();
    for _ in 0..8_000 {
        let batch = engine.tick(now);
        if batch.is_empty() {
            return out;
        }
        out.extend(batch);
    }
    panic!("story did not settle at {now} after {} fires", out.len());
}

/// Advances across elapsed timers until nothing else fires.
fn drain(engine: &mut StoryEngine, now: &mut f64) -> Vec<Fired> {
    let mut out = Vec::new();
    for _ in 0..8_000 {
        let batch = drain_at(engine, *now);
        if batch.is_empty() {
            let jumped = *now + 1.0e6;
            let next = drain_at(engine, jumped);
            if next.is_empty() {
                return out;
            }
            *now = jumped;
            out.extend(next);
            continue;
        }
        out.extend(batch);
    }
    panic!("story did not settle after {} fires", out.len());
}

fn key(file: &str, event: &str) -> (String, String) {
    (file.to_string(), event.to_string())
}

fn note(
    plots: &[Plot],
    batch: &[Fired],
    seen: &mut BTreeSet<(String, String)>,
    early: &mut Vec<String>,
) {
    for fired in batch {
        let plot = plots.iter().find(|p| p.file == fired.file).expect("plot");
        let event = plot
            .events
            .iter()
            .find(|e| e.name == fired.event)
            .expect("event");
        for prereq in &event.prereqs {
            let met = seen
                .iter()
                .any(|(f, n)| f == &fired.file && n.eq_ignore_ascii_case(prereq));
            if !met {
                early.push(format!(
                    "{} needs {prereq} before {}",
                    fired.file, fired.event
                ));
            }
        }
        seen.insert(key(&fired.file, &fired.event));
    }
}

#[test]
fn gated_story_triggers_fire_when_their_prerequisites_have() {
    let Some(fs) = fs() else { return };
    let plots = active_plots(&fs);
    assert_eq!(plots.len(), 52, "active Web Way plot files");

    let mut gated = 0usize;
    let mut force_click = 0usize;
    let mut waiting = BTreeSet::new();
    for plot in &plots {
        for event in &plot.events {
            if event.kind != "STORY_TRIGGER" {
                continue;
            }
            assert!(
                !event.prereqs.is_empty(),
                "{} / {} has no prerequisite",
                plot.file,
                event.name
            );
            gated += 1;
            let rewards: BTreeSet<&str> = event.rewards.iter().map(|r| r.kind.as_str()).collect();
            if rewards.iter().all(|k| *k == "FORCE_CLICK_GUI") && !rewards.is_empty() {
                force_click += 1;
            }
            // These wait on a construct or a GUI click. Time alone must not fire them.
            if rewards.iter().any(|k| {
                matches!(
                    *k,
                    "NEW_POWER_FOR_ALL" | "UNIQUE_UNIT" | "REMOVE_UNIT" | "FORCE_CLICK_GUI"
                )
            }) {
                waiting.insert(key(&plot.file, &event.name));
            }
        }
    }
    // The audit's 579 left out the 3 `FORCE_CLICK_GUI` triggers, which also have prerequisites.
    assert_eq!(gated, 582, "STORY_TRIGGER events in the active plots");
    assert_eq!(gated - force_click, 579);
    assert_eq!(waiting.len(), 35 + force_click);

    let mut engine = StoryEngine::default();
    for plot in plots.clone() {
        engine.activate(plot, 0, None, 0.0);
    }
    let mut now = 0.0;
    let at_start = drain_at(&mut engine, now);
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    let mut early = Vec::new();
    note(&plots, &at_start, &mut seen, &mut early);
    assert!(now < 1.0, "the opening tick stays at 0, got {now}");
    let intro_early = seen
        .iter()
        .any(|(_, n)| n.eq_ignore_ascii_case("Intro_Start_2"));
    assert!(
        !intro_early,
        "the second intro chapter waits for the 3 s elapsed chapter"
    );
    let opened_early: Vec<_> = waiting
        .iter()
        .filter(|k| seen.contains(k))
        .cloned()
        .collect();
    assert!(
        opened_early.is_empty(),
        "fired before their gate: {opened_early:?}"
    );

    let later = drain(&mut engine, &mut now);
    note(&plots, &later, &mut seen, &mut early);
    assert!(early.is_empty(), "fired before a prerequisite: {early:?}");
    let opened: Vec<_> = waiting
        .iter()
        .filter(|k| seen.contains(k))
        .map(|k| format!("{} / {}", k.0, k.1))
        .collect();
    assert!(
        opened.is_empty(),
        "a gated reward fired with no construct or click: {opened:?}"
    );
    // 211 under the reward-shape rule; the 32 intro chapters are the ones that rule left out.
    let time_only = seen
        .iter()
        .filter(|(f, n)| plots.iter().any(|p| p.file == *f && is_trigger(p, n)))
        .count();
    assert_eq!(time_only, 243, "gated triggers that fire on time alone");
    assert!(
        seen.iter()
            .any(|(_, n)| n.eq_ignore_ascii_case("Intro_Start_2")),
        "intro chapters fire once the elapsed chapter has"
    );
    assert!(
        !seen
            .iter()
            .any(|(_, n)| n.eq_ignore_ascii_case("Intro_Completed")),
        "closing the dialog is a separate event"
    );

    for param in [
        "Rebel_Shield_Upgrade",
        "DEFENDER_UPGRADE_Y_TO_K",
        "Black_Squadron_Upgrade",
        "181ST_UPGRADE_LN_TO_IN",
        "181ST_UPGRADE_IN_TO_D",
    ] {
        let batch = engine.notify(&["STORY_CONSTRUCT"], param, now);
        note(&plots, &batch, &mut seen, &mut early);
        let rest = drain(&mut engine, &mut now);
        note(&plots, &rest, &mut seen, &mut early);
    }
    assert!(early.is_empty(), "fired before a prerequisite: {early:?}");
    let still = waiting.iter().filter(|k| !seen.contains(k)).count();
    let fired_waiting = waiting.len() - still;
    assert_eq!(
        fired_waiting, 35,
        "the shield, 181st, Devastator and Defender grants"
    );
    assert_eq!(
        still, force_click,
        "only the GUI-click triggers are still waiting"
    );
    assert!(
        seen.iter()
            .any(|(f, n)| n == "Unlock_Shield_Upgrade_CR90" && f.contains("Shield")),
        "CR-90 reflector grant"
    );
    assert!(
        seen.iter().any(|(_, n)| n == "Unlock_Shield_Upgrade_DP20"),
        "DP-20 reflector grant"
    );
    assert!(seen.iter().any(|(_, n)| n == "Spawn_181_Interceptor"));
    assert!(seen.iter().any(|(_, n)| n == "Spawn_181_Defender"));
    assert!(seen.iter().any(|(_, n)| n == "Spawn_Devastator"));
    assert!(seen.iter().any(|(_, n)| n == "REMOVE_UNIT_Shield"));
}
