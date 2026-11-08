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

#[test]
fn plan_scripts_serving_several_goals_are_found_under_each_goal() {
    let Some(install) = GameInstall::detect() else { return };
    if !install.mod_data("1770851727").is_dir() {
        return;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).unwrap();
    let plans = galaxy_ai::discover_plans(&fs);
    // UpgradeStarbasePlan.lua: Category = "Build_Starbase_Upgrade | Build_Initial_Starbase".
    for goal in ["build_starbase_upgrade", "build_initial_starbase"] {
        assert_eq!(plans.get(goal).map(Vec::as_slice), Some(&["UPGRADESTARBASEPLAN".to_string()][..]), "{goal}");
    }
    assert!(plans.keys().all(|k| !k.contains('|')), "no combined keys left");
}

#[test]
fn remake_hints_chokepoints_are_loaded_and_evaluated() {
    let Some(install) = GameInstall::detect() else { return };
    if !install.mod_data("1770851727").is_dir() {
        return;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).unwrap();
    let d = AiData::load(&fs);

    // 17 chokepoints in RemakeHints.xml:
    // 7 sectors at 1.0, 7 systems at 0.75, 3 gas giants at 0.5.
    let expected_1_0 = ["sjworlds", "hapes", "csilla", "kaas", "etti_iv", "endor", "moncalamari"];
    let expected_0_75 = ["corellia", "kuat", "kashyyyk", "kamino", "nalhutta", "the_maw", "onderon"];
    let expected_0_5 = ["bespin", "reltooine", "yavin"];

    let remake_hints = d.markup.get("remakehints").expect("remakehints markup file loaded");
    let chokepoint_count = remake_hints
        .iter()
        .filter(|(k, hints)| !k.ends_with("_ww") && hints.contains_key("chokepoint"))
        .count();
    assert_eq!(chokepoint_count, 17, "expected 17 distinct planets with chokepoint hint in RemakeHints.xml");

    for p in expected_1_0 {
        assert_eq!(d.hint(p, "chokepoint"), Some(1.0), "sector {p}");
        assert_eq!(d.hint(&format!("{p}_ww"), "chokepoint"), Some(1.0), "sector {p}_ww");
    }
    for p in expected_0_75 {
        assert_eq!(d.hint(p, "chokepoint"), Some(0.75), "system {p}");
        assert_eq!(d.hint(&format!("{p}_ww"), "chokepoint"), Some(0.75), "system {p}_ww");
    }
    for p in expected_0_5 {
        assert_eq!(d.hint(p, "chokepoint"), Some(0.5), "gas giant {p}");
        assert_eq!(d.hint(&format!("{p}_ww"), "chokepoint"), Some(0.5), "gas giant {p}_ww");
    }
    assert_eq!(d.hint("coruscant", "chokepoint"), None);

    // Test equation evaluation using PerceptionCtx.
    let mut equations = d.equations.clone();
    equations.insert("test_chokepoint", "Variable_Target.Hints.Chokepoint");

    let snap = galaxy_ai::perception::Snapshot {
        owner: vec![None; 4],
        real: vec![true; 4],
        slots: vec![(0, 0); 4],
        starbase: vec![(0, 0); 4],
        structures: vec![vec![]; 4],
        queued: vec![vec![]; 4],
        adjacency: vec![vec![]; 4],
        income: vec![0.0; 4],
        short_names: vec!["corellia".into(), "coruscant".into(), "sjworlds".into(), "bespin".into()],
        ..Default::default()
    };
    let brain = galaxy_ai::perception::PlayerBrainView::default();
    let mut rng = 42u64;

    // Planet 0: Corellia (0.75)
    let mut stats0 = galaxy_ai::equation::EvalStats::default();
    let mut ctx0 = galaxy_ai::perception::PerceptionCtx {
        snap: &snap,
        player: 0,
        target: Some(0),
        brain: &brain,
        equations: &equations,
        hints: &d.hints,
        rng: &mut rng,
        stats: &mut stats0,
    };
    let mut eval_stats = galaxy_ai::equation::EvalStats::default();
    let val_corellia = equations.evaluate("test_chokepoint", &mut ctx0, &mut eval_stats);
    assert_eq!(val_corellia, 0.75, "Corellia chokepoint evaluated");

    // Planet 1: Coruscant (not in hints -> 0.0)
    let mut stats1 = galaxy_ai::equation::EvalStats::default();
    let mut ctx1 = galaxy_ai::perception::PerceptionCtx {
        snap: &snap,
        player: 0,
        target: Some(1),
        brain: &brain,
        equations: &equations,
        hints: &d.hints,
        rng: &mut rng,
        stats: &mut stats1,
    };
    let val_coruscant = equations.evaluate("test_chokepoint", &mut ctx1, &mut eval_stats);
    assert_eq!(val_coruscant, 0.0, "Coruscant chokepoint evaluated");

    // Planet 2: SJWorlds (1.0)
    let mut stats2 = galaxy_ai::equation::EvalStats::default();
    let mut ctx2 = galaxy_ai::perception::PerceptionCtx {
        snap: &snap,
        player: 0,
        target: Some(2),
        brain: &brain,
        equations: &equations,
        hints: &d.hints,
        rng: &mut rng,
        stats: &mut stats2,
    };
    let val_sj = equations.evaluate("test_chokepoint", &mut ctx2, &mut eval_stats);
    assert_eq!(val_sj, 1.0, "SJWorlds chokepoint evaluated");

    // Planet 3: Bespin (0.5)
    let mut stats3 = galaxy_ai::equation::EvalStats::default();
    let mut ctx3 = galaxy_ai::perception::PerceptionCtx {
        snap: &snap,
        player: 0,
        target: Some(3),
        brain: &brain,
        equations: &equations,
        hints: &d.hints,
        rng: &mut rng,
        stats: &mut stats3,
    };
    let val_bespin = equations.evaluate("test_chokepoint", &mut ctx3, &mut eval_stats);
    assert_eq!(val_bespin, 0.5, "Bespin chokepoint evaluated");
}
