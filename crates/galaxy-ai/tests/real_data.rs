//! Parses the real Remake AI data; skipped without an install.

use alamo_fs::{GameFs, GameInstall};
use galaxy_ai::AiData;

#[test]
fn all_remake_equations_parse() {
    let Some(install) = GameInstall::detect() else { return };
    if !install.mod_data("1770851727").is_dir() {
        return;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).unwrap();
    let d = AiData::load(&fs);
    eprintln!(
        "files {}, players {}, function sets {}, goals {}, equations {} (parse errors {}), templates {}",
        d.files,
        d.players.len(),
        d.function_sets.len(),
        d.goals.len(),
        d.equations.len(),
        d.equations.parse_errors.len(),
        d.templates.len()
    );
    for (n, e) in d.equations.parse_errors.iter().take(20) {
        eprintln!("  parse error in {n}: {e}");
    }
    assert!(d.equations.len() > 500);
    assert!(d.equations.parse_errors.is_empty(), "{:?}", d.equations.parse_errors);
    assert!(d.players.contains_key("gc_empireai"));
    assert!(d.goals.contains_key("gc_conquer_enemy_planet"));
    assert_eq!(d.goal_function(&d.players["gc_empireai"].function_sets, "GC_Conquer_Enemy_Planet"), Some("Need_To_Conquer_Enemy_Planet"));
    assert!(d.templates.contains_key("empire_default"));
}
