//! Silent end-to-end checks of music selection and galaxy advisor events against the real Remake data
//! (resolve names -> files only; nothing is decoded to a device). Skipped when the mod is not installed.

use alamo_audio::{
    GalaxyEvent, MusicLibrary, MusicPlanner, MusicScene, SampleResolver, SfxLibrary,
    resolve_galaxy_event,
};
use alamo_fs::{GameFs, GameInstall};

const REMAKE: &str = "1770851727";

fn fs() -> Option<GameFs> {
    let install = GameInstall::detect()?;
    if !install.mod_data(REMAKE).is_dir() {
        eprintln!("skipped: Remake mod not installed");
        return None;
    }
    GameFs::for_mod(&install, Some(REMAKE)).ok()
}

fn step_to(p: &mut MusicPlanner, secs: f32) {
    let mut t = 0.0;
    while t < secs {
        p.step(1.0);
        t += 1.0;
    }
}

#[test]
fn music_flow_menu_to_victory() {
    let Some(fs) = fs() else { return };
    let mut w = Vec::new();
    let lib = MusicLibrary::load(&fs, &mut w);
    let resolver = SampleResolver::new(&fs);
    for faction in ["Empire", "Rebel"] {
        let mut p = MusicPlanner::default();
        let want = |p: &MusicPlanner| {
            let n = p
                .wanted_event(&lib)
                .unwrap_or_else(|| panic!("no event for {:?}", p.scene()));
            let ev = lib.get(&n).unwrap();
            assert!(!ev.files.is_empty(), "{n} has no files");
            for f in &ev.files {
                assert!(resolver.resolve(&fs, f).is_some(), "{n}: {f} missing");
            }
            n
        };
        p.set_scene(MusicScene::MainMenu);
        assert_eq!(want(&p), "Main_Menu_Music_Event");
        p.set_scene(MusicScene::Galactic(faction.into()));
        assert_eq!(want(&p), format!("Galactic_Map_{faction}_Music_Event"));
        p.set_scene(MusicScene::BattleLoading);
        assert_eq!(want(&p), "Tactical_Battle_Loading_Loop_Event");
        p.set_scene(MusicScene::Space(faction.into()));
        p.set_battle_intensity(0.05);
        p.step(1.0);
        assert_eq!(want(&p), format!("Space_Map_{faction}_Ambient_Music_Event"));
        p.set_battle_intensity(0.8);
        p.step(1.0);
        assert_eq!(want(&p), format!("Space_Map_{faction}_Battle_Music_Event"));
        // Brief lull keeps battle music; a long one returns to ambient.
        p.set_battle_intensity(0.0);
        step_to(&mut p, 10.0);
        assert_eq!(want(&p), format!("Space_Map_{faction}_Battle_Music_Event"));
        step_to(&mut p, 30.0);
        assert_eq!(want(&p), format!("Space_Map_{faction}_Ambient_Music_Event"));
        for win in [true, false] {
            p.set_scene(MusicScene::Result {
                faction: faction.into(),
                win,
                tactical: true,
            });
            let n = want(&p);
            let kind = if win { "Win" } else { "Lose" };
            assert_eq!(n, format!("{faction}_{kind}_Tactical_Event"));
        }
    }
}

#[test]
fn galaxy_events_resolve_to_files() {
    let Some(fs) = fs() else { return };
    let mut w = Vec::new();
    let sfx = SfxLibrary::load(&fs, &mut w);
    let resolver = SampleResolver::new(&fs);
    // Events the Remake data lacks for a faction are simply silent; the core advisor lines must exist.
    let core = [
        GalaxyEvent::UnitBuilt,
        GalaxyEvent::PlanetCaptured,
        GalaxyEvent::PlanetLost,
        GalaxyEvent::BattlePending,
        GalaxyEvent::ResearchComplete,
        GalaxyEvent::StationUnderAttack,
        GalaxyEvent::UnitDestroyed,
        GalaxyEvent::StructureDestroyed,
        GalaxyEvent::ReinforcementsAvailable,
    ];
    for faction in ["Empire", "Rebel", "Underworld"] {
        for ev in GalaxyEvent::ALL {
            let Some(name) = resolve_galaxy_event(&sfx, faction, ev) else {
                assert!(!core.contains(&ev), "{faction}: no event for {ev:?}");
                eprintln!("{faction}: {ev:?} has no advisor line");
                continue;
            };
            // Some optional lines (e.g. UHD_Retreat_Started_Space) name samples the data never shipped.
            if !core.contains(&ev) && matches!(ev, GalaxyEvent::RetreatStarted | GalaxyEvent::EnemyFleet | GalaxyEvent::ReinforcementsEnRoute) {
                continue;
            }
            let e = sfx.get(&name).unwrap();
            assert!(
                e.samples.iter().all(|s| resolver.resolve(&fs, s).is_some()),
                "{name}: unresolved sample"
            );
        }
    }
    let r = resolve_galaxy_event(&sfx, "Rebel", GalaxyEvent::PlanetCaptured).unwrap();
    let e = resolve_galaxy_event(&sfx, "Empire", GalaxyEvent::PlanetCaptured).unwrap();
    assert_eq!(
        (r.as_str(), e.as_str()),
        ("RHD_Planet_Gained", "EHD_Planet_Gained")
    );
}
