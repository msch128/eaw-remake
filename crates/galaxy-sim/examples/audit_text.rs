//! Prints the localized strings whose key or text matches a regex-free substring list: `audit_text <mod> <word> [word...]`.
use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};

fn main() {
    let mut args = std::env::args().skip(1);
    let modid = args.next().unwrap_or_else(|| "1770851727".into());
    let words: Vec<String> = args.map(|w| w.to_ascii_lowercase()).collect();
    let install = GameInstall::detect().expect("game install");
    let fs = GameFs::for_mod(&install, Some(&modid)).expect("game files");
    let (data, _) = load_galaxy(&fs);
    for (k, v) in data.text.iter() {
        let (kl, vl) = (k.to_ascii_lowercase(), v.to_ascii_lowercase());
        if words.iter().any(|w| kl.contains(w) || vl.contains(w)) && v.len() < 200 {
            println!("{k} = {}", v.replace('\n', " | "));
        }
    }
}
