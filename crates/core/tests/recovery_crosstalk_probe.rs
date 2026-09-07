//! DIAGNOSTIC: can one fixture's standing entities satisfy another's anchor?
//!
//! `recover_anchor` trusts an anchor once **two** of a blueprint's entities
//! stand as designed at the right relative offsets. The chain scripts run
//! several of these blocks on ONE map in sequence, and the fixtures are
//! variations of each other sharing sub-layouts (chest/inserter pairs, belt
//! runs). If block B can find two of its own entities inside block A's
//! standing layout, recovery returns an anchor belonging to a different
//! block -- bypassing `search_site` and every screen it applies.
//!
//! This is pure geometry over decoded fixtures: no game, no PlanState.
use factorio_bot_core::blueprint::decode;
use std::collections::HashMap;
use std::path::PathBuf;

/// One decoded entity: prototype name, direction, and offset from the anchor.
type Placed = (String, u8, f64, f64);

/// A fixture's name and the entities it places.
type Fixture = (String, Vec<Placed>);

fn fixtures() -> Vec<Fixture> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("repo root")
        .join("scripts/rcontest.lua");
    let src = std::fs::read_to_string(path).expect("rcontest.lua");
    let mut out = Vec::new();
    for line in src.lines() {
        let Some((lhs, rest)) = line.split_once('=') else {
            continue;
        };
        let name = lhs.trim().trim_start_matches("local ").trim();
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let rest = rest.trim();
        let Some(body) = rest.strip_prefix('"') else {
            continue;
        };
        let Some(end) = body.find('"') else { continue };
        let text = &body[..end];
        if text.len() > 40
            && text.starts_with('0')
            && let Ok(bp) = decode(text)
        {
            out.push((
                name.to_string(),
                bp.entities
                    .iter()
                    .map(|e| (e.name.clone(), e.direction, e.offset.x(), e.offset.y()))
                    .collect(),
            ));
        }
    }
    out
}

/// **A block can recover an anchor belonging to a DIFFERENT block.**
///
/// Measured 2026-09-07 over the 12 fixtures in `scripts/rcontest.lua`. The
/// worst pair is `ElectricSmelter` finding **21 of its 28 entities** inside a
/// standing `FurnaceLine`; `OreToPlate` finds 14 of 24 inside `MinerLine`.
/// `recover_anchor` needs two.
///
/// This is the root cause of the stranded tile (see
/// `docs/superpowers/notes/2026-09-07-the-stranded-tile-was-siting-all-along.md`,
/// which named siting as the defect and was half right). `resolve_site` calls
/// `recover_anchor` FIRST and unconditionally, so a hit there means
/// `search_site` never runs -- no ring scan, no `first_obstruction`, no
/// per-drill ore check. The block is then planned on top of another block's
/// ground, where a mining drill lands wherever the other block's geometry puts
/// it, which is not on ore.
///
/// **Raising the vote threshold does not fix this.** 21 of 28 is 75% of the
/// blueprint; any threshold loose enough to recover a genuinely half-built
/// block is loose enough to accept this. Recovery by geometry is ambiguous
/// whenever two blocks share a sub-layout, and these fixtures are variations
/// of one another by construction. The fix is to persist the anchor with the
/// goal instead of re-deriving it from the ground -- a change to `Goal::Built`
/// semantics, deliberately not made in passing.
///
/// The mitigation that IS in place: `PlannerError::BlockDrillUnfed` screens
/// recovered anchors for ore, so a hijacked anchor is refused by name instead
/// of stranding the block behind a refusal that blames terrain.
///
/// This test pins the hazard so it stays visible. **If it starts failing
/// because the crosstalk is gone, that is good news** -- update it rather
/// than restoring the old behaviour.
#[test]
fn report_cross_fixture_recovery_crosstalk() {
    let all = fixtures();
    eprintln!("decoded {} fixtures", all.len());
    let key = |x: f64, y: f64| ((x * 2.0).round() as i64, (y * 2.0).round() as i64);

    let mut worst = 0usize;
    for (a_name, a) in &all {
        // Block A stands at the world origin.
        let standing: HashMap<(i64, i64), (String, u8)> = a
            .iter()
            .map(|(n, d, x, y)| (key(*x, *y), (n.clone(), *d)))
            .collect();
        for (b_name, b) in &all {
            if a_name == b_name {
                continue;
            }
            // Every anchor block B could infer from A's standing entities.
            let mut votes: HashMap<(i64, i64), usize> = HashMap::new();
            for (bn, bd, bx, by) in b {
                for (pos, (an, ad)) in &standing {
                    if an != bn || ad != bd {
                        continue;
                    }
                    let ax = pos.0 as f64 / 2.0 - bx;
                    let ay = pos.1 as f64 / 2.0 - by;
                    // How many of B's entities stand as designed from here?
                    let satisfied = b
                        .iter()
                        .filter(|(n2, d2, x2, y2)| {
                            standing
                                .get(&key(ax + x2, ay + y2))
                                .is_some_and(|(sn, sd)| sn == n2 && sd == d2)
                        })
                        .count();
                    if satisfied >= 2 {
                        votes.insert(key(ax, ay), satisfied);
                    }
                }
            }
            if let Some((_, best)) = votes.iter().max_by_key(|(_, v)| **v) {
                eprintln!(
                    "CROSSTALK: {b_name} recovers an anchor inside {a_name} \
                     with {best} of its {} entities satisfied",
                    b.len()
                );
                worst = worst.max(*best);
            }
        }
    }
    eprintln!("worst cross-fixture recovery: {worst} entities satisfied");
    assert!(
        worst >= 2,
        "the probe found no cross-fixture recovery at all, which either means \
         the hazard is fixed (good -- update this test and the note) or the \
         extraction stopped finding fixtures (bad -- it would then pass while \
         checking nothing, the exact failure rcontest_blueprints_decode.rs \
         exists to prevent)"
    );
    assert!(
        worst >= 20,
        "the worst measured pair was ElectricSmelter finding 21 of its 28 \
         entities inside FurnaceLine; got {worst}. A DROP here is progress \
         worth recording, not a test to silence"
    );
}
