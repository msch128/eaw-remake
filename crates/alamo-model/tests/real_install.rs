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
