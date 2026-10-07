//! Prints a summary of the galaxy data found in a mod's `Data` folder.
//!
//! Usage: `cargo run -p alamo-data --example galaxy_dump -- <data_dir> [campaign_name]`

use alamo_data::load_galaxy;
use alamo_fs::LooseDir;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(dir) = args.next() else {
        eprintln!("usage: galaxy_dump <data_dir> [campaign_name]");
        std::process::exit(2);
    };
    let wanted = args.next();
    let src = LooseDir::new(&dir).unwrap_or_else(|e| {
        eprintln!("cannot open {dir}: {e}");
        std::process::exit(1);
    });
    let (data, warnings) = load_galaxy(&src);

    println!(
        "{} campaigns, {} planet objects, {} trade routes, {} factions, {} strings, {} warnings",
        data.campaigns.len(),
        data.planets.len(),
        data.trade_routes.len(),
        data.factions.len(),
        data.text.len(),
        warnings.len()
    );
    for c in &data.campaigns {
        let title = c.text_id.as_deref().and_then(|id| data.text(id)).unwrap_or("-");
        println!(
            "  {:<40} set={:<12} locations={:<4} routes={:<4} title={title}",
            c.name,
            c.set.as_deref().unwrap_or("-"),
            c.locations.len(),
            c.trade_routes.len()
        );
    }

    let campaign = match &wanted {
        Some(n) => data.campaign(n),
        None => data.campaigns.iter().filter(|c| c.has_galaxy()).max_by_key(|c| c.locations.len()),
    };
    let Some(c) = campaign else {
        println!("no matching campaign with a galaxy");
        return;
    };
    let planets = data.campaign_planets(c);
    let real: Vec<_> = planets.iter().filter(|p| p.is_real_planet()).collect();
    let routes = data.campaign_trade_routes(c);
    println!("\nCampaign {} (active player: {})", c.name, c.starting_active_player.as_deref().unwrap_or("-"));
    println!("  locations: {} listed, {} resolved, {} real planets", c.locations.len(), planets.len(), real.len());
    println!("  trade routes: {} listed, {} resolved", c.trade_routes.len(), routes.len());
    println!("  factions:");
    for f in &c.factions {
        println!(
            "    {:<12} credits={:<8} tech={:?}/{:?} home={:<20} ai={:<16} planets={}",
            f.name,
            f.starting_credits.map_or("-".into(), |v| v.to_string()),
            f.starting_tech_level,
            f.max_tech_level,
            f.home_location.as_deref().unwrap_or("-"),
            f.ai_player_control.as_deref().unwrap_or("-"),
            f.presence_planets.len()
        );
    }
    println!("  sample planets:");
    for p in real.iter().take(8) {
        println!(
            "    {:<22} {:<28} pos=({:>7.1},{:>7.1},{:>5.1}) income={:<5} space_map={}",
            p.name,
            data.planet_display_name(p),
            p.position[0],
            p.position[1],
            p.position[2],
            p.credit_value,
            p.space_tactical_map.as_deref().unwrap_or("-")
        );
    }
    println!("  sample routes:");
    for r in routes.iter().take(5) {
        println!("    {:<8} {} <-> {} (speed {})", r.name, r.point_a, r.point_b, r.hs_speed_factor);
    }

    let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for w in &warnings {
        *counts.entry(w.message.split(['\'', '<']).next().unwrap_or("")).or_default() += 1;
    }
    println!("\nwarning kinds:");
    for (k, n) in counts {
        println!("  {n:>5}  {k}");
    }
}
