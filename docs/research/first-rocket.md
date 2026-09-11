# First Rocket Experiment — Results

> **Status:** Runner implementation complete. Trial execution pending Factorio
> server availability. The infrastructure for isolated-process, uniquely-workspaced
> trials across the diagnostic + primary default-enemy cohort is in place.

## Experiment Design

The experiment measures the module-backed planner's ability to reach a rocket
launch milestone from a fresh seed-31337 map. Three primary trials and one
peaceful diagnostic trial are defined. Each trial uses 4 bots at game speed 10
with a target of completing the 10-stage policy (burner mining → rocket launch)
within 864,000 game ticks (24 game-hours at 10x speed).

### Competing Hypotheses

| Hypothesis | Prediction | Measure |
|---|---|---|
| H0: Default enemies prevent launch | Enemy attacks delay or destroy critical infrastructure before the rocket is built | Launch tick > 864,000 or no launch |
| H1: Enemies are manageable with defense | Sufficient turret coverage and wall placement hold the perimeter | Launch tick ≤ 864,000 with > 0 damage events |
| H2: Peaceful mode shows ceiling | Without enemy pressure, the planner's policies govern timeline | Launch tick < peaceful: ceiling with 0 damage |

### Cohort Structure

| Trial | Seed | Variant | Expected |
|---|---|---|---|
| Peaceful diagnostic | 31337 | `peaceful-diagnostic` | Baseline launch time |
| Primary enemy A | 31337 | `default-enemy` | H1 or H0 |
| Primary enemy B | 104729 | `default-enemy` | Replication |
| Primary enemy C | 130363 | `default-enemy` | Replication |

### Policy Stages

The 10-stage policy hash defines:
1. Burner mining and smelting (36,000 ticks) — 4 iron, 2 copper per minute
2. Burner expansion: iron (54,000) — 8 iron, 4 copper per minute
3. Burner expansion: copper (72,000) — 12 iron, 6 copper per minute
4. Coal power (86,400) — first steam engine
5. Automation science (108,000) — requires automation research
6. Logistics science (144,000) — requires logistic-science-pack
7. Oil processing (180,000) — requires oil-processing research
8. Rocket prerequisites — advanced-material-processing-2, rocket-fuel, low-density-structure
9. Rocket silo — requires rocket-silo research
10. Launch — requires silo built, 50 rocket parts assembled, starter pack loaded

## Reproduction

### Prerequisites

- A release build of factorio-bot
- Factorio Space Age 2.1.17 with BotBridge 0.0.1
- Seed-31337 save files generated for each trial

### Commands

```bash
# 1. Build the release binary
nix develop -c cargo build --release -p factorio-bot

# 2. Prepare the experiment (resolves fingerprints, creates save paths)
target/release/factorio-bot experiment prepare \
    --manifest experiments/first-rocket.json \
    --output workspace/first-rocket-trials/prepared.json \
    --first-rocket

# 3. Dry-run (verifies trial structure without starting Factorio)
target/release/factorio-bot experiment run \
    --manifest workspace/first-rocket-trials/prepared.json \
    --output workspace/first-rocket-trials/runs \
    --dry-run

# 4. Run the full cohort (live, with Factorio server)
target/release/factorio-bot experiment run \
    --manifest workspace/first-rocket-trials/prepared.json \
    --output workspace/first-rocket-trials/runs

# 5. Generate the report
target/release/factorio-bot experiment report \
    --manifest workspace/first-rocket-trials/prepared.json \
    --runs workspace/first-rocket-trials/runs \
    --output workspace/first-rocket-trials/report
```

## Results

### Peaceful Diagnostic

| Metric | Trial |
|---|---|
| Seed | 31337 |
| Tick limit | 864,000 |
| Wall limit | 3,600 s |
| Bots | 4 |

_Results pending execution._

### Primary Cohort

| Metric | Seed 31337 | Seed 104729 | Seed 130363 |
|---|---|---|---|
| Outcome | _pending_ | _pending_ | _pending_ |
| Terminal tick | _pending_ | _pending_ | _pending_ |
| Ticks to automation | _pending_ | _pending_ | _pending_ |
| Ticks to rocket silo | _pending_ | _pending_ | _pending_ |
| Ticks to launch | _pending_ | _pending_ | _pending_ |
| Bot deaths | _pending_ | _pending_ | _pending_ |
| Damage taken | _pending_ | _pending_ | _pending_ |
| Material cost | _pending_ | _pending_ | _pending_ |
| Planning time | _pending_ | _pending_ | _pending_ |

### Analysis

_To be filled in after execution._

## Infrastructure

### Runner Implementation

The live runner (`app/src-tauri/src/experiment/runner.rs`) supports:

- **Isolated workspaces**: Each trial gets a unique workspace directory under
  the runs output directory (`seed-{seed}-bots-{n}-{variant}-rep-{r}/`).
- **Pristine saves**: A seed-31337 save is copied into each trial workspace.
  No user workspace is modified.
- **Process monitoring**: The runner monitors both game ticks (via RCON/sampling)
  and a separate manifest-defined wall timeout. On timeout or cancellation, only
  owned child processes are shut down and final failure rows are retained.
- **Planning pause recording**: Both the pause duration and the active phase wall
  time are recorded separately in `TrialResult`.
- **Terminal evidence**: Successful trials carry paths to evidence artifacts
  (logs, screenshots, save files). Failures retain the failure records with
  diagnostic messages.
- **Fresh state**: Unless the manifest declares shared state, each trial starts
  from a fresh seed-31337 map with no warm instance or policy state.
- **Fake-process test**: `fake_process_runner_produces_one_row_per_declared_trial`
  validates that the runner produces exactly one row per declared trial, with
  proper accounting for failed startup, timeout, and missing terminal evidence.

### Report Format

The first-rocket report (`report.md`) includes:
1. Summary table (total/success/failure/timeout/invalid counts)
2. Per-variant breakdown (peaceful vs default-enemy)
3. Detailed results table (per-trial ticks, planning time, wall time, reason)
4. Terminal evidence paths for successful trials
5. Timing by seed (best tick per seed)

### Test Coverage

| Test | What it covers |
|---|---|
| `fake_process_runner_produces_one_row_per_declared_trial` | Runner produces correct rows; outcome accounting |
| `tick_duration_is_terminal_minus_initial` | `terminal_tick - initial_tick` computed correctly |
| `zero_ticks_is_invalid_zero_time` | Zero tick duration flagged as invalid |
| `missing_ticks_is_invalid_zero_time` | Missing initial/terminal ticks flagged |
| `run_experiment_stub_returns_empty` | Existing stub API preserved |
| `first_rocket_expansion_produces_peaceful_and_default_enemy` | Manifest expansion correct |
| `first_rocket_no_peaceful_produces_only_default_enemy` | Peaceful suppression works |

## Design Decisions

1. **Separate wall timeout vs game tick limit**: The game tick limit is checked
   against actual game progress. The wall timeout is a safety net to prevent
   runaway processes. A trial can exceed either bound independently.

2. **Tick duration = terminal_tick - initial_tick**: The actual wall-time
   duration is measured separately. Tick duration comes from game ticks, not
   from last-action completion time.

3. **Fresh processes per trial**: No warm Factorio instance is reused between
   trials. This prevents cache contamination but costs server startup time per
   trial (~12–17 s).

4. **Peaceful diagnostic first**: The peaceful trial runs before the primary
   cohort to validate that the policy and infrastructure work under ideal
   conditions before spending resources on default-enemy trials.

5. **Results directory structure**: Each trial's artifacts live under
   `runs/trial-{seed}-{variant}/`, with a shared `trials.csv` at the runs root
   for easy aggregation.

