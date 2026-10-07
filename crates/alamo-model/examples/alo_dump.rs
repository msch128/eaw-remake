//! Dumps a parsed model: `alo_dump <path-in-gamefs> [workshop_id|-] [--tree]`.
use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::Model;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.iter().find(|a| !a.starts_with("--")) else {
        eprintln!("usage: alo_dump <path-in-gamefs> [workshop_id|-] [--tree]");
        std::process::exit(2);
    };
    let id = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .nth(1)
        .map_or("1770851727", String::as_str);
    let install = GameInstall::detect().expect("Empire at War install not found");
    let fs = GameFs::for_mod(&install, (id != "-").then_some(id)).expect("file system");
    let data = fs.read(path).expect("read");
    if args.iter().any(|a| a == "--tree") {
        println!(
            "run: cargo run -p alamo-model --example chunk_tree -- {path} {id} for the raw chunk tree"
        );
    }
    match Model::parse(&data) {
        Ok(m) => print!("{m}"),
        Err(e) => {
            eprintln!("parse error: {e}");
            std::process::exit(1);
        }
    }
}
