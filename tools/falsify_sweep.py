#!/usr/bin/env python3
"""The runner every `falsify_*.py` sweep shares, and the guards it needs.

A mutation sweep edits source files in place and puts them back. In a
checkout with **one** writer that is safe. This repo has several, and on
2026-09-08 a sweep restored `crates/planner/src/method/fabricate.rs` -- a
file it does not own -- while another agent was mid-refactor in it. Nothing
was provably lost, and nothing could have been proved either way, which is
the problem: a whole-file byte rewrite over somebody else's uncommitted work
leaves no trace that it happened.

The same run also produced seventeen `DID NOT COMPILE` lines that meant
nothing at all. The crate did not compile *before* the first mutation, so
every verdict was about that and not about any mutation -- a sweep whose
control was never established, reporting seventeen results as though they
were evidence. `silence is not success`, wearing the falsification harness's
hat.

Three guards, each closing one of those:

1. **The control comes first.** The test command is run once, unmutated,
   before anything is edited. Not green -> the sweep refuses to start and
   says why. A compile failure attributed to a mutation is only meaningful
   against a baseline that compiled.
2. **A dirty file is never touched.** `git status --porcelain <path>` decides,
   and a skip is reported as `SKIPPED`, never folded in with the mutations
   that ran -- a mutation that never ran must not read as one that survived.
   (This also skips a file *you* have edited and not committed, which is the
   older rule -- "commit before a sweep, not after" -- enforced rather than
   remembered.)
3. **The restore is verified, and a concurrent writer aborts the sweep.**
   Before restoring, the file must still hold exactly the bytes this sweep
   wrote; if it does not, somebody else wrote to it and their version is left
   **in place** -- this backup goes to `<file>.falsify-backup` and the sweep
   stops. After restoring, the bytes are compared against the backup and a
   mismatch is fatal.

The mtime hazard is older and still handled here: `shutil.copy2` and `cp -p`
preserve mtime, so a restored file can be *older* than the artifact cargo
built from the mutation, and cargo then re-runs the MUTATED binary against
restored source -- a false red over a clean tree. Every write here touches
the file afterwards.

A sweep is a `main(...)` call:

    from falsify_sweep import main
    sys.exit(main(MUTATIONS, "cargo test -p factorio-bot-planner --lib"))
"""

import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def run(cmd):
    return subprocess.run(cmd, cwd=ROOT, shell=True, capture_output=True, text=True)


def is_dirty(path):
    """Is `path` modified, staged or untracked against HEAD?

    Any non-empty `git status --porcelain` line is a yes, including `??`: an
    untracked file is somebody's work in progress by definition.
    """
    r = run(f"git status --porcelain -- {path}")
    return r.stdout.strip() != ""


def write(full, text):
    """Write `text` and make the file NEWER than anything cargo built from it."""
    with open(full, "w") as handle:
        handle.write(text)
    os.utime(full, (time.time(), time.time()))


def verdict_of(result, prefix):
    """One mutation's outcome, told apart the way this repo learned to.

    **A compile failure is recognised by the ABSENCE of a test run**, not by
    the word "error": `cargo test` prints `error: test failed, to rerun ...`
    on an ordinary red run too, and matching that string once reported six
    live mutations as uncompilable -- the "reads as green" failure arriving
    through the detector instead of through the compiler.
    """
    out = result.stdout + result.stderr
    if "test result:" not in out:
        # **Say WHY.** A bare "did not compile" is unactionable and, in a
        # shared checkout, is as likely to be a torn incremental object
        # (`mold: error: undefined symbol: anon.<hash>`) as a mutation that
        # does not typecheck. The first `error:` line tells those apart.
        first = next(
            (line.strip() for line in out.splitlines() if line.strip().startswith("error")),
            "no error line either -- the run produced no test result at all",
        )
        return f"DID NOT COMPILE -- reads as green, treat as no evidence: {first}"
    if result.returncode == 0:
        return "GREEN -- A FINDING: no test objects to this"
    failed = [
        line.strip()
        for line in out.splitlines()
        if line.strip().startswith(f"test {prefix}") and "FAILED" in line
    ]
    return "red (%d): %s" % (
        len(failed),
        ", ".join(f.split()[1] for f in failed[:4]),
    )


def main(mutations, test_cmd, prefix=""):
    # -- guard 1: the control ------------------------------------------------
    control = run(f"{test_cmd} 2>&1")
    if "test result:" not in (control.stdout + control.stderr):
        print(
            "\n  REFUSING TO SWEEP: the baseline does not COMPILE.\n"
            "      Every mutation would report `DID NOT COMPILE` about this,\n"
            "      not about itself. Fix the tree (or sweep in a clean\n"
            f"      worktree) and run again.\n      command: {test_cmd}"
        )
        return 2
    if control.returncode != 0:
        print(
            "\n  REFUSING TO SWEEP: the baseline is RED.\n"
            "      A mutation cannot be shown to break a suite that is already\n"
            f"      broken.\n      command: {test_cmd}"
        )
        return 2

    results = []
    for label, path, old, new in mutations:
        full = os.path.join(ROOT, path)
        # -- guard 2: never write over uncommitted work ----------------------
        if is_dirty(path):
            results.append(
                (
                    label,
                    f"SKIPPED -- {path} is dirty against HEAD; a restore here "
                    "would rewrite somebody's uncommitted work",
                )
            )
            continue
        original = open(full).read()
        n = original.count(old)
        if n != 1:
            results.append((label, f"SUBSTITUTION MATCHED {n} TIMES -- not run"))
            continue
        mutated = original.replace(old, new)
        write(full, mutated)
        result = run(f"{test_cmd} 2>&1")
        # -- guard 3: verify the restore -------------------------------------
        if open(full).read() != mutated:
            write(full + ".falsify-backup", original)
            print(
                f"\n  ABORTING: {path} changed underneath this sweep.\n"
                "      Somebody else wrote to it while the mutation was being\n"
                "      tested. THEIR version is left in place; the pre-mutation\n"
                f"      content is saved as {path}.falsify-backup.\n"
            )
            results.append((label, "ABORTED -- the file changed underneath the sweep"))
            break
        write(full, original)
        if open(full).read() != original:
            print(f"\n  ABORTING: {path} did not restore byte-for-byte.\n")
            results.append((label, "ABORTED -- restore did not verify"))
            break
        results.append((label, verdict_of(result, prefix)))

    print()
    for label, verdict in results:
        print(f"  {label}\n      {verdict}")
    return 0


if __name__ == "__main__":
    print(__doc__)
    sys.exit(0)
