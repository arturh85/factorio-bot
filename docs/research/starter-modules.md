# Starter Module Experiment — Results

> **Status:** Placeholder. Full results will be populated after running the
> experiment matrix defined in `experiments/starter-modules.json`.

## Reproduction

```bash
# 1. Build the release binary
nix develop -c cargo build --release -p factorio-bot

# 2. Prepare the experiment (resolves fingerprints, creates saves)
target/release/factorio-bot experiment prepare \
    --manifest experiments/starter-modules.json \
    --output experiments/prepared

# 3. Run the full matrix (360 trials, ~180 hours worst-case wall time)
target/release/factorio-bot experiment run \
    --manifest experiments/prepared/manifest.json \
    --output experiments/runs

# 4. Generate the comparison report
target/release/factorio-bot experiment report \
    --manifest experiments/prepared/manifest.json \
    --runs experiments/runs \
    --output experiments/report
```

## Design

The experiment compares three planner variants on the ten-seed benchmark suite
defined in the [hierarchical factory planning spec](../specs/2026-09-10-hierarchical-factory-planning-design.md):

| Variant | Description |
|---------|-------------|
| `legacy` | Current planner (baseline) |
| `modules-cache-on` | Module-backed planner with caching |
| `modules-cache-off` | Module-backed planner without caching |

## Results

_To be filled in after execution._
