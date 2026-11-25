//! Game process discovery, identity, and memory access for macOS.
use std::path::{Path, PathBuf};

mod macos;

pub use macos::*;

pub const GAME_EXE: &str = "AzerothSurvivorsGame";
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
    pub(crate) sessions: Vec<Session>,
    pub(crate) inaccessible: usize,
}

impl Games {
    pub fn sessions(&self) -> &[Session] {
        &self.sessions
    }

    pub fn inaccessible(&self) -> usize {
        self.inaccessible
    }

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
