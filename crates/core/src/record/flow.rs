//! `flow.jsonl`: the flow graph's own keyframes.
//!
//! One [`crate::graph::flow_export::FlowExport`] per line, written at the
//! same two moments `map.jsonl` gets a `MapKind::Keyframe` line -- see
//! `RunRecorder::record_flow`'s callers in `crates/scripting_lua`. Unlike
//! `map.jsonl`, every line here is a full snapshot, not a delta: nothing
//! reconstructs a flow graph from a base plus edits, so a reader wanting the
//! graph nearest some tick picks the latest line at or before it.

use crate::graph::flow_export::FlowExport;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ReadFlow {
    pub records: Vec<FlowExport>,
    /// Lines that did not parse -- in practice the truncated final line of a
    /// crashed run. Reported rather than swallowed, exactly like
    /// [`super::map::read_map`] and [`super::read_events`].
    pub skipped: usize,
}

pub fn read_flow(path: &Path) -> io::Result<ReadFlow> {
    let mut records = Vec::new();
    let mut skipped = 0usize;
    for line in BufReader::new(File::open(path)?).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<FlowExport>(&line) {
            Ok(record) => records.push(record),
            Err(_) => skipped += 1,
        }
    }
    Ok(ReadFlow { records, skipped })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::flow_export::{FlowExport, FlowExportEdge, FlowExportNode, FlowExportRate};
    use crate::types::Position;
    use std::io::Write;

    fn write_lines(dir: &std::path::Path, lines: &[&str]) -> std::path::PathBuf {
        let path = dir.join("flow.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
        path
    }

    fn sample_export(tick: u64) -> FlowExport {
        FlowExport {
            tick,
            nodes: vec![FlowExportNode {
                id: 0,
                position: Position::new(0.5, 0.5),
                name: "stone-furnace".to_string(),
                kind: "furnace".to_string(),
                recipe: None,
                miner_ore: None,
            }],
            edges: vec![FlowExportEdge {
                from: 0,
                to: 0,
                lanes: vec![vec![FlowExportRate {
                    item: "iron-plate".to_string(),
                    per_second: 0.3125,
                }]],
            }],
        }
    }

    #[test]
    fn read_flow_reads_every_record() {
        let tmp = tempfile::tempdir().unwrap();
        let export = sample_export(100);
        let line = serde_json::to_string(&export).unwrap();
        let path = write_lines(tmp.path(), &[&line]);

        let read = read_flow(&path).unwrap();
        assert_eq!(read.skipped, 0);
        assert_eq!(read.records, vec![export]);
    }

    #[test]
    fn read_flow_skips_a_truncated_final_line() {
        let tmp = tempfile::tempdir().unwrap();
        let good = serde_json::to_string(&sample_export(100)).unwrap();
        let path = write_lines(tmp.path(), &[&good, r#"{"tick":200,"nodes":[{"i"#]);

        let read = read_flow(&path).unwrap();
        assert_eq!(read.records.len(), 1);
        assert_eq!(read.skipped, 1);
    }

    #[test]
    fn read_flow_skips_blank_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let good = serde_json::to_string(&sample_export(100)).unwrap();
        let path = write_lines(tmp.path(), &[&good, "", "   "]);

        let read = read_flow(&path).unwrap();
        assert_eq!(read.records.len(), 1);
        assert_eq!(read.skipped, 0, "a blank line is not a parse failure");
    }
}
