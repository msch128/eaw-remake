//! Minimal command line parsing (no external dependency).

use bevy::prelude::Resource;
use std::path::PathBuf;

/// Parsed command line options.
#[derive(Resource, Debug, Clone)]
pub struct Args {
    /// Master volume override, linear 0..1 (`--volume <v>`, alias `--music-volume`).
    pub volume: Option<f32>,
    /// Extra seconds the loading screen is held (`--loading-delay <s>`, for screenshots).
    pub loading_delay: f32,
    /// Disable vertical sync (`--no-vsync`).
    pub no_vsync: bool,
    /// Never open the audio device; the audio logic still runs (`--no-audio`, implied by `--volume 0`).
    pub no_audio: bool,
    /// Start in borderless fullscreen (`--fullscreen`).
    pub fullscreen: bool,
    /// Game root directory override (`--game-dir <path>`).
    pub game_dir: Option<PathBuf>,
    /// Explicit mod directory override (`--mod-dir <path>`, or EAW_MOD_DIR).
    pub mod_dir: Option<PathBuf>,
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
    /// Offscreen only: open the planet view (fleet boxes, system text, ground board) of this planet once the scenario flags have run (`--planet-view <id or name>`).
    pub planet_view: Option<String>,
    /// Offscreen only: camera zoom from 0 (closest) to 1 (farthest) (`--zoom <f>`).
    pub zoom: Option<f32>,
    /// Offscreen only: advance the galaxy by this many weeks before the capture (`--weeks <n>`).
    pub weeks: Option<u32>,
    /// Offscreen only: queue the first available build option containing this text at the
    /// selected planet (`--build <text>`).
    pub build: Option<String>,
    /// Offscreen only: send the player's fleets parked at the selected planet to this planet
    /// (`--move <planet>`).
    pub move_to: Option<String>,
    /// Offscreen only: show the fleets of the selected planet picked up and carried over this planet, as in a drag (--drag-to <planet>).
    pub drag_to: Option<String>,
    /// Offscreen only: stop advancing when a battle waits for the player (`--stop-on-battle`).
    pub stop_on_battle: bool,
    /// Offscreen only: click this button of the battle dialog once it opens (`--battle-choice fight|auto`).
    pub battle_choice: Option<String>,
    /// Offscreen only: the AI plays both sides of a tactical battle (`--battle-auto`).
    pub battle_auto: bool,
    /// Offscreen only: select the player ships and give a move and an attack order (`--battle-orders`).
    pub battle_orders: bool,
    /// Offscreen only: before the mid-battle capture, put the camera on the closest pair of opposing ships (`--battle-fightcam`).
    pub battle_fightcam: bool,
    /// Offscreen only: reach the map the way a player does, main menu -> campaign list -> campaign (`--via-menu`).
    pub via_menu: bool,
    /// Open the real window but keep it invisible, run `--frames` frames and exit (`--hidden`; with `--volume 0`): checks the windowed plugin set.
    pub hidden: bool,
    /// Quit after this many frames (`--exit-after <n>`, for short windowed test runs).
    pub exit_after: Option<u32>,
    /// Offscreen only: grant the player this unit type at `--at` (needs the debug commands; `--grant-unit Death_Star`).
    pub grant_unit: Option<String>,
    /// Planet of `--grant-unit`.
    pub at: Option<String>,
    /// Offscreen only: press the superweapon button (`ask`) and confirm the dialog (`confirm`).
    pub fire: Option<String>,
    /// Offscreen only: show this planet as destroyed (`--force-wreck <planet>`).
    pub force_wreck: Option<String>,
    /// Offscreen only: pretend the campaign ended for the player (`--force-end win|lose`).
    pub force_end: Option<String>,
    /// Offscreen only, with `--battle-reserve`: the unit cap of every side equals the deployed units, so call-ins are refused (`--battle-low-cap`).
    pub battle_low_cap: bool,
    /// Offscreen only: deploy only the first N groups of the player side, the rest becomes the reinforcement reserve (`--battle-reserve N`).
    pub battle_reserve: Option<usize>,
    /// Offscreen only: click these command bar buttons on the map, 40 frames apart (`--click queue00,b_space_tab`).
    pub click: Option<String>,
    /// Offscreen only: click these buttons once the battle runs (`--battle-click b_reinforcement,r_0000`).
    pub battle_click: Option<String>,
    /// Offscreen only: open the save or load slot list on the map (`--slots save|load`).
    pub slots: Option<String>,
    /// Offscreen only: press this slot of the open list (`--slot-click N`).
    pub slot_click: Option<u8>,
    /// AI difficulty of a new game: easy, normal, hard, brutal or fair (`--difficulty`).
    pub difficulty: Option<String>,
    /// Use galaxy-sim's built-in AI instead of the Remake AI (`--classic-ai`).
    pub classic_ai: bool,
    /// Offscreen only: save the game here once the scenario finished (`--save-to <file>`).
    pub save_to: Option<PathBuf>,
    /// Start by loading this save (`--load-from <file>`).
    pub load_from: Option<PathBuf>,
    /// Tactical battle speed multiplier (`--battle-speed <x>`).
    pub battle_speed: Option<f32>,
    /// Offscreen only: battle time of the mid-battle capture (`--battle-seconds <s>`).
    pub battle_seconds: Option<f32>,
    /// Abandon the tactical battle (as F10 does) once its clock reaches this many seconds (`--battle-abort-after <s>`).
    pub battle_abort_after: Option<f32>,
    /// Press Esc in the tactical battle once its clock reaches this many seconds (`--battle-esc-after <s>`); the menu opens and pauses the clock.
    pub battle_esc_after: Option<f32>,
    /// Offscreen only: render target size in pixels (`--size 1920x1080`, default 3840x2160).
    pub size: Option<(u32, u32)>,
    /// Offscreen only: act as if the cursor hovered the selected planet (`--hover`).
    pub hover_selected: bool,
    /// Offscreen only: show the tooltip of this command bar button as if hovered (`--tooltip b_create_00`).
    pub tooltip: Option<String>,
    /// Offscreen only: open the F2 cheat menu (`--cheat-menu`).
    pub cheat_menu: bool,
    /// "From the Ground Up" start (`--ground-up`).
    pub ground_up: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            volume: None,
            loading_delay: 0.0,
            no_vsync: false,
            no_audio: false,
            fullscreen: false,
            game_dir: None,
            mod_dir: None,
            mod_id: None,
            campaign: None,
            list_campaigns: false,
            demo: false,
            offscreen: None,
            frames: 60,
            screen: None,
            select: None,
            planet_view: None,
            zoom: None,
            weeks: None,
            build: None,
            move_to: None,
            drag_to: None,
            stop_on_battle: false,
            battle_choice: None,
            battle_auto: false,
            battle_orders: false,
            battle_fightcam: false,
            via_menu: false,
            hidden: false,
            exit_after: None,
            grant_unit: None,
            at: None,
            fire: None,
            force_wreck: None,
            force_end: None,
            battle_low_cap: false,
            battle_reserve: None,
            click: None,
            battle_click: None,
            slots: None,
            slot_click: None,
            difficulty: None,
            classic_ai: false,
            save_to: None,
            load_from: None,
            battle_speed: None,
            battle_seconds: None,
            battle_abort_after: None,
            battle_esc_after: None,
            size: None,
            hover_selected: false,
            tooltip: None,
            cheat_menu: false,
            ground_up: false,
        }
    }
}

const USAGE: &str = "\
EAW:Remake - Galactic Conquest for Empire at War Remake 4.0

USAGE: eaw-remake [OPTIONS]

OPTIONS:
  --volume <0..1>        Master volume (overrides the stored setting)
  --loading-delay <s>    Hold the loading screen for this many extra seconds
  --game-dir <path>      Empire at War root (default: auto-detect / EAW_GAME_DIR)
  --mod <id>             Workshop mod id (default 1770851727)
  --campaign <name>      Galactic conquest campaign (default GC_Full_Web_Way_Rebel)
  --list-campaigns       List galaxy campaigns and exit
  --demo                 Use the built-in demo galaxy
  --offscreen <out.png>  Render headless at 3840x2160, save the PNG and exit
  --frames <n>           Frames to render in offscreen mode (default 60)
  --screen <name>        Start screen: menu, campaigns or galaxy
  --select <planet>      Offscreen: select and frame a planet (id or name)
  --planet-view <planet> Offscreen: open the planet view of a planet (id or name) after the other scenario flags ran
  --zoom <0..1>          Offscreen: camera zoom, 0 closest, 1 farthest
  --weeks <n>            Offscreen: advance n galactic weeks before the capture
  --build <text>         Offscreen: queue the first build option matching text at the selected planet
  --move <planet>        Offscreen: send the fleets at the selected planet to another planet
  --drag-to <planet>     Offscreen: carry the fleets of the selected planet over another planet, as while dragging them
  --stop-on-battle       Offscreen: stop advancing as soon as a battle waits for a decision
  --battle-choice <c>    Offscreen: click fight or auto in the battle dialog when it opens
  --battle-auto           Offscreen: the AI plays both sides of a tactical battle
  --via-menu            Offscreen: start from the main menu and click through to the campaign
  --hidden              Real but invisible window, run --frames frames and exit (smoke test of the windowed setup)
  --exit-after <n>      Quit after n frames (short windowed test runs)
  --grant-unit <u> --at <p>  Offscreen: give the player a unit at a planet (debug)
  --fire <ask|confirm>   Offscreen: press the superweapon button / also confirm the dialog
  --force-wreck <planet> Offscreen: draw this planet as a destroyed wreck
  --force-end <win|lose> Offscreen: end the campaign for the player to check the end screen
  --click <names>        Offscreen: click command bar buttons on the map (comma separated)
  --battle-click <names> Offscreen: click buttons once the battle runs (comma separated)
  --difficulty <d>      AI difficulty of a new game: easy, normal, hard, brutal, fair
  --classic-ai           Use the built-in classic AI instead of the Remake AI
  --save-to <file>       Offscreen: save the game after the scenario
  --load-from <file>     Start from a save file
  --battle-speed <x>     Tactical battle speed multiplier
  --battle-seconds <s>   Offscreen: battle time of the mid-battle capture (default 12)
  --battle-abort-after <s>  Abandon the tactical battle after s battle seconds (like F10)
  --battle-esc-after <s>    Press Esc in the tactical battle after s battle seconds (opens the battle menu)
  --size <WxH>          Offscreen: render target size (default 3840x2160)
  --hover               Offscreen: hover the selected planet (shows its hover panel)
  --tooltip <button>    Offscreen: show the tooltip of a command bar button (b_create_00, queue00, ...)
  --cheat-menu          Offscreen: open the F2 cheat menu
  --ground-up           New game From the Ground Up (1 planet for you, 3 for the other great powers, the rest passive)
  --no-vsync             Disable vertical sync
  --no-audio             Do not open the audio device (implied by --volume 0); the audio logic and SP_AUDIO_LOG still run
  --fullscreen           Start in borderless fullscreen
  -h, --help             Show this help
";

impl Args {
    /// True when no audio device is opened: `--no-audio` or `--volume 0`.
    pub fn audio_off(&self) -> bool {
        self.no_audio || self.volume == Some(0.0)
    }

    /// True for scripted test runs (offscreen, hidden window, or a timed windowed run): the scenario flags apply.
    pub fn scripted(&self) -> bool {
        self.offscreen.is_some() || self.hidden || self.exit_after.is_some()
    }

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
                "--volume" | "--music-volume" => {
                    let v = it.next().ok_or("--volume needs a value")?;
                    out.volume = Some(
                        v.parse::<f32>().map_err(|e| format!("bad --volume: {e}"))?.clamp(0.0, 1.0),
                    );
                }
                "--loading-delay" => {
                    let v = it.next().ok_or("--loading-delay needs a value")?;
                    out.loading_delay = v.parse().map_err(|e| format!("bad --loading-delay: {e}"))?;
                }
                "--game-dir" => out.game_dir = Some(it.next().ok_or("--game-dir needs a value")?.into()),
                "--mod-dir" => out.mod_dir = Some(it.next().ok_or("--mod-dir needs a value")?.into()),
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
                "--planet-view" => out.planet_view = Some(it.next().ok_or("--planet-view needs a value")?),
                "--zoom" => {
                    let v = it.next().ok_or("--zoom needs a value")?;
                    out.zoom = Some(v.parse().map_err(|e| format!("bad --zoom: {e}"))?);
                }
                "--weeks" => {
                    let v = it.next().ok_or("--weeks needs a value")?;
                    out.weeks = Some(v.parse().map_err(|e| format!("bad --weeks: {e}"))?);
                }
                "--build" => out.build = Some(it.next().ok_or("--build needs a value")?),
                "--drag-to" => out.drag_to = Some(it.next().ok_or("--drag-to needs a value")?),
                "--move" => out.move_to = Some(it.next().ok_or("--move needs a value")?),
                "--stop-on-battle" => out.stop_on_battle = true,
                "--battle-auto" => out.battle_auto = true,
                "--battle-orders" => out.battle_orders = true,
                "--battle-fightcam" => out.battle_fightcam = true,
                "--via-menu" => out.via_menu = true,
                "--hidden" => out.hidden = true,
                "--exit-after" => out.exit_after = Some(it.next().ok_or("--exit-after needs a number")?.parse().map_err(|e| format!("bad --exit-after: {e}"))?),
                "--grant-unit" => out.grant_unit = Some(it.next().ok_or("--grant-unit needs a unit")?),
                "--at" => out.at = Some(it.next().ok_or("--at needs a planet")?),
                "--fire" => out.fire = Some(it.next().ok_or("--fire needs ask or confirm")?),
                "--force-wreck" => out.force_wreck = Some(it.next().ok_or("--force-wreck needs a planet")?),
                "--force-end" => out.force_end = Some(it.next().ok_or("--force-end needs win or lose")?),
                "--battle-low-cap" => out.battle_low_cap = true,
                "--battle-reserve" => out.battle_reserve = Some(it.next().ok_or("--battle-reserve needs a number")?.parse().map_err(|e| format!("bad --battle-reserve: {e}"))?),
                "--click" => out.click = Some(it.next().ok_or("--click needs button names")?),
                "--battle-click" => out.battle_click = Some(it.next().ok_or("--battle-click needs button names")?),
                "--slots" => out.slots = Some(it.next().ok_or("--slots needs save or load")?),
                "--slot-click" => out.slot_click = Some(it.next().ok_or("--slot-click needs a number")?.parse().map_err(|e| format!("bad --slot-click: {e}"))?),
                "--difficulty" => out.difficulty = Some(it.next().ok_or("--difficulty needs a value")?.to_lowercase()),
                "--classic-ai" => out.classic_ai = true,
                "--save-to" => out.save_to = Some(it.next().ok_or("--save-to needs a value")?.into()),
                "--load-from" => out.load_from = Some(it.next().ok_or("--load-from needs a value")?.into()),
                "--battle-speed" => {
                    let v = it.next().ok_or("--battle-speed needs a value")?;
                    out.battle_speed = Some(v.parse().map_err(|e| format!("bad --battle-speed: {e}"))?);
                }
                "--hover" => out.hover_selected = true,
                "--tooltip" => out.tooltip = Some(it.next().ok_or("--tooltip needs a button name")?),
                "--cheat-menu" => out.cheat_menu = true,
                "--ground-up" => out.ground_up = true,
                "--size" => {
                    let v = it.next().ok_or("--size needs WxH")?;
                    let (w, h) = v.split_once(['x', 'X']).ok_or("bad --size (use WxH)")?;
                    out.size = Some((w.parse().map_err(|e| format!("bad --size: {e}"))?, h.parse().map_err(|e| format!("bad --size: {e}"))?));
                }
                "--battle-seconds" => {
                    let v = it.next().ok_or("--battle-seconds needs a value")?;
                    out.battle_seconds = Some(v.parse().map_err(|e| format!("bad --battle-seconds: {e}"))?);
                }
                "--battle-abort-after" => {
                    let v = it.next().ok_or("--battle-abort-after needs a value")?;
                    out.battle_abort_after = Some(v.parse().map_err(|e| format!("bad --battle-abort-after: {e}"))?);
                }
                "--battle-esc-after" => {
                    let v = it.next().ok_or("--battle-esc-after needs a value")?;
                    out.battle_esc_after = Some(v.parse().map_err(|e| format!("bad --battle-esc-after: {e}"))?);
                }
                "--battle-choice" => {
                    let v = it.next().ok_or("--battle-choice needs fight or auto")?;
                    if v != "fight" && v != "auto" {
                        return Err(format!("bad --battle-choice `{v}` (fight or auto)"));
                    }
                    out.battle_choice = Some(v);
                }
                "--no-vsync" => out.no_vsync = true,
                "--no-audio" => out.no_audio = true,
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
        let b = Args::parse(
            ["--screen", "menu", "--offscreen", "o.png", "--frames", "5", "--volume", "0.3", "--loading-delay", "2", "--zoom", "0.5", "--select", "P", "--game-dir", "g", "--mod", "1", "--demo"]
                .map(String::from),
        )
        .unwrap();
        assert_eq!(b.screen.as_deref(), Some("menu"));
        assert_eq!(b.frames, 5);
        assert_eq!(b.volume, Some(0.3));
        assert!(b.demo && b.offscreen.is_some() && b.game_dir.is_some() && b.mod_id.is_some());
        assert!(a.no_vsync);
        assert!(Args::parse(["--bogus".to_string()]).is_err());
    }
}

/// Lower-case name without "the ", " system", a "_WW" suffix, spaces and underscores: "The Coruscant System", "Coruscant_WW" and "coruscant" all match.
pub fn plain_name(s: &str) -> String {
    let mut t = s.trim().to_ascii_lowercase();
    if let Some(r) = t.strip_suffix("_ww") {
        t = r.to_owned();
    }
    if let Some(r) = t.strip_prefix("the ") {
        t = r.to_owned();
    }
    if let Some(r) = t.strip_suffix(" system") {
        t = r.to_owned();
    }
    t.retain(|c| c.is_ascii_alphanumeric());
    t
}

/// Picks the entry of `names` (`(internal name, display name)`) a command line argument means: exact (either name), then equal after [`plain_name`].
/// The error lists up to five close matches.
pub fn resolve_name(kind: &str, query: &str, names: &[(&str, &str)]) -> Result<usize, String> {
    if let Some(i) = names.iter().position(|(a, b)| a.eq_ignore_ascii_case(query.trim()) || b.eq_ignore_ascii_case(query.trim())) {
        return Ok(i);
    }
    let want = plain_name(query);
    if !want.is_empty()
        && let Some(i) = names.iter().position(|(a, b)| plain_name(a) == want || plain_name(b) == want)
    {
        return Ok(i);
    }
    let mut close: Vec<&str> = names
        .iter()
        .filter(|(a, b)| {
            let (pa, pb) = (plain_name(a), plain_name(b));
            !want.is_empty() && (pa.contains(&want) || pb.contains(&want) || want.contains(&pa) && !pa.is_empty() || (want.len() >= 3 && pa.starts_with(&want[..3])))
        })
        .map(|(_, b)| *b)
        .take(5)
        .collect();
    close.dedup();
    Err(if close.is_empty() { format!("unknown {kind} `{query}`") } else { format!("unknown {kind} `{query}`; close matches: {}", close.join(", ")) })
}

#[cfg(test)]
mod name_tests {
    use super::*;

    #[test]
    fn names_resolve_by_internal_or_display_name() {
        let planets = [("Coruscant_WW", "The Coruscant System"), ("Corulag_WW", "Corulag"), ("Ryloth_WW", "Ryloth")];
        assert_eq!(resolve_name("planet", "Coruscant_WW", &planets), Ok(0));
        assert_eq!(resolve_name("planet", "the coruscant system", &planets), Ok(0));
        assert_eq!(resolve_name("planet", "Coruscant", &planets), Ok(0));
        assert_eq!(resolve_name("planet", "ryloth", &planets), Ok(2));
        let err = resolve_name("planet", "Coru", &planets).unwrap_err();
        assert!(err.contains("The Coruscant System") && err.contains("Corulag"), "{err}");
        assert_eq!(resolve_name("planet", "Nowhere", &planets).unwrap_err(), "unknown planet `Nowhere`");
    }
}
