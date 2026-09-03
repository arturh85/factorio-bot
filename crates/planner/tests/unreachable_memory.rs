//! What the planner remembers when the game's pathfinder says there is no way
//! there.
//!
//! # The run this exists for
//!
//! `run-1788432181-42528` is the furthest this project has reached, and it
//! ended `stuck` with 48 steps pending and this in its last event:
//!
//! ```text
//! {"kind":"milestone_stuck","index":2,"last_error":
//!  "game rejected the command: the game's pathfinder returned no path:
//!   Error: failed to path find"}
//! ```
//!
//! Twenty walks failed in that run. Nineteen of them were that same
//! pre-dispatch refusal, and they were not twenty different mistakes — they
//! were four destinations, asked for again and again:
//!
//! ```text
//! 5 x bot 3 -> (-54.5, -12.5)     3 x bot 4 -> (-50.5, -12.5)
//! 4 x bot 2 -> (-46.5, -9.5)      2 x bots 2,3 -> (-29, -29)
//! ```
//!
//! `samples.jsonl` says why, and it is worth stating precisely because it
//! decides the shape of the fix. From tick 53 700 to the end of the run, bots
//! 2 and 3 reported the *same position to the byte* — `(-56.2578125,
//! 14.30859375)` and `(-56.2578125, 14.74609375)`. They were boxed in where
//! the power plant was being built around them. Every plan after that sent
//! them at an ore tile they could not reach, the walk was refused before it
//! was dispatched, `abandon_rest` cut the rest of that bot's chain, and the
//! next plan chose the same tile again because nothing had remembered.
//!
//! # What is pinned here
//!
//! That the memory exists, that it is scoped to the question the game
//! actually answered — *this bot, from here, to there* — and, just as
//! importantly, that it can never refuse a plan outright. A destination no bot
//! can reach is still scheduled: the ledger reorders candidates, it does not
//! remove them. Erring the other way would turn a run that limps into a run
//! that stops.
//!
//! # And the half the ledger could never reach
//!
//! Everything above is about `schedule`, and `schedule` can only choose
//! between the candidates an action has. A gathering share has exactly one:
//! `SplitAcrossBots` hands a bot a `Holder::Share` goal,
//! `crates/planner/src/method/mod.rs` gives the resulting chain that bot as
//! **owner**, and `schedule` treats an owner as a hard constraint with no
//! fallback tier. Reordering a one-element list is a no-op, so a share sized
//! against a frozen bot is work nothing can move —
//! [`an_owned_chains_candidate_list_is_one_bot_by_design`] pins that, and it is
//! still true.
//!
//! What changed is that expansion no longer *creates* such a chain. The second
//! half of this file is that: run `run-1788449752-46541`'s own geometry, and
//! the assertion that the work lands on the bots that can do it. See
//! `even_shares` / `participants_that_can_work` in
//! `crates/planner/src/method/have.rs`.

use factorio_bot_core::factorio::world::{Enclosure, FactorioWorld, WalkRefusal};
use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::{FactorioEntity, FactorioPlayer, Position};
use factorio_bot_planner::action::{Action, ActionKind, Actor, Condition, Effect};
use factorio_bot_planner::goal::{Goal, Holder};
use factorio_bot_planner::ids::{ActionIdGen, ChainId};
use factorio_bot_planner::method::expand;
use factorio_bot_planner::method::have::registry_for;
use factorio_bot_planner::{ActionNetwork, BotId, PlanState, Schedule, StepKind, schedule};
use std::collections::BTreeSet;
use std::sync::Arc;

/// Bot 3's position for the last 158 000 ticks of `run-1788432181-42528`,
/// verbatim from `samples.jsonl`. It never changed by a single bit.
const BOXED_IN: (f64, f64) = (-56.2578125, 14.74609375);

/// Where bot 1 was working while that happened. Bot 1 made 746 of the run's
/// 831 dispatches; it is the bot with somewhere else to be.
const ELSEWHERE: (f64, f64) = (-23.1875, -37.90625);

fn at(p: (f64, f64)) -> Position {
    Position::new(p.0, p.1)
}

fn refusal(player: u8, from: (f64, f64), to: &Position) -> WalkRefusal {
    WalkRefusal {
        // `None` is the honest tick for this refusal: the pathfinder answers
        // before the walk is dispatched, so the game never stamps one.
        tick: None,
        player,
        from: at(from),
        to: to.clone(),
    }
}

/// A fixture world carrying `refusals`, the way a run's `FactorioWorld`
/// carries them from one plan to the next.
fn world_with(refusals: &[WalkRefusal]) -> Arc<FactorioWorld> {
    let world = fixture_world();
    for refusal in refusals {
        world.record_walk_refusal(refusal.clone());
    }
    Arc::new(world)
}

/// The roster of the run: four bots, with 3 boxed in and 1 far away and busy.
///
/// Positions are set on the state rather than on the world's players because
/// that is the position `schedule` reasons with, and it is the same number in
/// a real run: `PlanState::from_world` seeds it straight from `world.players`.
fn state_with(refusals: &[WalkRefusal]) -> PlanState {
    let bots = roster();
    let mut state = PlanState::from_world(world_with(refusals), &bots);
    state.set_position(BotId(1), at(ELSEWHERE));
    state.set_position(BotId(2), at(BOXED_IN));
    state.set_position(BotId(3), at(BOXED_IN));
    state.set_position(BotId(4), at(ELSEWHERE));
    state
}

fn roster() -> Vec<BotId> {
    vec![BotId(1), BotId(2), BotId(3), BotId(4)]
}

/// An ore tile close to where bots 2 and 3 are standing — so that, with no
/// memory, one of them is the cheapest bot for it and wins.
fn ore_near_the_boxed_in_bots(state: &PlanState) -> Position {
    let mut tiles: Vec<Position> = state
        .resource_patches("coal")
        .into_iter()
        .flat_map(|patch| patch.elements)
        .collect();
    tiles.sort_by(|a, b| {
        let d = |p: &Position| (p.x - BOXED_IN.0).hypot(p.y - BOXED_IN.1);
        d(a).total_cmp(&d(b)).then(a.x.total_cmp(&b.x))
    });
    tiles.first().expect("the fixture has a coal patch").clone()
}

fn mine_at(id_gen: &mut ActionIdGen, pos: &Position) -> Action {
    Action {
        id: id_gen.next(),
        kind: ActionKind::Mine {
            pos: pos.clone(),
            item: "coal".into(),
            count: 1,
        },
        pre: vec![Condition::AtPosition {
            who: Actor::Role,
            pos: pos.clone(),
            radius: 3.0,
            min_radius: 0.0,
        }],
        eff: vec![Effect::GainItem {
            who: Actor::Role,
            item: "coal".into(),
            count: 1,
        }],
        duration: 60,
        pinned: None,
        label: "mine 1 coal".into(),
    }
}

/// Who the schedule sends to `to`, by reading the walk it emitted.
fn walker(schedule: &Schedule, to: &Position) -> Option<BotId> {
    schedule
        .steps
        .iter()
        .find(|step| match &step.what {
            StepKind::Walk { to: dest, .. } => {
                dest.x.total_cmp(&to.x).is_eq() && dest.y.total_cmp(&to.y).is_eq()
            }
            StepKind::Act { .. } => false,
        })
        .map(|step| step.bot)
}

fn schedule_one_mine(refusals: &[WalkRefusal]) -> (PlanState, Position, Schedule) {
    let state = state_with(refusals);
    let pos = ore_near_the_boxed_in_bots(&state);
    let mut id_gen = ActionIdGen::new();
    let mut net = ActionNetwork::new();
    net.add(mine_at(&mut id_gen, &pos));
    let scheduled = schedule(&net, &state, &roster()).expect("one mine, four bots");
    (state, pos, scheduled)
}

/// The control, and the shape of the defect.
///
/// With nothing remembered the nearest bot wins, which is the whole of the
/// scheduler's ranking and is right in every world where the bot can walk. In
/// `run-1788432181-42528` it was bot 3, twelve times over, and bot 3 could not
/// move.
#[test]
fn the_nearest_bot_wins_when_nothing_has_been_refused() {
    let (_, pos, scheduled) = schedule_one_mine(&[]);
    let sent = walker(&scheduled, &pos).expect("the mine needs a walk");
    assert!(
        sent == BotId(2) || sent == BotId(3),
        "the boxed-in bots are the nearest ones to this tile; got {sent}"
    );
}

/// The point of the change: a destination the pathfinder has already refused
/// this bot is not asked of it again.
#[test]
fn a_bot_the_pathfinder_refused_is_not_sent_there_again() {
    let state = state_with(&[]);
    let pos = ore_near_the_boxed_in_bots(&state);
    // Both boxed-in bots asked, from where they stand, and were told no —
    // which is what the run's `walk_settled` rows say happened.
    let refusals = [refusal(2, BOXED_IN, &pos), refusal(3, BOXED_IN, &pos)];
    let (_, pos, scheduled) = schedule_one_mine(&refusals);
    let sent = walker(&scheduled, &pos).expect("the mine still needs a walk");
    assert!(
        sent == BotId(1) || sent == BotId(4),
        "the two bots the game refused must not be re-selected; got {sent}"
    );
}

/// And the memory is about a bot, not about a place.
///
/// Two bots on opposite sides of an obstacle are not asking the same question.
/// Answering bot 1 with bot 3's evidence would fence a bot that can walk away
/// from ground it can reach — the over-eager half of this design, and the one
/// that turns a recoverable run into a refusal.
#[test]
fn one_bots_refusal_says_nothing_about_another_bots_route() {
    let state = state_with(&[]);
    let pos = ore_near_the_boxed_in_bots(&state);
    let (_, pos, scheduled) = schedule_one_mine(&[refusal(1, ELSEWHERE, &pos)]);
    let sent = walker(&scheduled, &pos).expect("the mine needs a walk");
    assert!(
        sent == BotId(2) || sent == BotId(3),
        "only bot 1 was refused, so the nearest bots are still the answer; got {sent}"
    );
}

/// `failed to path find` means unreachable *from here*, not unreachable.
///
/// A bot that has since moved is asking a question this ledger has no answer
/// to. Getting this wrong in the safe-looking direction — remembering a
/// destination as dead forever — is how a run refuses work it could do.
#[test]
fn a_bot_that_has_moved_is_offered_the_destination_again() {
    let state = state_with(&[]);
    let pos = ore_near_the_boxed_in_bots(&state);
    // Refused when it stood 30 tiles away. It is not standing there now.
    let stale = refusal(3, (BOXED_IN.0 - 30., BOXED_IN.1), &pos);
    let (_, pos, scheduled) = schedule_one_mine(&[refusal(2, BOXED_IN, &pos), stale]);
    let sent = walker(&scheduled, &pos).expect("the mine needs a walk");
    assert_eq!(
        sent,
        BotId(3),
        "bot 3's refusal was earned somewhere else and has nothing to say here"
    );
}

/// A refusal must never cost a plan.
///
/// When every bot has been refused, the work is still scheduled — to whoever
/// the ordinary ranking picks. The ledger reorders candidates; it never
/// removes them, and it never turns "we have no good option" into "we refuse
/// to plan". A plan that dispatches and fails leaves a record and a recovery
/// tier; a plan that was never made leaves neither.
#[test]
fn a_destination_no_bot_can_reach_is_still_planned() {
    let state = state_with(&[]);
    let pos = ore_near_the_boxed_in_bots(&state);
    let refusals: Vec<WalkRefusal> = roster()
        .into_iter()
        .map(|bot| {
            let from = if bot == BotId(2) || bot == BotId(3) {
                BOXED_IN
            } else {
                ELSEWHERE
            };
            refusal(bot.0, from, &pos)
        })
        .collect();
    let (_, pos, scheduled) = schedule_one_mine(&refusals);
    assert!(
        walker(&scheduled, &pos).is_some(),
        "the plan must still send somebody; refusing to plan is strictly worse \
         than planning a walk that may fail again"
    );
}

/// Determinism: the ledger is state, and state is where nondeterminism gets in.
///
/// The same refusals recorded in the opposite order must produce the same plan,
/// byte for byte — the planner's whole contract. `PlanState::from_world` sorts
/// them for exactly this reason.
#[test]
fn the_order_the_game_refused_things_in_does_not_reach_the_plan() {
    let state = state_with(&[]);
    let pos = ore_near_the_boxed_in_bots(&state);
    let forwards = [
        refusal(2, BOXED_IN, &pos),
        refusal(3, BOXED_IN, &pos),
        refusal(4, ELSEWHERE, &pos),
    ];
    let mut backwards = forwards.clone();
    backwards.reverse();

    let (a_state, _, a) = schedule_one_mine(&forwards);
    let (b_state, _, b) = schedule_one_mine(&backwards);
    assert_eq!(
        a_state.refused_walks(),
        b_state.refused_walks(),
        "the ledger reaches the planner sorted, not in arrival order"
    );
    assert_eq!(a.makespan, b.makespan);
    assert_eq!(
        a.steps.iter().map(|s| (s.bot, s.start)).collect::<Vec<_>>(),
        b.steps.iter().map(|s| (s.bot, s.start)).collect::<Vec<_>>(),
        "same inputs, same plan"
    );
}

/// **Why none of the above fired in a live run**, and why that is still true.
///
/// Every test above builds its mine as a *free* action — no chain, no owner —
/// and in that shape the ledger works exactly as designed: `schedule` splits
/// the roster into open and refused, and the refused bot loses.
///
/// A real run does not have that shape. Gathering goals are sized per bot
/// (`Holder::Share`), and `crates/planner/src/method/mod.rs` gives such a
/// chain an **owner**; `schedule` treats an owner as a hard constraint —
/// `None if owner.is_some() => vec![vec![owner]]`
/// (`crates/planner/src/schedule.rs`) — with no fallback tier, by design,
/// because the chain's bill was sized against that bot's inventory and nobody
/// else's. The refusal split then partitions a one-element tier and puts it
/// back together unchanged. There is nothing to reorder, so the ledger cannot
/// change the outcome.
///
/// That is what `run-1788449752-46541` shows and what the ledger was believed
/// to have fixed: bot 2 was sent at `(-46.5, -9.5)` on four separate plans,
/// bot 3 at `(-54.5, -12.5)` on five, from positions that never changed by a
/// bit — while every test above passed. The memory is written, read and
/// matched correctly; it just has one candidate to choose from.
///
/// **This test is deliberately unchanged, and it now pins a decision rather
/// than a defect.** The obvious way to "fix" it here — give the owner arm a
/// fallback tier — re-breaks `run-1788405365-21697`, where the scheduler bound
/// a share's chain to whichever bot was cheapest rather than the one its bill
/// was sized against and a downstream action failed for a bot that did not
/// hold the items. The owner has to stay hard. So the fix went one level up,
/// to where the chain is created rather than where it is assigned:
/// [`a_walled_in_bot_is_not_sized_a_share`] is the other half, and the two
/// together are what the file used to say could not compose.
#[test]
fn an_owned_chains_candidate_list_is_one_bot_by_design() {
    let state = state_with(&[]);
    let pos = ore_near_the_boxed_in_bots(&state);
    // Bot 3 asked from where it stands and was told no, exactly as in
    // `a_bot_the_pathfinder_refused_is_not_sent_there_again` above.
    let state = state_with(&[refusal(3, BOXED_IN, &pos)]);

    let mut id_gen = ActionIdGen::new();
    let mut net = ActionNetwork::new();
    let mine = net.add(mine_at(&mut id_gen, &pos));
    // The one difference from the passing test: the mine belongs to a chain
    // that a `Holder::Share` goal named bot 3 for.
    let chain = ChainId(0);
    net.set_chain(mine, chain);
    net.set_chain_owner(chain, BotId(3));

    let scheduled = schedule(&net, &state, &roster()).expect("one mine, four bots");
    assert_eq!(
        walker(&scheduled, &pos),
        Some(BotId(3)),
        "an owner is a hard constraint with no fallback tier, so the bot the \
         game refused is still the only candidate -- kept deliberately: the \
         chain's bill was sized against this bot's inventory, and handing it to \
         another is what broke run-1788405365-21697"
    );
}

// ---------------------------------------------------------------------------
// The other half: expansion, and `run-1788449752-46541`'s own geometry.
// ---------------------------------------------------------------------------

/// Bot 2's position from tick 47 100 to the end of `run-1788449752-46541`,
/// verbatim from `samples.jsonl`. 164 000 ticks — 78% of the run — without
/// moving by a bit.
const FROZEN_BOT_2: (f64, f64) = (-56.2421875, 14.28125);

/// Bot 3's, for the same 164 000 ticks. Forty centimetres from bot 2 and just
/// as stuck.
const FROZEN_BOT_3: (f64, f64) = (-56.26953125, 14.68359375);

/// A closed square ring of trees `radius` tiles out from `around`, spaced half
/// a tile so nothing a character-sized box could slip through is left between
/// two of them.
///
/// The same construction `crates/planner/tests/enclosure_prevention.rs` and
/// `crates/core/tests/enclosure_run13.rs` use, and for the reason those files
/// argue at length: the class of obstacle that actually boxes a bot in — trees,
/// rocks, cliffs, water — reaches `blocked_tree` only and is structurally
/// absent from a run's archived keyframe, so a scenario that needs a wall has
/// to build one.
fn tree_ring(around: &Position, radius: f64) -> Vec<FactorioEntity> {
    let mut ring = Vec::new();
    let mut offset = -radius;
    while offset <= radius {
        for pos in [
            Position::new(around.x() + offset, around.y() - radius),
            Position::new(around.x() + offset, around.y() + radius),
            Position::new(around.x() - radius, around.y() + offset),
            Position::new(around.x() + radius, around.y() + offset),
        ] {
            ring.push(FactorioEntity::new_tree(&pos));
        }
        offset += 0.5;
    }
    ring
}

/// The run's roster as the world saw it: four real players, two of them frozen
/// inside a ring of trees, and the enclosure ledger `crates/executor`'s walk
/// memory would have written for those two.
///
/// `record_enclosure` is called with the same numbers `note_enclosure` uses —
/// no tick (the pathfinder answers before the walk is dispatched), the position
/// the character was standing at, and `SEARCH_RADIUS` for the window. The
/// `pocket_tiles` value is not read by anything under test; it is carried into
/// the plan's own narration.
fn frozen_run_world() -> Arc<FactorioWorld> {
    let world = fixture_world();
    let midpoint = Position::new(
        (FROZEN_BOT_2.0 + FROZEN_BOT_3.0) / 2.,
        (FROZEN_BOT_2.1 + FROZEN_BOT_3.1) / 2.,
    );
    world
        .update_chunk_entities(tree_ring(&midpoint, 3.))
        .expect("the ring loads");
    for (id, at_) in [
        (1u8, ELSEWHERE),
        (2, FROZEN_BOT_2),
        (3, FROZEN_BOT_3),
        (4, (ELSEWHERE.0 + 2., ELSEWHERE.1)),
    ] {
        world.players.insert(
            id,
            FactorioPlayer {
                player_id: id,
                position: at(at_),
                ..Default::default()
            },
        );
    }
    for player in [2, 3] {
        world.record_enclosure(Enclosure {
            tick: None,
            player,
            at: at(if player == 2 {
                FROZEN_BOT_2
            } else {
                FROZEN_BOT_3
            }),
            pocket_tiles: 24.,
            searched_tiles: 24.,
        });
    }
    Arc::new(world)
}

/// Which bot, if any, owns the chain this action belongs to.
fn owner_of(net: &ActionNetwork, action: &Action) -> Option<BotId> {
    net.chain_of(action.id)
        .and_then(|chain| net.owner_of(chain))
}

/// The premise, checked before anything is asserted about a plan.
///
/// Two witnesses are required and the test needs both to be present, or it
/// would pass for the wrong reason: a state where nobody is walled in makes
/// every assertion below trivially true. This is also the statement that the
/// two witnesses agree — the game's recorded observation and the planner's own
/// fill — which is the whole basis for excluding a bot at all.
#[test]
fn the_two_frozen_bots_read_as_walled_in_and_the_other_two_do_not() {
    let state = PlanState::from_world(frozen_run_world(), &roster());
    assert!(
        state.is_walled_in(BotId(2)) && state.is_walled_in(BotId(3)),
        "bots 2 and 3 are inside the ring and the run recorded them enclosed \
         there; got {:?}",
        state.walled_in()
    );
    assert!(
        !state.is_walled_in(BotId(1)) && !state.is_walled_in(BotId(4)),
        "bots 1 and 4 worked freely all run and no enclosure was ever recorded \
         for them; got {:?}",
        state.walled_in()
    );
}

/// **The defect, at the level where it can actually be fixed.**
///
/// `plan_created` at tick 136 430 gave bot 2 "mine 6 iron-ore" and bot 3 the
/// same, to tiles neither had been able to reach for 89 000 ticks. Both walks
/// were refused before dispatch, `abandon_rest` cut the rest of each chain, and
/// the next plan derived the same shares from the same inputs. Bot 1 made 563
/// of the run's 617 dispatches.
///
/// So: no chain in this plan may be owned by a bot that cannot walk, and the
/// ore must still be gathered — by the bots that can.
#[test]
fn a_walled_in_bot_is_not_sized_a_share() {
    let state = PlanState::from_world(frozen_run_world(), &roster());
    let bots = roster();
    let goal = Goal::Have {
        item: "iron-ore".into(),
        count: 8,
        whose: Holder::Anyone,
    };
    let net = expand(&[goal], &state, &registry_for(&bots), BotId(1))
        .expect("eight iron ore, four bots, a fixture patch with room for them");

    let owners: BTreeSet<BotId> = net.actions().filter_map(|a| owner_of(&net, a)).collect();
    assert!(
        !owners.contains(&BotId(2)) && !owners.contains(&BotId(3)),
        "a share names an owner and no other bot may ever take it over, so a \
         share sized against a bot that cannot walk is work the run can never \
         do; owners were {owners:?}"
    );
    assert!(
        owners.contains(&BotId(1)) && owners.contains(&BotId(4)),
        "the work must land on the bots that can do it, not evaporate; owners \
         were {owners:?}"
    );

    let mined: u32 = net
        .actions()
        .filter_map(|a| match &a.kind {
            ActionKind::Mine { item, count, .. } if item == "iron-ore" => Some(*count),
            _ => None,
        })
        .sum();
    assert_eq!(
        mined, 8,
        "the same eight ore are still gathered; the split is narrower, not smaller"
    );
}

/// The control, and the reason the assertion above is about walling-in rather
/// than about walk refusals.
///
/// The same world with the enclosure ledger empty — a run where the pathfinder
/// has never been asked, or has been asked and found a way. Every bot is sized
/// a share, exactly as before this change. Without this, a bug that made
/// `is_walled_in` answer `true` for everybody would leave every assertion above
/// passing.
#[test]
fn with_nothing_recorded_every_bot_is_still_sized_a_share() {
    let world = fixture_world();
    for (id, at_) in [
        (1u8, ELSEWHERE),
        (2, FROZEN_BOT_2),
        (3, FROZEN_BOT_3),
        (4, (ELSEWHERE.0 + 2., ELSEWHERE.1)),
    ] {
        world.players.insert(
            id,
            FactorioPlayer {
                player_id: id,
                position: at(at_),
                ..Default::default()
            },
        );
    }
    let bots = roster();
    let state = PlanState::from_world(Arc::new(world), &bots);
    assert!(
        state.walled_in().is_empty(),
        "nothing wrote the enclosure ledger, so nothing may be inferred from it"
    );
    let goal = Goal::Have {
        item: "iron-ore".into(),
        count: 8,
        whose: Holder::Anyone,
    };
    let net = expand(&[goal], &state, &registry_for(&bots), BotId(1)).expect("expands");
    let owners: BTreeSet<BotId> = net.actions().filter_map(|a| owner_of(&net, a)).collect();
    assert_eq!(
        owners,
        bots.iter().copied().collect::<BTreeSet<_>>(),
        "with no evidence of anything wrong, the whole roster shares the work"
    );
}

/// **The exclusion may never empty the split.**
///
/// With every bot walled in there is no better bot to move the work to, and
/// refusing to plan is strictly worse than planning a walk that may fail: a
/// plan that dispatches and fails leaves a record, a failed walk and a recovery
/// tier; a plan that was never made leaves none of those, and the supervisor
/// closes the milestone `stuck` on the strength of a memory. Same rule the
/// scheduler's own refusal tier keeps, restated here because this one is a
/// filter and that one is a reordering.
#[test]
fn when_every_bot_is_walled_in_the_split_is_unchanged() {
    let world = fixture_world();
    let midpoint = Position::new(
        (FROZEN_BOT_2.0 + FROZEN_BOT_3.0) / 2.,
        (FROZEN_BOT_2.1 + FROZEN_BOT_3.1) / 2.,
    );
    world
        .update_chunk_entities(tree_ring(&midpoint, 3.))
        .expect("the ring loads");
    // All four inside the one ring, and all four on record as enclosed there.
    let positions = [
        (1u8, (midpoint.x() - 1., midpoint.y() - 1.)),
        (2, FROZEN_BOT_2),
        (3, FROZEN_BOT_3),
        (4, (midpoint.x() + 1., midpoint.y() + 1.)),
    ];
    for (id, at_) in positions {
        world.players.insert(
            id,
            FactorioPlayer {
                player_id: id,
                position: at(at_),
                ..Default::default()
            },
        );
        world.record_enclosure(Enclosure {
            tick: None,
            player: id,
            at: at(at_),
            pocket_tiles: 24.,
            searched_tiles: 24.,
        });
    }
    let bots = roster();
    let state = PlanState::from_world(Arc::new(world), &bots);
    assert_eq!(
        state.walled_in().len(),
        4,
        "the premise: every bot is stuck, got {:?}",
        state.walled_in()
    );
    let goal = Goal::Have {
        item: "iron-ore".into(),
        count: 8,
        whose: Holder::Anyone,
    };
    let net = expand(&[goal], &state, &registry_for(&bots), BotId(1))
        .expect("a plan is still made when nobody can walk");
    let owners: BTreeSet<BotId> = net.actions().filter_map(|a| owner_of(&net, a)).collect();
    assert_eq!(
        owners,
        bots.iter().copied().collect::<BTreeSet<_>>(),
        "with no bot better than any other, the split is what it always was"
    );
}

/// **Un-exclusion does not wait for the bot to move**, which matters because
/// being unable to move is the condition.
///
/// `FactorioWorld::enclosures` is append-only and never drained — a standing
/// fact, not an event — so the observation for bot 2 and bot 3 is still in the
/// ledger here. What is gone is the wall. `PlanState` re-runs the fill on every
/// `from_world` and the second witness disagrees, so both bots are back in the
/// split from the very next plan, without having taken a single step.
///
/// This is the guarantee that keeps a slow run from becoming a stuck one. Run
/// `run-1788449752-46541`'s bot 4 is the case it is written for: refused a path
/// from `(-51.25, 13.23)` on three consecutive plans and then, at tick 160 440,
/// simply walked away — because the world around it had changed, not because it
/// had.
#[test]
fn a_bot_whose_pocket_has_opened_is_sized_a_share_again() {
    // The same world and the same ledger rows, with no ring built.
    let world = fixture_world();
    for (id, at_) in [
        (1u8, ELSEWHERE),
        (2, FROZEN_BOT_2),
        (3, FROZEN_BOT_3),
        (4, (ELSEWHERE.0 + 2., ELSEWHERE.1)),
    ] {
        world.players.insert(
            id,
            FactorioPlayer {
                player_id: id,
                position: at(at_),
                ..Default::default()
            },
        );
    }
    for player in [2, 3] {
        world.record_enclosure(Enclosure {
            tick: None,
            player,
            at: at(if player == 2 {
                FROZEN_BOT_2
            } else {
                FROZEN_BOT_3
            }),
            pocket_tiles: 24.,
            searched_tiles: 24.,
        });
    }
    let bots = roster();
    let state = PlanState::from_world(Arc::new(world), &bots);
    assert!(
        state.walled_in().is_empty(),
        "the ledger still remembers, but the fill no longer agrees, and the \
         fresh answer is the one that counts; got {:?}",
        state.walled_in()
    );
}
