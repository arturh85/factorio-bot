# Factorio Bot Resurrection Plan

## Project Status Summary

**Current State (Jan 2026):**
- Factorio 2.0 compatibility recently added (inventory quality system)
- Server + client launch works on macOS via `just factorio`
- RCON communication functional
- rlua → mlua migration complete, but Lua scripts **untested** (expect issues)
- 22 tests passing, 1 clippy warning

**Comparison with Other Projects:**

| Project                                                                                       | Approach            | Multi-bot      | Factorio 2.0 |
|-----------------------------------------------------------------------------------------------|---------------------|----------------|--------------|
| [Factorio Learning Environment](https://github.com/JackHopkins/factorio-learning-environment) | LLM agents, Python  | No             | Partial      |
| [Factorio-AnyPct-TAS](https://github.com/gotyoke/Factorio-AnyPct-TAS)                         | Pre-scripted TAS    | No             | No (0.18)    |
| **Factorio Bot**                                                                              | Scripted + Planning | **Yes (8-16)** | **Yes**      |

**Unique Value:** Multi-bot coordination with task graphs - no other project does this.

---

## Primary Goals

1. **"Say launch rocket"** - High-level goal → automatic multi-bot cooperative execution
2. **TAS/Speedrun** - Beat world records with optimized coordination
3. **ML/AI Research** - Environment for future LLM/RL integration (defer for now)
4. **Personal Fun** - Playground for Factorio experiments

**Interface Priority:** CLI/REPL over GUI

---

## Phase 1: Stabilization (Get It Working)

### 1.1 Fix mlua Migration Issues
- [ ] Test existing Lua scripts, find a way to automatically start factorio server and a client, wait for that to complete, and then run the lua scripts in a way you can observe their output automatically.
- [ ] Fix any mlua API differences from rlua
- [ ] Verify all RCON functions work (rcon.move, rcon.mine, rcon.craft, etc.)
- [ ] Verify world.* functions work (world.recipe, world.player, world.inventory)
- [ ] Verify plan.* functions work (plan.mine, plan.walk, plan.place)

**Key files:**
- `crates/scripting_lua/src/globals/*.rs`
- `scripts/example.lua`, `scripts/lib.lua`

### 1.2 Validate Multi-Client Setup
- [ ] Test launching server + 2-4 clients
- [ ] Verify each client can be controlled independently via RCON
- [ ] Test basic multi-bot coordination (bot 1 mines, bot 2 smelts)

### 1.3 Fix Known Issues
- [ ] Complete `add_insert_into_inventory` FIXME in plan_builder.rs
- [ ] Fix clippy warning in `app/src-tauri/src/lib.rs:54`
- [ ] Review and commit pending changes in plan_builder.rs and planner.rs

---

## Phase 2: Core Engine (Task Graph Execution)

### 2.1 Task Graph Builder
The existing system has:
- `plan.mine()`, `plan.walk()`, `plan.place()` - create task nodes
- `plan.group_start()`, `plan.group_end()` - synchronization barriers
- `plan.task_graph_graphviz()` - visualization

Need to add:
- [ ] Task dependencies (task B waits for task A)
- [ ] Resource flow tracking (output of mining → input of smelting)
- [ ] Time estimation for scheduling

**Key files:**
- `crates/core/src/plan/plan_builder.rs`
- `crates/core/src/graph/task_graph.rs`

### 2.2 Multi-Bot Executor
Current state: Skeleton exists, core logic commented out.

Need to implement:
- [ ] Task assignment algorithm (which bot does what)
- [ ] Consider bot position, travel time, current task
- [ ] Parallel execution of independent tasks
- [ ] Wait for dependencies before starting dependent tasks
- [ ] Handle failures and re-planning

**Key files:**
- `crates/core/src/plan/planner.rs` - `execute_plan()` function

### 2.3 Bot Coordination (8-16 bots)
- [ ] Efficient task distribution
- [ ] Avoid collisions (two bots mining same tile)
- [ ] Load balancing (idle bots pick up slack)
- [ ] Visualization of bot assignments (Gantt chart via Mermaid)

---

## Phase 3: Goal Decomposition System

### 3.1 Milestone-Based Planning
Define high-level milestones:
```
launch_rocket
├── research_rocket_silo
│   ├── research_logistics_2
│   │   ├── research_automation
│   │   │   └── craft_10_red_science
│   │   └── ...
│   └── ...
└── build_rocket_silo
    ├── produce_1000_processing_units
    └── ...
```

- [ ] Define milestone graph (what requires what)
- [ ] Recipe/tech tree traversal to find requirements
- [ ] Recursive decomposition to primitive tasks

### 3.2 Resource Calculation
Given goal "research automation":
- Calculate: need 10 red science = 10 iron gear + 10 copper plate
- Calculate: need 20 iron plate + 10 copper plate
- Calculate: need 20 iron ore + 10 copper ore
- Generate mining, smelting, crafting tasks

- [ ] Recipe requirement calculator
- [ ] Inventory delta tracking (what we have vs need)
- [ ] Task generation from requirements

### 3.3 Example: "Research Automation"
```lua
-- User writes:
goal("research_automation")

-- System generates:
plan.group_start("gather resources")
plan.mine(bot1, iron_ore_pos, "iron-ore", 25)
plan.mine(bot2, copper_ore_pos, "copper-ore", 12)
plan.group_end()

plan.group_start("smelt")
plan.walk(bot1, furnace_pos, 1.0)
plan.insert(bot1, furnace_pos, "iron-ore", 25)
-- wait for smelting...
plan.group_end()

-- ... continue to crafting, inserting into lab, etc.
```

---

## Phase 4: Polish & Features

### 4.1 REPL Improvements
- [ ] Better error messages for Lua scripts
- [ ] Live reload of scripts
- [ ] History and auto-completion
- [ ] Progress visualization during execution

### 4.2 Seed Rolling (Performance Optimization)
- [ ] Resurrect `roll_best_seed.rs` (currently commented out)
- [ ] Parallel map seed evaluation
- [ ] Scoring function for seed quality

### 4.3 Documentation
- [ ] Complete user guide (docs/userguide/)
- [ ] API reference for Lua functions
- [ ] Example scripts for common scenarios

---

## Future: LLM Integration (Not Now)

When ready, could integrate with FLE approach:
- LLM generates high-level goals → our planner decomposes → multi-bot executes
- Differentiation: multi-bot coordination that FLE doesn't have
- Could use Claude API to interpret natural language → goal() calls

---

## Development Workflow

**Claude Code will automate development autonomously:**

- Run `just factorio` to test server + client start. expand justfile with more commands as needed. For example a just command to start factorio server and client and execute a single lua script would be useful. 
- Execute Lua scripts directly to verify functionality
- Run `cargo clippy`, `cargo test`, `just test` after changes
- Fix issues iteratively without asking user to manually test
- Only ask user when:
  - Major architectural decisions needed
  - Ambiguous requirements
  - Need to observe Factorio client visually

**Available commands:**
```bash
cargo repl          # REPL mode, fast testing
just factorio       # Full server + client
just test           # Clippy + tests + build
cargo clippy        # Lint only
cargo test          # Tests only
```

---

## Immediate Next Steps

1. **Figure out how to test basic Lua** - identify mlua issues
2. **Fix any blocking issues** found in step 1
3. **Write simple 2-bot coordination test** - bot1 mines, bot2 crafts
4. **Get task graph execution working** - the core differentiator

---

## Success Criteria

**MVP (Minimum Viable Product):**
- [ ] `goal("research_automation")` works with 4 bots
- [ ] Bots coordinate without collisions
- [ ] Visualization shows task assignment and progress
- [ ] Faster than single-player due to parallelism

**Full Vision:**
- [ ] `goal("launch_rocket")` completes a full game
- [ ] 8-16 bots coordinate efficiently
- [ ] Competitive with TAS world record times
- [ ] Clean API for adding new goals

---

## Is It Worth Continuing?

**Yes, because:**
1. **No multi-bot TAS exists** - your unique differentiator
2. **FLE proves demand** - academic/industry interest in Factorio AI
3. **Factorio 2.0 fresh start** - old TAS tools are obsolete
4. **Personal satisfaction** - you've invested significant effort already
5. **Architecture is solid** - recent updates show it's maintainable

**Risks:**
- Scope creep (too many features)
- Factorio updates breaking compatibility
- Time investment vs reward

**Recommendation:** Focus narrowly on Phase 1-2 first. Get multi-bot task execution working, then expand.
