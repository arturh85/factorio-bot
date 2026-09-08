#!/usr/bin/env python3
"""Falsification sweep for the chemistry rung and the obtain-cost tie-break.

Same shape and the same two traps as `tools/falsify_ambiguity.py`, which this
is modelled on rather than reinventing:

  * a mutation that fails to COMPILE reads as green, so it is reported as its
    own outcome and never folded into "the tests failed";
  * a restore that preserves mtime makes cargo re-run the MUTATED binary
    against restored source, so every restore here rewrites the bytes and
    then touches the file.

Runs the WHOLE planner lib, not one module: the rules under test live in
`products.rs` and `method/supply.rs` and are exercised from both.

Usage: nix develop -c python3 tools/falsify_fluid_supply.py
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from falsify_sweep import main  # noqa: E402  (after sys.path)

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PRODUCTS = "crates/planner/src/products.rs"
SUPPLY = "crates/planner/src/method/supply.rs"
FABRICATE = "crates/planner/src/method/fabricate.rs"
GATHER = "crates/planner/src/method/gather.rs"
POWER = "crates/planner/src/method/power.rs"

MUTATIONS = [
    # ---- the chemistry rung -----------------------------------------------
    (
        "supply: bottle a fluid the ground already yields",
        SUPPLY,
        "    if index.ground_supplies(fluid) {\n        return false;\n    }",
        "    if false && index.ground_supplies(fluid) {\n        return false;\n    }",
    ),
    (
        "supply: derive `Produced` for any fluid, whether or not a recipe makes it",
        SUPPLY,
        "        } else if makeable(state, fluid, &recipe) {",
        "        } else if true {",
    ),
    (
        "supply: prefer a plant over the ground",
        SUPPLY,
        "        if super::gather::can_gather(state, fluid) {",
        "        if false && super::gather::can_gather(state, fluid) {",
    ),
    # ---- the water rung: a fluid the GROUND yields --------------------------
    (
        "gather: forget that a fluid can come from the tiles, not a patch",
        GATHER,
        "    ground_yields(state, entity, &Position::default())\n}",
        "    false && ground_yields(state, entity, &Position::default())\n}",
    ),
    (
        "gather: call every world a source, lake or no lake",
        GATHER,
        "fn ground_yields(state: &PlanState, fluid: &str, origin: &Position) -> bool {\n    ground_source(state, fluid, origin).is_some()",
        "fn ground_yields(state: &PlanState, fluid: &str, origin: &Position) -> bool {\n    let _ = ground_source(state, fluid, origin);\n    true",
    ),
    (
        "gather: face the pump north, whatever side the water is on",
        GATHER,
        "    entity.direction = Direction::to_u8(&facing).unwrap_or(0);",
        "    entity.direction = 0;",
    ),
    (
        "gather: take the first shoreline tile without asking if it is clear",
        GATHER,
        "        if state.is_area_free_facing(&pump, &tile, facing) {\n            return Ok((pump, tile, facing));\n        }",
        "        if true {\n            return Ok((pump, tile, facing));\n        }",
    ),
    (
        "gather: search for water only from the actor, never the world anchor",
        GATHER,
        "    if calculate_distance(&anchor, origin) >= f64::EPSILON {\n        anchors.push(anchor);\n    }",
        "    if false && calculate_distance(&anchor, origin) >= f64::EPSILON {\n        anchors.push(anchor);\n    }",
    ),
    (
        "gather: read the tile by name only, dropping its own declaration",
        GATHER,
        "                tile.fluid.yields(fluid)",
        "                tile.yields_water()",
    ),
    (
        "power: offer only the ring the water tile itself is in",
        POWER,
        "    for radius in 0..=SHORE_SEARCH_RADIUS {",
        "    for radius in 0..=(SHORE_SEARCH_RADIUS - SHORE_SEARCH_RADIUS) {",
    ),
    (
        "supply: ask for one unit instead of the recipe's own amount",
        SUPPLY,
        "                amount: ingredient.amount,",
        "                amount: 1,",
    ),
    (
        "supply: let a recipe feed itself",
        SUPPLY,
        "    recipe.name != consumer.name && recipe_gate(state, recipe) != RecipeGate::Unobtainable",
        "    recipe_gate(state, recipe) != RecipeGate::Unobtainable",
    ),
    # ---- rule 3: cheapest to obtain ---------------------------------------
    (
        "rule 3: never consult obtain cost",
        PRODUCTS,
        "        let runnable = if runnable.len() > 1 {\n            self.cheapest_to_obtain(&runnable, product)",
        "        let runnable = if false && runnable.len() > 1 {\n            self.cheapest_to_obtain(&runnable, product)",
    ),
    (
        "rule 3: let a TIE win, so the preference becomes a filter",
        PRODUCTS,
        "            [(best, _, winner), (second, ..), ..] if best < second => vec![*winner],",
        "            [(_, _, winner), ..] => vec![*winner],",
    ),
    (
        "rule 3: count ingredients instead of pricing them",
        PRODUCTS,
        "                    Some(c) => inputs += u128::from(ingredient.amount) * u128::from(*c),",
        "                    Some(_) => inputs += u128::from(ingredient.amount),",
    ),
    (
        "rule 3: ignore how much a recipe yields",
        PRODUCTS,
        "                priced.push((inputs * YIELD_SCALE / amount, recipe.name.as_str(), recipe));",
        "                priced.push((inputs * YIELD_SCALE, recipe.name.as_str(), recipe));",
    ),
    # ---- siting ------------------------------------------------------------
    (
        "fabricate: site a fluid machine without checking a run can reach it",
        FABRICATE,
        "|candidate: &Position| ports_fit(candidate) && routes_from_every_source(candidate);",
        "|candidate: &Position| ports_fit(candidate);",
    ),
]


sys.exit(main(MUTATIONS, "cargo test -p factorio-bot-planner --lib", ""))
