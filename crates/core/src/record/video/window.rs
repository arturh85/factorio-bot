//! Finding, sizing and *observing* the X11 window that gets filmed.
//!
//! Factorio runs as an Xwayland client here (`SDL_VIDEODRIVER=x11`), so it has
//! a real X11 window id and `x11grab` can be pointed at that window alone
//! rather than at the screen.
//!
//! Two rules from
//! `docs/superpowers/specs/2026-09-02-video-capture-design.md` are enforced
//! here rather than left to the caller:
//!
//! - **More than one match is a failure, never a coin flip.** Picking
//!   arbitrarily films an unknown peer, and nothing downstream would say which.
//! - **The recorded geometry is the one `xwininfo` observes, never the one we
//!   asked for.** A window manager may refuse or adjust a size request, and the
//!   tiling compositor in use here (Hyprland) will certainly ignore it unless
//!   the window is floated. Recording the request would be the same error as
//!   logging where a bot was *sent* instead of where it landed.
//!
//! Everything that parses is a free function taking text, so the whole of this
//! module is testable without an X server. Only [`Tools`] runs anything.

use std::io;
use std::process::Command;

/// The capture resolution.
///
/// Reached by **sizing the window**, not by scaling the capture: sizing makes
/// the game render fewer pixels, so it is cheaper on the GPU *and* the encoder
/// and resamples nothing. Downscaling in ffmpeg instead would render every
/// pixel, pay for a scale on every frame, and soften the image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Resolution {
    /// 1280x720. The default.
    #[default]
    P720,
    /// 1920x1080. Opt-in, for a final run worth the size.
    P1080,
}

impl Resolution {
    /// Parses `"720p"` / `"1080p"`.
    ///
    /// **An unknown value is an error, never a silent fallback.** A run that
    /// quietly recorded at the wrong size is worse than one that refused to
    /// start, and the refusal happens at `record.start()` where somebody is
    /// still watching.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "720p" => Ok(Resolution::P720),
            "1080p" => Ok(Resolution::P1080),
            other => Err(format!(
                "unknown video resolution {other:?}: expected \"720p\" or \"1080p\""
            )),
        }
    }

    pub fn size(self) -> (u32, u32) {
        match self {
            Resolution::P720 => (1280, 720),
            Resolution::P1080 => (1920, 1080),
        }
    }
}

/// A window's size as `xwininfo` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub width: u32,
    pub height: u32,
}

/// Why a window could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WindowError {
    #[error("no X11 window matched (searched {searched})")]
    NotFound { searched: String },
    /// Several windows matched. Deliberately fatal: see the module docs.
    #[error("{count} X11 windows matched ({ids}) -- refusing to guess which one to film")]
    Ambiguous { count: usize, ids: String },
    #[error("{tool} is not on PATH: {reason}")]
    ToolMissing { tool: String, reason: String },
    #[error("{tool} failed: {reason}")]
    ToolFailed { tool: String, reason: String },
    #[error("could not read the window's geometry back: {reason}")]
    Unreadable { reason: String },
}

/// Parses `xdotool search` output: one decimal window id per line.
///
/// Anything that is not a bare decimal number is dropped rather than guessed
/// at -- `xdotool` prints diagnostics on stderr, but a future version printing
/// a header on stdout must not become a window id.
pub fn parse_window_ids(stdout: &str) -> Vec<u64> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| line.parse::<u64>().ok())
        .collect()
}

/// Pulls `Width:` / `Height:` out of `xwininfo -id <id>` output.
///
/// Reads the labelled lines rather than the trailing `-geometry WxH+X+Y`
/// summary: the summary is a *request string* meant to be passed back to a
/// program, and on some window managers it carries the size hints rather than
/// the mapped size. The labelled pair is what the server currently has.
pub fn parse_geometry(stdout: &str) -> Option<Geometry> {
    let mut width = None;
    let mut height = None;
    for line in stdout.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("Width:") {
            width = value.trim().parse::<u32>().ok();
        } else if let Some(value) = line.strip_prefix("Height:") {
            height = value.trim().parse::<u32>().ok();
        }
    }
    Some(Geometry {
        width: width?,
        height: height?,
    })
}

/// Whether `xwininfo -id <id>` describes a window that is actually on screen.
///
/// SDL creates more than one X window per process under some backends, and the
/// extra ones are unmapped 1x1 helpers. Filming one of those produces a
/// perfectly valid, entirely black recording -- so a candidate that is not
/// viewable is not a candidate.
pub fn is_viewable(stdout: &str) -> bool {
    stdout
        .lines()
        .map(str::trim)
        .any(|line| line.starts_with("Map State:") && line.contains("IsViewable"))
}

/// The external programs this module shells out to.
///
/// Named rather than hardcoded because **the binary's PATH at run time is not
/// guaranteed to be the dev shell's**: `xdotool`, `xwininfo` and an `ffmpeg`
/// with x11grab come from `flake.nix`, and a `factorio-bot` started outside
/// `nix develop` has none of them. Every failure below therefore reports the
/// tool by name, so "no video" says which program was missing instead of
/// leaving a truncated file to be discovered later.
#[derive(Debug, Clone)]
pub struct Tools {
    pub xdotool: String,
    pub xwininfo: String,
}

impl Default for Tools {
    fn default() -> Self {
        Self {
            xdotool: "xdotool".to_string(),
            xwininfo: "xwininfo".to_string(),
        }
    }
}

/// How to find the window: by the client process's pid where that works, by
/// window name otherwise.
///
/// **Whether SDL sets `_NET_WM_PID` under Xwayland is unverified** (design
/// §11.2), which is exactly why both are here: the pid search is tried first
/// because it is the one that can tell two Factorio clients apart, and the
/// name search is the fallback that cannot.
#[derive(Debug, Clone)]
pub struct WindowQuery {
    pub pid: Option<u32>,
    pub name: String,
}

impl Tools {
    fn run(&self, tool: &str, args: &[&str]) -> Result<String, WindowError> {
        let output = Command::new(tool).args(args).output().map_err(|err| {
            if err.kind() == io::ErrorKind::NotFound {
                WindowError::ToolMissing {
                    tool: tool.to_string(),
                    reason: err.to_string(),
                }
            } else {
                WindowError::ToolFailed {
                    tool: tool.to_string(),
                    reason: err.to_string(),
                }
            }
        })?;
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// `xwininfo -id <id>`, or an error naming the tool.
    pub fn describe(&self, id: u64) -> Result<String, WindowError> {
        let xwininfo = self.xwininfo.clone();
        self.run(&xwininfo, &["-id", &id.to_string()])
    }

    /// Reads back what the window actually is. The **only** source of the
    /// geometry that gets recorded.
    pub fn observe(&self, id: u64) -> Result<Geometry, WindowError> {
        let described = self.describe(id)?;
        parse_geometry(&described).ok_or_else(|| WindowError::Unreadable {
            reason: format!("xwininfo -id {id} printed no Width/Height"),
        })
    }

    /// Asks the window manager for `width`x`height`. Best-effort by
    /// construction: the answer is whatever [`Tools::observe`] says afterwards.
    pub fn resize(&self, id: u64, width: u32, height: u32) -> Result<(), WindowError> {
        let xdotool = self.xdotool.clone();
        self.run(
            &xdotool,
            &[
                "windowsize",
                &id.to_string(),
                &width.to_string(),
                &height.to_string(),
            ],
        )?;
        Ok(())
    }

    /// Finds the one window to film, or refuses.
    ///
    /// Candidates are narrowed by [`is_viewable`] before the count is judged,
    /// so an unmapped SDL helper window does not turn a single real match into
    /// an ambiguity.
    pub fn resolve_window(&self, query: &WindowQuery) -> Result<u64, WindowError> {
        let xdotool = self.xdotool.clone();
        let mut searched = Vec::new();

        if let Some(pid) = query.pid {
            searched.push(format!("--pid {pid}"));
            let out = self.run(
                &xdotool,
                &[
                    "search",
                    "--all",
                    "--pid",
                    &pid.to_string(),
                    "--name",
                    &query.name,
                ],
            )?;
            let candidates = self.viewable(parse_window_ids(&out));
            match candidates.len() {
                1 => return Ok(candidates[0]),
                0 => {}
                _ => return Err(ambiguous(&candidates)),
            }
        }

        searched.push(format!("--name {}", query.name));
        let out = self.run(&xdotool, &["search", "--name", &query.name])?;
        let candidates = self.viewable(parse_window_ids(&out));
        match candidates.len() {
            1 => Ok(candidates[0]),
            0 => Err(WindowError::NotFound {
                searched: searched.join(", "),
            }),
            _ => Err(ambiguous(&candidates)),
        }
    }

    /// Keeps only the candidates `xwininfo` says are mapped and on screen.
    ///
    /// A candidate `xwininfo` cannot describe at all is *kept*: the tool
    /// failing is not evidence about the window, and dropping it would turn a
    /// broken `xwininfo` into a confident "not found".
    fn viewable(&self, ids: Vec<u64>) -> Vec<u64> {
        ids.into_iter()
            .filter(|id| match self.describe(*id) {
                Ok(text) => is_viewable(&text),
                Err(_) => true,
            })
            .collect()
    }
}

fn ambiguous(ids: &[u64]) -> WindowError {
    WindowError::Ambiguous {
        count: ids.len(),
        ids: ids
            .iter()
            .map(|id| format!("0x{id:x}"))
            .collect::<Vec<_>>()
            .join(", "),
    }
}

/// The `0x`-prefixed form `ffmpeg -window_id` takes.
pub fn window_id_hex(id: u64) -> String {
    format!("0x{id:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_documented_resolutions_parse_and_nothing_else_does() {
        assert_eq!(Resolution::parse("720p").unwrap().size(), (1280, 720));
        assert_eq!(Resolution::parse("1080p").unwrap().size(), (1920, 1080));
        // The whole point of the decision: no silent fallback.
        let err = Resolution::parse("4k").unwrap_err();
        assert!(err.contains("4k"), "{err}");
        assert!(
            err.contains("720p"),
            "the error says what is allowed: {err}"
        );
        for bogus in ["", "720", "1080P", "hd"] {
            assert!(Resolution::parse(bogus).is_err(), "{bogus} must not parse");
        }
    }

    #[test]
    fn the_default_is_720p() {
        assert_eq!(Resolution::default(), Resolution::P720);
    }

    #[test]
    fn window_ids_come_off_xdotool_one_per_line() {
        assert_eq!(
            parse_window_ids("46137349\n46137352\n"),
            vec![46137349, 46137352]
        );
        assert_eq!(parse_window_ids(""), Vec::<u64>::new());
        // Not a window id, whatever it is.
        assert_eq!(parse_window_ids("0x2c00007\nwindows:\n12\n"), vec![12]);
    }

    /// Real `xwininfo -id` output, abbreviated. The labelled pair is read, not
    /// the trailing `-geometry` summary.
    #[test]
    fn geometry_comes_from_the_labelled_width_and_height() {
        let out = "\
xwininfo: Window id: 0x2c00007 \"Factorio\"

  Absolute upper-left X:  16
  Absolute upper-left Y:  48
  Width: 1278
  Height: 715
  Depth: 24
  Map State: IsViewable
  -geometry 1280x720+16+48
";
        assert_eq!(
            parse_geometry(out),
            Some(Geometry {
                width: 1278,
                height: 715
            }),
            "the observed size, not the 1280x720 the -geometry line still claims"
        );
        assert!(is_viewable(out));
    }

    #[test]
    fn output_with_no_size_yields_no_geometry_rather_than_a_zero() {
        assert_eq!(parse_geometry("xwininfo: no such window\n"), None);
        assert_eq!(
            parse_geometry("  Width: 1280\n"),
            None,
            "half is not a size"
        );
    }

    #[test]
    fn an_unmapped_window_is_not_viewable() {
        assert!(!is_viewable("  Map State: IsUnMapped\n"));
        assert!(!is_viewable(""));
    }

    #[test]
    fn ambiguity_names_every_match_in_the_form_ffmpeg_takes() {
        let err = ambiguous(&[16, 255]);
        let text = err.to_string();
        assert!(text.contains("0x10"), "{text}");
        assert!(text.contains("0xff"), "{text}");
        assert!(text.contains("refusing to guess"), "{text}");
    }

    #[test]
    fn a_window_id_is_rendered_in_hex_for_ffmpeg() {
        assert_eq!(window_id_hex(46137349), "0x2c00005");
    }

    /// A tool that is not on PATH must say so by name. `nix develop` is what
    /// supplies `xdotool` here, and a run started outside it fails exactly this
    /// way -- the message is the whole diagnosis.
    #[test]
    fn a_missing_tool_is_reported_by_name() {
        let tools = Tools {
            xdotool: "definitely-not-a-real-binary-xdotool".to_string(),
            xwininfo: "definitely-not-a-real-binary-xwininfo".to_string(),
        };
        let err = tools
            .resolve_window(&WindowQuery {
                pid: None,
                name: "Factorio".to_string(),
            })
            .expect_err("there is no such binary");
        assert!(
            matches!(&err, WindowError::ToolMissing { tool, .. } if tool.contains("xdotool")),
            "{err}"
        );
    }
}
