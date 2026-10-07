//! The engine side of the script API: [`ScriptHost`] is what the game simulation (galaxy-sim) implements.
//!
//! Scripts see *handles* (players, planets, game objects, object types, task forces); the host speaks in
//! plain ids. Every method has a conservative default so a host can start with a few and grow.

use std::collections::BTreeMap;

/// Which part of the game the scripts run in (`Get_Game_Mode()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameMode {
    /// Strategic map.
    Galactic,
    /// Space tactical battle.
    Space,
    /// Land tactical battle (not supported by this project, kept for completeness).
    Land,
}

impl GameMode {
    /// The string scripts compare against (`"Galactic"`, `"Space"`, `"Land"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Galactic => "Galactic",
            Self::Space => "Space",
            Self::Land => "Land",
        }
    }
}

/// Player id (index in the host's player list).
pub type PlayerId = u32;
/// Game object id. Planets are game objects too.
pub type ObjectId = u64;

/// Facts about a player.
#[derive(Debug, Clone, Default)]
pub struct PlayerInfo {
    /// Display / faction name used by `Find_Player` (`Rebel`, `Empire`, `Underworld`, `Pirates`, ...).
    pub name: String,
    /// Human player.
    pub is_human: bool,
    /// Current credits.
    pub credits: f32,
    /// AI difficulty (`Easy` / `Normal` / `Hard`) as the engine reports it.
    pub difficulty: String,
    /// Current tech level.
    pub tech_level: i32,
    /// The neutral player (owner of unowned planets).
    pub is_neutral: bool,
}

/// Facts about a game object (unit, structure, planet, ...).
#[derive(Debug, Clone, Default)]
pub struct ObjectInfo {
    /// Id.
    pub id: ObjectId,
    /// Name of the game object type (`Star_Destroyer`, `Coruscant`, ...).
    pub type_name: String,
    /// Instance name (planets: planet name).
    pub name: String,
    /// Owning player, if any.
    pub owner: Option<PlayerId>,
    /// Planet the object is at / is.
    pub planet: Option<ObjectId>,
    /// World or map position.
    pub position: [f32; 3],
    /// True for planets.
    pub is_planet: bool,
    /// Object categories (`Capital`, `Fighter`, `Structure`, ...), upper case.
    pub categories: Vec<String>,
    /// Hull fraction 0..1.
    pub hull: f32,
}

/// Where to spawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpawnAt {
    /// At / orbiting a planet.
    Planet(ObjectId),
    /// At a world position.
    Position([f32; 3]),
}

/// Which objects to enumerate.
#[derive(Debug, Clone, Default)]
pub struct ObjectFilter {
    /// Only this object type (case-insensitive name).
    pub type_name: Option<String>,
    /// Only this category (case-insensitive).
    pub category: Option<String>,
    /// Only objects owned by this player.
    pub owner: Option<PlayerId>,
    /// Only objects at this planet.
    pub planet: Option<ObjectId>,
}

/// A story reward fired by the story engine, to be applied by the host.
#[derive(Debug, Clone, PartialEq)]
pub struct StoryReward {
    /// `Reward_Type` (`SPAWN_HERO`, `SET_TECH_LEVEL`, `CREDITS`, ...).
    pub kind: String,
    /// `Reward_Param1..` as written.
    pub params: Vec<String>,
    /// Name of the event that granted it.
    pub event: String,
    /// Plot owner faction, if known.
    pub faction: Option<String>,
}

/// Everything scripts can ask of the game. All methods have defaults.
pub trait ScriptHost {
    /// Current game mode.
    fn game_mode(&self) -> GameMode {
        GameMode::Galactic
    }
    /// All player ids.
    fn players(&self) -> Vec<PlayerId> {
        Vec::new()
    }
    /// Finds a player by name; `"local"` is handled by the runtime via [`local_player`](Self::local_player).
    fn find_player(&self, name: &str) -> Option<PlayerId> {
        self.players().into_iter().find(|p| {
            self.player(*p)
                .is_some_and(|i| i.name.eq_ignore_ascii_case(name))
        })
    }
    /// The human player.
    fn local_player(&self) -> Option<PlayerId> {
        self.players()
            .into_iter()
            .find(|p| self.player(*p).is_some_and(|i| i.is_human))
    }
    /// Player facts.
    fn player(&self, _id: PlayerId) -> Option<PlayerInfo> {
        None
    }
    /// Adds (or subtracts) credits.
    fn give_money(&mut self, _player: PlayerId, _amount: f32) {}
    /// Sets the tech level.
    fn set_tech_level(&mut self, _player: PlayerId, _level: i32) {}
    /// Makes two players allies.
    fn make_ally(&mut self, _a: PlayerId, _b: PlayerId) {}
    /// True if the two players are enemies.
    fn is_enemy(&self, _a: PlayerId, _b: PlayerId) -> bool {
        false
    }
    /// True if an object type with this name exists in the game data.
    fn object_type_exists(&self, _name: &str) -> bool {
        true
    }
    /// Facts about an object; `None` if it does not exist (any more).
    fn object(&self, _id: ObjectId) -> Option<ObjectInfo> {
        None
    }
    /// All planets in the galaxy.
    fn all_planets(&self) -> Vec<ObjectId> {
        self.find_objects(&ObjectFilter {
            category: Some("Planet".into()),
            ..ObjectFilter::default()
        })
    }
    /// A numeric property of an object type from the game data: "combat_rating", "build_cost", "score_cost".
    fn type_number(&self, _type_name: &str, _what: &str) -> Option<f32> {
        None
    }
    /// Evaluates an AI perception (EvaluatePerception(name, player, target)); None if unsupported.
    fn evaluate_perception(
        &self,
        _name: &str,
        _player: PlayerId,
        _target: Option<ObjectId>,
    ) -> Option<f32> {
        None
    }
    /// Looks a planet up by name.
    fn planet_by_name(&self, _name: &str) -> Option<ObjectId> {
        None
    }
    /// Enumerates objects (planets included when `type_name` matches a planet).
    fn find_objects(&self, _filter: &ObjectFilter) -> Vec<ObjectId> {
        Vec::new()
    }
    /// Spawns one unit of `type_name`; returns its id (`None` if it could not be created).
    fn spawn(&mut self, _type_name: &str, _at: SpawnAt, _owner: PlayerId) -> Option<ObjectId> {
        None
    }
    /// Removes an object from the game.
    fn despawn(&mut self, _id: ObjectId) {}
    /// Transfers ownership.
    fn change_owner(&mut self, _id: ObjectId, _owner: PlayerId) {}
    /// Moves an object to a planet (galactic) or position.
    fn move_to(&mut self, _id: ObjectId, _to: SpawnAt) {}
    /// Shows a message to the player (`Game_Message`).
    fn message(&mut self, _text: &str) {}
    /// Debug / script log line.
    fn log(&mut self, _script: &str, _text: &str) {}
    /// A story event shows a dialog (`Story_Dialog` file, chapter index); the host resolves the text.
    fn story_dialog(&mut self, _dialog: &str, _chapter: i32, _event: &str, _faction: Option<&str>) {}
    /// Applies a story reward.
    fn story_reward(&mut self, _reward: &StoryReward) {}
    /// Called once per unknown or unimplemented API function (first use only).
    fn unimplemented_api(&mut self, _name: &str) {}
}

/// A minimal in-memory host for tests and demos.
#[derive(Debug, Default)]
pub struct MockHost {
    /// Game mode reported to scripts.
    pub mode: Option<GameMode>,
    /// Players by id.
    pub players: BTreeMap<PlayerId, PlayerInfo>,
    /// Objects by id.
    pub objects: BTreeMap<ObjectId, ObjectInfo>,
    /// Next object id.
    pub next_id: ObjectId,
    /// Alliances (unordered pairs, stored sorted).
    pub allies: Vec<(PlayerId, PlayerId)>,
    /// Log of everything interesting that scripts did: `"spawn Name owner planet"`, `"message ..."`, ...
    pub events: Vec<String>,
    /// Story rewards received.
    pub rewards: Vec<StoryReward>,
    /// Names reported through [`ScriptHost::unimplemented_api`].
    pub unimplemented: Vec<String>,
}

impl MockHost {
    /// Creates a galaxy with the three main factions plus pirates and a few planets.
    pub fn galactic_demo() -> Self {
        let mut h = Self {
            mode: Some(GameMode::Galactic),
            next_id: 1000,
            ..Self::default()
        };
        for (i, (n, human)) in [
            ("Rebel", true),
            ("Empire", false),
            ("Underworld", false),
            ("Pirates", false),
        ]
        .iter()
        .enumerate()
        {
            h.players.insert(
                i as PlayerId,
                PlayerInfo {
                    name: (*n).into(),
                    is_human: *human,
                    credits: 5000.0,
                    difficulty: "Normal".into(),
                    tech_level: 1,
                    is_neutral: false,
                },
            );
        }
        for (i, (n, owner)) in [("Coruscant", 1), ("Yavin", 0), ("Tatooine", 2), ("Kuat", 1)]
            .iter()
            .enumerate()
        {
            h.objects.insert(
                i as ObjectId + 1,
                ObjectInfo {
                    id: i as ObjectId + 1,
                    type_name: (*n).into(),
                    name: (*n).into(),
                    owner: Some(*owner),
                    is_planet: true,
                    categories: vec!["PLANET".into()],
                    hull: 1.0,
                    ..ObjectInfo::default()
                },
            );
        }
        h
    }
}

impl ScriptHost for MockHost {
    fn game_mode(&self) -> GameMode {
        self.mode.unwrap_or(GameMode::Galactic)
    }
    fn players(&self) -> Vec<PlayerId> {
        self.players.keys().copied().collect()
    }
    fn player(&self, id: PlayerId) -> Option<PlayerInfo> {
        self.players.get(&id).cloned()
    }
    fn give_money(&mut self, player: PlayerId, amount: f32) {
        if let Some(p) = self.players.get_mut(&player) {
            p.credits += amount;
        }
        self.events.push(format!("give_money {player} {amount}"));
    }
    fn set_tech_level(&mut self, player: PlayerId, level: i32) {
        if let Some(p) = self.players.get_mut(&player) {
            p.tech_level = level;
        }
    }
    fn make_ally(&mut self, a: PlayerId, b: PlayerId) {
        self.allies.push((a.min(b), a.max(b)));
    }
    fn is_enemy(&self, a: PlayerId, b: PlayerId) -> bool {
        a != b && !self.allies.contains(&(a.min(b), a.max(b)))
    }
    fn object(&self, id: ObjectId) -> Option<ObjectInfo> {
        self.objects.get(&id).cloned()
    }
    fn planet_by_name(&self, name: &str) -> Option<ObjectId> {
        self.objects
            .values()
            .find(|o| o.is_planet && o.name.eq_ignore_ascii_case(name))
            .map(|o| o.id)
    }
    fn find_objects(&self, f: &ObjectFilter) -> Vec<ObjectId> {
        self.objects
            .values()
            .filter(|o| {
                f.type_name
                    .as_ref()
                    .is_none_or(|t| o.type_name.eq_ignore_ascii_case(t))
            })
            .filter(|o| {
                f.category
                    .as_ref()
                    .is_none_or(|c| o.categories.iter().any(|x| x.eq_ignore_ascii_case(c)))
            })
            .filter(|o| f.owner.is_none_or(|p| o.owner == Some(p)))
            .filter(|o| f.planet.is_none_or(|p| o.planet == Some(p)))
            .map(|o| o.id)
            .collect()
    }
    fn spawn(&mut self, type_name: &str, at: SpawnAt, owner: PlayerId) -> Option<ObjectId> {
        let id = self.next_id;
        self.next_id += 1;
        let (planet, position) = match at {
            SpawnAt::Planet(p) => (Some(p), [0.0; 3]),
            SpawnAt::Position(v) => (None, v),
        };
        self.objects.insert(
            id,
            ObjectInfo {
                id,
                type_name: type_name.into(),
                name: type_name.into(),
                owner: Some(owner),
                planet,
                position,
                hull: 1.0,
                ..ObjectInfo::default()
            },
        );
        self.events
            .push(format!("spawn {type_name} owner {owner} at {planet:?}"));
        Some(id)
    }
    fn despawn(&mut self, id: ObjectId) {
        self.objects.remove(&id);
    }
    fn change_owner(&mut self, id: ObjectId, owner: PlayerId) {
        if let Some(o) = self.objects.get_mut(&id) {
            o.owner = Some(owner);
        }
        self.events.push(format!("change_owner {id} {owner}"));
    }
    fn type_number(&self, _type_name: &str, what: &str) -> Option<f32> {
        (what == "combat_rating").then_some(100.0)
    }
    fn message(&mut self, text: &str) {
        self.events.push(format!("message {text}"));
    }
    fn story_reward(&mut self, reward: &StoryReward) {
        self.rewards.push(reward.clone());
    }
    fn unimplemented_api(&mut self, name: &str) {
        self.unimplemented.push(name.to_string());
    }
}
