use serde::{Deserialize, Serialize};

/// A count of Factorio game ticks. 60 ticks = 1 second.
pub type Ticks = u32;

/// An item or entity prototype name, e.g. `"iron-plate"`.
pub type ItemId = String;

/// A bot, identified by the **Factorio player id** it drives.
///
/// `BotId(3)` is player 3 — not "the third bot", not an index into any roster.
/// There is deliberately no translation anywhere in the stack: the run's roster
/// (`Planner::roster`) is a list of player ids, `PlanState::from_world` looks a
/// bot's inventory up by `world.players[&id.0]`, and `RconActuator` sends
/// `id.0` to the game. A mapping layer is what this type exists to make
/// impossible — an earlier version numbered the actuator's bots `0..n` while
/// the scheduler numbered them `1..=n`, and every action drove the wrong player
/// or none at all.
///
/// A roster is therefore not necessarily `1..=n` and must never be packed down
/// to make it one. `Planner::roster` drops any bot whose Factorio client failed
/// to connect, so `[1, 3]` is an ordinary roster; renumbering it to `[1, 2]`
/// would drive player 2 for every step bot 3 owns.
///
/// The only thing the executor may do with a `BotId` is *check* that the player
/// is connected; it must never renumber.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BotId(pub u8);

impl std::fmt::Display for BotId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "bot {}", self.0)
    }
}

/// Distinct from `factorio_bot_core::types::ActionId`, which correlates RCON calls.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ActionId(pub u32);

#[derive(Debug, Default)]
pub struct ActionIdGen(u32);

impl ActionIdGen {
    pub fn new() -> Self {
        ActionIdGen(0)
    }
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> ActionId {
        let id = ActionId(self.0);
        self.0 += 1;
        id
    }
}

/// Identifies a chain of actions that must all run on the same bot.
///
/// A chain is the expansion of one subtree whose steps hand items to each
/// other through a single inventory, so splitting it across bots leaves the
/// consumer empty-handed. The driver opens one in exactly three situations: a
/// caller names a bot (`Holder::Bot`), a goal is sized against one bot's
/// inventory without naming it as a caller instruction (`Holder::Share`), or a
/// method reports that its decomposition converges, meaning several produced
/// items must meet in one inventory (`Method::converges`). The first two also
/// give the chain an **owner** — see `ActionNetwork::owner_of` — because both
/// name a specific bot the chain's sizing depends on; the scheduler still
/// chooses which bot for a converging chain, since nothing named one. Chain
/// identity is a property of the network's structure, so it lives in
/// `ActionNetwork`, not on `Action`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ChainId(pub u32);

#[derive(Debug, Default)]
pub struct ChainIdGen(u32);

impl ChainIdGen {
    pub fn new() -> Self {
        ChainIdGen(0)
    }
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> ChainId {
        let id = ChainId(self.0);
        self.0 += 1;
        id
    }
}
