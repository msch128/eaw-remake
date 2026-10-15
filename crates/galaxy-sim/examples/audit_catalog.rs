//! Build-bar vs battle audit: `audit_catalog <mod id or fft> <campaign>`.
//! For every ship in a faction's build bar: does the battle catalog know it, has it a model file, hull, shield, weapons?
//! Output lines: `BAR faction | unit | category | tech | model(ok/MISSING) | hull | shield | hardpoints | dps | range | notes`.
use alamo_data::{ShipCategory, load_galaxy};
use alamo_fs::{FileSource, GameFs, GameInstall};
use battle_sim::Catalog;
use galaxy_sim::*;

fn main() {
    let mut args = std::env::args().skip(1);
    let modid = args.next().unwrap_or_else(|| "1770851727".into());
    let campaign = args.next().unwrap_or_else(|| "GC_Full_Web_Way_Rebel".into());
    let install = GameInstall::detect().expect("game install");
    let fs = GameFs::for_mod(&install, Some(&modid)).expect("game files");
    let (data, _) = load_galaxy(&fs);
    let c = data.campaign(&campaign).expect("campaign");
    let sim = Sim::new(&data, c, SimConfig::default()).expect("sim");
    let w = sim.world();
    let mut names: Vec<String> = Vec::new();
    for (fi, _) in w.factions.iter().enumerate() {
        for &u in &w.buildable_units[fi] {
            let ut = &w.units[u as usize];
            if ut.role == UnitRole::Ship {
                names.push(ut.name.clone());
            }
        }
    }
    names.sort();
    names.dedup();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let cat = Catalog::build(&data, Some(&fs), &refs);
    let model_ok = |m: &Option<String>| match m {
        Some(m) if fs.exists(&format!("Art/Models/{m}")) => "ok",
        Some(_) => "MISSING",
        None => "NONE",
    };
    let mut bad = 0;
    for (fi, f) in w.factions.iter().enumerate() {
        let mut n = 0;
        for &u in &w.buildable_units[fi] {
            let ut = &w.units[u as usize];
            if ut.role != UnitRole::Ship {
                continue;
            }
            n += 1;
            let Some(id) = cat.ship_id(&ut.name) else {
                println!("BAR {} | {} | {:?} | tech {} | NOT IN BATTLE CATALOG", f.name, ut.name, ut.category, ut.tech);
                bad += 1;
                continue;
            };
            let d = cat.ship(id);
            let dps: f32 = d.bearing_dps.iter().sum::<f32>() / d.bearing_dps.len() as f32;
            let mut note = String::new();
            if d.is_squadron {
                let ms: Vec<String> = d.members.iter().map(|(m, n)| format!("{}x{}[{}]", n, cat.ship(*m).name, model_ok(&cat.ship(*m).model))).collect();
                note = format!("members {}", ms.join(","));
            }
            let mo = model_ok(&d.model);
            let weak = !d.is_squadron && d.hull <= 0.0;
            let armed = dps > 0.0 || d.direct.is_some() || d.is_squadron || d.hardpoints.iter().any(|h| h.destroyable) || ut.category == ShipCategory::Transport;
            if mo != "ok" && !d.is_squadron || weak || !armed {
                bad += 1;
            }
            println!(
                "BAR {} | {} | {:?} | tech {} | model {} | hull {:.0} | shield {:.0} | hp {} | dps {:.0} | range {:.0} | armed {} | {}",
                f.name, ut.name, ut.category, ut.tech, mo, d.hull, d.shield, d.hardpoints.len(), dps, d.max_weapon_range, armed, note
            );
        }
        println!("# {}: {} ships in the build bar", f.name, n);
    }
    println!("# problems: {bad}");
}
