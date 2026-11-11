//! The tactical build bar of the player's star base or station (`battle_sim::buildables`): while one of the player's structures that offers
//! tactical builds is selected, the `i_build_buttons` shell (`Build_00` .. `Build_05`, group `Tactical Build Buttons` in
//! `CommandBarComponents.xml`) shows the items `Battle::build_options` lists, one button per upgrade line in the station's data order, with
//! the item's `Icon_Name`, its price or its progress, and a tooltip. A click buys the item (`Command::TacticalBuild`), or takes an item in
//! production out again (`Command::CancelTacticalBuild`). `Text_Credits_tactical` shows the credits left.

use bevy::prelude::*;
use battle_sim::Command;
use battle_sim::buildables::{BuildId, BuildOption};
use eaw_ui::{EawButtonClicked, PanelData, PanelSlot, ShellAnchor, ShellSpec, UiFaction, spawn_shell};

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
    /// Per button: the item is in production and cannot be bought again, so a click takes it out of production (refund).
    pub cancels: Vec<bool>,
    /// The structure the buttons belong to.
    pub station: Option<u32>,
    /// Offset of the shell's box from its origin (design px, read once after the spawn: the root's left / bottom at offset 0).
    lo: Option<Vec2>,
}

pub(crate) fn build(app: &mut App) {
    app.init_resource::<BuildBarUi>();
    app.add_systems(Update, (build_bar_sync, build_bar_clicks, build_bar_follow).chain().run_if(resource_exists::<BattleRuntime>));
    app.add_systems(Update, build_bar_close.run_if(resource_removed::<BattleRuntime>));
}

/// Orders build options by the station's `Tactical_Buildable_Objects_Campaign` definition order.
pub fn sort_options_by_data_order(
    options: &mut [BuildOption],
    station_buildable: &[(String, BuildId)],
    catalog: &battle_sim::Catalog,
) {
    options.sort_by_key(|o| data_position(o.item, station_buildable, catalog));
}

/// Place of `item` in the station's list: its own entry, or that of the level it follows (`Previous_Upgrade_Level_Type`, else its first
/// prerequisite); `usize::MAX` when the chain reaches none.
fn data_position(mut item: BuildId, station_buildable: &[(String, BuildId)], catalog: &battle_sim::Catalog) -> usize {
    let mut visited = [false; 256];
    loop {
        if let Some(pos) = station_buildable.iter().position(|&(_, b)| b == item) {
            return pos;
        }
        if (item as usize) < visited.len() {
            if visited[item as usize] {
                return usize::MAX;
            }
            visited[item as usize] = true;
        }
        let def = catalog.buildable(item);
        if let Some(repl) = def.replaces {
            item = repl;
        } else if let Some(&prereq) = def.prerequisites.first() {
            item = prereq;
        } else {
            return usize::MAX;
        }
    }
}

/// First level of the upgrade line `item` belongs to and how many levels lie below it (following `Previous_Upgrade_Level_Type`, else the
/// first prerequisite).
fn line_root(mut item: BuildId, catalog: &battle_sim::Catalog) -> (BuildId, usize) {
    let mut depth = 0;
    while depth < 64 {
        let def = catalog.buildable(item);
        match def.replaces.or_else(|| def.prerequisites.first().copied()) {
            Some(below) => {
                item = below;
                depth += 1;
            }
            None => break,
        }
    }
    (item, depth)
}

/// The item each build button shows: one button per upgrade line, in the order of the station's list (the lines keep their buttons when
/// an item is bought: before, the next level appeared next to the one being built and pushed the other lines one button on). A line shows
/// its level in production with its progress, otherwise the next level to buy.
fn button_options(options: &[BuildOption], station_buildable: &[(String, BuildId)], catalog: &battle_sim::Catalog) -> Vec<BuildOption> {
    let mut lines: Vec<((usize, BuildId), usize, &BuildOption)> = Vec::new();
    for o in options {
        let (root, depth) = line_root(o.item, catalog);
        let key = (data_position(root, station_buildable, catalog), root);
        let rank = depth + if o.progress.is_some() { 0 } else { 1 << 16 };
        match lines.iter_mut().find(|l| l.0 == key) {
            Some(l) if rank < l.1 => *l = (key, rank, o),
            Some(_) => {}
            None => lines.push((key, rank, o)),
        }
    }
    lines.sort_by_key(|l| l.0);
    lines.into_iter().map(|l| l.2.clone()).collect()
}

/// The build buttons: the item's icon, its price (none while it builds, the progress shows instead), greyed while it cannot be bought,
/// and a tooltip with the item's name, effect, price and build time from the data (`tip`).
fn button_slots(options: &[BuildOption], catalog: &battle_sim::Catalog, tip: &dyn Fn(&BuildOption) -> String) -> Vec<PanelSlot> {
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
                tooltip: tip(o),
                ..default()
            }
        })
        .collect()
}

/// Tooltip of a build button: the item's `Text_ID` as the title, its `Encyclopedia_Text` (the effect), the price and the build time as stat
/// rows, and why it cannot be bought now (`Build_NN`'s own `Tooltip_Text` keys) or that a click takes it out of production again.
fn build_tooltip(o: &BuildOption, catalog: &battle_sim::Catalog, game: Option<&alamo_data::GameData>, text: &dyn Fn(&str) -> Option<String>) -> String {
    let def = catalog.buildable(o.item);
    let name = def.text_id.as_deref().and_then(text).unwrap_or_else(|| def.name.replace('_', " "));
    let effect = game
        .and_then(|g| g.objects.resolve(&def.name))
        .and_then(|r| r.get("encyclopedia_text").and_then(|v| v.split_whitespace().next()).and_then(text))
        .unwrap_or_default();
    let mut tip = format!("{name}\n{effect}\n@Cost\t{} credits\n@Build time\t{:.0} s", def.cost, def.time);
    if let Some(p) = o.progress {
        tip.push_str(&format!("\n@In production\t{:.0}%", p * 100.0));
    }
    let why = match o.ready {
        Err(battle_sim::CommandError::NotEnoughCredits) if o.progress.is_none() => text("TEXT_PRODUCTION_INSUFFICIENT_FUNDS"),
        Err(battle_sim::CommandError::UnitCap) if o.progress.is_none() => text("TEXT_PRODUCTION_POP_CAP_REACHED"),
        _ => None,
    };
    if let Some(w) = why {
        tip.push_str(&format!("\n!{w}"));
    }
    if o.progress.is_some() && o.ready.is_err() {
        tip.push_str("\nClick to cancel it.");
    }
    tip
}

/// Opens or closes the shell with the selection and fills its buttons and the credits. The production shows on the buttons (the item in
/// production of a line with its progress); the command bar's queue icons stay empty in a battle: they sat on the ability buttons of the
/// unit bar.
#[allow(clippy::too_many_arguments)]
pub fn build_bar_sync(
    mut commands: Commands,
    rt: Res<BattleRuntime>,
    sel: Res<Selection>,
    settings: Res<BattleViewSettings>,
    assets: Option<Res<eaw_ui::EawUiAssets>>,
    mut ui: ResMut<BuildBarUi>,
    mut panel: ResMut<PanelData>,
    mut bar: ResMut<eaw_ui::CommandBarData>,
) {
    let Some(cur) = rt.cur.as_ref() else { return };
    let player = settings.player_side;
    let snap = cur.builds.get(player as usize).cloned().unwrap_or_default();
    let station_ship = sel.ships.iter().filter_map(|&id| cur.ship(id)).find(|s| s.side == player && !rt.catalog.ship(s.def).buildable.is_empty());
    let offers = station_ship.is_some();
    ui.station = station_ship.map(|s| s.id);
    let open = offers && !snap.options.is_empty() && !cur.over;
    if open && ui.root.is_none() {
        let faction = UiFaction::from_name(rt.player_faction.as_deref().unwrap_or("Empire"));
        // `Manual_Offset`: the game places the shell at the selected object; `build_bar_follow` keeps its origin on the structure.
        commands.queue(move |world: &mut World| {
            let mut spec = ShellSpec::new("I_Build_Buttons.ALO", faction);
            spec.anchor = ShellAnchor::BottomLeft;
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
        ui.lo = None;
        if e != Entity::PLACEHOLDER {
            commands.entity(e).try_despawn();
        }
        for k in 0..BUTTONS {
            panel.slots.remove(&format!("build_{k:02}"));
        }
    }
    // The slots are filled once the shell exists: a change of `PanelData` before that reaches no button.
    if open && ui.root.is_some_and(|e| e != Entity::PLACEHOLDER) {
        let station_list = station_ship.map(|s| rt.catalog.ship(s.def).buildable.clone()).unwrap_or_default();
        let options = button_options(&snap.options, &station_list, &rt.catalog);
        ui.items = options.iter().take(BUTTONS).map(|o| o.item).collect();
        ui.cancels = options.iter().take(BUTTONS).map(|o| o.progress.is_some() && o.ready.is_err()).collect();
        let game = assets.as_ref().and_then(|a| a.game.clone());
        let text = |key: &str| assets.as_ref().and_then(|a| a.text.get(key.trim())).filter(|t| !t.is_empty()).map(str::to_string);
        let tip = |o: &BuildOption| build_tooltip(o, &rt.catalog, game.as_deref(), &text);
        let slots = button_slots(&options, &rt.catalog, &tip);
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
    if !bar.queue.is_empty() {
        bar.queue.clear();
    }
    if bar.credits != snap.credits {
        bar.credits = snap.credits;
    }
}

/// Where the shell's origin goes for a structure seen at `screen` (logical viewport px, y down) in a viewport `height` px high, at UI
/// scale `scale`: its root's left / bottom (design px). `lo` is the box's offset from the origin; `i_build_buttons.alo` puts its buttons
/// 67 and 120 px below the origin, so they hang under the structure's centre.
fn shell_place(screen: Vec2, height: f32, scale: f32, lo: Vec2) -> Vec2 {
    Vec2::new(screen.x / scale + lo.x, (height - screen.y) / scale + lo.y)
}

/// `Manual_Offset`: keeps the build buttons' origin on the selected structure's screen position (hidden while it is off screen).
fn build_bar_follow(
    mut ui: ResMut<BuildBarUi>,
    ships: Res<crate::RenderedShips>,
    scale: Res<UiScale>,
    camera: Single<(&Camera, &GlobalTransform), With<crate::camera::BattleCamera>>,
    mut roots: Query<(&mut Node, &mut Visibility)>,
) {
    let (Some(root), Some(station)) = (ui.root.filter(|&e| e != Entity::PLACEHOLDER), ui.station) else { return };
    let Ok((mut node, mut vis)) = roots.get_mut(root) else { return };
    let lo = *ui.lo.get_or_insert_with(|| {
        let px = |v: Val| if let Val::Px(p) = v { p } else { 0.0 };
        Vec2::new(px(node.left), px(node.bottom))
    });
    let (cam, gt) = *camera;
    let seen = ships.0.get(&station).and_then(|s| cam.world_to_viewport(gt, s.pos).ok());
    let (Some(screen), Some(size)) = (seen, cam.logical_viewport_size()) else {
        vis.set_if_neq(Visibility::Hidden);
        return;
    };
    vis.set_if_neq(Visibility::Inherited);
    let at = shell_place(screen, size.y, scale.0.max(1e-3), lo);
    if node.left != Val::Px(at.x) || node.bottom != Val::Px(at.y) {
        node.left = Val::Px(at.x);
        node.bottom = Val::Px(at.y);
    }
}

/// `Build_NN` buys its item, or takes it out of production again when it is in production and cannot be bought twice (the queue icons
/// that did that are not shown in a battle); `tqueueNN` takes the queued job out again.
fn build_bar_clicks(mut clicks: MessageReader<EawButtonClicked>, rt: Res<BattleRuntime>, settings: Res<BattleViewSettings>, ui: Res<BuildBarUi>) {
    let player = settings.player_side;
    let Some(cur) = rt.cur.as_ref() else { return };
    for c in clicks.read() {
        if let Some(k) = c.name.strip_prefix("build_").and_then(|n| n.parse::<usize>().ok()) {
            if let Some(&item) = ui.items.get(k) {
                let command = if ui.cancels.get(k).copied().unwrap_or(false) { Command::CancelTacticalBuild { item } } else { Command::TacticalBuild { item } };
                rt.command(player, command);
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
    ui.cancels.clear();
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
    fn the_buttons_hang_below_the_structure_on_screen() {
        // `i_build_buttons.alo`: Build_00 .. 05 at x -54, 0, 52 and y -67, -120 (52 px squares) round the origin; the box spans -80..78 by
        // -146..-41. A structure at the centre of a 1080 px viewport (UI scale 1): the box's top edge sits 41 px below it.
        let at = shell_place(Vec2::new(960.0, 540.0), 1080.0, 1.0, Vec2::new(-80.0, -146.0));
        assert_eq!(at, Vec2::new(880.0, 394.0));
        assert_eq!(at.y + 105.0, 540.0 - 41.0);
        // UI scale 1.5 (1620 px): the same place in design px.
        assert_eq!(shell_place(Vec2::new(1440.0, 810.0), 1620.0, 1.5, Vec2::new(-80.0, -146.0)), at);
    }

    #[test]
    fn the_snapshot_carries_the_build_bar_and_its_slots_show_icon_price_and_progress() {
        let (b, cat) = battle();
        let sim = crate::sim::SimHandle::spawn(b);
        sim.request(1);
        let cur = sim.fetch(0, true).0.expect("snapshot");
        let bar = &cur.builds[1];
        assert_eq!(bar.credits, 300);
        let list = cat.ships[1].buildable.clone();
        let no_text = |_: &str| None;
        let tip = |o: &BuildOption| build_tooltip(o, &cat, None, &no_text);
        let slots = button_slots(&button_options(&bar.options, &list, &cat), &cat, &tip);
        assert_eq!(slots.len(), 1, "the second level waits for the first");
        assert_eq!((slots[0].icon.as_str(), slots[0].label.as_str(), slots[0].disabled), ("i_multi_improved_weapons1.tga", "160", false));
        assert_eq!(slots[0].tooltip, "weapons l1\n\n@Cost\t160 credits\n@Build time\t10 s");
        assert!(cur.builds[0].options.is_empty(), "the attacker has no station");

        sim.command(1, Command::TacticalBuild { item: 0 });
        for tick in 1..=30 * 5u64 {
            sim.request(tick + 1);
            sim.fetch(tick, true);
        }
        let cur = sim.fetch(150, true).0.expect("snapshot");
        let bar = &cur.builds[1];
        assert_eq!(bar.credits, 140);
        assert_eq!(bar.queue.len(), 1);
        // The sim offers the second level already (its prerequisite is in production), but the line keeps one button: the first level,
        // building, no price, half done, the progress on the button (before, level 2 got a button of its own next to it and pushed the
        // other lines one button on).
        assert_eq!(bar.options.len(), 2);
        let slots = button_slots(&button_options(&bar.options, &list, &cat), &cat, &tip);
        assert_eq!(slots.len(), 1);
        assert_eq!((slots[0].icon.as_str(), slots[0].label.as_str(), slots[0].disabled), ("i_multi_improved_weapons1.tga", "", false));
        assert!((slots[0].progress - 0.5).abs() < 0.02, "{}", slots[0].progress);
        assert!(slots[0].tooltip.contains("@In production\t50%") && slots[0].tooltip.ends_with("Click to cancel it."), "{}", slots[0].tooltip);
    }

    #[test]
    fn every_upgrade_line_keeps_its_button_through_a_purchase() {
        // Reactors L1 (0) and Armor L1 (1) in the station's list; Reactors L2 (2) follows L1. The play test at Brentaal: after a
        // purchase the buttons changed places.
        let frigate = ShipDef { name: "frigate".into(), class: TargetClass::Frigate, ..ShipDef::default() };
        let station = ShipDef { name: "station".into(), class: TargetClass::Structure, buildable: vec![("empire".into(), 0), ("empire".into(), 1)], ..ShipDef::default() };
        let mut cat = Catalog::from_defs(vec![frigate, station], vec![]);
        let item = |name: &str, prerequisites: Vec<BuildId>, replaces| BuildableDef { name: name.into(), cost: 100, time: 10.0, prerequisites, replaces, ..BuildableDef::default() };
        cat.buildables = vec![item("reactors_l1", vec![], None), item("armor_l1", vec![], None), item("reactors_l2", vec![0], Some(0))];
        let list = cat.ships[1].buildable.clone();
        let opt = |item, progress: Option<f32>| BuildOption { item, ready: if progress.is_some() { Err(battle_sim::CommandError::BuildLimit) } else { Ok(()) }, progress };
        let shown = |options: &[BuildOption]| button_options(options, &list, &cat).iter().map(|o| (o.item, o.progress)).collect::<Vec<_>>();
        assert_eq!(shown(&[opt(1, None), opt(0, None)]), [(0, None), (1, None)]);
        // Reactors L1 bought: it stays on the first button with its progress, Armor keeps the second; L2 waits for L1.
        assert_eq!(shown(&[opt(1, None), opt(2, None), opt(0, Some(0.3))]), [(0, Some(0.3)), (1, None)]);
        // L1 finished: L2 takes the first button.
        assert_eq!(shown(&[opt(1, None), opt(2, None)]), [(2, None), (1, None)]);
    }

    #[test]
    fn build_bar_buttons_follow_station_definition_order_not_alphabetical() {
        let frigate = ShipDef { name: "frigate".into(), class: TargetClass::Frigate, ..ShipDef::default() };
        // Station definition order: Reactors (0), Armor (1), Weapons (2), Defenses (3)
        let station = ShipDef {
            name: "station".into(),
            class: TargetClass::Structure,
            buildable: vec![
                ("empire".into(), 0),
                ("empire".into(), 1),
                ("empire".into(), 2),
                ("empire".into(), 3),
            ],
            ..ShipDef::default()
        };
        let mut cat = Catalog::from_defs(vec![frigate, station], vec![]);
        let item = |name: &str, replaces: Option<BuildId>| BuildableDef {
            name: name.into(),
            cost: 100,
            time: 10.0,
            replaces,
            ..BuildableDef::default()
        };
        // 0: Reactors L1, 1: Armor L1, 2: Weapons L1, 3: Defenses L1, 4: Weapons L2 (replaces 2)
        cat.buildables = vec![
            item("reactors_l1", None),
            item("armor_l1", None),
            item("weapons_l1", None),
            item("defenses_l1", None),
            item("weapons_l2", Some(2)),
        ];

        // Suppose sim returned options in alphabetical sort:
        // Armor (1), Defenses (3), Reactors (0), Weapons (2)
        let opt = |id| BuildOption { item: id, ready: Ok(()), progress: None };
        let mut options = vec![opt(1), opt(3), opt(0), opt(2)];

        sort_options_by_data_order(&mut options, &cat.ships[1].buildable, &cat);
        let items: Vec<BuildId> = options.iter().map(|o| o.item).collect();
        assert_eq!(items, vec![0, 1, 2, 3], "must follow station data order (Reactors, Armor, Weapons, Defenses)");

        // Now suppose Weapons L1 was replaced by Weapons L2 (item 4):
        let mut upgraded_options = vec![opt(3), opt(4), opt(1), opt(0)];
        sort_options_by_data_order(&mut upgraded_options, &cat.ships[1].buildable, &cat);
        let upgraded_items: Vec<BuildId> = upgraded_options.iter().map(|o| o.item).collect();
        assert_eq!(upgraded_items, vec![0, 1, 4, 3], "replacement upgrade retains original slot in data order");
    }
}

