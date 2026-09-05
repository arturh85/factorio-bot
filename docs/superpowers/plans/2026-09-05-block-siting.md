# Block Siting Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `Goal::Built` chooses its own anchor — on clear ground, and on ore when the block contains mining drills — and recovers that anchor on every replan instead of recomputing it.

**Architecture:** A new `Site` enum replaces the bare `anchor: Position` on `Goal::Built`. Resolution happens once per expansion in `method::blueprint`, in a fixed order: recover the anchor from entities of this block already standing; failing that, search rings outward from a seed for a footprint that is clear and (if the block has drills) covers ore. Everything reuses the predicates that already exist in `PlanState` — `placement_occupant` for clearance, `covers_resource` for ore — so the acceptance test and the refusal test are literally the same code.

**Tech Stack:** Rust 2024, `crates/planner` (pure, deterministic, no I/O, no clock, ordered collections, floats via `total_cmp`), `crates/scripting_lua` for the `goal.built` binding, `mlua` 5.4.

**Spec:** `docs/superpowers/specs/2026-09-05-block-siting-design.md`

## Global Constraints

- **`crates/planner` is pure and deterministic.** No I/O, no async, no wall-clock, no RNG. Ordered collections only (`BTreeMap`/`BTreeSet`, never `HashMap`). Compare floats with `total_cmp`, never `<`/`partial_cmp().unwrap()`.
- **Every cargo command needs `nix develop -c`.** A bare `cargo` dies in `mlua-sys` with a `pkg-config` error that reads like a missing system package and is not one.
- **Never `cargo fmt --all` or `cargo fmt -p`.** Use `rustfmt --edition 2024 <file>`. The edition flag is not optional — bare `rustfmt` defaults to Rust 2015 and fails on every `async fn`.
- **Commit with explicit paths**: `git commit -m "..." -- <paths>`. Never `git add -A`, never `git commit --amend`, never `git stash`, never `git reset --hard`. Other agents write in this repo.
- **Resource positions are tile centres.** Every real resource entity sits at `(-40.5, -48.5)`, never `(-41, -49)`. `EntityGraph` keys resources by `Pos(i32,i32)`, which floors. **A test that builds ore at integer positions proves nothing** — that is the one input for which the lossy round-trip is lossless, and it hid a bug that made mining fail for every ore on every map while every test passed.
- **A pipeline reports the LAST command's exit code.** Never read `cargo test | grep` as the suite's status. Redirect to a file and test the exit code.
- Run the full check with `nix develop -c cargo test --workspace` and `nix develop -c cargo clippy --workspace --all-features --all-targets -- --deny warnings`.

---

### Task 1: `Site` replaces the bare anchor

Introduce the goal shape before any siting logic, so every later task has a stable type to build against. Behaviour is unchanged in this task: `Site::At` does exactly what `anchor` did.

**Files:**
- Modify: `crates/planner/src/goal.rs` (the `Built` variant ~line 168, and its `Display` arm ~line 198)
- Modify: `crates/planner/src/method/blueprint.rs` (the destructuring at the top of `expand`, ~line 253)
- Modify: `crates/planner/src/method/have.rs` (the `Goal::Built { .. } => None` arm in `holds()`)
- Modify: `crates/scripting_lua/src/globals/goal/value.rs` (the exhaustive match over `Goal`)
- Test: `crates/planner/src/goal.rs` (inline `#[cfg(test)]` module)

**Interfaces:**
- Produces: `pub enum Site { At(Position), Near(Position), Anywhere }` in `crates/planner/src/goal.rs`, and `Goal::Built { blueprint: String, site: Site }`.

- [ ] **Step 1: Write the failing test**

In the `#[cfg(test)]` module at the bottom of `crates/planner/src/goal.rs`:

```rust
#[test]
fn a_built_goal_displays_its_siting_mode() {
    let at = Goal::Built {
        blueprint: "0eJyrVkrKz1cCoxQlK6VEJR2lYqVYHQVjIz0DPQMDPUM9IwMlHaVSJStDPQNTMDbUM9AzMlXSUcpMUbIy0jMwBWMDsFCsDgBnexPQ".to_string(),
        site: Site::At(Position::new(3.0, 4.0)),
    };
    assert!(format!("{at}").contains("at [3, 4]"));

    let near = Goal::Built {
        blueprint: "x".to_string(),
        site: Site::Near(Position::new(-8.0, 2.0)),
    };
    assert!(format!("{near}").contains("near [-8, 2]"));

    let anywhere = Goal::Built {
        blueprint: "x".to_string(),
        site: Site::Anywhere,
    };
    assert!(format!("{anywhere}").contains("anywhere"));
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `nix develop -c cargo test -p factorio-bot-planner a_built_goal_displays_its_siting_mode`
Expected: FAIL to compile — `Site` does not exist and `Built` has no `site` field.

- [ ] **Step 3: Add the type and change the variant**

In `crates/planner/src/goal.rs`, above the `Goal` enum:

```rust
/// Where a block goes.
///
/// `Goal::Built` used to carry a bare `anchor: Position`, which made siting
/// the caller's problem: a 37-entity `MinerLine` was attempted at three
/// anchors and never got past planning, because one obstructed tile anywhere
/// along its 21-tile belt run makes the whole block infeasible. On real
/// terrain that is the normal case.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Site {
    /// This exact anchor, or refuse. The pre-siting behaviour, kept because a
    /// caller that has already chosen must still be able to say so — and
    /// because every existing test and script says it.
    At(Position),
    /// Search outward from here.
    Near(Position),
    /// Search outward from the roster's centroid.
    Anywhere,
}
```

Change the variant to:

```rust
    Built {
        /// The blueprint string, decoded on each expansion.
        blueprint: String,
        /// Where the block goes: a fixed anchor, a hint, or nothing.
        site: Site,
    },
```

And its `Display` arm:

```rust
            Goal::Built { blueprint, site } => {
                let where_ = match site {
                    Site::At(p) => format!("at {p}"),
                    Site::Near(p) => format!("near {p}"),
                    Site::Anywhere => "anywhere".to_string(),
                };
                write!(f, "build {}-byte block {}", blueprint.len(), where_)
            }
```

- [ ] **Step 4: Repair the three call sites**

In `crates/planner/src/method/blueprint.rs`, replace the destructuring at the top of `expand` with:

```rust
        let Goal::Built { blueprint, site } = goal else {
            return Ok(Vec::new());
        };
        // Task 2 replaces this with resolution. Behaviour is unchanged for
        // now: only an explicit anchor is honoured.
        let anchor = match site {
            Site::At(p) => p.clone(),
            Site::Near(_) | Site::Anywhere => {
                return Err(PlannerError::BlueprintRefused {
                    reason: "siting is not implemented yet; pass an explicit anchor".to_string(),
                });
            }
        };
```

In `crates/planner/src/method/have.rs` and `crates/scripting_lua/src/globals/goal/value.rs`, the arms already use `Goal::Built { .. }` — confirm they compile untouched, and fix them to `{ .. }` form if they name `anchor`.

- [ ] **Step 5: Run the test and the crate suite**

Run: `nix develop -c cargo test -p factorio-bot-planner 2>&1 | tail -20`
Expected: the new test PASSes. Existing `blueprint.rs` tests that construct `Goal::Built { blueprint, anchor }` will fail to compile — update each to `site: Site::At(...)`. That is a mechanical rename across the test module.

- [ ] **Step 6: Commit**

```bash
rustfmt --edition 2024 crates/planner/src/goal.rs crates/planner/src/method/blueprint.rs
nix develop -c cargo clippy -p factorio-bot-planner --all-targets -- --deny warnings
git commit -m "refactor(planner): a block's goal says where it goes, not just where it is

Site::At is exactly the old anchor. Near and Anywhere are the shapes siting
will fill in; they refuse by name until it does, rather than silently
defaulting to the origin." -- crates/planner/src/goal.rs crates/planner/src/method/blueprint.rs crates/planner/src/method/have.rs crates/scripting_lua/src/globals/goal/value.rs
```

---

### Task 2: Recover the anchor from what already stands

**This is the task the whole design exists for.** Do it before the search, so that no version of this branch can ever site a partially-built block twice.

**Files:**
- Modify: `crates/planner/src/method/blueprint.rs` (new `fn recover_anchor`, called from `expand`)
- Test: `crates/planner/src/method/blueprint.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Consumes: `Site` from Task 1; the existing `already_stands(state, e, world) -> Standing` and `Standing::{Nothing, AsDesigned, Differently}`.
- Produces: `fn recover_anchor(state: &PlanState, bp: &Blueprint) -> Option<Position>`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn a_partly_built_block_recovers_its_own_anchor_and_does_not_move() {
    // Two furnaces of a four-furnace block, standing at an anchor the caller
    // never names again. A replan must find them, not choose somewhere new.
    let bp = Blueprint {
        entities: vec![
            at_named(0.0, 0.0, "stone-furnace"),
            at_named(3.0, 0.0, "stone-furnace"),
            at_named(6.0, 0.0, "stone-furnace"),
            at_named(9.0, 0.0, "stone-furnace"),
        ],
    };
    let mut state = test_state();
    // The block was sited at (20.5, 20.5) on a previous plan and two of its
    // furnaces got built before the replan.
    state.create_entity(stone_furnace_at(20.5, 20.5));
    state.create_entity(stone_furnace_at(23.5, 20.5));

    let recovered = recover_anchor(&state, &bp).expect("two standing furnaces imply an anchor");
    assert_eq!(Pos::from(&recovered), Pos::from(&Position::new(20.5, 20.5)));
}

#[test]
fn a_block_with_nothing_standing_recovers_no_anchor() {
    let bp = Blueprint {
        entities: vec![at_named(0.0, 0.0, "stone-furnace")],
    };
    let state = test_state();
    assert!(recover_anchor(&state, &bp).is_none());
}

#[test]
fn the_anchor_satisfying_the_most_entities_wins() {
    // A decoy furnace unrelated to the block stands alone; the block's own
    // two furnaces stand together. The pair must outvote the single.
    let bp = Blueprint {
        entities: vec![
            at_named(0.0, 0.0, "stone-furnace"),
            at_named(3.0, 0.0, "stone-furnace"),
        ],
    };
    let mut state = test_state();
    state.create_entity(stone_furnace_at(-40.5, -40.5)); // decoy
    state.create_entity(stone_furnace_at(10.5, 10.5));
    state.create_entity(stone_furnace_at(13.5, 10.5));

    let recovered = recover_anchor(&state, &bp).expect("the pair implies an anchor");
    assert_eq!(Pos::from(&recovered), Pos::from(&Position::new(10.5, 10.5)));
}
```

Add these helpers to the test module if not already present:

```rust
fn at_named(x: f64, y: f64, name: &str) -> BlueprintEntity {
    BlueprintEntity {
        name: name.to_string(),
        offset: Position::new(x, y),
        direction: 0,
        underground_half: None,
    }
}

fn stone_furnace_at(x: f64, y: f64) -> FactorioEntity {
    FactorioEntity::new_stone_furnace(&Position::new(x, y), Direction::North)
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `nix develop -c cargo test -p factorio-bot-planner recovers_its_own_anchor`
Expected: FAIL to compile — `recover_anchor` does not exist.

- [ ] **Step 3: Implement recovery**

```rust
/// The anchor this block is ALREADY sited at, read back off the ground.
///
/// Siting must not be recomputed on a replan. `Goal::Built`'s whole shape
/// assumes an anchor is stable — "building it twice is a no-op rather than a
/// second factory" — and a search that re-runs against a world we have since
/// built into can answer differently than it did last time. A block
/// half-built at site A would then restart at site B: two half-factories, no
/// error, and a production curve that still rises.
///
/// So the site is chosen exactly once, when the first entity goes down, and
/// every later expansion rediscovers it from the entities themselves. This
/// needs no new state and nothing to keep in sync, because `already_stands`
/// answers the question backwards: each standing entity that matches a
/// blueprint entity implies `standing.position - blueprint.offset`.
///
/// Scored by how many of the block's entities that candidate satisfies, so an
/// unrelated entity of the same name cannot outvote the block itself. **The
/// honest limit**: a one-entity blueprint whose single entity happens to match
/// something unrelated recovers a wrong anchor. Scoring cannot distinguish
/// them, and a one-entity block is not a case this claims to handle.
fn recover_anchor(state: &PlanState, bp: &Blueprint) -> Option<Position> {
    let mut votes: BTreeMap<Pos, usize> = BTreeMap::new();
    for e in &bp.entities {
        for other in &bp.entities {
            // Candidate: `e` is standing where `other`'s offset would put it.
            let candidate = Position::new(
                e.offset.x() - other.offset.x(),
                e.offset.y() - other.offset.y(),
            );
            let _ = candidate;
        }
    }
    // Collect every standing entity that shares a name with some blueprint
    // entity, and let it imply an anchor for each matching offset.
    for e in &bp.entities {
        for candidate in state.entities_named_within(&e.name) {
            let anchor = Position::new(
                candidate.position.x() - e.offset.x(),
                candidate.position.y() - e.offset.y(),
            );
            let satisfied = bp
                .entities
                .iter()
                .filter(|b| {
                    matches!(
                        already_stands(state, b, &anchor.add(&b.offset)),
                        Standing::AsDesigned
                    )
                })
                .count();
            if satisfied > 0 {
                votes.insert(Pos::from(&anchor), satisfied);
            }
        }
    }
    votes
        .into_iter()
        // Most entities satisfied wins; `Pos`'s own ordering breaks ties, so
        // the answer does not depend on iteration order.
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
        .map(|(pos, _)| Position::new(pos.0 as f64 + 0.5, pos.1 as f64 + 0.5))
}
```

Delete the dead first loop before committing — it is shown above only to mark that the pairwise form was considered and rejected as O(n²) in the blueprint with no gain.

You need a way to list standing entities by name. If `PlanState` has no such method, add:

```rust
    /// Every entity of this name in the overlay and the base world.
    ///
    /// Used by block siting to run `already_stands` backwards. Returns them in
    /// a deterministic order — the planner is pure and an iteration order that
    /// varies would make a plan vary.
    pub fn entities_named(&self, name: &str) -> Vec<FactorioEntity> {
```

Implement it over the same structures `entities_within` reads, filtered by name, sorted by `Pos::from(&e.position)`.

- [ ] **Step 4: Run the tests**

Run: `nix develop -c cargo test -p factorio-bot-planner recover 2>&1 | tail -20`
Expected: all three PASS.

- [ ] **Step 5: Commit**

```bash
rustfmt --edition 2024 crates/planner/src/method/blueprint.rs crates/planner/src/state.rs
git commit -m "feat(planner): a block recovers its anchor instead of recomputing it

already_stands run backwards: each standing entity matching a blueprint entity
implies an anchor, scored by how many of the block's entities it satisfies.
Needs no new state, because the ground is the state." -- crates/planner/src/method/blueprint.rs crates/planner/src/state.rs
```

---

### Task 3: The ring search, on clear ground

**Files:**
- Modify: `crates/planner/src/method/blueprint.rs` (new `fn search_site`)
- Modify: `crates/planner/src/error.rs` (new `PlannerError::NoSiteFound`)
- Test: `crates/planner/src/method/blueprint.rs`

**Interfaces:**
- Consumes: `recover_anchor` (Task 2), `PlanState::placement_occupant(name, position, facing) -> Option<String>`.
- Produces: `fn search_site(state: &PlanState, bp: &Blueprint, seed: &Position, max_radius: i32) -> Result<Position, PlannerError>`, and `PlannerError::NoSiteFound { entities: usize, seed: String, searched: i32, nearest_obstruction: String }`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn a_block_is_sited_on_the_first_clear_ring_and_is_deterministic() {
    let bp = Blueprint {
        entities: vec![at_named(0.0, 0.0, "stone-furnace")],
    };
    let mut state = test_state();
    // Block the seed tile itself, so the search must step outward.
    state.create_entity(stone_furnace_at(0.5, 0.5));

    let first = search_site(&state, &bp, &Position::new(0.5, 0.5), 20).expect("open ground exists");
    let again = search_site(&state, &bp, &Position::new(0.5, 0.5), 20).expect("same answer");
    assert_eq!(Pos::from(&first), Pos::from(&again), "siting must be deterministic");
    assert_ne!(Pos::from(&first), Pos::from(&Position::new(0.5, 0.5)));
}

#[test]
fn a_search_that_finds_nothing_says_how_far_it_looked() {
    let bp = Blueprint {
        entities: vec![at_named(0.0, 0.0, "stone-furnace")],
    };
    let mut state = test_state();
    // Wall off every tile within the search bound.
    for x in -3..=3 {
        for y in -3..=3 {
            state.create_entity(stone_furnace_at(x as f64 + 0.5, y as f64 + 0.5));
        }
    }
    let err = search_site(&state, &bp, &Position::new(0.5, 0.5), 2).unwrap_err();
    let text = format!("{err}");
    assert!(text.contains('2'), "the refusal must say how far it searched: {text}");
    assert!(
        text.contains("stone-furnace"),
        "the refusal must name what is in the way: {text}"
    );
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `nix develop -c cargo test -p factorio-bot-planner sited_on_the_first_clear_ring`
Expected: FAIL to compile — `search_site` does not exist.

- [ ] **Step 3: Add the error variant**

In `crates/planner/src/error.rs`:

```rust
    /// Siting searched out to its bound and every candidate footprint was
    /// occupied.
    ///
    /// Distinct from [`PlannerError::BlockGroundOccupied`], which is about one
    /// named anchor the CALLER chose. This one is about the planner's own
    /// search, so it carries how far it looked — without that, "cannot site"
    /// is indistinguishable from "looked one tile".
    #[error(
        "no clear site for a {entities}-entity block within {searched} tiles of {seed}; \
         nearest obstruction: {nearest_obstruction}"
    )]
    NoSiteFound {
        entities: usize,
        seed: String,
        searched: i32,
        nearest_obstruction: String,
    },
```

- [ ] **Step 4: Implement the search**

```rust
/// Rings outward from `seed`, first clear footprint wins.
///
/// Deterministic by construction: rings ascend, and within a ring tiles are
/// visited in `(x, y)` order. No RNG, no float comparison, no hash iteration —
/// the planner is pure, and a site that varied between two plans of the same
/// world would make every offline comparison meaningless.
///
/// The candidate test is `placement_occupant`, which is the SAME predicate
/// `expand` already uses to refuse an anchor. Two predicates meant to agree,
/// written twice, eventually disagree — and here a disagreement would site a
/// block on ground the very next check refuses.
fn search_site(
    state: &PlanState,
    bp: &Blueprint,
    seed: &Position,
    max_radius: i32,
) -> Result<Position, PlannerError> {
    let mut nearest = String::from("nothing (the search bound was reached first)");
    for radius in 0..=max_radius {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                // Ring, not disc: skip what an inner radius already tried.
                if dx.abs() != radius && dy.abs() != radius {
                    continue;
                }
                let anchor = Position::new(seed.x() + dx as f64, seed.y() + dy as f64);
                match first_obstruction(state, bp, &anchor) {
                    None => return Ok(anchor),
                    Some(what) => {
                        if radius == 0 || nearest.starts_with("nothing") {
                            nearest = what;
                        }
                    }
                }
            }
        }
    }
    Err(PlannerError::NoSiteFound {
        entities: bp.entities.len(),
        seed: format!("{seed}"),
        searched: max_radius,
        nearest_obstruction: nearest,
    })
}

/// The first thing standing in this block's way at `anchor`, if any.
///
/// An entity already standing AS DESIGNED is not an obstruction — it is this
/// block, already partly built, which is exactly the case `recover_anchor`
/// hands here.
fn first_obstruction(state: &PlanState, bp: &Blueprint, anchor: &Position) -> Option<String> {
    for e in &bp.entities {
        let world = anchor.add(&e.offset);
        if matches!(already_stands(state, e, &world), Standing::AsDesigned) {
            continue;
        }
        let facing = Direction::from_u8(e.direction).unwrap_or(Direction::North);
        if let Some(occupant) = state.placement_occupant(&e.name, &world, facing) {
            return Some(occupant.to_string());
        }
    }
    None
}
```

- [ ] **Step 5: Run the tests**

Run: `nix develop -c cargo test -p factorio-bot-planner search_site 2>&1 | tail -20`
Expected: both PASS.

- [ ] **Step 6: Commit**

```bash
rustfmt --edition 2024 crates/planner/src/method/blueprint.rs crates/planner/src/error.rs
git commit -m "feat(planner): site a block by searching rings outward from a seed

The candidate test is placement_occupant -- the same predicate expand already
refuses with, not a second one written to agree with it. The refusal says how
far it looked, because 'cannot site' and 'looked one tile' are otherwise the
same message." -- crates/planner/src/method/blueprint.rs crates/planner/src/error.rs
```

---

### Task 4: Ore-aware siting

A drill on bare ground places perfectly and produces nothing. `MinerLine` is 13 `electric-mining-drill`s, so clear ground is not merely insufficient for it — it is wrong.

**Files:**
- Modify: `crates/planner/src/method/blueprint.rs` (`first_obstruction` gains an ore check; `search_site` gains ranking)
- Test: `crates/planner/src/method/blueprint.rs`

**Interfaces:**
- Consumes: `PlanState::stands_on_resources(name) -> bool`, `PlanState::collision_area_facing(...) -> Option<Rect>`, `PlanState::covers_resource(area, item) -> bool`, `PlanState::mine_products(name) -> Vec<String>`.
- Produces: `fn drills_are_fed(state: &PlanState, bp: &Blueprint, anchor: &Position) -> Option<String>`.

- [ ] **Step 1: Write the failing test**

**Build ore at HALF-TILE positions.** Integer ore positions are the one input for which `Pos`'s floor round-trip is lossless, and a test using them proves nothing about real maps.

```rust
#[test]
fn a_drill_block_is_refused_on_bare_ground_and_accepted_over_ore() {
    let bp = Blueprint {
        entities: vec![at_named(0.0, 0.0, "electric-mining-drill")],
    };

    let bare = test_state();
    assert!(
        search_site(&bare, &bp, &Position::new(0.5, 0.5), 3).is_err(),
        "a drill over no ore at all must be refused, not sited"
    );

    let mut ored = test_state();
    // Tile CENTRES, as every real resource entity is: (-40.5, -48.5), never
    // (-41, -49).
    for dx in -1..=1 {
        for dy in -1..=1 {
            ored.create_entity(iron_ore_at(2.5 + dx as f64, 2.5 + dy as f64));
        }
    }
    let sited = search_site(&ored, &bp, &Position::new(0.5, 0.5), 6)
        .expect("a drill must be sited onto the ore patch");
    let area = ored
        .collision_area_facing("electric-mining-drill", &sited, Direction::North)
        .expect("the drill has a collision box");
    assert!(
        ored.covers_resource(&area, "iron-ore"),
        "the chosen site {sited} does not cover ore"
    );
}
```

Add the helper:

```rust
fn iron_ore_at(x: f64, y: f64) -> FactorioEntity {
    FactorioEntity {
        name: "iron-ore".to_string(),
        position: Position::new(x, y),
        ..FactorioEntity::default()
    }
}
```

Check how the existing tests in this file construct resource entities and follow that pattern exactly rather than the sketch above — `test_utils::spawn_ore` exists in `crates/core` and builds ore at **integer** positions, which is precisely what this test must not do.

- [ ] **Step 2: Run it and watch it fail**

Run: `nix develop -c cargo test -p factorio-bot-planner drill_block_is_refused`
Expected: FAIL — a drill is currently sited happily on bare ground.

- [ ] **Step 3: Implement the ore constraint**

```rust
/// Does every mining drill in this block have ore under it at `anchor`?
///
/// Returns the reason it does not, or `None` when they all do.
///
/// **Conservative on purpose.** A real electric mining drill mines a 5x5 area
/// while its collision box is 3x3, and no mining radius reaches us — nothing
/// on the prototypes carries it. So this asks whether ore lies under the
/// drill's own FOOTPRINT, which can reject a site where the drill would in
/// fact reach ore just outside it. That direction is the safe one: a false
/// refusal is a site not taken, a false acceptance is a drill that places
/// perfectly and produces nothing, which is the failure this project has paid
/// for repeatedly. Recorded so the next author knows it is a floor, not a
/// measurement.
fn drills_are_fed(state: &PlanState, bp: &Blueprint, anchor: &Position) -> Option<String> {
    for e in &bp.entities {
        if !state.stands_on_resources(&e.name) {
            continue;
        }
        let world = anchor.add(&e.offset);
        let facing = Direction::from_u8(e.direction).unwrap_or(Direction::North);
        let Some(area) = state.collision_area_facing(&e.name, &world, facing) else {
            continue;
        };
        let wants = state.mine_products(&e.name);
        let fed = wants
            .iter()
            .any(|item| state.covers_resource(&area, item))
            || state.resource_category(&e.name).is_some()
                && state.covers_any_resource(&area);
        if !fed {
            return Some(format!(
                "the {} at ({}, {}) would stand on no ore it can mine",
                e.name,
                world.x(),
                world.y()
            ));
        }
    }
    None
}
```

`mine_products` answers what a *resource* yields, not what a *drill* accepts, so it is the wrong question for a drill. Use whichever of these `PlanState` actually offers — inspect `extractors_for(category)` and `resource_category` and invert the relation: a drill is fed when the area covers a resource whose category the drill extracts. If no such helper exists, add `covers_any_resource(&self, area: &Rect) -> Option<String>` to `PlanState` returning the name of the first resource covered, and use that. **Do not guess the API — read `crates/planner/src/state.rs` lines 2455-2610 and 4120-4140 first.**

Then call it from `first_obstruction`:

```rust
    if let Some(why) = drills_are_fed(state, bp, anchor) {
        return Some(why);
    }
    None
```

- [ ] **Step 4: Run the tests**

Run: `nix develop -c cargo test -p factorio-bot-planner drill 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
rustfmt --edition 2024 crates/planner/src/method/blueprint.rs crates/planner/src/state.rs
git commit -m "feat(planner): a block's drills must stand on ore, or the site is refused

Conservative by design: it asks whether ore lies under the drill's footprint,
where a real drill mines a wider area than it occupies. A false refusal costs a
site; a false acceptance is a drill that places perfectly and produces
nothing." -- crates/planner/src/method/blueprint.rs crates/planner/src/state.rs
```

---

### Task 5: Wire resolution into `expand`, and the replan-stability test

**Files:**
- Modify: `crates/planner/src/method/blueprint.rs` (`expand`'s anchor resolution)
- Test: `crates/planner/src/method/blueprint.rs`

**Interfaces:**
- Consumes: `recover_anchor`, `search_site`, `Site`.

- [ ] **Step 1: Write the failing test — the one that matters**

```rust
#[test]
fn a_replan_after_partial_construction_keeps_the_same_site() {
    let bp_string = miner_line_blueprint(); // the real fixture used elsewhere in this file
    let bp = decode(&bp_string).expect("the fixture decodes");
    let mut state = ore_world();

    let goal = Goal::Built {
        blueprint: bp_string.clone(),
        site: Site::Anywhere,
    };

    let first = resolve_site(&state, &bp, &goal_site(&goal), &roster_centroid(&state))
        .expect("a first site exists");

    // Build one entity of the block, as a real run would, then replan.
    let e = &bp.entities[0];
    let world = first.add(&e.offset);
    state.create_entity(entity_for(&state, e, &world));

    let second = resolve_site(&state, &bp, &goal_site(&goal), &roster_centroid(&state))
        .expect("a second site exists");

    assert_eq!(
        Pos::from(&first),
        Pos::from(&second),
        "a partly-built block must not be re-sited: that builds it twice, in two places"
    );
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `nix develop -c cargo test -p factorio-bot-planner keeps_the_same_site`
Expected: FAIL to compile — `resolve_site` does not exist.

- [ ] **Step 3: Implement resolution and call it from `expand`**

```rust
/// Where this block goes, resolved in one fixed order.
///
/// Recovery comes FIRST and unconditionally, even for `Site::At`: if the block
/// is already partly built, the ground outranks anything the caller says,
/// because the alternative is two half-blocks.
fn resolve_site(
    state: &PlanState,
    bp: &Blueprint,
    site: &Site,
    centroid: &Position,
) -> Result<Position, PlannerError> {
    if let Some(recovered) = recover_anchor(state, bp) {
        return Ok(recovered);
    }
    match site {
        Site::At(p) => Ok(p.clone()),
        Site::Near(p) => search_site(state, bp, p, SEARCH_RADIUS),
        Site::Anywhere => search_site(state, bp, centroid, SEARCH_RADIUS),
    }
}

/// How far siting looks before refusing, in tiles.
///
/// 48 covers the whole starting area of a fresh map without making a failed
/// search scan 10,000 candidate anchors: the cost is O(radius^2) footprint
/// scans, and each scan is O(entities).
const SEARCH_RADIUS: i32 = 48;
```

In `expand`, replace Task 1's placeholder with:

```rust
        let centroid = roster_centroid(&ctx.state);
        let anchor = resolve_site(&ctx.state, &bp, site, &centroid)?;
```

Add `roster_centroid`, computed from `state.bot_ids()` and `state.bot(id).position`, averaged, falling back to the origin for an empty roster. Use `total_cmp` nowhere here — an average needs no comparison — but keep the iteration over `bot_ids()`, which reads a `BTreeMap`'s keys and is therefore ordered.

- [ ] **Step 4: Run the whole planner suite**

Run: `nix develop -c cargo test -p factorio-bot-planner > /tmp/t.log 2>&1; echo "EXIT=$?"; tail -20 /tmp/t.log`
Expected: EXIT=0.

- [ ] **Step 5: Commit**

```bash
rustfmt --edition 2024 crates/planner/src/method/blueprint.rs
git commit -m "feat(planner): resolve a block's site, recovery first

Recovery outranks even an explicit anchor: if the block is already partly
built, the ground is the authority, because the alternative is two half-blocks
and no error." -- crates/planner/src/method/blueprint.rs
```

---

### Task 6: The Lua binding

**Files:**
- Modify: `crates/scripting_lua/src/globals/goal/mod.rs` (`goal.built`, and its `__doc_entry_built` string)
- Modify: `crates/scripting_lua/src/globals/goal/value.rs`
- Test: `crates/scripting_lua/src/globals/goal/mod.rs`

**Interfaces:**
- Consumes: `Site` from Task 1.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn goal_built_accepts_a_position_a_near_hint_and_nothing() {
    let lua = test_lua();
    // Explicit position -> Site::At
    let at: Goal = lua.load(r#"goal.built("BP", {x = 3, y = 4})"#).eval().unwrap();
    assert!(matches!(at, Goal::Built { site: Site::At(_), .. }));
    // A near hint -> Site::Near
    let near: Goal = lua
        .load(r#"goal.built("BP", {near = {x = 3, y = 4}})"#)
        .eval()
        .unwrap();
    assert!(matches!(near, Goal::Built { site: Site::Near(_), .. }));
    // Nothing -> Site::Anywhere
    let anywhere: Goal = lua.load(r#"goal.built("BP")"#).eval().unwrap();
    assert!(matches!(anywhere, Goal::Built { site: Site::Anywhere, .. }));
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `nix develop -c cargo test -p factorio-bot-scripting-lua goal_built_accepts`
Expected: FAIL — `goal.built` currently requires a position.

- [ ] **Step 3: Implement, and update the doc string**

Make the second argument optional. A table with `x`/`y` is `Site::At`; a table with a `near` key is `Site::Near`; absent is `Site::Anywhere`. Update `__doc_entry_built` to describe all three — **the generated Lua docs are built from these Rust strings** (`crates/scripting_lua/src/lua_docs.rs` writes `docs/lua/src/*.lua`, which are gitignored build artifacts; never edit those files).

- [ ] **Step 4: Run and commit**

```bash
nix develop -c cargo test -p factorio-bot-scripting-lua > /tmp/t.log 2>&1; echo "EXIT=$?"; tail -5 /tmp/t.log
rustfmt --edition 2024 crates/scripting_lua/src/globals/goal/mod.rs crates/scripting_lua/src/globals/goal/value.rs
git commit -m "feat(scripting): goal.built sites itself when you do not say where

goal.built(bp) searches from the roster; goal.built(bp, {near = p}) searches
from a hint; goal.built(bp, p) is the old explicit anchor." -- crates/scripting_lua/src/globals/goal/mod.rs crates/scripting_lua/src/globals/goal/value.rs
```

---

### Task 7: Offline proof against the real map, then live

**Files:**
- Create: `scripts/siting_check.lua`
- Modify: `docs/superpowers/notes/2026-09-05-block-siting-first-run.md` (create)

- [ ] **Step 1: Offline plan both fixtures, sited**

```bash
nix develop -c cargo build --no-default-features --features cli,lua
./target/debug/factorio-bot plan --world /home/arturh/projects/private/factorio-bot/workspace/scripts/map.json \
    --goal built:minerline --bots 1,2,3,4 --steps > /tmp/siting-miner.log 2>&1
echo "EXIT=$?"
```

If the `plan` CLI has no `built:` goal syntax, drive it from Lua via `goal.plan(goal.built(BP))` in `scripts/siting_check.lua` and run it with `--clients 0 --bots 4`, which needs no Factorio.

Record: the chosen anchor for each fixture, the action count, and the makespan. `MinerLine` never got past planning at three hand-chosen anchors — a sited plan that succeeds is the result this whole sub-project exists to produce.

- [ ] **Step 2: Determinism check**

Run the same plan twice and diff the chosen anchors. They must be identical.

- [ ] **Step 3: Headless live run — ASK FIRST**

**Coordinate before running.** The speedrun session owns the box for 1x client runs and has its own headless work; message it and wait for a yes before starting. Then:

```bash
nix develop -c ./target/debug/factorio-bot lua siting_check.lua --headless --bots 4 --game-speed 5
```

Read every entity back off the live surface and count how many stand at the right tile facing the right way. **A placement count is not evidence** — this project has twice shipped layouts that placed 100% correctly and did nothing.

- [ ] **Step 4: Write the note honestly**

State what was proven and what was not, in the shape of `2026-09-05-first-block-built.md`. In particular: entities standing is not entities working, and a sited `FurnaceLine` still carries no generator.

- [ ] **Step 5: Full suite, then land**

```bash
nix develop -c cargo test --workspace > /tmp/full.log 2>&1; echo "EXIT=$?"
nix develop -c cargo clippy --workspace --all-features --all-targets -- --deny warnings > /tmp/clippy.log 2>&1; echo "EXIT=$?"
```

Both must be 0. Then merge master in, re-run, and land per `superpowers:finishing-a-development-branch`.

---

## Self-Review

**Spec coverage:** replan stability → Task 2 + Task 5's test; ring search with bounded radius and typed refusal → Task 3; ore-awareness with tile-centre warning → Task 4; determinism → Tasks 3 and 7; offline-then-headless evidence → Task 7; `Site` shape → Tasks 1 and 6. The spec's "honest limit" on one-entity blueprints is documented in `recover_anchor`'s doc comment in Task 2.

**Known soft spot, deliberately left for the implementer:** Task 4's `drills_are_fed` sketches an API (`mine_products`, `covers_any_resource`) that may not match what `PlanState` actually offers — the step says so explicitly and directs the implementer to read `state.rs` first rather than trust the sketch. That is honest about what I verified (the helpers exist) versus what I did not (their exact semantics for this use).
