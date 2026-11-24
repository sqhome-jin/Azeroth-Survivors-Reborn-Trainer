//! Game process discovery, identity, and memory access. The platform modules
//! provide `discover`, `launch`, `stop`, `Reader`, and `StatWriter` with the
//! same signatures so the rest of the trainer stays platform neutral.
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
pub use macos::*;
#[cfg(windows)]
pub use windows::*;

#[cfg(windows)]
pub const GAME_EXE: &str = "AzerothSurvivorsGame.exe";
#[cfg(not(windows))]
pub const GAME_EXE: &str = "AzerothSurvivorsGame";

/// Human-readable platform label used in diagnostics and release checks.
#[cfg(windows)]
pub const PLATFORM: &str = "Windows x64";
#[cfg(target_os = "macos")]
pub const PLATFORM: &str = "macOS arm64";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Session {
    pub pid: u32,
    /// Process creation time in platform units; only compared for equality.
    pub created: u64,
    pub path: PathBuf,
}

#[derive(Debug, Default)]
pub struct Games {
    sessions: Vec<Session>,
    inaccessible: usize,
}

impl Games {
    pub fn running(&self) -> bool {
        self.inaccessible > 0 || !self.sessions.is_empty()
    }

    pub fn select(&self, preferred: Option<&Path>, current: Option<&Session>) -> Option<&Session> {
        if let Some(session) =
            current.and_then(|current| self.sessions.iter().find(|s| *s == current))
        {
            return Some(session);
        }
        if self.inaccessible > 0 {
            return None;
        }
        if let [session] = self.sessions.as_slice() {
            return Some(session);
        }
        let preferred = preferred?;
        let mut matches = self
            .sessions
            .iter()
            .filter(|session| same_path(&session.path, preferred));
        let selected = matches.next()?;
        matches.next().is_none().then_some(selected)
    }
}

#[cfg(windows)]
pub(crate) fn same_path(a: &Path, b: &Path) -> bool {
    fn comparable(path: &Path) -> String {
        let raw = path.to_string_lossy().replace('/', "\\");
        raw.strip_prefix(r"\\?\").unwrap_or(&raw).to_lowercase()
    }
    comparable(a) == comparable(b)
}

#[cfg(not(windows))]
pub(crate) fn same_path(a: &Path, b: &Path) -> bool {
    a == b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(pid: u32, path: &str) -> Session {
        Session {
            pid,
            created: 5,
            path: path.into(),
        }
    }

    #[test]
    fn ambiguous_processes_need_an_exact_selection() {
        let first = session(1, "/Game/AzerothSurvivorsGame");
        let second = session(2, "/Other/AzerothSurvivorsGame");
        let games = Games {
            sessions: vec![first.clone(), second],
            inaccessible: 0,
        };
        assert!(games.select(None, None).is_none());
        assert_eq!(
            games.select(Some(Path::new("/Game/AzerothSurvivorsGame")), None),
            Some(&first)
        );
        assert_eq!(games.select(None, Some(&first)), Some(&first));
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_compare_without_case_or_separator_differences() {
        let first = session(1, "C:/Game/AzerothSurvivorsGame.exe");
        let games = Games {
            sessions: vec![
                first.clone(),
                session(2, "D:/Game/AzerothSurvivorsGame.exe"),
            ],
            inaccessible: 0,
        };
        assert_eq!(
            games.select(Some(Path::new(r"c:\game\AzerothSurvivorsGame.exe")), None),
            Some(&first)
        );
    }

    #[test]
    fn inaccessible_process_blocks_automatic_selection() {
        let games = Games {
            sessions: vec![session(1, "/Game/AzerothSurvivorsGame")],
            inaccessible: 1,
        };
        assert!(games.running());
        assert!(games.select(None, None).is_none());
    }
}
