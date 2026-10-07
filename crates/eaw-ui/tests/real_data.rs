//! Tests against the real game install (skipped when it is not found).

use std::sync::Arc;

use alamo_fs::{GameFs, GameInstall};
use eaw_ui::components::CommandBarDb;
use eaw_ui::faction::UiFaction;
use eaw_ui::shell::ShellLayout;
use eaw_ui::textures::TextureLibrary;

fn fs() -> Option<Arc<GameFs>> {
    let install = GameInstall::detect()?;
    GameFs::for_mod(&install, Some("1770851727")).ok().map(Arc::new)
}

#[test]
fn galactic_shell_layout() {
    let Some(fs) = fs() else { return };
    let Some(layout) = ShellLayout::load(&fs, "I_Galactic_Controls_New.ALO") else {
        return; // mod not installed
    };
    let zoom = layout.item("B_ZOOM").expect("b_zoom");
    assert!((zoom.center.x - 28.0).abs() < 2.0 && (zoom.center.y - 26.0).abs() < 2.0);
    let (lo, hi) = layout.art_bounds(UiFaction::Rebel);
    assert!(hi.x - lo.x > 1300.0 && hi.y > 280.0);
    // One frame variant per faction.
    for f in [UiFaction::Empire, UiFaction::Rebel, UiFaction::Underworld] {
        assert!(layout.art_for(f).any(|a| a.name.contains("Faceplate")));
    }
}

#[test]
fn mega_texture_directory_and_components() {
    let Some(fs) = fs() else { return };
    let lib = TextureLibrary::new(fs.clone());
    if !lib.in_mega_texture("i_button_build.tga") {
        return;
    }
    assert!(!lib.is_ready());
    // Before the mega texture is decoded, packed textures are unavailable.
    assert!(lib.load("i_button_build.tga").is_none());
    let lib = TextureLibrary::new_blocking(fs.clone());
    let t = lib.load("I_BUTTON_BUILD.tga").expect("packed texture");
    assert_eq!((t.width, t.height), (50, 50));
    let (db, _) = CommandBarDb::load(&*fs);
    let zoom = db.get("b_zoom").expect("component");
    assert_eq!(zoom.tooltips, ["TEXT_TOOLTIP_ZOOM_BUTTON"]);
}

