//! Verification of pirate starting forces across Web Way campaigns.
//!
//! In XML data (`Starting_Forces`), only 13 pirate planets define ships (the remaining
//! 56 pirate planets hold only a starbase). At week 0 (t = 0), `Universal_Story_Start`
//! triggers `Spawn_Starting_Forces_Web_Way.lua`, which populates pirate fleets on every
//! pirate planet.

use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{ScriptedConfig, ScriptedSim};
use galaxy_sim::{Sim, SimConfig};

fn setup(campaign_name: &str) -> Option<(Sim, ScriptedSim)> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        return None;
    }
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).ok()?);
    let (data, _) = load_galaxy(&*fs);
    let data = Arc::new(data);
    let campaign = data.campaign(campaign_name)?.clone();
    let sim = Sim::new(
        &data,
        &campaign,
        SimConfig {
            victory_enabled: false,
            ground_up: false,
            seed: 42,
            ..SimConfig::default()
        },
    )
    .ok()?;
    let scripted = ScriptedSim::new(
        fs,
        data,
        &campaign,
        SimConfig {
            victory_enabled: false,
            ground_up: false,
            seed: 42,
            ..SimConfig::default()
        },
        ScriptedConfig {
            remake_ai: true,
            difficulty: None,
            human: campaign.starting_active_player.clone(),
            ..ScriptedConfig::default()
        },
    )
    .ok()?;
    Some((sim, scripted))
}

#[test]
fn measure_pirate_fleets_all_campaigns() {
    let campaigns = [
        "GC_Full_Web_Way_Imperial",
        "GC_Full_Web_Way_Rebel",
        "GC_Full_Web_Way_Underworld",
    ];

    for campaign in campaigns {
        let Some((sim, mut scripted)) = setup(campaign) else {
            eprintln!("Skipping {campaign}: mod data not found");
            return;
        };
        let world = sim.world().clone();
        let pirates = world.faction_id("Pirates").unwrap();

        let mut pirate_planet_indices = Vec::new();
        for (i, p) in sim.galaxy().planets.iter().enumerate() {
            if p.owner == Some(pirates) {
                pirate_planet_indices.push(i);
            }
        }

        // 1. galaxy-sim alone at week 0 (XML Starting_Forces only: 13 planets have ships)
        let mut sim_fleet_count = 0;
        for &idx in &pirate_planet_indices {
            let has_fleet = sim
                .galaxy()
                .fleets
                .values()
                .any(|f| f.owner == pirates && f.planet() == Some(idx as u32));
            if has_fleet {
                sim_fleet_count += 1;
            }
        }
        assert_eq!(
            sim_fleet_count, 13,
            "{campaign}: XML starting forces must have ships on exactly 13 planets"
        );

        // 2. scripted-sim after step(1.0) (Spawn_Starting_Forces_Web_Way.lua populates all)
        scripted.step(1.0);
        let mut scripted_fleet_count = 0;
        scripted.with_sim(|s| {
            for &idx in &pirate_planet_indices {
                let has_fleet = s
                    .galaxy()
                    .fleets
                    .values()
                    .any(|f| f.owner == pirates && f.planet() == Some(idx as u32));
                if has_fleet {
                    scripted_fleet_count += 1;
                }
            }
        });
        assert_eq!(
            scripted_fleet_count,
            pirate_planet_indices.len(),
            "{campaign}: All {} pirate planets must hold a fleet after story start",
            pirate_planet_indices.len()
        );

        // 3. Test tech level event (Spawn_Neutral_Reinforcements_Web_Way.lua)
        if campaign == "GC_Full_Web_Way_Rebel" {
            scripted.runtime_mut().story_notify("STORY_TECH_LEVEL", "2");
            scripted.step(0.5);
            let snap = scripted.runtime().snapshot();
            let mut neutral_state = String::new();
            for s in &snap.scripts {
                if s.name == "Spawn_Neutral_Reinforcements_Web_Way" {
                    for (k, v) in &s.globals {
                        if k == "CurrentState" {
                            if let alamo_lua::LuaVal::Str(st) = v {
                                neutral_state = st.clone();
                            }
                        }
                    }
                }
            }
            assert_eq!(
                neutral_state, "Rebel_Tech_2",
                "Spawn_Neutral_Reinforcements_Web_Way must transition on tech level event"
            );
        }
    }
}
