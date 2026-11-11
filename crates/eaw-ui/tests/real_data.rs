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
fn tactical_shell_layout() {
    let Some(fs) = fs() else { return };
    let Some(layout) = ShellLayout::load(&fs, "I_Tactical_Controls_New.ALO") else {
        return;
    };
    // 12 columns: each column c has top slot s_select_{2*c} and ability buttons special_button_{2*c}, special_button_{2*c+1}
    let btn00 = layout.item("special_button_00").expect("special_button_00");
    let btn01 = layout.item("special_button_01").expect("special_button_01");
    let btn02 = layout.item("special_button_02").expect("special_button_02");
    assert!((btn00.center.x - 380.0).abs() < 2.0);
    assert!((btn01.center.x - 407.6).abs() < 2.0);
    assert!((btn02.center.x - 435.0).abs() < 2.0);
    let sel00 = layout.item("s_select_00").expect("s_select_00");
    let sel02 = layout.item("s_select_02").expect("s_select_02");
    assert!((sel00.center.x - 380.0).abs() < 2.0 && (sel00.center.y - 93.0).abs() < 2.0);
    assert!((sel02.center.x - 435.0).abs() < 2.0 && (sel02.center.y - 93.0).abs() < 2.0);
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


/// The six repair phases name `I_Button_Repair_<Ship>.tga`, which exists nowhere: the pad shows the base game's repair button instead of
/// an empty disc.
#[test]
fn a_repair_phase_pad_has_an_icon() {
    let Some(fs) = fs() else { return };
    let lib = TextureLibrary::new_blocking(fs);
    let Some(generic) = lib.load("I_BUTTON_REPAIR.TGA") else { return };
    let phase = lib.load("I_Button_Repair_Executor.tga").expect("fallback icon");
    assert_eq!((phase.width, phase.height), (generic.width, generic.height));
}

/// The dialog emblems are bigger than the box that shows them (96 px): no enlargement.
#[test]
fn the_dialog_emblems_are_larger_than_their_box() {
    let Some(fs) = fs() else { return };
    let lib = TextureLibrary::new_blocking(fs);
    for n in ["Rebel_Symbol.dds", "Empire_Symbol.dds", "Consortium_Symbol.dds"] {
        let Some(t) = lib.load(n) else { return };
        assert!(t.width >= 96 && t.height >= 96, "{n} is {}x{}", t.width, t.height);
    }
}

/// A picked squadron's box shows the unit's icon art (Remake video v0700s): the craft of the play test's battles name an `Icon_Name`
/// that the texture library loads (before, the box had no art and showed the craft, a few pixels at the far zoom: an empty frame).
#[test]
fn every_squadron_craft_of_the_play_test_has_icon_art() {
    let Some(fs) = fs() else { return };
    let (game, _) = alamo_data::load_galaxy(&*fs);
    let lib = TextureLibrary::new_blocking(fs);
    for craft in ["TIE_Fighter", "TIE_Interceptor", "TIE_Bomber", "TIE_Fighter_G", "TIE_Fighter_181st_Soontir", "TIE_Advanced", "Z-95", "N1", "Skipray", "Skipray_E", "Tartan_Gunship", "Lambda_CT_G"] {
        let Some(icon) = eaw_ui::unit_icon(&game, craft) else { panic!("{craft} names no Icon_Name") };
        let t = lib.load(&icon).unwrap_or_else(|| panic!("{craft}: {icon} does not load"));
        assert!(t.width >= 32 && t.height >= 32, "{craft}: {icon} is {}x{}", t.width, t.height);
    }
}
