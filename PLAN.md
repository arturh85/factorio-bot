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

### 1.1 Fix mlua Migration Issues ✅ COMPLETE
- [x] Test existing Lua scripts, find a way to automatically start factorio server and a client, wait for that to complete, and then run the lua scripts in a way you can observe their output automatically.
- [x] Fix any mlua API differences from rlua
- [x] Verify all RCON functions work (rcon.print, rcon.find_entities, rcon.cheat_*, etc.)
- [x] Verify world.* functions work (world.recipe, world.player, world.inventory)
- [x] Verify plan.* functions work (plan.walk, plan.group_start/end, task graph generation)

**Completed work:**
- Fixed naming convention bug in `scripts/rcontest.lua` (camelCase → snake_case)
- Fixed Factorio 2.0 inventory format incompatibility (`Vec<InventoryItemWithQuality>`)
- Created comprehensive API test suite (`scripts/api_test.lua`)
- All 4 API modules verified: global, rcon, world, plan
- Process cleanup added to CLI mode

**Key files:**
- `crates/scripting_lua/src/globals/*.rs`
- `scripts/example.lua`, `scripts/rcontest.lua`, `scripts/api_test.lua`

### 1.2 Validate Multi-Client Setup ✅ COMPLETE
- [x] Test launching server + 2-4 clients
- [x] Verify each client can be controlled independently via RCON
- [x] Test basic multi-bot coordination (bot 1 mines, bot 2 smelts)

**Completed work:**
- Created `scripts/multi_client_test.lua` to verify independent control
- Successfully tested server + 2 clients
- Verified each bot can be assigned different tasks
- Task graph generation working with multiple bots
- **Fixed critical bug**: config.ini creation was skipped when `config/` directory existed
  - Root cause: Factorio archive extraction creates empty `config/` directory
  - Fix: Changed `if !config_path.exists()` → `if !config_ini_path.exists()` in instance_setup.rs:299
  - Result: Client2 now launches successfully, both clients connect
- **Visual confirmation**: 2 Factorio icons appear in macOS Dock when running 2 clients

### 1.3 Fix Known Issues ✅ COMPLETE
- [x] ~~Client2+ failing to launch due to missing config.ini~~ - FIXED in Phase 1.2
- [x] Complete `add_insert_into_inventory` FIXME in plan_builder.rs:67
- [x] Fix clippy warning in `app/src-tauri/src/cli/lua.rs:97`
- [x] Review and commit pending changes - No uncommitted changes found
- [x] Fix script path resolution panic in CLI mode (path not found when running scripts)

**Completed work:**
- Fixed inventory state update in `add_insert_into_inventory` (plan_builder.rs:140-174)
- Renamed unused `context` parameter to `_context` (cli/lua.rs:97)
- Added path normalization to fix doubled path bug (scripting.rs:51-60)
- All three path formats now work: `api_test.lua`, `scripts/api_test.lua`, `/scripts/api_test.lua`
- ✅ All clippy warnings resolved
- ✅ All 22 tests pass

---

## Phase 2: Core Engine (Task Graph Execution)

### 2.1 Task Graph Builder ✅ COMPLETE
The existing system has:
- `plan.mine()`, `plan.walk()`, `plan.place()` - create task nodes
- `plan.group_start()`, `plan.group_end()` - synchronization barriers
- `plan.task_graph_graphviz()` - visualization

Implemented:
- [x] Task dependencies (implicit, based on resource flow)
- [x] Resource flow tracking (inputs/outputs per task)
- [x] Automatic dependency resolution
- [x] Resource flow validation
- [x] `plan.finalize()` - validates graph before execution

**Completed work:**
- Added `ResourceFlow` struct to track item production/consumption (task_graph.rs:287-292)
- Extended `TaskNode` with `inputs` and `outputs` fields (task_graph.rs:318-319)
- Updated `add_mine_node()` to populate outputs (task_graph.rs:102-111)
- Updated `add_place_node()` to populate inputs (task_graph.rs:120-129)
- Updated `add_insert_into_inventory_node()` to populate inputs (task_graph.rs:131-151)
- Implemented `resolve_dependencies()` - creates edges based on resource flow (task_graph.rs:166-213)
- Implemented `validate_resource_flow()` - ensures resources are available (task_graph.rs:215-265)
- Added `InsufficientResources` error type (errors.rs:173-182)
- Added `PlanBuilder::finalize()` method (plan_builder.rs:186-193)
- Exposed `plan.finalize()` to Lua (scripting_lua/src/globals/plan.rs:200-219)
- Created integration test script: `scripts/test_phase_2_1.lua`
- ✅ All existing tests still pass (test_simple_group, test_diverging_group)

**Design decisions:**
- Dependencies are **implicit** via resource flow (no manual `task.depends_on()` needed)
- Resource tracking is **per-player** (bot 1's resources != bot 2's resources)
- Validation runs during `finalize()`, not during task creation
- Time estimation for crafting deferred to Phase 2.2 (craft API implementation)

**Key files:**
- `crates/core/src/plan/plan_builder.rs`
- `crates/core/src/graph/task_graph.rs`
- `crates/core/src/errors.rs`
- `crates/scripting_lua/src/globals/plan.rs`

### 2.2 Multi-Bot Executor ✅ COMPLETE
Current state: All task types execute with proper status transitions and dependency checking.

Implemented:
- [x] All 6 task types execute via RCON (Mine, Walk, Craft, Place, InsertToInventory, RemoveFromInventory)
- [x] Status transitions (Planned → Running → Success/Failed)
- [x] Dependency checking (waits for incoming edges to complete)
- [x] Error handling (no `.expect()` panics, graceful failure propagation)
- [x] Parallel execution per bot (via `execute()` and `execute_single()`)
- [x] Fail-fast on errors (stops bot execution on first failure)

**Completed work:**
- Replaced all `.expect()` with proper error handling using `?` and `ok_or_else()` (execute.rs)
- Implemented 5 missing task handlers using existing RCON methods (execute.rs:52-131)
  - Walk → `move_player()`
  - Craft → `player_craft()`
  - PlaceEntity → `place_entity()`
  - InsertToInventory → `insert_to_inventory()`
  - RemoveFromInventory → `remove_from_inventory()`
- Added status transitions before/after task execution (execute.rs:38-46, 138-152)
- Implemented dependency checking via incoming edge validation (execute.rs:37-82)
  - Polls dependencies every 100ms if not ready
  - Fails if dependency task failed
  - Skips structural nodes (start, group markers)
- Updated existing test to mock `move_player()` (execute.rs:237-240)
- Created integration test script: `scripts/test_phase_2_2.lua`
- ✅ All existing tests pass (test_execution)

**Design decisions:**
- **Fail-fast approach:** Bot stops on first error, no retry/re-planning (simple for Phase 2.2)
- **Per-bot parallelism:** `execute()` spawns async `execute_single()` per bot, each walks graph sequentially
- **Polling-based dependencies:** Simple 100ms polling loop, no complex async coordination
- **No task reassignment:** Tasks pre-assigned to bots during planning, no dynamic assignment yet

**Key files:**
- `crates/core/src/plan/execute.rs` - Complete execution logic (~150 lines modified)
- `scripts/test_phase_2_2.lua` - Integration test (new file)

**Deferred to Phase 2.3:**
- Task assignment algorithm (dynamic bot selection)
- Re-planning on failure
- Task optimization (considering travel time, current task)
- Real game tick tracking (currently use placeholder 0)
- Group synchronization (explicit wait at group_end)

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
