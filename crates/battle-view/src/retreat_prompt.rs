//! The retreat question (design request 2026-10-16): with [`BattleViewSettings::retreat_prompt`] (the game turns it on) a press on Retreat opens a
//! banner in the look of the battle-start banner (`eaw_ui::spawn_battle_banner`, `i_attack_mode_choice.alo`) instead of ordering the retreat:
//! RETREAT starts the jump countdown, AUTO-RESOLVE asks the host to settle the battle from the units that are left ([`RetreatChoice`]), CANCEL
//! (or Esc) closes it. The battle clock stands still while it is open. Texts from `MasterTextFile`: `TEXT_TOOLTIP_RETREAT_BUTTON` ("Retreat from
//! battle"), `TEXT_BUTTON_RETREAT`, `TEXT_BUTTON_AUTORESOLVE`, `TEXT_BUTTON_CANCEL`.

use bevy::prelude::*;
use eaw_ui::{BANNER_AUTO, BANNER_BEGIN, BANNER_CANCEL, BannerSide, BattleBannerSpec, EawButtonClicked, EawDialogResult, EawUiAssets, UiFaction};

use crate::{BattleRuntime, BattleViewSettings};

/// State of the retreat question (resource).
#[derive(Resource, Default, Debug)]
pub struct RetreatPrompt {
    /// Set to open it (the Retreat button, or a host's Esc-menu Retreat).
    pub requested: bool,
    /// Root of the open banner.
    root: Option<Entity>,
    /// The player's own pause before the question opened (restored when it closes).
    paused_before: Option<bool>,
}

impl RetreatPrompt {
    /// True while the question is up or about to open (a host's Esc key should not open its own menu then: Esc answers Cancel).
    pub fn is_open(&self) -> bool {
        self.root.is_some() || self.requested
    }
}

/// An answer to the retreat question.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetreatChoice {
    /// Order the retreat (the battle view sends it to the sim).
    Retreat,
    /// Settle the battle with the auto-resolve from the units that are alive now, with their hull and shield (the host acts on it).
    AutoResolve,
}

/// The game's own text for `key`, upper case; `fallback` without the string table.
fn text(world: &World, key: &str, fallback: &str) -> String {
    world.get_resource::<EawUiAssets>().and_then(|a| a.text.get(key).map(str::to_owned)).unwrap_or_else(|| fallback.to_owned()).to_uppercase()
}

/// The banner: the planet and "RETREAT FROM BATTLE" as the title, the player's forces on the left and the strongest enemy side on the right
/// (discs by the number of living ships, the icon of the biggest one, as on the battle-start banner).
fn spec(world: &World) -> BattleBannerSpec {
    let rt = world.get_resource::<BattleRuntime>();
    let player = world.get_resource::<BattleViewSettings>().map_or(0, |s| s.player_side);
    let faction = UiFaction::from_name(rt.and_then(|r| r.player_faction.as_deref()).unwrap_or("Empire"));
    let planet = rt.and_then(|r| r.request.as_ref()).map(|q| q.planet_name.trim_end_matches("_WW").replace('_', " ").to_uppercase());
    let what = text(world, "TEXT_TOOLTIP_RETREAT_BUTTON", "Retreat from battle");
    let side = |side: u8, tint: Color| -> BannerSide {
        let (mut count, mut icon) = (1, None);
        if let Some((rt, cur)) = rt.and_then(|r| r.cur.as_ref().map(|c| (r, c))) {
            count = cur.alive.get(side as usize).copied().unwrap_or(0).clamp(1, 5);
            let biggest = cur.ships.iter().filter(|s| s.side == side).max_by(|a, b| rt.catalog.ship(a.def).hull.total_cmp(&rt.catalog.ship(b.def).hull));
            let game = world.get_resource::<EawUiAssets>().and_then(|a| a.game.clone());
            icon = biggest.zip(game).and_then(|(s, g)| eaw_ui::unit_icon(&g, &rt.catalog.ship(s.def).name));
        }
        BannerSide { stacks: count, icon, tint }
    };
    let enemy = rt
        .and_then(|r| r.cur.as_ref())
        .and_then(|c| (0..c.alive.len() as u8).filter(|&s| s != player).max_by_key(|&s| c.alive[s as usize]))
        .unwrap_or(if player == 0 { 1 } else { 0 });
    BattleBannerSpec {
        title: planet.map_or_else(|| what.clone(), |p| format!("{p}: {what}")),
        faction,
        begin: text(world, "TEXT_BUTTON_RETREAT", "Retreat"),
        auto: text(world, "TEXT_BUTTON_AUTORESOLVE", "Auto-Resolve"),
        left: side(player, Color::srgb(0.35, 1.0, 0.45)),
        right: side(enemy, Color::srgb(1.0, 0.3, 0.3)),
        cancel: Some(text(world, "TEXT_BUTTON_CANCEL", "Cancel")),
    }
}

/// Opens the banner when it was asked for and pauses the battle clock. Without the interface (or before its textures are ready) the
/// question cannot be shown, and the retreat is ordered as before.
pub(crate) fn open_prompt(world: &mut World) {
    {
        let Some(mut prompt) = world.get_resource_mut::<RetreatPrompt>() else { return };
        if !prompt.requested {
            return;
        }
        if prompt.root.is_some() {
            // Asked again while it is up (Retreat pressed once more): nothing to do.
            prompt.requested = false;
            return;
        }
    }
    let spec = spec(world);
    let root = eaw_ui::spawn_battle_banner(world, &spec);
    let paused = world.get_resource::<BattleViewSettings>().is_some_and(|s| s.paused);
    {
        let mut prompt = world.resource_mut::<RetreatPrompt>();
        prompt.requested = false;
        prompt.root = root;
        prompt.paused_before = root.map(|_| paused);
    }
    if root.is_some() {
        if let Some(mut s) = world.get_resource_mut::<BattleViewSettings>() {
            s.paused = true;
        }
        info!("tactical: retreat question open");
    } else {
        warn!("tactical: retreat question could not be shown, ordering the retreat");
        world.write_message(RetreatChoice::Retreat);
    }
}

/// Answers of the open banner: RETREAT / AUTO-RESOLVE (the banner's two buttons), CANCEL (its third button, which closes the banner itself) or Esc.
#[allow(clippy::too_many_arguments)]
pub(crate) fn prompt_input(
    mut commands: Commands,
    mut prompt: ResMut<RetreatPrompt>,
    mut settings: ResMut<BattleViewSettings>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut clicks: MessageReader<EawButtonClicked>,
    mut results: MessageReader<EawDialogResult>,
    mut choices: MessageWriter<RetreatChoice>,
    parents: Query<&ChildOf>,
    alive: Query<()>,
) {
    let Some(root) = prompt.root else {
        clicks.clear();
        results.clear();
        return;
    };
    let under_root = |mut e: Entity| loop {
        if e == root {
            return true;
        }
        match parents.get(e) {
            Ok(p) => e = p.parent(),
            Err(_) => return false,
        }
    };
    let mut answer = None;
    for c in clicks.read() {
        if answer.is_none() && under_root(c.entity) {
            answer = match c.name.as_str() {
                BANNER_BEGIN => Some(Some(RetreatChoice::Retreat)),
                BANNER_AUTO => Some(Some(RetreatChoice::AutoResolve)),
                _ => None,
            };
        }
    }
    for r in results.read() {
        if r.dialog == root && r.button == BANNER_CANCEL {
            answer.get_or_insert(None);
        }
    }
    if keys.just_pressed(KeyCode::Escape) {
        keys.clear_just_pressed(KeyCode::Escape);
        answer.get_or_insert(None);
    }
    // Gone without an answer (closed from elsewhere): the same as Cancel.
    if answer.is_none() && alive.get(root).is_err() {
        answer = Some(None);
    }
    let Some(choice) = answer else { return };
    if let Some(c) = choice {
        choices.write(c);
    }
    info!("tactical: retreat question answered: {choice:?}");
    commands.entity(root).try_despawn();
    prompt.root = None;
    if let Some(p) = prompt.paused_before.take() {
        settings.paused = p;
    }
}

/// RETREAT: the same order as the button without the question.
pub(crate) fn order_retreat(rt: Res<BattleRuntime>, settings: Res<BattleViewSettings>, mut choices: MessageReader<RetreatChoice>) {
    for c in choices.read() {
        if *c == RetreatChoice::Retreat {
            rt.order_retreat(settings.player_side);
        }
    }
}

/// A battle that ends with the question open takes it along and gives the player's pause back.
pub(crate) fn close_prompt(mut commands: Commands, mut prompt: ResMut<RetreatPrompt>, mut settings: ResMut<BattleViewSettings>) {
    if let Some(root) = prompt.root.take() {
        commands.entity(root).try_despawn();
    }
    prompt.requested = false;
    if let Some(p) = prompt.paused_before.take() {
        settings.paused = p;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eaw_ui::EawButton;
    use std::sync::Arc;

    /// The question's systems with the real interface resources of the Remake (skipped without it) and no battle: the answers come out as
    /// [`RetreatChoice`] messages.
    fn app() -> Option<App> {
        let install = alamo_fs::GameInstall::detect()?;
        if !install.mod_data("1770851727").is_dir() {
            return None;
        }
        let fs = Arc::new(alamo_fs::GameFs::for_mod(&install, Some("1770851727")).ok()?);
        let textures = eaw_ui::textures::UiTextures::new(fs.clone());
        let _ = textures.library().mega_slot().set(eaw_ui::textures::decode_mega_texture(&fs));
        let (db, _) = eaw_ui::CommandBarDb::load(&*fs);
        let text = alamo_data::text::TextTable::load(&*fs, "Text/MasterTextFile_ENGLISH.DAT").ok()?;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<Image>>()
            .insert_resource(textures)
            .insert_resource(db)
            .insert_resource(EawUiAssets { fs, text, game: None })
            .insert_resource(eaw_ui::EawUiSettings::default())
            .init_resource::<eaw_ui::PanelData>()
            .init_resource::<RetreatPrompt>()
            .init_resource::<BattleViewSettings>()
            .init_resource::<ButtonInput<KeyCode>>()
            .add_message::<EawButtonClicked>()
            .add_message::<EawDialogResult>()
            .add_message::<RetreatChoice>()
            .add_systems(Update, (open_prompt, prompt_input).chain());
        Some(app)
    }

    fn root(app: &App) -> Option<Entity> {
        app.world().resource::<RetreatPrompt>().root
    }

    fn answers(app: &mut App) -> Vec<RetreatChoice> {
        app.world_mut().resource_mut::<Messages<RetreatChoice>>().drain().collect()
    }

    /// The button named `name` inside the banner `root`.
    fn button(app: &mut App, root: Entity, name: &str) -> Entity {
        let mut q = app.world_mut().query::<(Entity, &EawButton)>();
        let found: Vec<Entity> = q.iter(app.world()).filter(|(_, b)| b.name == name).map(|(e, _)| e).collect();
        let world = app.world();
        found
            .into_iter()
            .find(|&e| {
                let mut at = e;
                loop {
                    if at == root {
                        return true;
                    }
                    match world.get::<ChildOf>(at) {
                        Some(p) => at = p.parent(),
                        None => return false,
                    }
                }
            })
            .unwrap_or_else(|| panic!("no {name} in the banner"))
    }

    #[test]
    fn the_retreat_question_stays_up_until_answered_and_every_answer_closes_it_for_good() {
        let Some(mut app) = app() else { return };
        app.update();
        // Asked: the banner opens, stops the battle clock and stays the same banner frame after frame (no closing and reopening).
        app.world_mut().resource_mut::<RetreatPrompt>().requested = true;
        app.update();
        let first = root(&app).expect("the banner is open");
        assert!(app.world().get::<Node>(first).is_some(), "it is the battle-start banner's shell");
        for _ in 0..10 {
            app.update();
            assert_eq!(root(&app), Some(first));
            assert!(app.world().get_entity(first).is_ok());
            assert!(app.world().resource::<BattleViewSettings>().paused);
        }
        // Retreat pressed once more while it is up: still the same banner, and it does not come back after the answer.
        app.world_mut().resource_mut::<RetreatPrompt>().requested = true;
        app.update();
        assert_eq!(root(&app), Some(first));
        // CANCEL: the banner's third button reports it and closes the banner itself (`dialog_buttons`).
        app.world_mut().write_message(EawDialogResult { dialog: first, button: BANNER_CANCEL });
        app.world_mut().entity_mut(first).despawn();
        for _ in 0..10 {
            app.update();
            assert_eq!(root(&app), None);
            assert!(!app.world().resource::<BattleViewSettings>().paused, "the player's own pause is back");
        }
        assert!(answers(&mut app).is_empty(), "Cancel orders nothing");
        // RETREAT and AUTO-RESOLVE: the banner's two buttons; one answer each, and the banner stays closed.
        for (name, want) in [(BANNER_BEGIN, RetreatChoice::Retreat), (BANNER_AUTO, RetreatChoice::AutoResolve)] {
            app.world_mut().resource_mut::<RetreatPrompt>().requested = true;
            app.update();
            let open = root(&app).expect("open again");
            let entity = button(&mut app, open, name);
            app.world_mut().write_message(EawButtonClicked { name: name.into(), entity, shift: false });
            let mut got = Vec::new();
            for _ in 0..10 {
                app.update();
                got.extend(answers(&mut app));
                assert_eq!(root(&app), None);
                assert!(app.world().get_entity(open).is_err(), "the banner is gone");
                assert!(!app.world().resource::<BattleViewSettings>().paused);
            }
            assert_eq!(got, vec![want]);
        }
        // Esc answers Cancel.
        app.world_mut().resource_mut::<RetreatPrompt>().requested = true;
        app.update();
        assert!(root(&app).is_some());
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Escape);
        for _ in 0..5 {
            app.update();
            assert_eq!(root(&app), None);
        }
        assert!(answers(&mut app).is_empty());
    }
}
