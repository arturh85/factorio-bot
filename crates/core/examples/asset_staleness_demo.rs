//! Manual demonstration for `crates/core/src/process/asset_sync.rs`.
//!
//! Not part of `cargo test`: it is meant to be built and run twice, with a
//! real edit to a tracked file under `mods/` or `scripts/` in between, to
//! show the staleness warning actually firing against this repo's real
//! embedded content (not a synthetic fixture) and then clearing after a
//! refresh. See `.superpowers/sdd/workspace-staleness.md` for the transcript
//! this produced.
//!
//! Usage (from the repo root):
//!   cargo build --release -p factorio-bot-core --example asset_staleness_demo
//!   ./target/release/examples/asset_staleness_demo --step 1   # extract, simulating a prior run
//!   # ... edit mods/BotBridge/types.lua or scripts/example.lua, rebuild ...
//!   ./target/release/examples/asset_staleness_demo --step 2   # shows the warning
//!   FACTORIO_BOT_REFRESH_MODS=1 FACTORIO_BOT_REFRESH_SCRIPTS=1 \
//!     ./target/release/examples/asset_staleness_demo --step 2 # shows it clear
//!
//! Only compiled in a release build: `MODS_CONTENT`/`PLANS_CONTENT` do not
//! exist under `cfg(debug_assertions)`.
#[cfg(not(debug_assertions))]
fn main() {
    use factorio_bot_core::paths::resolve_workspace;
    use factorio_bot_core::process::asset_sync::{refresh_if_requested, warn_if_stale};
    use factorio_bot_core::process::instance_setup::{
        MODS_CONTENT, PLANS_CONTENT, REFRESH_MODS_ENV, REFRESH_PLANS_ENV,
    };
    use factorio_bot_core::scripts::ensure_scripts_dir;

    // Fixed, not process-id-based: the whole point is that step 2 reuses the
    // directory step 1 populated, across a separate process invocation (and,
    // in the real demonstration, a rebuild in between).
    let workspace = std::env::temp_dir().join("factorio-bot-asset-staleness-demo");
    let mods_path = workspace.join("mods");
    let plans_path = workspace.join("plans");
    // `ensure_scripts_dir` derives the path itself as `<given>/scripts`, so it
    // gets its own workspace root rather than sharing one with mods/plans.
    let scripts_workspace = workspace.join("live_scripts_workspace");
    // `ensure_scripts_dir` demands a resolved workspace, same as every real
    // caller -- this path is always absolute (built from `temp_dir()`), so
    // resolution never fails here.
    let scripts_workspace = resolve_workspace(&scripts_workspace.to_string_lossy())
        .expect("temp_dir()-based workspace is absolute");

    let step = std::env::args().any(|a| a == "--step=1" || a == "1");

    if step {
        let _ = std::fs::remove_dir_all(&workspace);
        std::fs::create_dir_all(&mods_path).expect("create mods dir");
        std::fs::create_dir_all(&plans_path).expect("create plans dir");
        std::fs::create_dir_all(scripts_workspace.as_path()).expect("create scripts workspace");
        MODS_CONTENT.extract(&mods_path).expect("extract mods");
        PLANS_CONTENT.extract(&plans_path).expect("extract plans");
        // Exercises the actual production entry point (`crates/core/src/scripts.rs`),
        // not `asset_sync` directly, so the once-per-process guard and the wiring
        // are covered too, not just the shared comparison logic.
        ensure_scripts_dir(&scripts_workspace).expect("ensure_scripts_dir");
        println!(
            "[step 1] extracted the embedded mods/, plans/ and scripts/ snapshots into {:?}, \
       simulating a workspace populated by a prior run.",
            workspace
        );
        println!("[step 1] checking immediately -- must be silent (nothing extracted, nothing edited yet):");
        warn_if_stale(&MODS_CONTENT, &mods_path, "mods", REFRESH_MODS_ENV);
        warn_if_stale(&PLANS_CONTENT, &plans_path, "plans", REFRESH_PLANS_ENV);
        println!("[step 1] done. Now edit a tracked file under mods/ or scripts/, rebuild this example, and run with --step=2.");
        return;
    }

    println!(
        "[step 2] checking {:?} against the embed baked into THIS binary:",
        workspace
    );
    let mods_refreshed =
        refresh_if_requested(&MODS_CONTENT, &mods_path, REFRESH_MODS_ENV).expect("refresh mods");
    if mods_refreshed {
        println!(
            "[step 2] {}=1 was set: refreshed the mods workspace copy.",
            REFRESH_MODS_ENV
        );
    }
    warn_if_stale(&MODS_CONTENT, &mods_path, "mods", REFRESH_MODS_ENV);

    let plans_refreshed = refresh_if_requested(&PLANS_CONTENT, &plans_path, REFRESH_PLANS_ENV)
        .expect("refresh plans");
    if plans_refreshed {
        println!(
            "[step 2] {}=1 was set: refreshed the plans workspace copy.",
            REFRESH_PLANS_ENV
        );
    }
    warn_if_stale(&PLANS_CONTENT, &plans_path, "plans", REFRESH_PLANS_ENV);

    println!("[step 2] now the same check through the real ensure_scripts_dir() entry point, called twice:");
    ensure_scripts_dir(&scripts_workspace).expect("ensure_scripts_dir first call");
    println!("[step 2] second call in the same process -- must be silent even if still stale (once-per-process guard):");
    ensure_scripts_dir(&scripts_workspace).expect("ensure_scripts_dir second call");

    println!("[step 2] done. No warning above a cache's line means it is up to date.");
}

#[cfg(debug_assertions)]
fn main() {
    eprintln!(
    "this demo only makes sense against MODS_CONTENT/PLANS_CONTENT, which only exist in a release build; run with --release"
  );
    std::process::exit(1);
}
