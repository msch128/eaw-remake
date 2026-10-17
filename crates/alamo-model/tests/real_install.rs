//! Tests against a real Empire at War install; skipped when none is found.
use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::image::decode_texture;
use alamo_model::{Animation, Model};

fn game_fs() -> Option<GameFs> {
    let install = GameInstall::detect()?;
    GameFs::new(std::slice::from_ref(&install.eaw_data)).ok()
}

#[test]
fn parses_all_base_game_models_and_animations() {
    let Some(fs) = game_fs() else { return };
    let (mut models, mut anims) = (0, 0);
    for f in fs.list("Art/Models") {
        let data = fs.read(&f).unwrap();
        if f.ends_with(".ALO") {
            match Model::parse(&data) {
                Ok(_) => models += 1,
                Err(alamo_model::Error::NotAModel(_)) => {}
                Err(e) => panic!("{f}: {e}"),
            }
        } else if f.ends_with(".ALA") {
            Animation::parse(&data).unwrap_or_else(|e| panic!("{f}: {e}"));
            anims += 1;
        }
    }
    assert!(models > 0 && anims > 0);
}

#[test]
fn decodes_a_texture() {
    let Some(fs) = game_fs() else { return };
    let Some(f) = fs
        .list("Art/Textures")
        .into_iter()
        .find(|f| f.ends_with(".DDS"))
    else {
        return;
    };
    let img = decode_texture(&fs.read(&f).unwrap()).unwrap();
    assert_eq!(img.pixels.len(), img.width * img.height * 4);
}

/// The Remake's files, if installed.
fn remake_fs() -> Option<GameFs> {
    let install = GameInstall::detect().filter(|i| i.mod_data("1770851727").is_dir())?;
    GameFs::for_mod(&install, Some("1770851727")).ok()
}

#[test]
fn the_eclipse_engine_runs_continuously_and_one_emitter_is_a_flat_sprite() {
    let Some(fs) = remake_fs() else { return };
    for system in ["Pe_Eclipse_Star_Dreadnaught_L", "Pe_Eclipse_Star_Dreadnaught_S"] {
        let data = fs.read(&format!("Art/Models/{system}.ALO")).unwrap();
        let sys = alamo_model::ParticleSystem::parse(&data).unwrap();
        assert_eq!(sys.emitters.len(), 8, "{system}");
        for e in &sys.emitters {
            // numParticlesPerBurst is the editor default 1 next to a rate of 1 to 20 per second: not bursts.
            assert!(!e.in_bursts && e.max_particles == 1 && e.spawn_rate >= 1.0, "{system}/{}", e.name);
            // Only the teal `engine_pulseG` disc is not billboarded.
            assert_eq!(!e.billboard, e.name == "engine_pulseG", "{system}/{}", e.name);
        }
    }
}
