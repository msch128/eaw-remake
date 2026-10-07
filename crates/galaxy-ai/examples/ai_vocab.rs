//! Lists, per AI player type, the galactic goals, their desire equations and the variables / evaluator
//! scripts they need.
//!
//! ```text
//! ai_vocab [player type ...]
//! ```

use std::collections::BTreeSet;

use alamo_fs::{GameFs, GameInstall};
use galaxy_ai::AiData;
use galaxy_ai::equation::Dependencies;

fn main() {
    let install = GameInstall::detect().expect("game not found");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("fs");
    let d = AiData::load(&fs);
    let mut names: Vec<String> = std::env::args().skip(1).collect();
    if names.is_empty() {
        names = ["GC_EmpireAI", "GC_RebelAi", "GC_UnderworldAi", "GC_PiratesAi", "GC_MinorAi"].map(String::from).to_vec();
    }
    let mut all = Dependencies::default();
    for n in names {
        let Some(p) = d.players.get(&n.to_ascii_lowercase()) else {
            println!("{n}: not found");
            continue;
        };
        println!("== {} sets {:?} templates {:?}", p.name, p.function_sets, p.galactic_templates);
        let mut dep = Dependencies::default();
        let mut galactic = 0;
        let mut cats = BTreeSet::new();
        for set in &p.function_sets {
            for (goal, func) in d.function_sets.get(&set.to_ascii_lowercase()).into_iter().flatten() {
                let Some(g) = d.goals.get(&goal.to_ascii_lowercase()) else { continue };
                if !g.game_mode.eq_ignore_ascii_case("galactic") {
                    continue;
                }
                galactic += 1;
                cats.insert(g.category.clone());
                d.equations.dependencies(func, &mut dep);
            }
        }
        for t in &p.galactic_templates {
            if let Some(t) = d.templates.get(&t.to_ascii_lowercase()) {
                for eq in t.budget.values() {
                    d.equations.dependencies(eq, &mut dep);
                }
            }
        }
        println!("  galactic goals {galactic}, categories {cats:?}, equations {}, variables {}, scripts {:?}", dep.equations.len(), dep.variables.len(), dep.scripts);
        all.equations.extend(dep.equations);
        all.variables.extend(dep.variables);
        all.scripts.extend(dep.scripts);
    }
    println!("\nunion: {} equations, {} variables, scripts {:?}", all.equations.len(), all.variables.len(), all.scripts);
    for v in &all.variables {
        println!("  {v}");
    }
}