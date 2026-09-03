//! Stage 2 of the starter factory, at the seam a script sees.
//!
//! One goal — `Goal::Producing { item: "automation-science-pack", .. }` — and
//! the two questions a run asks of it: does it *plan*, and does the thing it
//! planned *hold* once it stands. The geometry that decides both lives in
//! `crates/planner/src/method/assemble.rs` and is tested there; this file is
//! the outside view, plus the game-data premises the whole design rests on.
//!
//! **The premises are checked against a live capture, not against memory.**
//! `crates/core/tests/live-2.1.17-world-snapshot.json` is a byte-for-byte RCON
//! reply from a real Factorio 2.1.17 game with 0 of 277 technologies
//! researched, so its `enabled` flags *are* the answer to "available with no
//! research". `crates/core/tests/recipes-fixtures.json` is a Factorio **1.1**
//! capture from 2022 and is not used for any claim about the chain — that
//! directory's README lists four defects it has already caused. It is used
//! only as the *shape* of a world to plan against, which is what every other
//! planner test does with it.

use factorio_bot_core::factorio::snapshot::WorldSnapshot;
use factorio_bot_core::serde_json;
use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::{FactorioEntity, FactorioRecipe, Position};
use factorio_bot_planner::goal::Goal;
use factorio_bot_planner::{ActionKind, ActionNetwork, BotId, PlanState, expand, registry_for};
use std::sync::Arc;

const WORLD_SNAPSHOT: &str = include_str!("../../core/tests/live-2.1.17-world-snapshot.json");

const PACK: &str = "automation-science-pack";

// ---------------------------------------------------------------------------
// The premises, from the game's own data
// ---------------------------------------------------------------------------

fn live() -> WorldSnapshot {
    serde_json::from_str(WORLD_SNAPSHOT).expect("the live capture")
}

/// What red science is made of, and what makes it, read off a live game.
///
/// Every number the layout uses is here, and each one of them decides
/// something: two ingredients decides that the cell has two input chests; one
/// of them being craftable from a single other item decides that the cell has
/// an intermediate machine rather than a third chest; the 5 s energy against
/// the machine's 0.5 crafting speed decides the 600 ticks a cell takes per
/// pack, and therefore that one cell is six packs a minute and not twelve.
#[test]
fn the_red_science_chain_is_what_the_live_game_says_it_is() {
    let live = live();
    let recipe = |name: &str| -> FactorioRecipe {
        live.recipes
            .iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("the live capture has a {name} recipe"))
            .clone()
    };

    let pack = recipe(PACK);
    assert_eq!(pack.category, "crafting", "a furnace will not make one");
    assert_eq!(pack.energy.raw(), 5.0);
    let mut ingredients: Vec<(String, u32)> = pack
        .ingredients
        .clone()
        .unwrap_or_default()
        .iter()
        .map(|i| (i.name.clone(), i.amount))
        .collect();
    ingredients.sort();
    assert_eq!(
        ingredients,
        vec![
            ("copper-plate".to_string(), 1),
            ("iron-gear-wheel".to_string(), 1)
        ],
        "one copper plate and one iron gear wheel, not two plates and not a gear pair"
    );

    // The half of the chain the cell builds a machine for: a gear is crafted,
    // from exactly one other item, so one assembling machine standing beside
    // the pack machine turns iron plates into the gears it eats.
    let gear = recipe("iron-gear-wheel");
    assert_eq!(gear.category, "crafting");
    assert_eq!(
        gear.ingredients
            .clone()
            .unwrap_or_default()
            .iter()
            .map(|i| (i.name.clone(), i.amount))
            .collect::<Vec<_>>(),
        vec![("iron-plate".to_string(), 2)],
        "two iron plates a gear -- the number the cell's iron charge is sized from"
    );

    // The other half: a copper plate is *smelted*, so nothing in an assembling
    // machine makes one and it has to arrive in a chest.
    assert_eq!(recipe("copper-plate").category, "smelting");

    // And what runs the recipe. `automation` unlocks exactly one assembling
    // machine, and its crafting speed is the divisor in every rate below.
    let machine = live
        .entity_prototypes
        .iter()
        .find(|p| p.name == "assembling-machine-1")
        .expect("the live capture has an assembling-machine-1 prototype")
        .clone();
    assert_eq!(machine.entity_type, "assembling-machine");
    assert_eq!(
        machine.crafting_speed,
        Some(0.5),
        "5 s of recipe at speed 0.5 is 10 s a pack: six a minute per machine, not twelve"
    );

    // Locked at the start, all four of them, which is why the cell's bill
    // drags the whole research ladder behind it.
    for locked in [
        PACK,
        "assembling-machine-1",
        "inserter",
        "small-electric-pole",
    ] {
        assert!(
            !recipe(locked).enabled,
            "{locked} is enabled with no research in the live capture; the cell's gating is \
             then asserting nothing"
        );
    }
    // And the two that are not, which is why the chests and the gears are free.
    for open in ["iron-chest", "iron-gear-wheel"] {
        assert!(recipe(open).enabled, "{open} needs no research");
    }
}

/// The spec's §6.1 figure of ~620 kW, checked against the demand ledger that
/// now exists rather than against the table it was written from.
///
/// It is exact: three assembling machines, two electric drills, a lab and
/// twelve inserters is **621 kW** by `consumer_kw`. What the cell built here
/// actually draws is a different and much smaller number, because it has no
/// drills and three inserters rather than twelve — and that difference is the
/// point of checking, not a discrepancy.
#[test]
fn the_specs_621_kw_is_what_the_demand_ledger_says() {
    let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
    let kw = |name: &str| {
        state
            .consumer_draw_kw(name)
            .unwrap_or_else(|| panic!("{name} must be in the demand table"))
    };
    let spec_bill = 3. * kw("assembling-machine-1")
        + 2. * kw("electric-mining-drill")
        + kw("lab")
        + 12. * kw("inserter");
    assert_eq!(spec_bill, 621.0);

    // What one red-science cell draws, from the same table.
    let cell = 2. * kw("assembling-machine-1") + 3. * kw("inserter");
    assert_eq!(cell, 189.0);
    assert!(
        cell < 900.,
        "one steam engine is 900 kW, and the plant the planner builds has exactly one"
    );
}

// ---------------------------------------------------------------------------
// The plan
// ---------------------------------------------------------------------------

/// `fixture_world()`, plus the one recipe it is missing.
///
/// The 1.1 recipe capture has no `assembling-machine-1` at all, so a bill that
/// asks for one is unplannable in it for a reason that has nothing to do with
/// this design. The recipe added here is **the live 2.1.17 one**, ingredients
/// and energy taken from the capture asserted above, and it is added `enabled`
/// so these tests are about the layout rather than about the research ladder
/// (which `crates/planner/src/method/have.rs` already covers at length).
fn world_that_can_build_an_assembler() -> factorio_bot_core::factorio::world::FactorioWorld {
    let world = fixture_world();
    let recipe: FactorioRecipe = serde_json::from_str(
        r#"{
          "name": "assembling-machine-1",
          "valid": true,
          "enabled": true,
          "category": "crafting",
          "ingredients": [
            { "name": "iron-plate", "ingredient_type": "item", "amount": 9 },
            { "name": "iron-gear-wheel", "ingredient_type": "item", "amount": 5 },
            { "name": "electronic-circuit", "ingredient_type": "item", "amount": 3 }
          ],
          "products": [
            { "name": "assembling-machine-1", "product_type": "item", "amount": 1, "probability": 1.0 }
          ],
          "hidden": false,
          "energy": 0.5,
          "order": "a[items]-a[assembling-machine-1]",
          "group": "production",
          "subgroup": "production-machine"
        }"#,
    )
    .expect("the assembling machine recipe parses");
    world
        .update_recipes(vec![recipe])
        .expect("update_recipes cannot fail for a well-formed recipe");
    world
}

/// A state with a plant already standing: a small pole and a steam engine, in
/// the overlay, exactly where `crate::test_world::with_steam_power` puts them
/// for the research tests.
///
/// **And one wood per bot, which is not decoration.** `small-electric-pole` is
/// one wood and two copper cable, the planner cannot *make* wood (`Mine`
/// sources only resources, and a tree is an obstacle), and every bot a real
/// run starts carries exactly one — confirmed in
/// `crates/core/tests/live-2.1.17-players.json`. The shared fixture has no
/// players at all, so its bots start empty and the wood has to be put there
/// for the fixture to model a real roster. It is also the hard cap this cell
/// lives under: four bots, four wood, eight poles ever, one of which the power
/// plant has already spent.
fn powered_state(bots: &[BotId]) -> PlanState {
    let mut state = PlanState::from_world(Arc::new(world_that_can_build_an_assembler()), bots);
    for bot in bots {
        state.gain(*bot, "wood", 1);
    }
    for (name, position) in [
        ("small-electric-pole", Position::new(10.5, 10.5)),
        ("steam-engine", Position::new(12.5, 10.5)),
    ] {
        state.create_entity(FactorioEntity {
            name: name.into(),
            position,
            ..Default::default()
        });
    }
    state
}

fn plan(per_minute: u32) -> Result<ActionNetwork, factorio_bot_planner::error::PlannerError> {
    let bots = [BotId(1)];
    let state = powered_state(&bots);
    expand(
        &[Goal::Producing {
            item: PACK.into(),
            per_minute,
        }],
        &state,
        &registry_for(&bots),
        BotId(1),
    )
}

/// The headline: red science is planned as a cell of machines, not hand-crafted.
///
/// Two assembling machines, three inserters, two chests and a pole get placed;
/// two recipes get set; and the chests get charged. Every one of those counts
/// is a decision the layout made, so all of them are asserted rather than "a
/// plan came back".
#[test]
fn a_producing_goal_for_red_science_builds_an_assembly_cell() {
    let net = plan(6).expect("a powered world can build a red-science cell");

    let placed: Vec<String> = net
        .actions()
        .filter_map(|a| match &a.kind {
            ActionKind::Place { entity } => Some(entity.name.clone()),
            _ => None,
        })
        .collect();
    let count = |name: &str| placed.iter().filter(|n| n.as_str() == name).count();
    assert_eq!(
        count("assembling-machine-1"),
        2,
        "one for gears, one for packs"
    );
    assert_eq!(
        count("inserter"),
        3,
        "chest->gears, gears->packs, chest->packs"
    );
    assert_eq!(
        count("iron-chest"),
        2,
        "one for iron plates, one for copper"
    );
    assert_eq!(
        count("small-electric-pole"),
        1,
        "the cell carries its own supply"
    );

    let recipes: Vec<String> = net
        .actions()
        .filter_map(|a| match &a.kind {
            ActionKind::SetRecipe { recipe, .. } => Some(recipe.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        recipes,
        vec!["iron-gear-wheel".to_string(), PACK.to_string()],
        "a machine with no recipe on it is the placed-but-dead machine this whole stage \
         exists to make impossible"
    );
}

/// One cell is six packs a minute, and a seventh needs a second cell.
///
/// The boundary `Goal::Producing`'s integer rate exists for: 5 s of recipe in
/// a 0.5-speed machine is 600 ticks a pack, so `ceil(per_minute * 600 / 3600)`
/// is 1 at six and 2 at seven, with no float anywhere near the ceiling.
#[test]
fn the_cell_count_follows_the_rate_and_nothing_else() {
    let machines = |per_minute: u32| {
        plan(per_minute)
            .expect("a powered world can build a red-science cell")
            .actions()
            .filter(|a| {
                matches!(&a.kind, ActionKind::Place { entity } if entity.name == "assembling-machine-1")
            })
            .count()
    };
    assert_eq!(machines(6), 2, "one cell");
    assert_eq!(machines(7), 4, "two cells");
}

/// **The four-bot run of 2026-09-03, reduced to a test.**
///
/// `run-1788405365-21697` cleared rung 1 (`researched("automation")`) and then
/// refused rung 2 three seconds after the goal was accepted, at plan time:
///
/// ```text
/// precondition has 3 iron-ore of action ActionId(41) does not hold for bot 2
/// ```
///
/// Everything here is taken from that run's `samples.jsonl` at tick 107,820 —
/// the last `bots` sample before `milestone_stuck` at 107,837 — so the
/// inventories are the ones the four bots really carried: **unequal**, and
/// unequal in the way that matters, one bot holding no ore at all. The same
/// goal plans fine against a roster holding nothing, which is why every
/// existing `Producing` test passes.
#[test]
fn the_live_four_bot_red_science_run_plans_and_schedules() {
    let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let mut state = PlanState::from_world(Arc::new(world_that_can_build_an_assembler()), &bots);
    // The plant, arranged so its own supply area has room for the cell --
    // `assemble.rs`' `powered_with_room`. A cell that had to bring its own
    // pole would refuse for want of wood before ever reaching the ore, which
    // is the *previous* run's defect (`run-1788396958-07935`) and not this
    // one.
    for (name, entity_type, position) in [
        (
            "small-electric-pole",
            "electric-pole",
            Position::new(30.5, 32.5),
        ),
        (
            "small-electric-pole",
            "electric-pole",
            Position::new(30.5, 39.5),
        ),
        ("steam-engine", "generator", Position::new(32.5, 39.5)),
    ] {
        state.create_entity(FactorioEntity {
            name: name.into(),
            entity_type: entity_type.into(),
            position,
            ..Default::default()
        });
    }
    // samples.jsonl, tick 107820. Bot 1 has spent its wood on the plant's pole
    // and its furnace on the smelting; bot 3 mined nothing in the last stretch
    // and holds no ore at all.
    for (bot, items) in [
        (
            BotId(1),
            &[
                ("burner-mining-drill", 1u32),
                ("copper-cable", 30),
                ("iron-ore", 48),
            ][..],
        ),
        (
            BotId(2),
            &[
                ("burner-mining-drill", 1),
                ("copper-ore", 4),
                ("iron-ore", 14),
                ("iron-plate", 8),
                ("stone-furnace", 1),
                ("wood", 1),
            ][..],
        ),
        (
            BotId(3),
            &[
                ("burner-mining-drill", 1),
                ("iron-plate", 8),
                ("stone-furnace", 1),
                ("wood", 1),
            ][..],
        ),
        (
            BotId(4),
            &[
                ("burner-mining-drill", 1),
                ("copper-ore", 3),
                ("iron-ore", 14),
                ("iron-plate", 8),
                ("stone-furnace", 1),
                ("wood", 1),
            ][..],
        ),
    ] {
        for (item, count) in items {
            state.gain(bot, item, *count);
        }
    }

    // Where the run left them. Bot 1 had walked back to the plant to smelt;
    // bots 2, 3 and 4 were still standing on the ore patch they had spent
    // rung 1 mining — `samples.jsonl` reports one position each for the last
    // 25,000 ticks. That is not decoration either: the cell's bill is sized
    // against bot 1, and the scheduler binds a chain to whichever bot can run
    // its *first* action cheapest. With three bots parked on the ore, the
    // cheapest bot for the plan's first mine is bot 2.
    for (bot, x, y) in [
        (BotId(2), -29.0, 45.0),
        (BotId(3), -29.0, 46.0),
        (BotId(4), -29.0, 47.0),
    ] {
        state.set_position(bot, Position::new(x, y));
    }

    let net = expand(
        &[Goal::Producing {
            item: PACK.into(),
            per_minute: 6,
        }],
        &state,
        &registry_for(&bots),
        BotId(1),
    )
    .expect("the cell expands");

    // And the run's actual failure: `precondition has 3 iron-ore of action
    // ActionId(41) does not hold for bot 2`, raised three seconds after the
    // goal was accepted.
    let result = factorio_bot_planner::schedule::schedule(&net, &state, &bots)
        .expect("and every action it emitted has a bot that can run it");
    // The mechanism, stated where it broke. `AssembleCell::converges` is
    // `true` and `Goal::Producing` names no holder, so the cell's chain opens
    // *unowned*; every `Goal::Have { whose: Holder::Share(BotId(1)) }` in
    // `bill()` is then expanded inside it and used to record no owner at all.
    // The sizing said bot 1 and nothing committed the chain to bot 1.
    let cell_chain = net
        .actions()
        .find(|a| {
            matches!(&a.kind, ActionKind::Place { entity } if entity.name == "assembling-machine-1")
        })
        .and_then(|a| net.chain_of(a.id))
        .expect("the machines are placed inside a chain");
    assert_eq!(
        net.owner_of(cell_chain),
        Some(BotId(1)),
        "the cell's bill is sized against bot 1's inventory, so bot 1 has to be the bot \
         committed to running it"
    );

    for action in net.actions() {
        if net.chain_of(action.id) == Some(cell_chain) {
            assert_eq!(
                result.assignment(action.id),
                Some(BotId(1)),
                "{} belongs to the cell's chain and must run on the bot it was sized for",
                action.label
            );
        }
    }
}
