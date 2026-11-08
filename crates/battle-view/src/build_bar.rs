//! The tactical build bar of the player's star base or station (`battle_sim::buildables`): while one of the player's structures that offers
//! tactical builds is selected, the `i_build_buttons` shell (`Build_00` .. `Build_05`, group `Tactical Build Buttons` in
//! `CommandBarComponents.xml`) shows the items `Battle::build_options` lists, with the item's `Icon_Name` and its price. A click buys the
//! item (`Command::TacticalBuild`). The command bar's `tqueue00` .. `tqueue09` show the side's production queue (a click takes the item
//! out again, `Command::CancelTacticalBuild`) and `Text_Credits_tactical` the credits left.

use bevy::prelude::*;
use battle_sim::Command;
use battle_sim::buildables::{BuildId, BuildOption};
use eaw_ui::{EawButtonClicked, PanelData, PanelSlot, QueueEntry, ShellAnchor, ShellSpec, UiFaction, spawn_shell};

use crate::control::Selection;
use crate::{BattleRuntime, BattleViewSettings};

/// Buttons of the `i_build_buttons` shell.
const BUTTONS: usize = 6;

/// The open build buttons and the items on them.
#[derive(Resource, Default, Debug)]
pub struct BuildBarUi {
    /// Root of the open shell.
    pub root: Option<Entity>,
    /// Item per button.
    pub items: Vec<BuildId>,
}

pub(crate) fn build(app: &mut App) {
    app.init_resource::<BuildBarUi>();
    app.add_systems(Update, (build_bar_sync, build_bar_clicks).chain().run_if(resource_exists::<BattleRuntime>));
    app.add_systems(Update, build_bar_close.run_if(resource_removed::<BattleRuntime>));
}

/// The build buttons: the item's icon, its price (none while it builds, the progress shows instead), greyed while it cannot be bought.
fn button_slots(options: &[BuildOption], catalog: &battle_sim::Catalog) -> Vec<PanelSlot> {
    options
        .iter()
        .take(BUTTONS)
        .map(|o| {
            let def = catalog.buildable(o.item);
            PanelSlot {
                icon: def.icon.clone().unwrap_or_default(),
                label: if o.progress.is_some() { String::new() } else { def.cost.to_string() },
                progress: o.progress.unwrap_or(0.0),
                disabled: o.ready.is_err() && o.progress.is_none(),
                ..default()
            }
        })
        .collect()
}

/// The production queue slots: one per job, the first one with its progress.
fn queue_entries(queue: &[(BuildId, f32)], catalog: &battle_sim::Catalog) -> Vec<QueueEntry> {
    queue
        .iter()
        .enumerate()
        .map(|(k, &(item, left))| {
            let def = catalog.buildable(item);
            let progress = if k == 0 && def.time > 0.0 { (1.0 - left / def.time).clamp(0.0, 1.0) } else { 0.0 };
            QueueEntry { icon: def.icon.clone().unwrap_or_default(), progress, lane: 0, count: 1, tooltip: String::new() }
        })
        .collect()
}

/// Opens or closes the shell with the selection and fills its buttons, the queue and the credits.
fn build_bar_sync(
    mut commands: Commands,
    rt: Res<BattleRuntime>,
    sel: Res<Selection>,
    settings: Res<BattleViewSettings>,
    mut ui: ResMut<BuildBarUi>,
    mut panel: ResMut<PanelData>,
    mut bar: ResMut<eaw_ui::CommandBarData>,
) {
    let Some(cur) = rt.cur.as_ref() else { return };
    let player = settings.player_side;
    let snap = cur.builds.get(player as usize).cloned().unwrap_or_default();
    let offers = sel.ships.iter().filter_map(|&id| cur.ship(id)).any(|s| s.side == player && !rt.catalog.ship(s.def).buildable.is_empty());
    let open = offers && !snap.options.is_empty() && !cur.over;
    if open && ui.root.is_none() {
        let faction = UiFaction::from_name(rt.player_faction.as_deref().unwrap_or("Empire"));
        // `Manual_Offset`: the game places the shell itself; estimate: the screen centre.
        commands.queue(move |world: &mut World| {
            let mut spec = ShellSpec::new("I_Build_Buttons.ALO", faction);
            spec.anchor = ShellAnchor::Center;
            spec.offset = Vec2::ZERO;
            let e = spawn_shell(world, &spec);
            let mut panel = world.resource_mut::<PanelData>();
            for k in 0..BUTTONS {
                panel.slots.remove(&format!("build_{k:02}"));
            }
            if let Some(e) = e {
                world.resource_mut::<BuildBarUi>().root = Some(e);
            }
        });
        ui.root = Some(Entity::PLACEHOLDER);
    } else if !open && let Some(e) = ui.root.take() {
        if e != Entity::PLACEHOLDER {
            commands.entity(e).try_despawn();
        }
        for k in 0..BUTTONS {
            panel.slots.remove(&format!("build_{k:02}"));
        }
    }
    // The slots are filled once the shell exists: a change of `PanelData` before that reaches no button.
    if open && ui.root.is_some_and(|e| e != Entity::PLACEHOLDER) {
        ui.items = snap.options.iter().take(BUTTONS).map(|o| o.item).collect();
        let slots = button_slots(&snap.options, &rt.catalog);
        for k in 0..BUTTONS {
            let name = format!("build_{k:02}");
            match slots.get(k) {
                Some(s) if panel.slots.get(&name) != Some(s) => panel.set_slot(&name, s.clone()),
                None if panel.slots.contains_key(&name) => {
                    panel.slots.remove(&name);
                }
                _ => {}
            }
        }
    }
    let queue = queue_entries(&snap.queue, &rt.catalog);
    if bar.queue != queue {
        bar.queue = queue;
    }
    if bar.credits != snap.credits {
        bar.credits = snap.credits;
    }
}

/// `Build_NN` buys its item, `tqueueNN` takes the queued job out again.
fn build_bar_clicks(mut clicks: MessageReader<EawButtonClicked>, rt: Res<BattleRuntime>, settings: Res<BattleViewSettings>, ui: Res<BuildBarUi>) {
    let player = settings.player_side;
    let Some(cur) = rt.cur.as_ref() else { return };
    for c in clicks.read() {
        if let Some(k) = c.name.strip_prefix("build_").and_then(|n| n.parse::<usize>().ok()) {
            if let Some(&item) = ui.items.get(k) {
                rt.command(player, Command::TacticalBuild { item });
            }
        } else if let Some(k) = c.name.strip_prefix("tqueue").and_then(|n| n.parse::<usize>().ok())
            && let Some(&(item, _)) = cur.builds.get(player as usize).and_then(|b| b.queue.get(k))
        {
            rt.command(player, Command::CancelTacticalBuild { item });
        }
    }
}

fn build_bar_close(mut commands: Commands, mut ui: ResMut<BuildBarUi>, mut panel: ResMut<PanelData>) {
    if let Some(e) = ui.root.take().filter(|&e| e != Entity::PLACEHOLDER) {
        commands.entity(e).try_despawn();
    }
    ui.items.clear();
    for k in 0..BUTTONS {
        panel.slots.remove(&format!("build_{k:02}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use battle_sim::buildables::BuildableDef;
    use battle_sim::{Battle, BattleConfig, BattleSetup, Catalog, ProjDef, ShipDef, SideSetup, TargetClass, UnitSetup};

    /// A station of side 1 that offers two upgrades (the second needs the first) and 300 credits.
    fn battle() -> (Battle, std::sync::Arc<Catalog>) {
        let frigate = ShipDef { name: "frigate".into(), class: TargetClass::Frigate, hull: 1000.0, radius: 50.0, population: 1, ..ShipDef::default() };
        let station = ShipDef { name: "station".into(), class: TargetClass::Structure, is_static: true, hull: 1.0e6, radius: 100.0, buildable: vec![("rebel".into(), 0), ("rebel".into(), 1)], ..ShipDef::default() };
        let mut cat = Catalog::from_defs(vec![frigate, station], vec![ProjDef { name: "laser".into(), ..ProjDef::default() }]);
        let item = |name: &str, icon: &str, prerequisites: Vec<BuildId>| BuildableDef { name: name.into(), icon: Some(icon.into()), cost: 160, time: 10.0, limit: 1, prerequisites, stealth_duration: 1.0, ..BuildableDef::default() };
        cat.buildables = vec![item("weapons_l1", "i_multi_improved_weapons1.tga", vec![]), item("weapons_l2", "i_multi_improved_weapons2.tga", vec![0])];
        let cat = std::sync::Arc::new(cat);
        let side = |n: &str, units: Vec<UnitSetup>, structures| SideSetup { name: n.into(), units, structures, ai: false };
        let empire = side("Empire", vec![UnitSetup { ship: 0, count: 1, tag: 1 }], vec![]);
        let rebel = side("Rebel", vec![], vec![1]);
        let config = BattleConfig { side_credits: vec![0, 300], fog_of_war: false, ..BattleConfig::default() };
        (Battle::new(BattleSetup { sides: vec![empire, rebel], catalog: cat.clone(), config }), cat)
    }

    #[test]
    fn the_snapshot_carries_the_build_bar_and_its_slots_show_icon_price_and_progress() {
        let (b, cat) = battle();
        let sim = crate::sim::SimHandle::spawn(b);
        sim.request(1);
        let cur = sim.fetch(0, true).0.expect("snapshot");
        let bar = &cur.builds[1];
        assert_eq!(bar.credits, 300);
        let slots = button_slots(&bar.options, &cat);
        assert_eq!(slots.len(), 1, "the second level waits for the first");
        assert_eq!((slots[0].icon.as_str(), slots[0].label.as_str(), slots[0].disabled), ("i_multi_improved_weapons1.tga", "160", false));
        assert!(cur.builds[0].options.is_empty(), "the attacker has no station");

        sim.command(1, Command::TacticalBuild { item: 0 });
        for tick in 1..=30 * 5u64 {
            sim.request(tick + 1);
            sim.fetch(tick, true);
        }
        let cur = sim.fetch(150, true).0.expect("snapshot");
        let bar = &cur.builds[1];
        assert_eq!(bar.credits, 140);
        let slots = button_slots(&bar.options, &cat);
        // Building: no price, half done; the next level shows, greyed (140 credits are too few).
        assert_eq!((slots[0].label.as_str(), slots[0].disabled), ("", false));
        assert!((slots[0].progress - 0.5).abs() < 0.02, "{}", slots[0].progress);
        assert_eq!((slots[1].icon.as_str(), slots[1].disabled), ("i_multi_improved_weapons2.tga", true));
        let queue = queue_entries(&bar.queue, &cat);
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].icon, "i_multi_improved_weapons1.tga");
        assert!((queue[0].progress - 0.5).abs() < 0.02);
    }
}

