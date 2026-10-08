//! Robustness run: every campaign of the installed Remake data is built and played headless for a few weeks, with the Remake AI and with the
//! built-in AI, with and without a human. A panic or a build error is reported per campaign; exit code 1 if any occurred.
//!
//! ```text
//! all_campaigns [weeks=10] [damage=<missing|truncated|corrupt>:<percent of files>:<seed>] [campaign=<name>]
//! ```
//! With `damage` the file source loses, cuts short or corrupts a share of the files (a partly downloaded or patched mod); everything must still load leniently.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{ScriptedConfig, ScriptedSim};
use galaxy_sim::{Event, SimConfig};

/// A file source that damages a deterministic share of the files of `inner`.
struct Damaged {
    inner: Arc<dyn FileSource>,
    mode: String,
    percent: u64,
    seed: u64,
}

impl Damaged {
    fn hash(&self, path: &str, salt: u64) -> u64 {
        let mut h = self.seed ^ salt ^ 0xCBF2_9CE4_8422_2325;
        for b in alamo_fs::normalize(path).bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3);
        }
        h ^ (h >> 29)
    }

    fn hit(&self, path: &str) -> bool {
        self.hash(path, 1) % 100 < self.percent
    }
}

impl FileSource for Damaged {
    fn read(&self, path: &str) -> std::io::Result<Vec<u8>> {
        let mut data = self.inner.read(path)?;
        if !self.hit(path) {
            return Ok(data);
        }
        match self.mode.as_str() {
            "missing" => return Err(std::io::Error::from(std::io::ErrorKind::NotFound)),
            "truncated" => data.truncate((self.hash(path, 2) % (data.len() as u64 + 1)) as usize),
            _ => {
                let n = (data.len() / 50).max(1);
                for k in 0..n as u64 {
                    let i = (self.hash(path, 10 + k) % data.len().max(1) as u64) as usize;
                    if let Some(b) = data.get_mut(i) {
                        *b = (self.hash(path, 1000 + k) & 0xFF) as u8;
                    }
                }
            }
        }
        Ok(data)
    }

    fn exists(&self, path: &str) -> bool {
        self.inner.exists(path) && !(self.mode == "missing" && self.hit(path))
    }

    fn list(&self, dir: &str) -> Vec<String> {
        self.inner.list(dir).into_iter().filter(|p| !(self.mode == "missing" && self.hit(p))).collect()
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let weeks: u64 = args.first().and_then(|a| a.parse().ok()).unwrap_or(10);
    let tactical = args.iter().any(|a| a == "tactical");
    let label_filter = args.iter().find_map(|a| a.strip_prefix("label=")).map(str::to_string);
    let only = args.iter().find_map(|a| a.strip_prefix("campaign=")).map(str::to_string);
    let install = GameInstall::detect().expect("game install not found");
    let vanilla = args.iter().any(|a| a == "vanilla");
    let mut fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, if vanilla { None } else { Some("1770851727") }).expect("cannot open game files"));
    if let Some(spec) = args.iter().find_map(|a| a.strip_prefix("damage=")) {
        let f: Vec<&str> = spec.split(':').collect();
        fs = Arc::new(Damaged { inner: fs, mode: f[0].to_string(), percent: f.get(1).and_then(|v| v.parse().ok()).unwrap_or(5), seed: f.get(2).and_then(|v| v.parse().ok()).unwrap_or(1) });
        println!("damaged data: {spec}");
    }
    let Ok((data, warnings)) = catch_unwind(AssertUnwindSafe(|| load_galaxy(&*fs))) else {
        println!("PANIC while loading the game data");
        std::process::exit(1);
    };
    let data = Arc::new(data);
    println!("{} data warnings, {} campaigns", warnings.len(), data.campaigns.len());
    let mut failures = 0;
    for campaign in data.campaigns.clone().into_iter().filter(|c| only.as_ref().is_none_or(|o| &c.name == o)) {
        for (label, remake_ai, human) in [("remake-ai", true, "none"), ("builtin-ai", false, "none"), ("remake-ai+human", true, "")] {
            if label_filter.as_ref().is_some_and(|l| l != label) {
                continue;
            }
            let (fs, data, campaign) = (fs.clone(), data.clone(), campaign.clone());
            let (fs2, data2) = (fs.clone(), data.clone());
            let name = campaign.name.clone();
            let t = std::time::Instant::now();
            let run = catch_unwind(AssertUnwindSafe(move || -> Result<String, String> {
                let human = if human.is_empty() { None } else { Some(human.to_string()) };
                let mut sim = ScriptedSim::new(fs, data, &campaign, SimConfig { victory_enabled: false, resolver: if tactical { galaxy_sim::BattleResolver::Tactical } else { galaxy_sim::BattleResolver::Statistical }, tactical_resolver_max_ships: 24, ..SimConfig::default() }, ScriptedConfig { remake_ai, human, game_scoring: false, ..ScriptedConfig::default() })
                    .map_err(|e| format!("build: {e}"))?;
                if tactical {
                    sim.with_sim_mut(|s| {
                        let names = s.tactical_names();
                        let names: Vec<&str> = names.iter().map(String::as_str).collect();
                        s.set_tactical_catalog(Arc::new(battle_sim::Catalog::build(&data2, Some(&*fs2), &names)));
                        let arenas: std::collections::HashMap<String, Arc<battle_sim::Arena>> = s.tactical_map_names().into_iter().filter_map(|n| battle_sim::Arena::load(&data2, &*fs2, &n).map(|a| (n, Arc::new(a)))).collect();
                        s.set_tactical_arenas(arenas);
                    });
                }
                if !remake_ai {
                    sim.with_sim_mut(|s| s.set_ai_factions(0..s.world().factions.len() as u32));
                }
                let (mut week, mut guard) = (0, 0);
                while week < weeks && guard < weeks * 80 {
                    guard += 1;
                    for e in sim.step(1.0).sim_events {
                        if let Event::WeekEnded { week: w } = e {
                            week = w;
                        }
                    }
                }
                let planets = sim.with_sim(|s| s.world().planets.len());
                Ok(format!("{planets} planets, week {week}, {} start errors, {} script errors", sim.start_errors.len(), sim.diagnostics().script_errors))
            }));
            match run {
                Ok(Ok(summary)) => println!("ok    {name:<36} {label:<16} {summary} ({:.1} s)", t.elapsed().as_secs_f32()),
                Ok(Err(e)) => println!("ERROR {name:<36} {label:<16} {e}"),
                Err(p) => {
                    failures += 1;
                    let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
                    println!("PANIC {name:<36} {label:<16} {msg}");
                }
            }
        }
    }
    println!("{}", if failures == 0 { "ALL CAMPAIGNS OK (no panics)".to_string() } else { format!("{failures} PANICS") });
    std::process::exit(i32::from(failures > 0));
}
