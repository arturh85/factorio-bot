//! A name a comment cites must exist.
//!
//! This crate is one lint, run as an ordinary `cargo test --workspace` test
//! (`tests/cited_names_resolve.rs`), over the whole checkout rather than over
//! one crate.
//!
//! # Why it exists
//!
//! `docs/superpowers/notes/2026-09-06-stale-constraints.md` catalogues a defect
//! class this repository produces faster than it fixes: **a doc comment states
//! something another commit made false, nothing goes red, and the next reader
//! reasons from a premise that no longer exists.** Its finding 2 is the worked
//! example -- `method::extract` carried a titled section whose measured ceiling
//! had been removed the same day, citing
//! `a_pole_chain_past_the_power_search_radius_is_not_seen` (doclint-allow: the
//! retired name is the subject here), a test that had been
//! inverted and renamed **five lines above the citation**. That review's own
//! recommendation (mitigation (c)) is this crate:
//!
//! > every `` `snake_case_name` `` in a doc comment that looks like a test must
//! > resolve to a `fn` in the tree -- would have caught finding 2 at the commit
//! > that caused it, mechanically, with no judgement involved.
//!
//! Prose is unowned; a citation is not. This lint gives the *citation* an owner
//! that can fail, which is the whole of what it claims to do. It cannot tell
//! whether a paragraph is true. Finding 2's section was false in every sentence,
//! not only in its test name -- **a flag here is a symptom; the claim around it
//! is what has to be read.** That is stated in the failure message too, because
//! whoever sees the failure is the person who has to act on it.
//!
//! # The rule, and why it is drawn exactly here
//!
//! A comment line is flagged when it contains a **backticked** identifier that
//! is [`is_cited_name`]-shaped -- lowercase `snake_case`, **four or more
//! underscore-separated components**, **at least one of them an English
//! function word** -- and no declaration of that name exists anywhere in the
//! tree ([`collect_declarations`]).
//!
//! Every one of those conditions buys a specific class of false positive away,
//! and **a check with many false positives gets disabled, which is worse than no
//! check**. Each was chosen after running the looser rule over the whole tree
//! and reading what it caught:
//!
//! * **Backticks only.** The house style backticks every code name; prose that
//!   merely says "the pole chain test" is not a citation and cannot be resolved
//!   without judgement. Requiring the backticks is what keeps this mechanical.
//! * **Four components.** Rust test names here are sentence-shaped
//!   (`a_generator_past_the_last_pole_is_not_found`), while the identifiers that
//!   would otherwise collide with English -- `game.tick`, `iron-plate`,
//!   `refresh_buffers` -- are short, hyphenated or dotted. Three components
//!   admits things like `map_gen_settings` and `plan_created`, which are wire
//!   field names rather than functions; four does not, and every real citation
//!   found in this tree clears it.
//! * **A function word** ([`FUNCTION_WORDS`]). Four components alone flagged 36
//!   citations here, and 20 of them were **Factorio's own API vocabulary** --
//!   `on_player_mined_entity`, `find_non_colliding_position`,
//!   `belt_to_ground_type`, `call_for_help_radius`. Those names are noun
//!   phrases; this repository's test names are sentences, and a sentence needs
//!   a function word. Measured over the tree at the commit this landed on:
//!   **2,615 of 2,740** `fn` names with four or more components carry one, so
//!   the rule keeps 95% of the house style while dropping the entire
//!   foreign-API class. The 5% it cannot see is a stated false *negative*, and
//!   that is the right direction to be wrong in.
//! * **Any declaration, not just `fn`.** The review's wording says `fn`, and
//!   `fn` is what the motivating case needed. Widening the index to consts,
//!   statics, types, modules, macros and struct fields -- plus Lua `function`s
//!   and Python `def`s, because this tree's docs cite the mod and the analysis
//!   tools -- costs nothing in detection power (a *stale* citation names
//!   something that exists nowhere at all) and removes the whole "it is a real
//!   thing, just not a function" class of noise.
//!
//! # What it deliberately does not check
//!
//! Considered and rejected, each because it would cost false positives that
//! this check cannot afford:
//!
//! * **`file.rs:line` citations.** The file half is checkable and the line half
//!   is not: line numbers drift with every edit above them, so a stale line is
//!   the normal state of a correct citation and flagging it would fire
//!   constantly. Checking only the *file* half was tried on paper and finds
//!   almost nothing -- files are renamed far less often than they are edited.
//! * **`SCREAMING_CASE` constants.** They resolve through `[`CONST`]` intra-doc
//!   links already, and the bare-backtick ones collide with prose nouns
//!   (`TODO`, `NOT`, `RCON`, `SATISFIED`, `MINING`) and with Factorio's own
//!   vocabulary. The signal-to-noise is inverted from the snake_case case.
//! * **Run ids** (`run-1788465258-49050`). The archive is pruned and runs move
//!   to other machines; a missing run id is "not here", never "never existed",
//!   and the check would fail on a clean checkout of an old commit.
//! * **Trailing comments after code on the same line.** Only lines whose first
//!   non-space characters are `//` are read, which makes a string literal
//!   containing `//` mid-line structurally unable to reach the scanner.
//!   Citations live in `///` and `//!` blocks here; buying that class of noise
//!   away is worth the handful of trailing comments not covered.
//!
//! The converse of that last rule is the one known false positive: a *multi-line
//! string literal whose own lines begin with* `//`, which this scanner cannot
//! distinguish from a comment without parsing Rust. The only such fixtures in
//! the tree are this crate's own, and they are assembled with `format!` rather
//! than written out for exactly that reason.
//!
//! # The two escape hatches, and why they are loud
//!
//! A comment line containing the literal `doclint-allow` is skipped. It exists
//! for a citation that is deliberately historical -- naming a test that was
//! deleted, in a sentence written as history. The marker is in the comment, next
//! to the claim, so the next reader sees the exemption and not just its effect.
//!
//! [`FOREIGN_VOCABULARY`] is the second: a name that belongs to Factorio or to
//! a dependency rather than to this tree, listed once with where it really
//! lives. It is **self-expiring** -- an entry nothing cites any more fails
//! [`unused_foreign_vocabulary`], because a suppression that outlives its cause
//! is how a check quietly stops checking.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// One backticked, test-shaped name found in a comment.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Citation {
    /// Path relative to the scan root, so a failure message is copy-pasteable.
    pub file: String,
    /// 1-based, matching what an editor and `rustc` both show.
    pub line: usize,
    /// The identifier as it was written between the backticks.
    pub name: String,
}

/// Directory names never descended into.
///
/// `target` and `node_modules` are build output; `docs` is prose about the
/// code and is exactly where a *retired* name is legitimately still discussed
/// (the stale-constraints note itself quotes the missing test twice), so
/// indexing it would make the lint agree with the thing it is checking --
/// this repository's most expensive recurring defect
/// (`docs/superpowers/notes/2026-09-06-fixtures-agree-with-their-code.md`).
const SKIP_DIRS: &[&str] = &[
    "target",
    "node_modules",
    ".git",
    ".worktrees",
    "docs",
    "dist",
    "workspace",
    "runs",
];

/// Extensions scanned for *declarations*.
const DECL_EXTENSIONS: &[&str] = &["rs", "lua", "py"];

/// The English function words that make a `snake_case` name a *sentence*.
///
/// See the module doc for the measurement behind this list. It is deliberately
/// closed-class -- articles, auxiliaries, negations, conjunctions, prepositions
/// and quantifiers -- because those are the words a Factorio API name never has
/// and a house-style test name almost always does. Adding a *content* word
/// ("place", "build", "walk") would reopen the class this list exists to close.
pub const FUNCTION_WORDS: &[&str] = &[
    "a", "an", "the", "is", "are", "was", "were", "be", "does", "do", "did", "not", "no", "never",
    "always", "only", "still", "rather", "than", "then", "when", "while", "but", "and", "or", "if",
    "so", "that", "which", "its", "it", "we", "must", "may", "can", "cannot", "means", "mean",
    "has", "have", "had", "without", "with", "into", "across", "past", "over", "under", "before",
    "after", "both", "either", "neither", "each", "every", "any", "this", "these", "those", "out",
    "back", "again", "per", "none", "nothing", "one", "itself",
];

/// Names that are real, and are not this tree's to declare.
///
/// Each entry says where the name actually lives. They are Factorio's own
/// vocabulary, cited in a comment about the game rather than about our code, and
/// they clear the sentence-shape filter only because the game happens to spell a
/// machine status with the word "not" in it.
///
/// **Self-expiring by design**: [`unused_foreign_vocabulary`] fails on an entry
/// no comment cites any more. A suppression list nobody prunes is how a check
/// stops checking without anyone noticing.
pub const FOREIGN_VOCABULARY: &[(&str, &str)] = &[
    (
        "not_enough_space_in_output",
        "Factorio `defines.entity_status`, named by the mod and archived by name",
    ),
    (
        "not_plugged_in_electric_network",
        "Factorio `defines.entity_status`, named by the mod and archived by name",
    ),
];

/// Is this the shape of a name worth resolving?
///
/// Lowercase `snake_case`, four or more components, at least one of them a
/// [`FUNCTION_WORDS`] entry. See the module doc for why each condition is there
/// and what each one was measured to buy.
pub fn is_cited_name(candidate: &str) -> bool {
    let name = candidate.trim();
    if name.len() > 120 {
        return false;
    }
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return false;
    }
    if name.ends_with('_') || name.contains("__") {
        return false;
    }
    if name.split('_').count() < 4 {
        return false;
    }
    name.split('_').any(|word| FUNCTION_WORDS.contains(&word))
}

/// The backticked spans of one line, in order.
///
/// Single backticks, paired left to right; an unpaired trailing backtick opens
/// nothing. A rustdoc intra-doc link `[`foo`]` yields `foo`, which is what we
/// want to resolve anyway.
fn backticked_spans(line: &str) -> Vec<&str> {
    let mut spans = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find('`') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('`') else {
            break;
        };
        spans.push(&after[..close]);
        rest = &after[close + 1..];
    }
    spans
}

/// Strip the decorations a citation may carry: a call's parentheses, a macro's
/// bang.
fn normalise(span: &str) -> &str {
    let s = span.trim();
    let s = s.strip_suffix("()").unwrap_or(s);
    s.strip_suffix('!').unwrap_or(s)
}

/// Every cited name in one file's comment lines.
///
/// Only lines whose first non-space characters are `//` are read: see the
/// module doc.
pub fn citations_in_source(path: &str, source: &str) -> Vec<Citation> {
    let mut out = Vec::new();
    for (idx, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("//") {
            continue;
        }
        if trimmed.contains("doclint-allow") {
            continue;
        }
        for span in backticked_spans(trimmed) {
            let name = normalise(span);
            if is_cited_name(name) {
                out.push(Citation {
                    file: path.to_string(),
                    line: idx + 1,
                    name: name.to_string(),
                });
            }
        }
    }
    out
}

/// Names *declared* by one file, by language.
///
/// Deliberately generous -- see the module doc. A name that exists anywhere as
/// anything is not a stale citation.
pub fn declarations_in_source(extension: &str, source: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    match extension {
        "rs" => {
            for line in source.lines() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") {
                    continue;
                }
                for keyword in [
                    "fn ",
                    "const ",
                    "static ",
                    "struct ",
                    "enum ",
                    "trait ",
                    "type ",
                    "union ",
                    "mod ",
                    "macro_rules! ",
                ] {
                    let mut rest = trimmed;
                    while let Some(at) = rest.find(keyword) {
                        let before_ok = at == 0
                            || !rest[..at]
                                .chars()
                                .next_back()
                                .is_some_and(|c| c.is_alphanumeric() || c == '_');
                        let after = &rest[at + keyword.len()..];
                        if before_ok && let Some(ident) = leading_ident(after) {
                            out.insert(ident.to_string());
                        }
                        rest = after;
                    }
                }
                // A struct field or a named argument: `some_long_name: Type`.
                let field = trimmed.trim_start_matches("pub ").trim_start();
                if let Some(ident) = leading_ident(field)
                    && field[ident.len()..].trim_start().starts_with(':')
                {
                    out.insert(ident.to_string());
                }
            }
        }
        "lua" => {
            for line in source.lines() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("--") {
                    continue;
                }
                // `function name(`, `local function name(`, `name = function(`,
                // `["name"] = function`, `name = {`: any binding at all.
                if let Some(after) = trimmed.strip_prefix("function ")
                    && let Some(ident) = leading_ident(after)
                {
                    out.insert(ident.to_string());
                }
                if let Some(after) = trimmed.strip_prefix("local function ")
                    && let Some(ident) = leading_ident(after)
                {
                    out.insert(ident.to_string());
                }
                let bound = trimmed.trim_start_matches("local ").trim_start();
                if let Some(ident) = leading_ident(bound)
                    && bound[ident.len()..].trim_start().starts_with('=')
                {
                    out.insert(ident.to_string());
                }
                // A table key, quoted or bare: `name = value` inside a literal
                // is the same shape as the binding above; `["name"]` is not.
                for span in trimmed.split('"').skip(1).step_by(2) {
                    if is_cited_name(span) {
                        out.insert(span.to_string());
                    }
                }
            }
        }
        "py" => {
            for line in source.lines() {
                let trimmed = line.trim_start();
                if let Some(after) = trimmed.strip_prefix("def ")
                    && let Some(ident) = leading_ident(after)
                {
                    out.insert(ident.to_string());
                }
                if let Some(ident) = leading_ident(trimmed)
                    && trimmed[ident.len()..].trim_start().starts_with('=')
                {
                    out.insert(ident.to_string());
                }
            }
        }
        _ => {}
    }
    out
}

/// The identifier at the start of `s`, if there is one.
fn leading_ident(s: &str) -> Option<&str> {
    let end = s
        .char_indices()
        .take_while(|(_, c)| c.is_alphanumeric() || *c == '_')
        .map(|(i, c)| i + c.len_utf8())
        .last()?;
    let ident = &s[..end];
    if ident.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(ident)
}

/// Every source file under `root`, skipping [`SKIP_DIRS`].
pub fn source_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if !SKIP_DIRS.contains(&name.as_str()) && !name.starts_with('.') {
                    stack.push(path);
                }
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| DECL_EXTENSIONS.contains(&e))
            {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// Every name declared anywhere under `root`.
pub fn collect_declarations(root: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for path in source_files(root) {
        let Ok(source) = fs::read_to_string(&path) else {
            continue;
        };
        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        out.extend(declarations_in_source(extension, &source));
    }
    out
}

/// Every citation in every `.rs` file under `root`, resolved or not.
pub fn all_citations(root: &Path) -> Vec<Citation> {
    let mut out = Vec::new();
    for path in source_files(root) {
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let Ok(source) = fs::read_to_string(&path) else {
            continue;
        };
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        out.extend(citations_in_source(&relative, &source));
    }
    out.sort();
    out
}

/// Every cited name under `root` that resolves to no declaration.
///
/// The whole check. Returns them sorted, so a failure message is stable across
/// runs.
pub fn unresolved_citations(root: &Path) -> Vec<Citation> {
    let declared = collect_declarations(root);
    all_citations(root)
        .into_iter()
        .filter(|c| {
            !declared.contains(&c.name)
                && !FOREIGN_VOCABULARY.iter().any(|(name, _)| *name == c.name)
        })
        .collect()
}

/// [`FOREIGN_VOCABULARY`] entries that no comment cites any more.
///
/// Each is a suppression with nothing left to suppress: delete it. Checked by
/// the same test that runs the lint, so the list cannot rot.
pub fn unused_foreign_vocabulary(root: &Path) -> Vec<&'static str> {
    let cited: BTreeSet<String> = all_citations(root).into_iter().map(|c| c.name).collect();
    FOREIGN_VOCABULARY
        .iter()
        .filter(|(name, _)| !cited.contains(*name))
        .map(|(name, _)| *name)
        .collect()
}

/// The workspace root, found by walking up from this crate.
///
/// `CARGO_MANIFEST_DIR` is `<root>/crates/doclint`; the root is the first
/// ancestor whose `Cargo.toml` declares `[workspace]`. Walking rather than
/// hard-coding `../..` keeps the lint working from a worktree and from a
/// vendored copy.
pub fn workspace_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        let manifest = dir.join("Cargo.toml");
        if let Ok(text) = fs::read_to_string(&manifest)
            && text.contains("[workspace]")
        {
            return dir;
        }
        if !dir.pop() {
            panic!(
                "no [workspace] Cargo.toml above {}",
                env!("CARGO_MANIFEST_DIR")
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The motivating case, reduced: finding 2 of the stale-constraints note.
    ///
    /// The citation is real (`crates/planner/src/method/extract.rs:116` as it
    /// stood at `2d2f75a4`) and the name it gave had been renamed away.
    #[test]
    fn a_doc_comment_citing_a_test_that_does_not_exist_is_flagged() {
        // Assembled rather than written out, because a fixture whose lines
        // literally begin with `//!` is scanned as a comment by this very
        // scanner -- see `code_lines_are_not_scanned_for_citations`, whose
        // converse this is.
        let source = format!(
            "{doc} `a_pole_chain_past_the_power_search_radius_is_not_seen` pins the fact\n\
             {doc} itself, against `PlanState` rather than against this module.\n\
             pub fn pole_run() {{}}\n",
            doc = "//!",
        );
        let source = source.as_str();
        let found = citations_in_source("extract.rs", source);
        assert_eq!(found.len(), 1, "one cited name, got {found:?}");
        assert_eq!(found[0].line, 1);
        assert_eq!(
            found[0].name,
            "a_pole_chain_past_the_power_search_radius_is_not_seen"
        );
        let declared = declarations_in_source("rs", source);
        assert!(
            !declared.contains(&found[0].name),
            "the renamed-away test must not resolve"
        );
    }

    /// The same citation, once the test it names exists.
    #[test]
    fn a_doc_comment_citing_a_test_that_exists_resolves() {
        let source = "\
//! `a_pole_chain_carries_however_long_it_is_but_a_broken_one_does_not` pins it.
fn a_pole_chain_carries_however_long_it_is_but_a_broken_one_does_not() {}
";
        let found = citations_in_source("extract.rs", source);
        assert_eq!(found.len(), 1);
        let declared = declarations_in_source("rs", source);
        assert!(declared.contains(&found[0].name), "declared: {declared:?}");
    }

    /// Prose is not a citation. Without this the check would have to guess which
    /// English sentences name tests, and guessing is how a lint gets disabled.
    #[test]
    fn a_name_in_prose_without_backticks_is_not_a_citation() {
        let source = "//! a_pole_chain_past_the_power_search_radius_is_not_seen pins the fact\n";
        assert!(citations_in_source("x.rs", source).is_empty());
    }

    /// Three components is where wire field names live (`plan_created`,
    /// `map_gen_settings`, `walk_dispatched`); four is where test names do.
    #[test]
    fn short_snake_case_names_are_below_the_threshold() {
        for short in [
            "refresh_buffers",
            "map_gen_settings",
            "walk_dispatched",
            "electric_supply_kw",
        ] {
            assert!(!is_cited_name(short), "{short} should be below threshold");
        }
        assert!(is_cited_name("a_generator_past_the_last_pole_is_not_found"));
    }

    /// Everything that is not a bare lowercase identifier is left alone:
    /// paths, prototype names, kebab-case items, constants, expressions.
    #[test]
    fn only_bare_lowercase_identifiers_are_candidates() {
        for other in [
            "crate::state::PlanState",
            "logistic-science-pack",
            "POWER_SEARCH_RADIUS",
            "max_underground: None",
            "plan_created.bots",
            "run-1788465258-49050",
            "a_name_that_ends_in_an_underscore_",
            "PlanState::electric_supply_kw",
        ] {
            assert!(!is_cited_name(other), "{other} should not be a candidate");
        }
    }

    /// A code line is not a comment line, so a string literal cannot reach the
    /// scanner however it is punctuated.
    #[test]
    fn code_lines_are_not_scanned_for_citations() {
        let source = "let s = \"`a_name_that_is_only_ever_a_string_literal`\";\n";
        assert!(citations_in_source("x.rs", source).is_empty());
    }

    /// The escape hatch works and is visible in the comment that uses it.
    #[test]
    fn a_doclint_allow_marker_exempts_the_line() {
        let source = "//! `a_test_that_was_deleted_on_purpose_long_ago` (doclint-allow: history)\n";
        assert!(citations_in_source("x.rs", source).is_empty());
    }

    /// The class the sentence-shape filter exists to drop: Factorio's own API
    /// names, every one of which was flagged by the looser four-component rule
    /// and none of which is this tree's to declare.
    #[test]
    fn factorio_api_noun_phrases_are_not_citations() {
        for foreign in [
            "on_player_mined_entity",
            "on_player_rotated_entity",
            "find_non_colliding_position",
            "request_to_generate_chunks",
            "force_generate_chunk_requests",
            "belt_to_ground_type",
            "mining_drill_filter_mode",
            "skip_crash_site_cutscene",
            "call_for_help_radius",
            "on_biter_base_built",
            "vector_to_place_result",
            "power_plant_too_far_from_water",
            "receive_single_packet_response",
            "find_free_resource_rect",
        ] {
            assert!(
                !is_cited_name(foreign),
                "{foreign} is a foreign noun phrase, not a citation"
            );
        }
    }

    /// And the house style it exists to keep.
    #[test]
    fn house_style_test_names_are_citations() {
        for name in [
            "a_pole_chain_past_the_power_search_radius_is_not_seen",
            "a_generator_past_the_last_pole_is_not_found",
            "two_poles_a_wire_reach_apart_are_one_network",
            "inference_does_not_link_across_chains",
            "transfer_success_means_items_moved",
            "no_operation_publishes_a_path_parameter",
        ] {
            assert!(is_cited_name(name), "{name} should be a citation");
        }
    }

    /// A citation resolving to something that is not a function still resolves:
    /// a stale citation names something that exists nowhere at all.
    #[test]
    fn a_const_a_field_and_a_lua_function_all_count_as_declarations() {
        let rust = "\
const a_very_long_lower_case_const: u8 = 1;
struct S {
    a_struct_field_with_four_words: u8,
}
";
        let declared = declarations_in_source("rs", rust);
        assert!(declared.contains("a_very_long_lower_case_const"));
        assert!(declared.contains("a_struct_field_with_four_words"));
        let lua = "function set_research_trigger_emulation(x)\nend\n";
        assert!(declarations_in_source("lua", lua).contains("set_research_trigger_emulation"));
        let py = "def a_python_helper_with_four_words():\n    pass\n";
        assert!(declarations_in_source("py", py).contains("a_python_helper_with_four_words"));
    }
}
