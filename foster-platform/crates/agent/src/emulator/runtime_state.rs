use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use chrono::{DateTime, Utc};
use foster_domain::{EmulatorActivity, EmulatorOccupancyStatus};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct EmulatorRuntimeSnapshot {
    pub occupancy: EmulatorOccupancyStatus,
    pub activity: EmulatorActivity,
    pub activity_stage: Option<String>,
    pub current_command_id: Option<Uuid>,
    pub current_job_id: Option<i64>,
    pub current_login_session_no: Option<String>,
    pub current_game_account_id: Option<i64>,
    pub activity_started_at: Option<DateTime<Utc>>,
}

impl Default for EmulatorRuntimeSnapshot {
    fn default() -> Self {
        Self {
            occupancy: EmulatorOccupancyStatus::Idle,
            activity: EmulatorActivity::None,
            activity_stage: None,
            current_command_id: None,
            current_job_id: None,
            current_login_session_no: None,
            current_game_account_id: None,
            activity_started_at: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EmulatorRuntimeError {
    #[error("emulator {emulator_code} is busy")]
    Busy { emulator_code: String },
}

#[derive(Debug, Clone, Default)]
pub struct EmulatorRuntimeRegistry {
    states: Arc<Mutex<HashMap<String, EmulatorRuntimeSnapshot>>>,
}

impl EmulatorRuntimeRegistry {
    pub fn snapshot(&self, emulator_code: &str) -> EmulatorRuntimeSnapshot {
        self.states
            .lock()
            .expect("emulator runtime registry poisoned")
            .get(emulator_code)
            .cloned()
            .unwrap_or_default()
    }

    pub fn try_acquire(
        &self,
        emulator_code: &str,
        command_id: Uuid,
        activity: EmulatorActivity,
        game_account_id: Option<i64>,
        job_id: Option<i64>,
        session_no: Option<String>,
    ) -> Result<EmulatorRuntimeLease, EmulatorRuntimeError> {
        let mut states = self
            .states
            .lock()
            .expect("emulator runtime registry poisoned");
        let state = states.entry(emulator_code.to_string()).or_default();

        if state.occupancy != EmulatorOccupancyStatus::Idle {
            return Err(EmulatorRuntimeError::Busy {
                emulator_code: emulator_code.to_string(),
            });
        }

        *state = EmulatorRuntimeSnapshot {
            occupancy: EmulatorOccupancyStatus::Busy,
            activity,
            activity_stage: Some("STARTING".to_string()),
            current_command_id: Some(command_id),
            current_job_id: job_id,
            current_login_session_no: session_no,
            current_game_account_id: game_account_id,
            activity_started_at: Some(Utc::now()),
        };

        Ok(EmulatorRuntimeLease {
            registry: self.clone(),
            emulator_code: emulator_code.to_string(),
            command_id,
        })
    }

    fn update_stage(&self, emulator_code: &str, command_id: Uuid, stage: &str) {
        let mut states = self
            .states
            .lock()
            .expect("emulator runtime registry poisoned");
        let Some(state) = states.get_mut(emulator_code) else {
            return;
        };
        if state.current_command_id == Some(command_id) {
            state.activity_stage = Some(stage.to_string());
        }
    }

    fn release(&self, emulator_code: &str, command_id: Uuid) {
        let mut states = self
            .states
            .lock()
            .expect("emulator runtime registry poisoned");
        let Some(state) = states.get_mut(emulator_code) else {
            return;
        };
        if state.current_command_id == Some(command_id) {
            *state = EmulatorRuntimeSnapshot::default();
        }
    }
}

pub struct EmulatorRuntimeLease {
    registry: EmulatorRuntimeRegistry,
    emulator_code: String,
    command_id: Uuid,
}

impl EmulatorRuntimeLease {
    pub fn set_stage(&self, stage: &str) {
        self.registry
            .update_stage(&self.emulator_code, self.command_id, stage);
    }
}

impl Drop for EmulatorRuntimeLease {
    fn drop(&mut self) {
        self.registry
            .release(&self.emulator_code, self.command_id);
    }
}
