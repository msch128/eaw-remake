//! Tells for each save file given on the command line what wrote it (format id, version, campaign, catalog counts, name tables or not) and
//! whether this build loads it; a file that loads is summarised by name and played on for two minutes of game time.
//! Read-only: `cargo run -p galaxy-script --example save_check -- <save.json>...`.
use std::collections::BTreeMap;
use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{SaveFile, ScriptedSim};
use galaxy_sim::UnitRole;

fn main() {
    let install = GameInstall::detect().expect("game install not found");
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).expect("game files"));
    let data = Arc::new(load_galaxy(&*fs).0);
    for path in std::env::args().skip(1) {
        println!("== {path}");
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                println!("   unreadable: {e}");
                continue;
            }
        };
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            let h = &v["header"];
            println!(
                "   format {} version {} campaign {} human {} world {} fingerprint {} name tables {} week {} time {}",
                v["format"],
                v["version"],
                h["campaign"],
                h["config"]["human"],
                h["world"],
                h["fingerprint"],
                if h["names"].is_object() { "yes" } else { "no" },
                v["checkpoint"]["week"],
                v["checkpoint"]["time"]
            );
        }
        let file = match SaveFile::read(&path) {
            Ok(f) => f,
            Err(e) => {
                println!("   READ FAILS: {e}");
                continue;
            }
        };
        let saved_factions = file.checkpoint.factions.clone();
        let t = std::time::Instant::now();
        match ScriptedSim::from_save(file, fs.clone(), data.clone(), |_| {}) {
            Ok(mut sim) => {
                let c = sim.checkpoint();
                println!("   LOADS in {:?}: week {} hash {:x}, {} warnings", t.elapsed(), c.week, c.galaxy_hash, sim.load_warnings.len());
                for note in &sim.load_notes {
                    println!("     note: {note}");
                }
                for w in sim.load_warnings.iter().take(8) {
                    println!("     warning: {w}");
                }
                // The summary of the file was written by the build that saved; the same numbers from the loaded state must agree.
                println!("   summary agrees with the file: {}", c.factions == saved_factions);
                sim.with_sim(|s| {
                    let (w, g) = (s.world(), s.galaxy());
                    // Ids that landed on the wrong names would show up as build items flying in fleets or foreign ships in a faction's hands.
                    let mut odd = 0;
                    let mut foreign = 0;
                    let mut by_faction: BTreeMap<u32, BTreeMap<&str, u32>> = BTreeMap::new();
                    for f in g.fleets.values() {
                        for (&u, &n) in &f.units {
                            let ut = &w.units[u as usize];
                            if !matches!(ut.role, UnitRole::Ship | UnitRole::Agent) {
                                odd += n;
                            }
                            let owner = w.factions[f.owner as usize].name.to_ascii_lowercase();
                            // Story heroes are `Neutral` in the data and fly for whoever the plot gives them to.
                            let anyones = ut.affiliation.is_empty() || ut.affiliation.iter().any(|a| a == "neutral");
                            if c.factions.iter().take(3).any(|x| x.0.eq_ignore_ascii_case(&owner)) && !anyones && !ut.affiliation.contains(&owner) {
                                foreign += n;
                                println!("     not of its roster: {n} {} ({:?}) in a fleet of {owner}", ut.name, ut.affiliation);
                            }
                            *by_faction.entry(f.owner).or_default().entry(ut.name.as_str()).or_default() += n;
                        }
                    }
                    let structures: usize = g.planets.iter().map(|p| p.structures.len()).sum();
                    println!("   {} fleets, {} structures on planets; fleet units that are build items: {odd}; ships of another faction's roster in a main faction's fleets: {foreign}", g.fleets.len(), structures);
                    for (f, units) in by_faction.iter().take(3) {
                        let mut top: Vec<(&&str, &u32)> = units.iter().collect();
                        top.sort_by(|a, b| b.1.cmp(a.1));
                        println!("     {}: {}", w.factions[*f as usize].name, top.iter().take(6).map(|(n, c)| format!("{c} {n}")).collect::<Vec<_>>().join(", "));
                    }
                });
                for _ in 0..120 {
                    sim.step(1.0);
                }
                let d = sim.diagnostics();
                println!("   played 120 s on: week {}, script errors {} ({:?})", sim.checkpoint().week, d.script_errors, d.last_script_error);
                let again = sim.to_save().expect("save again");
                println!("   saved again: name tables {}", if again.header.names.is_some() { "yes" } else { "no" });
            }
            Err(e) => println!("   LOAD FAILS: {e}"),
        }
    }
}
