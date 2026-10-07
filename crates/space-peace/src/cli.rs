//! Minimal command line parsing (no external dependency).

use bevy::prelude::Resource;
use std::path::PathBuf;

/// Parsed command line options.
#[derive(Resource, Debug, Clone)]
pub struct Args {
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
    /// Render offscreen to this PNG instead of opening a window (`--offscreen <png>`).
    pub offscreen: Option<PathBuf>,
    /// Number of frames to render in offscreen mode (`--frames <n>`).
    pub frames: u32,
    /// Screen to start on: `menu`, `campaigns` or `galaxy` (`--screen <name>`).
    pub screen: Option<String>,
    /// Offscreen only: select and frame this planet (`--select <id or name>`).
    pub select: Option<String>,
    /// Offscreen only: camera zoom from 0 (closest) to 1 (farthest) (`--zoom <f>`).
    pub zoom: Option<f32>,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            music_volume: 0.5,
            no_vsync: false,
            fullscreen: false,
            game_dir: None,
            mod_id: None,
            campaign: None,
            list_campaigns: false,
            demo: false,
            offscreen: None,
            frames: 60,
            screen: None,
            select: None,
            zoom: None,
        }
    }
}

const USAGE: &str = "\
Space Peace: Not at War

USAGE: space-peace [OPTIONS]

OPTIONS:
  --music-volume <0..1>  Master volume (default 0.5)
  --game-dir <path>      Empire at War root (default: auto-detect / EAW_GAME_DIR)
  --mod <id>             Workshop mod id (default 1770851727)
  --campaign <name>      Galactic conquest campaign (default GC_Full_Web_Way_Rebel)
  --list-campaigns       List galaxy campaigns and exit
  --demo                 Use the built-in demo galaxy
  --offscreen <out.png>  Render headless at 3840x2160, save the PNG and exit
  --frames <n>           Frames to render in offscreen mode (default 60)
  --screen <name>        Start screen: menu, campaigns or galaxy
  --select <planet>      Offscreen: select and frame a planet (id or name)
  --zoom <0..1>          Offscreen: camera zoom, 0 closest, 1 farthest
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
                "--offscreen" => out.offscreen = Some(it.next().ok_or("--offscreen needs a value")?.into()),
                "--frames" => {
                    let v = it.next().ok_or("--frames needs a value")?;
                    out.frames = v.parse().map_err(|e| format!("bad --frames: {e}"))?;
                }
                "--screen" => out.screen = Some(it.next().ok_or("--screen needs a value")?),
                "--select" => out.select = Some(it.next().ok_or("--select needs a value")?),
                "--zoom" => {
                    let v = it.next().ok_or("--zoom needs a value")?;
                    out.zoom = Some(v.parse().map_err(|e| format!("bad --zoom: {e}"))?);
                }
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
        let a = Args::parse(["--campaign", "x", "--no-vsync"].map(String::from)).unwrap();
        assert_eq!(a.campaign.as_deref(), Some("x"));
        assert!(a.no_vsync);
        assert!(Args::parse(["--bogus".to_string()]).is_err());
    }
}
