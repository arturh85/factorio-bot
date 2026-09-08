#!/usr/bin/env python3
"""Falsify every test added for the pollution / evolution reading.

A test that passes on broken code proves nothing, and this repo has found six
such tests in a week. So each mutation below breaks exactly one thing the test
claims to check, and the sweep asserts the test goes RED. **A green mutation is
a finding, not a pass.**

Three mechanics matter, each of which has produced a false result here before:

* **Backup and restore by FILE COPY, never `git checkout -- <file>`.** That
  restores the last COMMITTED state and silently discards uncommitted work --
  it cost an agent four files on 2026-09-08.
* **`touch` the restored file.** `shutil.copy2`/`cp -p` preserve mtime, so
  cargo sees nothing to rebuild and re-runs the MUTATED binary against restored
  source: a false RED over a clean tree.
* **A mutation that fails to COMPILE reads as red for the wrong reason.** Each
  mutation here is chosen to compile, and the sweep reports the failure text so
  a compile error is visible rather than counted as a pass.

Run: ``python3 tools/pollution_falsification_sweep.py`` from the worktree root.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

SAMPLES = "crates/core/src/record/samples.rs"
CONTROL = "mods/BotBridge/control.lua"
ANALYSIS = "tools/run_analysis.py"

CARGO_CORE_LIB = [
    "nix", "develop", "-c", "cargo", "test", "-p", "factorio-bot-core",
    "--lib", "record::samples::tests::",
]
CARGO_MOD = [
    "nix", "develop", "-c", "cargo", "test", "-p", "factorio-bot-core",
    "--test", "botbridge_sampling_session",
]
PY_POLLUTION = [sys.executable, "tools/test_run_analysis_pollution.py"]


# (name, file, find, replace, command, the test that must go red)
MUTATIONS = [
    (
        "the decoder stops accepting `by_pollution`",
        SAMPLES,
        "    /// `get_evolution_factor_by_pollution(surface)`.\n    pub by_pollution: f64,",
        "    /// `get_evolution_factor_by_pollution(surface)`.\n"
        '    #[serde(rename = "by_pollution_NOPE")]\n    pub by_pollution: f64,',
        CARGO_CORE_LIB,
        "reads_pollution_and_the_four_evolution_terms",
    ),
    (
        "a missing `total` stops being allowed, so absent cannot be told from zero",
        SAMPLES,
        '    #[serde(default, skip_serializing_if = "Option::is_none")]\n    pub total: Option<f64>,',
        '    #[serde(skip_serializing_if = "Option::is_none")]\n    pub total: Option<f64>,',
        CARGO_CORE_LIB,
        "a_surface_we_failed_to_read_is_not_a_surface_with_no_pollution",
    ),
    (
        "an archived schema-2 line stops decoding without the new field",
        SAMPLES,
        "        #[serde(default)]\n        pollution: Option<PollutionSample>,",
        "        pollution: Option<PollutionSample>,",
        CARGO_CORE_LIB,
        "reads_a_force_sample_with_no_research_queued",
    ),
    (
        "the mod stops putting evolution on the wire",
        CONTROL,
        "\t\t\tif ok then entry.evolution = value end",
        "\t\t\tif false then entry.evolution = value end",
        CARGO_MOD,
        "pollution_and_the_four_evolution_terms_reach_the_force_sample",
    ),
    (
        "the surface-name fallback goes away, so a nameless surface raises again",
        CONTROL,
        "\t\tif name == nil then name = tostring(key) end",
        "\t\tif false then name = tostring(key) end",
        CARGO_MOD,
        "a_game_without_the_pollution_api_still_writes_the_rest_of_the_force_sample",
    ),
    (
        "the mod stops reading pollution at spawn",
        CONTROL,
        "\t\t\tif ok then entry.at_spawn = value end",
        "\t\t\tif false then entry.at_spawn = value end",
        CARGO_MOD,
        "pollution_and_the_four_evolution_terms_reach_the_force_sample",
    ),
    (
        "`not-captured` collapses into `ok`, so a run that never looked reads as calm",
        ANALYSIS,
        '        out["status"] = "not-captured"',
        '        out["status"] = "ok"',
        PY_POLLUTION,
        "test_a_run_that_never_looked_is_not_a_calm_run",
    ),
    (
        "the verdict stops naming the dominant cause",
        ANALYSIS,
        'cause = "pollution" if d_poll > d_time else "time"',
        'cause = "time"',
        PY_POLLUTION,
        "test_the_verdict_names_the_dominant_cause_not_just_the_rise",
    ),
    (
        "a missing reading is rendered as 0 instead of `?`",
        ANALYSIS,
        '        return "?" if v is None else format(v, fmt)',
        '        return format(0.0, fmt) if v is None else format(v, fmt)',
        PY_POLLUTION,
        "test_a_surface_we_failed_to_read_renders_as_unknown_not_zero",
    ),
    (
        "the emitter ranking is reversed, so the top emitter is no longer the top one",
        ANALYSIS,
        "                    (produced or {}).items(), key=lambda kv: -kv[1]",
        "                    (produced or {}).items(), key=lambda kv: kv[1]",
        PY_POLLUTION,
        "test_emitters_are_ranked_so_the_cause_is_nameable",
    ),
]


def run(cmd: list[str]) -> tuple[int, str]:
    p = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
    return p.returncode, p.stdout + p.stderr


def main() -> int:
    baseline_failures = []
    for cmd in (CARGO_CORE_LIB, CARGO_MOD, PY_POLLUTION):
        code, out = run(cmd)
        if code != 0:
            baseline_failures.append((cmd, out[-2000:]))
    if baseline_failures:
        print("BASELINE IS NOT GREEN -- nothing below would mean anything.")
        for cmd, out in baseline_failures:
            print(" ".join(cmd))
            print(out)
        return 2
    print(f"baseline green ({len(MUTATIONS)} mutations to try)\n")

    findings = []
    with tempfile.TemporaryDirectory() as backup_dir:
        for name, rel, find, repl, cmd, expect in MUTATIONS:
            path = os.path.join(ROOT, rel)
            backup = os.path.join(backup_dir, rel.replace("/", "_"))
            shutil.copyfile(path, backup)
            try:
                src = open(path).read()
                hits = src.count(find)
                if hits != 1:
                    findings.append(f"{name}: substitution matched {hits} times, not once")
                    print(f"  !! {name}: matched {hits} times -- SKIPPED")
                    continue
                open(path, "w").write(src.replace(find, repl, 1))
                code, out = run(cmd)
                if code == 0:
                    findings.append(f"{name}: the suite stayed GREEN -- `{expect}` does not check it")
                    print(f"  GREEN (finding) {name}")
                elif "error[E0" in out or "could not compile" in out or "SyntaxError" in out:
                    findings.append(f"{name}: did not compile/parse, so the red proves nothing")
                    print(f"  BROKEN BUILD (finding) {name}")
                elif expect not in out:
                    findings.append(
                        f"{name}: went red, but `{expect}` is not among the failures"
                    )
                    print(f"  RED but wrong test {name}")
                else:
                    print(f"  red: {name}  <- {expect}")
            finally:
                shutil.copyfile(backup, path)
                # NOT copy2: the mtime must move forward or cargo re-runs the
                # mutated binary against restored source.
                os.utime(path, None)

    # One restore check, because a sweep that corrupts the tree is worse than
    # no sweep. Everything here is committed, so `git status` must be clean.
    code, out = run(["git", "status", "--porcelain", "--", SAMPLES, CONTROL, ANALYSIS])
    if out.strip():
        findings.append(f"restore left the tree dirty:\n{out}")

    print()
    if findings:
        print("FINDINGS:")
        for f in findings:
            print(f"  - {f}")
        return 1
    print("every mutation went red in the test that claims to check it")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
