//! The statistical resolver counts ground-to-space weapon structures (Hypervelocity Gun, Seismic Cannon, Nuke Launcher): they shoot with the damage
//! and recharge of their projectile, cannot be hit, and do not keep a side in the fight on their own. Real-data test, skips without the game and mod.

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::combat::{Member, SideInput, Source, resolve};
use galaxy_sim::rng::Rng;
use galaxy_sim::*;

fn load() -> Option<GameData> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).ok()?;
    Some(load_galaxy(&fs).0)
}

fn side(faction: FactionId, members: Vec<(Source, u32)>) -> SideInput {
    SideInput { faction, members: members.into_iter().map(|(source, count)| Member { source, fleet: None, count }).collect() }
}

#[test]
fn special_weapon_structures_have_the_damage_of_their_projectile() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    let sim = Sim::new(&data, campaign, SimConfig::default()).expect("sim");
    let w = sim.world();
    // Hypervelocity Gun: 60000 damage, 1 shot per 80 s.
    let hv = w.structure_id("Hypervelocity_Gun").unwrap();
    let stats = w.stats_of(Source::Structure(hv)).expect("the gun has numbers for the resolver");
    assert!((stats.raw_dps() - 60000.0 / 80.0).abs() < 1.0, "dps {}", stats.raw_dps());
    assert!(w.structures[hv as usize].combat.is_none(), "it stays no blockade or defender on its own");
    // Seismic Cannon: 2 shots of 9000 per 81 s; Nuke Launcher: 35000 hull damage per 80 s.
    let seismic = w.stats_of(Source::Structure(w.structure_id("Seismic_Cannon").unwrap())).unwrap();
    assert!((seismic.raw_dps() - 2.0 * 9000.0 / 81.0).abs() < 1.0, "dps {}", seismic.raw_dps());
    let nuke = w.stats_of(Source::Structure(w.structure_id("Nuke_Launcher").unwrap())).unwrap();
    assert!((nuke.raw_dps() - 35000.0 / 80.0).abs() < 1.0, "dps {}", nuke.raw_dps());
}

#[test]
fn a_gun_hurts_the_attackers_but_cannot_be_killed_or_keep_a_side_alive() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Imperial").expect("campaign");
    let sim = Sim::new(&data, campaign, SimConfig::default()).expect("sim");
    let w = sim.world();
    let (empire, rebel) = (w.faction_id("Empire").unwrap(), w.faction_id("Rebel").unwrap());
    let (gun, mc80, isd) = (w.structure_id("Hypervelocity_Gun").unwrap(), w.unit_id("MC-80_Star_Cruiser").unwrap(), w.unit_id("Star_Destroyer").unwrap());
    // A gun alone loses at once and is not reported as a survivor that wins: the attackers win.
    let r = resolve(w, &[side(rebel, vec![(Source::Unit(mc80), 2)]), side(empire, vec![(Source::Structure(gun), 1)])], &mut Rng::new(1));
    assert_eq!(r.winner, Some(0), "attackers beat a planet that only has a gun");
    // With a defending ship the gun adds damage: over many seeds the attackers keep fewer ships than against the ship alone.
    let kept = |with_gun: bool| -> u32 {
        (0..40)
            .map(|seed| {
                let mut defenders = vec![(Source::Unit(mc80), 1)];
                if with_gun {
                    defenders.push((Source::Structure(gun), 1));
                }
                let r = resolve(w, &[side(rebel, vec![(Source::Unit(isd), 1)]), side(empire, defenders)], &mut Rng::new(seed));
                r.survivors[0].iter().map(|m| m.count).sum::<u32>()
            })
            .sum()
    };
    assert!(kept(true) < kept(false), "the gun costs the attackers ships: {} with, {} without", kept(true), kept(false));
}
