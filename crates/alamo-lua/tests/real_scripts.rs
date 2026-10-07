//! Tests against the real Remake scripts; skipped when the game or mod is not installed.

use std::sync::Arc;

use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_lua::{MockHost, ScriptRuntime};

const REMAKE: &str = "1770851727";

fn fs() -> Option<Arc<GameFs>> {
    let install = GameInstall::detect()?;
    if !install.mod_data(REMAKE).is_dir() {
        eprintln!("skipped: Remake mod not installed");
        return None;
    }
    GameFs::for_mod(&install, Some(REMAKE)).ok().map(Arc::new)
}

#[test]
fn every_script_compiles() {
    let Some(fs) = fs() else { return };
    let lua = mlua::Lua::new();
    let files: Vec<String> = fs
        .list("Scripts")
        .into_iter()
        .filter(|f| f.ends_with(".LUA"))
        .collect();
    let mut failed = Vec::new();
    for f in &files {
        let text = String::from_utf8_lossy(&fs.read(f).unwrap()).into_owned();
        if let Err(e) = lua.load(&text).set_name(f.as_str()).into_function() {
            failed.push(format!("{f}: {e}"));
        }
    }
    eprintln!(
        "compiled {}/{} scripts",
        files.len() - failed.len(),
        files.len()
    );
    assert!(files.len() > 600, "expected the Remake + base scripts");
    assert!(failed.is_empty(), "{failed:#?}");
}

#[test]
fn rebel_galactic_story_runs_clean() {
    let Some(fs) = fs() else { return };
    let mut rt = ScriptRuntime::new(fs, Box::new(MockHost::galactic_demo()));
    let plots = rt
        .load_story_plots("Story_Plots_Rebel_Web_Way_Full.xml", Some("Rebel"))
        .unwrap();
    assert_eq!(plots, 10);
    rt.spawn_script("GameScoring", &[])
        .expect("GameScoring starts");
    let mut t = 0.0;
    while t < 120.0 {
        rt.update(0.1);
        t += 0.1;
    }
    for e in rt.errors() {
        eprintln!("ERROR {}: {}", e.script, e.message);
    }
    assert!(rt.errors().is_empty());
    assert!(rt.story_fired() >= 10);
    let hits = rt.api_hits();
    let mut top: Vec<_> = hits.iter().collect();
    top.sort_by_key(|h| std::cmp::Reverse(*h.1));
    eprintln!(
        "api hits ({} distinct): {:?}",
        hits.len(),
        &top[..top.len().min(20)]
    );
    eprintln!("unimplemented: {:?}", rt.unimplemented());
    assert!(
        hits.contains_key("Spawn_Unit"),
        "the starting-force script spawned units"
    );
}
