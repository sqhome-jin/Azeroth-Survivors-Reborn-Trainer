//! Job registry, opaque handles, and JSON DTOs for the Swift bridge.
use std::{
    collections::{HashMap, VecDeque},
    io,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use serde::Serialize;
use serde_json::Value;

use crate::{
    diagnostic_log::{self, Log},
    hunt::{self, VerifiedStats},
    process::{self, Session},
    settings::{self, Settings, SettingsDto},
    stats_ui::STATS,
    update,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct SessionId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ProofId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct JobId(pub u64);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendEvent {
    pub kind: String,
    pub job_id: Option<u64>,
    pub operation: Option<String>,
    pub code: Option<String>,
    pub message: Option<String>,
    pub payload: Value,
}

struct ProofEntry {
    stats: VerifiedStats,
}

struct BackendInner {
    next_id: u64,
    sessions: HashMap<SessionId, Session>,
    proofs: HashMap<ProofId, ProofEntry>,
    jobs: HashMap<JobId, Arc<AtomicBool>>,
    events: VecDeque<BackendEvent>,
    _log: Option<Log>,
}

pub struct Backend {
    inner: Arc<Mutex<BackendInner>>,
}

impl Backend {
    pub fn create() -> Self {
        let log = settings::state_directory()
            .and_then(|directory| diagnostic_log::Log::start(directory.join("reborn-rust.log")))
            .ok();
        diagnostic_log::event(
            "backend.start",
            format_args!("version={}", env!("CARGO_PKG_VERSION")),
        );
        Self {
            inner: Arc::new(Mutex::new(BackendInner {
                next_id: 1,
                sessions: HashMap::new(),
                proofs: HashMap::new(),
                jobs: HashMap::new(),
                events: VecDeque::new(),
                _log: log,
            })),
        }
    }

    fn with_mut<R>(&self, f: impl FnOnce(&mut BackendInner) -> R) -> R {
        f(&mut self.inner.lock().expect("backend lock"))
    }

    fn next_id(inner: &mut BackendInner) -> u64 {
        let id = inner.next_id;
        inner.next_id = inner.next_id.saturating_add(1);
        id
    }

    fn push_event(inner: &mut BackendInner, event: BackendEvent) {
        if inner.events.len() >= 256 {
            inner.events.pop_front();
        }
        inner.events.push_back(event);
    }

    pub fn poll_event(&self) -> Option<BackendEvent> {
        self.with_mut(|inner| inner.events.pop_front())
    }

    pub fn cancel(&self, job_id: JobId) {
        self.with_mut(|inner| {
            if let Some(flag) = inner.jobs.get(&job_id) {
                flag.store(true, Ordering::Relaxed);
            }
        });
    }

    pub fn release_proof(&self, proof_id: ProofId) {
        self.with_mut(|inner| {
            inner.proofs.remove(&proof_id);
        });
    }

    pub fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_owned()
    }

    pub fn state_directory(&self) -> io::Result<PathBuf> {
        settings::state_directory()
    }

    pub fn settings_path(&self) -> io::Result<PathBuf> {
        Ok(settings::state_directory()?.join("reborn-settings.json"))
    }

    pub fn load_settings(&self) -> io::Result<SettingsDto> {
        Ok(Settings::load()?.to_dto())
    }

    pub fn save_settings(&self, dto: SettingsDto) -> io::Result<()> {
        let mut settings = Settings::load()?;
        settings.apply_dto(&dto)
    }

    pub fn default_game_path(&self) -> Option<String> {
        process::default_game_path().map(|path| path.display().to_string())
    }

    pub fn diagnostics_paths(&self) -> io::Result<Value> {
        let directory = settings::state_directory()?;
        Ok(serde_json::json!({
            "stateDirectory": directory.display().to_string(),
            "settingsPath": directory.join("reborn-settings.json").display().to_string(),
            "logPath": directory.join("reborn-rust.log").display().to_string(),
            "previousLogPath": directory.join("reborn-rust.previous.log").display().to_string(),
            "platform": process::PLATFORM,
            "version": env!("CARGO_PKG_VERSION"),
        }))
    }

    pub fn recent_log_lines(&self) -> io::Result<String> {
        let path = settings::state_directory()?.join("reborn-rust.log");
        diagnostic_log::recent_lines(&path)
    }

    pub fn stats_metadata(&self, chinese: bool) -> Value {
        let stats: Vec<_> = STATS
            .iter()
            .map(|stat| {
                serde_json::json!({
                    "key": stat.key,
                    "label": if chinese { stat.chinese_label } else { stat.label },
                    "min": stat.min,
                    "max": stat.max,
                    "percent": stat.percent,
                    "defaultEnabled": stat.default_enabled(),
                })
            })
            .collect();
        Value::Array(stats)
    }

    fn store_session(inner: &mut BackendInner, session: Session) -> SessionId {
        if let Some((id, _)) = inner
            .sessions
            .iter()
            .find(|(_, stored)| **stored == session)
        {
            return *id;
        }
        let id = SessionId(Self::next_id(inner));
        inner.sessions.insert(id, session);
        id
    }

    fn session(&self, id: SessionId) -> io::Result<Session> {
        self.with_mut(|inner| {
            inner
                .sessions
                .get(&id)
                .cloned()
                .ok_or_else(|| io::Error::other("Unknown session handle."))
        })
    }

    pub fn discover_sync(&self) -> io::Result<Value> {
        let games = process::discover()?;
        self.with_mut(|inner| {
            let known: Vec<_> = games.sessions().to_vec();
            inner
                .sessions
                .retain(|_, session| known.iter().any(|live| live == session));
            let sessions: Vec<_> = known
                .into_iter()
                .map(|session| {
                    let id = Self::store_session(inner, session.clone());
                    serde_json::json!({
                        "sessionId": id.0,
                        "pid": session.pid,
                        "created": session.created,
                        "path": session.path.display().to_string(),
                    })
                })
                .collect();
            let preferred = games.select(None, None).and_then(|session| {
                inner
                    .sessions
                    .iter()
                    .find(|(_, stored)| *stored == session)
                    .map(|(id, _)| id.0)
            });
            Ok(serde_json::json!({
                "sessions": sessions,
                "inaccessibleCount": games.inaccessible(),
                "preferredSessionId": preferred,
                "running": games.running(),
            }))
        })
    }

    fn spawn_job<F>(&self, operation: &'static str, work: F) -> JobId
    where
        F: FnOnce(JobId, Arc<AtomicBool>, Arc<Mutex<BackendInner>>) + Send + 'static,
    {
        let cancel = Arc::new(AtomicBool::new(false));
        let job_id = self.with_mut(|inner| {
            let job_id = JobId(Self::next_id(inner));
            inner.jobs.insert(job_id, cancel.clone());
            Self::push_event(
                inner,
                BackendEvent {
                    kind: "job_started".into(),
                    job_id: Some(job_id.0),
                    operation: Some(operation.into()),
                    code: None,
                    message: None,
                    payload: Value::Null,
                },
            );
            job_id
        });
        let inner = self.inner.clone();
        thread::Builder::new()
            .name(format!("reborn-{operation}"))
            .spawn(move || work(job_id, cancel, inner))
            .expect("spawn backend job");
        job_id
    }

    fn finish_ok(inner: &mut BackendInner, job_id: JobId, operation: &str, payload: Value) {
        inner.jobs.remove(&job_id);
        Self::push_event(
            inner,
            BackendEvent {
                kind: "job_completed".into(),
                job_id: Some(job_id.0),
                operation: Some(operation.into()),
                code: None,
                message: None,
                payload,
            },
        );
    }

    fn finish_err(inner: &mut BackendInner, job_id: JobId, operation: &str, error: io::Error) {
        inner.jobs.remove(&job_id);
        let cancelled = error.kind() == io::ErrorKind::Interrupted;
        Self::push_event(
            inner,
            BackendEvent {
                kind: if cancelled {
                    "job_cancelled".into()
                } else {
                    "job_failed".into()
                },
                job_id: Some(job_id.0),
                operation: Some(operation.into()),
                code: Some(error_code(&error).into()),
                message: Some(error.to_string()),
                payload: Value::Null,
            },
        );
    }

    pub fn launch_async(&self, path: PathBuf) -> JobId {
        self.spawn_job("launch", move |job_id, cancel, inner| {
            let result = (|| {
                if cancel.load(Ordering::Relaxed) {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "Launch cancelled.",
                    ));
                }
                let session = process::launch(&path)?;
                let mut guard = inner.lock().expect("backend lock");
                let session_id = Backend::store_session(&mut guard, session.clone());
                Ok(serde_json::json!({
                    "sessionId": session_id.0,
                    "pid": session.pid,
                    "created": session.created,
                    "path": session.path.display().to_string(),
                }))
            })();
            let mut guard = inner.lock().expect("backend lock");
            match result {
                Ok(payload) => Backend::finish_ok(&mut guard, job_id, "launch", payload),
                Err(error) => Backend::finish_err(&mut guard, job_id, "launch", error),
            }
        })
    }

    pub fn stop_async(&self, session_id: SessionId) -> JobId {
        let session = match self.session(session_id) {
            Ok(session) => session,
            Err(error) => {
                return self.spawn_job("stop", move |job_id, _, inner| {
                    let mut guard = inner.lock().expect("backend lock");
                    Backend::finish_err(&mut guard, job_id, "stop", error);
                });
            }
        };
        self.spawn_job("stop", move |job_id, cancel, inner| {
            let result = (|| {
                if cancel.load(Ordering::Relaxed) {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "Stop cancelled.",
                    ));
                }
                process::stop(&session)?;
                Ok(Value::Null)
            })();
            let mut guard = inner.lock().expect("backend lock");
            match result {
                Ok(payload) => {
                    guard.sessions.remove(&session_id);
                    guard
                        .proofs
                        .retain(|_, proof| proof.stats.session_ref() != &session);
                    Backend::finish_ok(&mut guard, job_id, "stop", payload);
                }
                Err(error) => Backend::finish_err(&mut guard, job_id, "stop", error),
            }
        })
    }

    pub fn find_async(&self, session_id: SessionId) -> JobId {
        let session = match self.session(session_id) {
            Ok(session) => session,
            Err(error) => {
                return self.spawn_job("find", move |job_id, _, inner| {
                    let mut guard = inner.lock().expect("backend lock");
                    Backend::finish_err(&mut guard, job_id, "find", error);
                });
            }
        };
        self.spawn_job("find", move |job_id, cancel, inner| {
            let result = hunt::find(&session, &cancel).map(|snapshot| {
                let mut guard = inner.lock().expect("backend lock");
                guard
                    .proofs
                    .retain(|_, proof| proof.stats.session_ref() != &session);
                let proof_id = snapshot.stats.as_ref().map(|stats| {
                    let id = ProofId(Backend::next_id(&mut guard));
                    guard.proofs.insert(
                        id,
                        ProofEntry {
                            stats: stats.clone(),
                        },
                    );
                    id.0
                });
                snapshot_dto(&snapshot, proof_id)
            });
            let mut guard = inner.lock().expect("backend lock");
            match result {
                Ok(payload) => Backend::finish_ok(&mut guard, job_id, "find", payload),
                Err(error) => Backend::finish_err(&mut guard, job_id, "find", error),
            }
        })
    }

    pub fn apply_async(&self, proof_id: ProofId, targets: hunt::Targets) -> JobId {
        let proof = self.with_mut(|inner| {
            inner
                .proofs
                .get(&proof_id)
                .map(|entry| entry.stats.clone())
                .ok_or_else(|| io::Error::other("Unknown or expired proof handle."))
        });
        let proof = match proof {
            Ok(proof) => proof,
            Err(error) => {
                return self.spawn_job("apply", move |job_id, _, inner| {
                    let mut guard = inner.lock().expect("backend lock");
                    Backend::finish_err(&mut guard, job_id, "apply", error);
                });
            }
        };
        self.spawn_job("apply", move |job_id, cancel, inner| {
            let result = hunt::apply_stats(&proof, &targets, &cancel).map(|outcome| {
                serde_json::json!({
                    "values": outcome.values,
                    "applied": outcome.applied.into_iter().collect::<Vec<_>>(),
                    "skipped": outcome.skipped,
                })
            });
            let mut guard = inner.lock().expect("backend lock");
            match result {
                Ok(payload) => Backend::finish_ok(&mut guard, job_id, "apply", payload),
                Err(error) => Backend::finish_err(&mut guard, job_id, "apply", error),
            }
        })
    }

    pub fn update_check_async(&self, current: String) -> JobId {
        self.spawn_job("update_check", move |job_id, cancel, inner| {
            let result = (|| {
                if cancel.load(Ordering::Relaxed) {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "Update check cancelled.",
                    ));
                }
                let status = update::check(&current)?;
                Ok(serde_json::to_value(status).unwrap_or(Value::Null))
            })();
            let mut guard = inner.lock().expect("backend lock");
            match result {
                Ok(payload) => Backend::finish_ok(&mut guard, job_id, "update_check", payload),
                Err(error) => Backend::finish_err(&mut guard, job_id, "update_check", error),
            }
        })
    }
}

fn snapshot_dto(snapshot: &hunt::Snapshot, proof_id: Option<u64>) -> Value {
    let character = snapshot.character.as_ref().map(|character| {
        serde_json::json!({
            "name": character.name,
            "count": character.count,
            "index": character.index,
            "healthCandidate": character.health_candidate,
        })
    });
    let layout = match snapshot.stat_layout {
        hunt::MatchStatus::DifferentLayout => "different_layout",
        hunt::MatchStatus::MissingInstructionProof => "missing_instruction_proof",
        hunt::MatchStatus::Verified => "verified",
    };
    serde_json::json!({
        "build": snapshot.build,
        "character": character,
        "statLayout": layout,
        "mappedStats": snapshot.mapped_stats.iter().copied().collect::<Vec<_>>(),
        "values": snapshot.values,
        "proofId": proof_id,
    })
}

fn error_code(error: &io::Error) -> &'static str {
    match error.kind() {
        io::ErrorKind::Interrupted => "cancelled",
        io::ErrorKind::NotFound => "not_found",
        io::ErrorKind::PermissionDenied => "permission_denied",
        io::ErrorKind::InvalidInput => "invalid_input",
        io::ErrorKind::InvalidData => "invalid_data",
        _ => "failed",
    }
}

pub fn parse_targets(value: &Value) -> io::Result<hunt::Targets> {
    let object = value
        .as_object()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Targets must be an object."))?;
    let mut targets = hunt::Targets::new();
    for (key, raw) in object {
        let stat = STATS.iter().find(|stat| stat.key == key).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("Unknown stat {key}."))
        })?;
        let number = raw
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("Invalid target for {key}."),
                )
            })?;
        if !stat.valid_target(number) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{key} target is out of range."),
            ));
        }
        targets.insert(stat.key, number);
    }
    Ok(targets)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_lists_all_stats() {
        let backend = Backend::create();
        let Value::Array(stats) = backend.stats_metadata(false) else {
            panic!("expected array");
        };
        assert_eq!(stats.len(), 17);
        assert_eq!(stats[0]["key"], "damage");
    }

    #[test]
    fn parse_targets_rejects_unknown_keys() {
        let value = serde_json::json!({"nope": 1});
        assert!(parse_targets(&value).is_err());
    }
}
