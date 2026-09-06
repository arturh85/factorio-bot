//! The lint itself, over the whole checkout.
//!
//! Run by `cargo test --workspace`, and therefore by `just test`. It is a test
//! rather than a separate tool on purpose: a tool has to be remembered, and the
//! defect this exists to catch is precisely one that nobody remembers to look
//! for. `crates/doclint/src/lib.rs` carries the rule and the reasoning.

use factorio_bot_doclint::{unresolved_citations, unused_foreign_vocabulary, workspace_root};

/// A suppression that has outlived its cause is a check that has quietly
/// stopped checking. `FOREIGN_VOCABULARY` is small and exempts real names from
/// other projects; an entry nothing cites any more must be deleted.
#[test]
fn no_foreign_vocabulary_entry_has_outlived_its_citation() {
    let root = workspace_root();
    let unused = unused_foreign_vocabulary(&root);
    assert!(
        unused.is_empty(),
        "\nFOREIGN_VOCABULARY entries nothing cites any more -- delete them from\n\
         crates/doclint/src/lib.rs: {unused:?}\n"
    );
}

#[test]
fn every_name_a_comment_cites_resolves_to_a_declaration() {
    let root = workspace_root();
    let unresolved = unresolved_citations(&root);
    if unresolved.is_empty() {
        return;
    }
    let mut message = String::new();
    message.push_str(&format!(
        "\n{} comment citation(s) name something that exists nowhere in this tree.\n\n",
        unresolved.len()
    ));
    for citation in &unresolved {
        message.push_str(&format!(
            "  {}:{}\n      cites `{}` -- no fn, const, type, module, field, Lua function or\n      Python def of that name exists under {}\n",
            citation.file,
            citation.line,
            citation.name,
            root.display()
        ));
    }
    message.push_str(
        "\nA flag here is a SYMPTOM, not the defect. The usual cause is that the\n\
         thing was renamed and the prose around the citation was written about the\n\
         old behaviour -- so fixing the name alone leaves a false paragraph\n\
         standing. Read the claim, not just the identifier:\n\
         docs/superpowers/notes/2026-09-06-stale-constraints.md, finding 2.\n\n\
         If the citation is deliberately historical, say so in the comment and add\n\
         the marker `doclint-allow` to that line, so the next reader sees the\n\
         exemption beside the claim.\n",
    );
    panic!("{message}");
}
