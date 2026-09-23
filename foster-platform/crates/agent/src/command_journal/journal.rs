use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use chrono::Utc;
use foster_protocol::{AgentCommandKind, AgentCommandState, AgentCommandStatus, AgentEvent};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const JOURNAL_VERSION: u16 = 1;
const DEFAULT_MAX_FINISHED: usize = 100;

#[derive(Debug, thiserror::Error)]
pub enum CommandJournalError {
    #[error("command journal lock poisoned")]
    LockPoisoned,
    #[error("failed to read command journal: {0}")]
    Read(#[source] std::io::Error),
    #[error("failed to parse command journal: {0}")]
    Parse(#[source] serde_json::Error),
    #[error("failed to serialize command journal: {0}")]
    Serialize(#[source] serde_json::Error),
    #[error("failed to write command journal: {0}")]
    Write(#[source] std::io::Error),
}

#[derive(Debug, Clone)]
pub enum CommandDecision {
    StartNew,
    AlreadyRunning,
    ReplayFinished(AgentEvent),
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JournalEntry {
    command_id: Uuid,
    execution_key: String,
    kind: AgentCommandKind,
    status: AgentCommandStatus,
    job_id: Option<i64>,
    attempt: Option<i32>,
    session_no: Option<String>,
    updated_at: chrono::DateTime<Utc>,
    terminal_event: Option<AgentEvent>,
}

#[derive(Debug, Serialize, Deserialize)]
struct JournalFile {
    version: u16,
    entries: Vec<JournalEntry>,
}

struct JournalInner {
    path: Option<PathBuf>,
    entries: Vec<JournalEntry>,
    max_finished: usize,
}

#[derive(Clone)]
pub struct CommandJournal {
    inner: Arc<Mutex<JournalInner>>,
}

impl Default for CommandJournal {
    fn default() -> Self {
        Self::in_memory()
    }
}

impl CommandJournal {
    pub fn in_memory() -> Self {
        Self {
            inner: Arc::new(Mutex::new(JournalInner {
                path: None,
                entries: Vec::new(),
                max_finished: DEFAULT_MAX_FINISHED,
            })),
        }
    }

    pub fn open(path: impl Into<PathBuf>) -> Result<Self, CommandJournalError> {
        let path = path.into();
        let mut entries = if path.exists() {
            let content = fs::read_to_string(&path).map_err(CommandJournalError::Read)?;
            if content.trim().is_empty() {
                Vec::new()
            } else {
                serde_json::from_str::<JournalFile>(&content)
                    .map_err(CommandJournalError::Parse)?
                    .entries
            }
        } else {
            Vec::new()
        };

        let now = Utc::now();
        let mut changed = false;
        for entry in &mut entries {
            if entry.status == AgentCommandStatus::Running {
                entry.status = AgentCommandStatus::Interrupted;
                entry.updated_at = now;
                changed = true;
            }
        }

        let journal = Self {
            inner: Arc::new(Mutex::new(JournalInner {
                path: Some(path),
                entries,
                max_finished: DEFAULT_MAX_FINISHED,
            })),
        };

        if changed {
            journal.persist()?;
        }

        Ok(journal)
    }

    pub fn begin_foster(
        &self,
        command_id: Uuid,
        job_id: i64,
        attempt: i32,
    ) -> Result<CommandDecision, CommandJournalError> {
        let execution_key = foster_execution_key(job_id, attempt);
        let mut inner = self.lock()?;

        if let Some(entry) = inner
            .entries
            .iter()
            .find(|entry| entry.command_id == command_id || entry.execution_key == execution_key)
            .cloned()
        {
            return Ok(match entry.status {
                AgentCommandStatus::Running => CommandDecision::AlreadyRunning,
                AgentCommandStatus::Finished => entry
                    .terminal_event
                    .map(CommandDecision::ReplayFinished)
                    .unwrap_or(CommandDecision::Interrupted),
                AgentCommandStatus::Interrupted => CommandDecision::Interrupted,
            });
        }

        inner.entries.push(JournalEntry {
            command_id,
            execution_key,
            kind: AgentCommandKind::Foster,
            status: AgentCommandStatus::Running,
            job_id: Some(job_id),
            attempt: Some(attempt),
            session_no: None,
            updated_at: Utc::now(),
            terminal_event: None,
        });

        prune_entries(&mut inner);
        persist_inner(&inner)?;
        Ok(CommandDecision::StartNew)
    }

    pub fn begin_login(
        &self,
        command_id: Uuid,
        session_no: &str,
    ) -> Result<CommandDecision, CommandJournalError> {
        let execution_key = login_execution_key(session_no);
        let mut inner = self.lock()?;

        if let Some(entry) = inner
            .entries
            .iter()
            .find(|entry| entry.command_id == command_id || entry.execution_key == execution_key)
            .cloned()
        {
            return Ok(match entry.status {
                AgentCommandStatus::Running => CommandDecision::AlreadyRunning,
                AgentCommandStatus::Finished => entry
                    .terminal_event
                    .map(CommandDecision::ReplayFinished)
                    .unwrap_or(CommandDecision::Interrupted),
                AgentCommandStatus::Interrupted => CommandDecision::Interrupted,
            });
        }

        inner.entries.push(JournalEntry {
            command_id,
            execution_key,
            kind: AgentCommandKind::Login,
            status: AgentCommandStatus::Running,
            job_id: None,
            attempt: None,
            session_no: Some(session_no.to_string()),
            updated_at: Utc::now(),
            terminal_event: None,
        });

        prune_entries(&mut inner);
        persist_inner(&inner)?;
        Ok(CommandDecision::StartNew)
    }

    pub fn interrupt_login(&self, session_no: &str) -> Result<(), CommandJournalError> {
        let execution_key = login_execution_key(session_no);
        let mut inner = self.lock()?;

        if let Some(entry) = inner
            .entries
            .iter_mut()
            .find(|entry| entry.execution_key == execution_key)
            && entry.status == AgentCommandStatus::Running
        {
            entry.status = AgentCommandStatus::Interrupted;
            entry.updated_at = Utc::now();
            entry.terminal_event = None;
        }

        persist_inner(&inner)
    }

    pub fn finish(
        &self,
        command_id: Uuid,
        terminal_event: AgentEvent,
    ) -> Result<(), CommandJournalError> {
        let mut inner = self.lock()?;

        if let Some(entry) = inner
            .entries
            .iter_mut()
            .find(|entry| entry.command_id == command_id)
        {
            entry.status = AgentCommandStatus::Finished;
            entry.updated_at = Utc::now();
            entry.terminal_event = Some(terminal_event);
        }

        prune_entries(&mut inner);
        persist_inner(&inner)
    }

    pub fn states(&self) -> Result<Vec<AgentCommandState>, CommandJournalError> {
        let inner = self.lock()?;
        Ok(inner
            .entries
            .iter()
            .map(|entry| AgentCommandState {
                command_id: entry.command_id,
                kind: entry.kind,
                status: entry.status,
                job_id: entry.job_id,
                attempt: entry.attempt,
                session_no: entry.session_no.clone(),
                updated_at: entry.updated_at,
            })
            .collect())
    }

    pub fn finished_events(&self) -> Result<Vec<AgentEvent>, CommandJournalError> {
        let inner = self.lock()?;
        Ok(inner
            .entries
            .iter()
            .filter(|entry| entry.status == AgentCommandStatus::Finished)
            .filter_map(|entry| entry.terminal_event.clone())
            .collect())
    }

    fn persist(&self) -> Result<(), CommandJournalError> {
        let inner = self.lock()?;
        persist_inner(&inner)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, JournalInner>, CommandJournalError> {
        self.inner
            .lock()
            .map_err(|_| CommandJournalError::LockPoisoned)
    }
}

pub fn foster_execution_key(job_id: i64, attempt: i32) -> String {
    format!("foster:{job_id}:{attempt}")
}

pub fn login_execution_key(session_no: &str) -> String {
    format!("login:{session_no}")
}

fn prune_entries(inner: &mut JournalInner) {
    let mut finished = inner
        .entries
        .iter()
        .filter(|entry| entry.status == AgentCommandStatus::Finished)
        .map(|entry| (entry.command_id, entry.updated_at))
        .collect::<Vec<_>>();

    if finished.len() <= inner.max_finished {
        return;
    }

    finished.sort_by_key(|(_, updated_at)| *updated_at);
    let remove_count = finished.len() - inner.max_finished;
    let remove_ids = finished
        .into_iter()
        .take(remove_count)
        .map(|(command_id, _)| command_id)
        .collect::<HashSet<_>>();

    inner
        .entries
        .retain(|entry| !remove_ids.contains(&entry.command_id));
}

fn persist_inner(inner: &JournalInner) -> Result<(), CommandJournalError> {
    let Some(path) = inner.path.as_deref() else {
        return Ok(());
    };

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(CommandJournalError::Write)?;
    }

    let payload = serde_json::to_vec_pretty(&JournalFile {
        version: JOURNAL_VERSION,
        entries: inner.entries.clone(),
    })
    .map_err(CommandJournalError::Serialize)?;

    let temp_path = temp_path(path);
    fs::write(&temp_path, payload).map_err(CommandJournalError::Write)?;

    if let Err(first_error) = fs::rename(&temp_path, path) {
        if path.exists() {
            fs::remove_file(path).map_err(CommandJournalError::Write)?;
            fs::rename(&temp_path, path).map_err(CommandJournalError::Write)?;
        } else {
            return Err(CommandJournalError::Write(first_error));
        }
    }

    Ok(())
}

fn temp_path(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(".tmp");
    PathBuf::from(value)
}

#[cfg(test)]
mod tests {
    use foster_domain::FosterErrorCode;
    use foster_protocol::{AgentCommandStatus, FosterFailed};

    use super::*;

    fn terminal_event(job_id: i64, attempt: i32) -> AgentEvent {
        AgentEvent::FosterFailed(FosterFailed {
            job_id,
            attempt,
            failed_at: Utc::now(),
            error_code: FosterErrorCode::NetworkError,
            message: "test".into(),
            screenshot_url: None,
        })
    }

    #[test]
    fn same_execution_key_dedupes_even_with_different_command_id() {
        let journal = CommandJournal::in_memory();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();

        assert!(matches!(
            journal.begin_foster(first, 7, 2).unwrap(),
            CommandDecision::StartNew
        ));
        assert!(matches!(
            journal.begin_foster(second, 7, 2).unwrap(),
            CommandDecision::AlreadyRunning
        ));
    }

    #[test]
    fn finished_command_replays_cached_terminal_event() {
        let journal = CommandJournal::in_memory();
        let command_id = Uuid::new_v4();

        journal.begin_foster(command_id, 9, 0).unwrap();
        journal.finish(command_id, terminal_event(9, 0)).unwrap();

        assert!(matches!(
            journal.begin_foster(command_id, 9, 0).unwrap(),
            CommandDecision::ReplayFinished(AgentEvent::FosterFailed(_))
        ));
    }

    #[test]
    fn same_login_session_dedupes_even_with_different_command_id() {
        let journal = CommandJournal::in_memory();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();

        assert!(matches!(
            journal.begin_login(first, "LOGIN-1").unwrap(),
            CommandDecision::StartNew
        ));
        assert!(matches!(
            journal.begin_login(second, "LOGIN-1").unwrap(),
            CommandDecision::AlreadyRunning
        ));
    }

    #[test]
    fn cancelled_login_becomes_interrupted() {
        let journal = CommandJournal::in_memory();
        let command_id = Uuid::new_v4();

        journal.begin_login(command_id, "LOGIN-CANCEL").unwrap();
        journal.interrupt_login("LOGIN-CANCEL").unwrap();

        assert!(matches!(
            journal.begin_login(command_id, "LOGIN-CANCEL").unwrap(),
            CommandDecision::Interrupted
        ));
    }

    #[test]
    fn persisted_running_command_becomes_interrupted_after_reload() {
        let path =
            std::env::temp_dir().join(format!("foster-command-journal-{}.json", Uuid::new_v4()));
        let command_id = Uuid::new_v4();

        {
            let journal = CommandJournal::open(&path).unwrap();
            journal.begin_foster(command_id, 11, 3).unwrap();
        }

        let journal = CommandJournal::open(&path).unwrap();
        let states = journal.states().unwrap();

        assert_eq!(states.len(), 1);
        assert_eq!(states[0].command_id, command_id);
        assert_eq!(states[0].status, AgentCommandStatus::Interrupted);
        assert!(matches!(
            journal.begin_foster(command_id, 11, 3).unwrap(),
            CommandDecision::Interrupted
        ));

        let _ = std::fs::remove_file(path);
    }
}
