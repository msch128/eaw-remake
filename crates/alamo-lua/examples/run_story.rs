//! Runs a campaign's story scripts against the mock host and prints what happened.
//!
//! ```text
//! run_story [workshop_id] [plots.xml] [seconds]
//! ```
use std::sync::Arc;

use alamo_fs::{GameFs, GameInstall};
use alamo_lua::{MockHost, ScriptRuntime};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let id = args.first().map(String::as_str).filter(|s| *s != "-");
    let plots = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "Story_Plots_Rebel_Web_Way_Full.xml".into());
    let secs: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(120.0);
    let install = GameInstall::detect().expect("game not found");
    let fs = Arc::new(GameFs::for_mod(&install, id).expect("fs"));
    let mut rt = ScriptRuntime::new(fs, Box::new(MockHost::galactic_demo()));
    match rt.load_story_plots(&plots, Some("Rebel")) {
        Ok(n) => println!("{n} plots activated"),
        Err(e) => println!("load error: {e}"),
    }
    if let Err(e) = rt.spawn_script("GameScoring", &[]) {
        println!("GameScoring: {e}");
    }
    let mut t = 0.0;
    while t < secs {
        rt.update(0.1);
        t += 0.1;
    }
    println!(
        "scripts alive {}, story events fired {}",
        rt.alive_scripts(),
        rt.story_fired()
    );
    for e in rt.errors() {
        println!("ERROR {}: {}", e.script, e.message);
    }
    println!("unimplemented: {:?}", rt.unimplemented());
    let mut hits: Vec<_> = rt.api_hits().into_iter().collect();
    hits.sort_by_key(|h| std::cmp::Reverse(h.1));
    println!("api hits: {:?}", &hits[..hits.len().min(25)]);
    for l in rt.log().iter().take(15) {
        println!("log: {l}");
    }
    rt.with_host(|_| ());
}
