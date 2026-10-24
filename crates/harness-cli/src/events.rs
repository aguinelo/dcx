use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use harness_artifacts::{RunEvent, SCHEMA_VERSION};

pub struct EventLog {
    path: PathBuf,
    run_id: String,
    sequence: u64,
}

impl EventLog {
    pub fn create(directory: &Path, run_id: &str) -> Result<Self, String> {
        fs::create_dir_all(directory)
            .map_err(|error| format!("cannot create event directory: {error}"))?;
        let path = directory.join("events.jsonl");
        fs::write(&path, b"").map_err(|error| format!("cannot reset event log: {error}"))?;
        Ok(Self {
            path,
            run_id: run_id.to_owned(),
            sequence: 0,
        })
    }

    pub fn append(&mut self, event_type: &str, payload: serde_json::Value) -> Result<(), String> {
        self.sequence += 1;
        let event = RunEvent {
            schema_version: SCHEMA_VERSION,
            run_id: self.run_id.clone(),
            sequence: self.sequence,
            event_type: event_type.to_owned(),
            payload,
        };
        let mut file = OpenOptions::new()
            .append(true)
            .open(&self.path)
            .map_err(|error| format!("cannot open event log: {error}"))?;
        serde_json::to_writer(&mut file, &event)
            .map_err(|error| format!("cannot serialize event: {error}"))?;
        file.write_all(b"\n")
            .map_err(|error| format!("cannot append event: {error}"))
    }

    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn events_are_appended_in_order() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("harness-events-{}-{nanos}", std::process::id()));
        let mut log = EventLog::create(&directory, "run-1").expect("create log");
        log.append("plan.created", serde_json::json!({"steps": 2}))
            .expect("first event");
        log.append("step.completed", serde_json::json!({"step": "implement"}))
            .expect("second event");
        let lines = fs::read_to_string(log.path()).expect("read log");
        let events = lines
            .lines()
            .map(|line| serde_json::from_str::<RunEvent>(line).expect("valid event"))
            .collect::<Vec<_>>();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].sequence, 1);
        assert_eq!(events[1].event_type, "step.completed");
        let _ = fs::remove_dir_all(directory);
    }
}
