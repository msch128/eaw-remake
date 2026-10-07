//! Minimal command line parsing (no external dependency).

use bevy::prelude::Resource;
use std::path::PathBuf;

/// Parsed command line options.
#[derive(Resource, Debug, Clone)]
pub struct Args {
    /// Directory containing music files to play (`--music-dir <dir>`).
    pub music_dir: Option<PathBuf>,
    /// Only play files whose name starts with this prefix (`--music-prefix <p>`, case-insensitive).
    pub music_prefix: Option<String>,
    /// Master music volume, linear 0..1 (`--music-volume <v>`).
    pub music_volume: f32,
    /// Disable vertical sync (`--no-vsync`).
    pub no_vsync: bool,
    /// Start in borderless fullscreen (`--fullscreen`).
    pub fullscreen: bool,
    /// Game root directory override (`--game-dir <path>`).
    pub game_dir: Option<PathBuf>,
    /// Workshop mod id (`--mod <id>`).
    pub mod_id: Option<String>,
    /// Campaign to display (`--campaign <name>`).
    pub campaign: Option<String>,
    /// Print the galaxy campaigns and exit (`--list-campaigns`).
    pub list_campaigns: bool,
    /// Use the built-in demo galaxy even if a game install exists (`--demo`).
    pub demo: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            music_dir: None,
            music_prefix: None,
            music_volume: 0.5,
            no_vsync: false,
            fullscreen: false,
            game_dir: None,
            mod_id: None,
            campaign: None,
            list_campaigns: false,
            demo: false,
        }
    }
}

const USAGE: &str = "\
Space Peace: Not at War

USAGE: space-peace [OPTIONS]

OPTIONS:
  --music-dir <dir>      Play shuffled mp3 files from this directory
  --music-prefix <name>  Only play files starting with this prefix (e.g. GC_)
  --music-volume <0..1>  Master music volume (default 0.5)
  --game-dir <path>      Empire at War root (default: auto-detect / EAW_GAME_DIR)
  --mod <id>             Workshop mod id (default 1770851727)
  --campaign <name>      Galactic conquest campaign (default GC_Full_Web_Way_Rebel)
  --list-campaigns       List galaxy campaigns and exit
  --demo                 Use the built-in demo galaxy
  --no-vsync             Disable vertical sync
  --fullscreen           Start in borderless fullscreen
  -h, --help             Show this help
";

impl Args {
    /// Parses `std::env::args()`; prints usage and exits on `--help` or invalid input.
    pub fn from_env() -> Self {
        match Self::parse(std::env::args().skip(1)) {
            Ok(args) => args,
            Err(msg) => {
                eprintln!("{msg}\n\n{USAGE}");
                std::process::exit(if msg.is_empty() { 0 } else { 2 });
            }
        }
    }

    /// Parses an argument list (without the program name). An empty `Err` means "help requested".
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut out = Self::default();
        let mut it = args.into_iter();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--music-dir" => {
                    out.music_dir = Some(it.next().ok_or("--music-dir needs a value")?.into())
                }
                "--music-prefix" => {
                    out.music_prefix = Some(it.next().ok_or("--music-prefix needs a value")?)
                }
                "--music-volume" => {
                    let v = it.next().ok_or("--music-volume needs a value")?;
                    out.music_volume = v
                        .parse::<f32>()
                        .map_err(|e| format!("bad --music-volume: {e}"))?
                        .clamp(0.0, 1.0);
                }
                "--game-dir" => {
                    out.game_dir = Some(it.next().ok_or("--game-dir needs a value")?.into())
                }
                "--mod" => out.mod_id = Some(it.next().ok_or("--mod needs a value")?),
                "--campaign" => out.campaign = Some(it.next().ok_or("--campaign needs a value")?),
                "--list-campaigns" => out.list_campaigns = true,
                "--demo" => out.demo = true,
                "--no-vsync" => out.no_vsync = true,
                "--fullscreen" => out.fullscreen = true,
                "-h" | "--help" => return Err(String::new()),
                other => return Err(format!("unknown argument `{other}`")),
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flags() {
        let a = Args::parse(["--music-dir", "x", "--no-vsync"].map(String::from)).unwrap();
        assert_eq!(a.music_dir, Some(PathBuf::from("x")));
        assert!(a.no_vsync);
        assert!(Args::parse(["--bogus".to_string()]).is_err());
    }
}
