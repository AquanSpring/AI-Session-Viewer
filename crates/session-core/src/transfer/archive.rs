//! Archive format shared by export and import.
//!
//! ```text
//! manifest.json                     – written last; describes everything below
//! files/<source>/<relative path>    – raw files, relative to the source root
//! extras/codex/threads.jsonl        – Codex `threads` rows (state_5.sqlite)
//! extras/codex/session_index.jsonl  – Codex Desktop thread names
//! extras/codex/viewer-meta.json     – Viewer aliases/tags for Codex sessions
//! extras/bookmarks.json             – Viewer bookmarks of exported sessions
//! ```
//!
//! Source roots: Claude `~/.claude/projects`, Codex `$CODEX_HOME`, Grok
//! `~/.grok/sessions`, OMP sessions dir. Relative paths always use `/`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::bookmarks::Bookmark;
use crate::paths::SessionSourceKind;
use crate::provider::{codex, grok, omp};

pub const FORMAT: &str = "ai-session-viewer-archive";
pub const VERSION: u32 = 1;
pub const MANIFEST: &str = "manifest.json";
pub const CODEX_THREADS: &str = "extras/codex/threads.jsonl";
pub const CODEX_SESSION_INDEX: &str = "extras/codex/session_index.jsonl";
pub const CODEX_VIEWER_META: &str = "extras/codex/viewer-meta.json";
pub const BOOKMARKS: &str = "extras/bookmarks.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    pub app_version: String,
    pub created_at: String,
    pub host_os: String,
    pub file_count: u64,
    pub total_bytes: u64,
    pub sources: Vec<ManifestSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestSource {
    pub source: String,
    pub projects: Vec<ManifestProject>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestProject {
    /// Project id on the exporting machine (Claude: encoded dir name; others: cwd
    /// or a virtual bucket id).
    pub id: String,
    pub name: String,
    /// Real project directory on the exporting machine; `None` for virtual
    /// buckets, which cannot be remapped.
    pub path: Option<String>,
    pub sessions: Vec<ManifestSession>,
    /// Project-level files that belong to no single session (Claude memory,
    /// sessions-index.json, viewer metadata).
    #[serde(default)]
    pub files: Vec<ManifestFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestSession {
    pub id: String,
    /// Relative path of the file the providers list (and bookmarks point at).
    pub primary: String,
    /// Every file of the session, including `primary`.
    pub files: Vec<ManifestFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestFile {
    pub path: String,
    pub size: u64,
    /// Unix seconds. Restored on import so lists keep their order.
    #[serde(default)]
    pub mtime: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchivedBookmark {
    pub source: String,
    /// `primary` of the bookmarked session.
    pub primary: String,
    pub bookmark: Bookmark,
}

pub fn entry_name(source: &str, rel: &str) -> String {
    format!("files/{source}/{rel}")
}

pub fn host_os() -> &'static str {
    std::env::consts::OS
}

/// Root directory every relative path of `source` is resolved against.
pub fn source_root(kind: SessionSourceKind) -> Option<PathBuf> {
    match kind {
        SessionSourceKind::Claude => crate::parser::path_encoder::get_projects_dir(),
        SessionSourceKind::Codex => codex::get_sessions_dir()?.parent().map(Path::to_path_buf),
        SessionSourceKind::Grok => grok::get_sessions_dir(),
        SessionSourceKind::Omp => omp::get_sessions_dir(),
    }
}

/// `/`-joined path of `path` below `root`, or `None` if it is not below it.
pub fn relative_to(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for component in rel.components() {
        match component {
            std::path::Component::Normal(part) => parts.push(part.to_str()?.to_string()),
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn valid_segment(segment: &str) -> bool {
    if segment.is_empty() || segment == "." || segment == ".." {
        return false;
    }
    if segment.contains(['/', '\\', '\0']) {
        return false;
    }
    // Names Windows cannot store would fail later with a confusing error.
    !(cfg!(windows) && segment.contains(['<', '>', ':', '"', '|', '?', '*']))
}

/// Split and validate an archive-relative path. Rejects absolute paths, `..`,
/// drive prefixes and separators inside segments, so a crafted manifest can
/// never address a file outside the source root.
pub fn split_rel(rel: &str) -> Result<Vec<&str>, String> {
    let parts: Vec<&str> = rel.split('/').collect();
    if parts.iter().all(|part| valid_segment(part)) {
        Ok(parts)
    } else {
        Err(format!("归档包含非法路径：{rel}"))
    }
}

/// Validate the layout of a file path inside the source root.
pub fn validate_layout(kind: SessionSourceKind, rel: &str) -> Result<(), String> {
    let parts = split_rel(rel)?;
    let ok = match kind {
        SessionSourceKind::Claude => parts.len() >= 2,
        SessionSourceKind::Codex => {
            parts.len() >= 2
                && matches!(parts[0], "sessions" | "archived_sessions")
                && parts.last().is_some_and(|name| {
                    name.starts_with("rollout-") && name.ends_with(".jsonl")
                })
        }
        SessionSourceKind::Grok => parts.len() >= 3,
        SessionSourceKind::Omp => !parts.is_empty(),
    };
    if ok {
        Ok(())
    } else {
        Err(format!("归档路径与 {} 的目录结构不符：{rel}", source_name(kind)))
    }
}

pub fn source_name(kind: SessionSourceKind) -> &'static str {
    match kind {
        SessionSourceKind::Claude => "claude",
        SessionSourceKind::Codex => "codex",
        SessionSourceKind::Grok => "grok",
        SessionSourceKind::Omp => "omp",
    }
}

/// Split a Windows verbatim prefix (`\\?\C:\…`, `\\?\UNC\server\…`) off a
/// path: `(prefix, path in its plain form)`. Codex stores some paths verbatim.
fn split_verbatim(path: &str) -> (&'static str, std::borrow::Cow<'_, str>) {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        (r"\\?\UNC\", format!(r"\\{rest}").into())
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        (r"\\?\", rest.into())
    } else {
        ("", path.into())
    }
}

fn is_windows_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with(r"\\") || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
}

/// Replace the `from` directory prefix of `value` with `to`. Only whole path
/// components match (`/a/b` maps `/a/b/c` but not `/a/bc`); Windows paths
/// match case-insensitively and with or without a verbatim prefix. The
/// remainder's separators follow the style of `to`, so Windows ↔ Unix moves
/// stay valid.
pub fn remap_path(value: &str, from: &str, to: &str) -> Option<String> {
    let (verbatim, plain) = split_verbatim(value);
    let (_, from) = split_verbatim(from);
    let from = trim_separators(&from);
    let head = plain.get(..from.len())?;
    let matches = if is_windows_path(from) {
        head.eq_ignore_ascii_case(from)
    } else {
        head == from
    };
    let rest = &plain[from.len()..];
    if !matches || !(rest.is_empty() || rest.starts_with(['/', '\\'])) {
        return None;
    }
    let windows_style = to.contains('\\') && !to.contains('/');
    let rest = if windows_style {
        rest.replace('/', "\\")
    } else {
        rest.replace('\\', "/")
    };
    let to = trim_separators(to);
    // Keep Codex's verbatim form when the new location is a drive path too.
    let prefix = if !verbatim.is_empty() && is_windows_path(to) && !to.starts_with(r"\\") {
        r"\\?\"
    } else {
        ""
    };
    Some(format!("{prefix}{to}{rest}"))
}

fn trim_separators(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() || trimmed.ends_with(':') {
        path // keep `/` and `C:\` intact
    } else {
        trimmed
    }
}

/// A path rewrite applied to the cwd fields of imported transcripts.
#[derive(Debug, Clone)]
pub struct Remap {
    pub from: String,
    pub to: String,
}

impl Remap {
    pub fn apply(&self, value: &str) -> Option<String> {
        remap_path(value, &self.from, &self.to)
    }

    /// Quick pre-check so unaffected lines are copied byte-for-byte.
    pub fn needle(&self) -> String {
        let (_, plain) = split_verbatim(&self.from);
        let from = trim_separators(&plain);
        // Drive letters differ in case between tools; skip `C:` in the filter.
        let from = if is_windows_path(from) && !from.starts_with(r"\\") {
            &from[2..]
        } else {
            from
        };
        let escaped = serde_json::to_string(from).unwrap_or_default();
        escaped.trim_matches('"').to_string()
    }
}

/// Rewrite the cwd fields that the supported CLIs persist: Claude/OMP rows
/// (`cwd`), Codex `session_meta` / `turn_context` (`payload.cwd`) and the Grok
/// summary (`info.cwd`). Message bodies are history and stay untouched.
pub fn rewrite_cwd_fields(value: &mut Value, remap: &Remap) -> bool {
    let mut changed = false;
    for pointer in ["/cwd", "/payload/cwd", "/info/cwd"] {
        if let Some(Value::String(current)) = value.pointer_mut(pointer) {
            if let Some(next) = remap.apply(current) {
                if next != *current {
                    *current = next;
                    changed = true;
                }
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remap_matches_whole_components_and_converts_separators() {
        assert_eq!(
            remap_path("C:\\Users\\a\\proj\\sub", "C:\\Users\\a\\proj", "/home/b/proj").as_deref(),
            Some("/home/b/proj/sub")
        );
        assert_eq!(
            remap_path("/Users/a/proj", "/Users/a/proj/", "D:\\work\\proj").as_deref(),
            Some("D:\\work\\proj")
        );
        assert_eq!(remap_path("/Users/a/project", "/Users/a/proj", "/x"), None);
    }

    #[test]
    fn remap_handles_verbatim_and_drive_case() {
        // Codex SQLite stores verbatim paths; the rollout header does not.
        assert_eq!(
            remap_path(r"\\?\C:\Users\a\proj", r"C:\Users\a\proj", r"D:\work\proj").as_deref(),
            Some(r"\\?\D:\work\proj")
        );
        assert_eq!(
            remap_path(r"c:\users\a\proj\src", r"C:\Users\a\proj", "/home/b/proj").as_deref(),
            Some("/home/b/proj/src")
        );
        assert_eq!(
            remap_path(r"\\?\C:\Users\a\proj", r"C:\Users\a\proj", "/home/b/proj").as_deref(),
            Some("/home/b/proj")
        );
        assert_eq!(remap_path("/Users/A/proj", "/Users/a/proj", "/x"), None);
        let remap = Remap {
            from: r"C:\Users\a\proj".into(),
            to: "/x".into(),
        };
        assert_eq!(remap.needle(), r"\\Users\\a\\proj");
    }

    #[test]
    fn rejects_escaping_paths() {
        assert!(split_rel("../etc/passwd").is_err());
        assert!(split_rel("/abs").is_err());
        assert!(split_rel("a\\..\\b").is_err());
        assert!(split_rel("proj/sess.jsonl").is_ok());
        assert!(validate_layout(SessionSourceKind::Codex, "sessions/2026/01/01/x.jsonl").is_err());
        assert!(validate_layout(
            SessionSourceKind::Codex,
            "sessions/2026/01/01/rollout-x.jsonl"
        )
        .is_ok());
    }

    #[test]
    fn rewrites_known_cwd_fields_only() {
        let remap = Remap {
            from: "/old".into(),
            to: "/new".into(),
        };
        let mut row = serde_json::json!({
            "cwd": "/old/a",
            "payload": { "cwd": "/old" },
            "message": { "content": "/old/a stays" }
        });
        assert!(rewrite_cwd_fields(&mut row, &remap));
        assert_eq!(row["cwd"], "/new/a");
        assert_eq!(row["payload"]["cwd"], "/new");
        assert_eq!(row["message"]["content"], "/old/a stays");
    }
}
