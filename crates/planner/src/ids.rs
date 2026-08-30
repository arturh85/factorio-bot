use serde::{Deserialize, Serialize};

/// A count of Factorio game ticks. 60 ticks = 1 second.
pub type Ticks = u32;

/// An item or entity prototype name, e.g. `"iron-plate"`.
pub type ItemId = String;

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
/// A chain is the expansion of one `Goal::Have { whose: Holder::Bot(_) }`
/// subtree: its steps hand items to each other through a single inventory, so
/// splitting it across bots leaves the consumer empty-handed. The scheduler
/// still chooses *which* bot — it just chooses once per chain instead of once
/// per action. Chain identity is a property of the network's structure, so it
/// lives in `ActionNetwork`, not on `Action`.
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
