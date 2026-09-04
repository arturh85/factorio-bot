//! What kind of bots a run has, and how fast its world runs.
//!
//! Written into the server instance directory by the code that started the
//! server, read back by `record.start()` when it fills in provenance -- the
//! same road `resumed_from` travels (see [`super::savepoint::ResumeMarker`]),
//! and for the same reason: "how was this server started" is a fact about the
//! *instance*, known by the process that launched it and by nothing between
//! there and the Lua API.
//!
//! Two facts, both start-time:
//!
//! - **Bot mode.** A run's bots are either graphical clients that connected
//!   (`Clients`) or `character` entities the mod created on the headless
//!   server (`Characters`, the `--headless` flag). Play fidelity is the same
//!   -- same walk speed, mining time, crafting time and reach -- so this is a
//!   note for a comparison rather than a refusal, but it is a note that must
//!   exist: only a client run can be filmed.
//! - **Game speed.** `game.speed` as set at start. Every timing in the record
//!   is in game ticks, so a run at speed 5 measures the same thing as one at
//!   speed 1 -- unless the server could not keep up, which the record can only
//!   suspect if it knows the speed that was asked for.
//!
//! Cleared on every non-headless server start rather than left to expire, for
//! the reason `clear_resume_marker` gives: a start-time fact that stays on disk
//! after it stops being true is worse than none.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// File name of the marker inside the instance directory.
pub const RUN_MODE_MARKER: &str = "run-mode.json";

/// What a run's bots are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BotMode {
    /// Graphical Factorio clients, one process per bot.
    Clients,
    /// Server-side `character` entities created by the mod; no client.
    Characters,
}

impl BotMode {
    /// The word provenance records.
    pub fn as_str(self) -> &'static str {
        match self {
            BotMode::Clients => "clients",
            BotMode::Characters => "characters",
        }
    }
}

/// The marker's content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunMode {
    pub bot_mode: BotMode,
    /// `game.speed` as set at start; `1.0` is normal.
    pub game_speed: f64,
}

/// Writes the marker into `<workspace>/<instance>`.
pub fn write_run_mode(instance_dir: &Path, mode: &RunMode) -> io::Result<PathBuf> {
    fs::create_dir_all(instance_dir)?;
    let path = instance_dir.join(RUN_MODE_MARKER);
    let json = serde_json::to_string_pretty(mode).map_err(io::Error::other)?;
    fs::write(&path, json.as_bytes())?;
    Ok(path)
}

/// Removes the marker; a missing one is not an error.
pub fn clear_run_mode(instance_dir: &Path) -> io::Result<()> {
    match fs::remove_file(instance_dir.join(RUN_MODE_MARKER)) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Reads the marker, or `None` when the instance has none (a server started
/// by a build that predates it, or an attached server this process did not
/// start).
pub fn read_run_mode(instance_dir: &Path) -> Option<RunMode> {
    let bytes = fs::read(instance_dir.join(RUN_MODE_MARKER)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marker_round_trips_and_clears() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_run_mode(dir.path()).is_none());
        write_run_mode(
            dir.path(),
            &RunMode {
                bot_mode: BotMode::Characters,
                game_speed: 5.0,
            },
        )
        .unwrap();
        let back = read_run_mode(dir.path()).unwrap();
        assert_eq!(back.bot_mode, BotMode::Characters);
        assert_eq!(back.game_speed, 5.0);
        clear_run_mode(dir.path()).unwrap();
        assert!(read_run_mode(dir.path()).is_none());
        // Clearing twice is fine: absence is the normal state.
        clear_run_mode(dir.path()).unwrap();
    }

    #[test]
    fn bot_mode_serialises_as_the_word_provenance_records() {
        let json = serde_json::to_string(&RunMode {
            bot_mode: BotMode::Clients,
            game_speed: 1.0,
        })
        .unwrap();
        assert!(json.contains("\"clients\""), "{json}");
        assert_eq!(BotMode::Characters.as_str(), "characters");
    }
}
