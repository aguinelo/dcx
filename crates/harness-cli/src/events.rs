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

    pub fn open(directory: &Path, run_id: &str) -> Result<Self, String> {
        fs::create_dir_all(directory)
            .map_err(|error| format!("cannot create event directory: {error}"))?;
        let path = directory.join("events.jsonl");
        let sequence = fs::read_to_string(&path)
            .map(|contents| {
                contents
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .count() as u64
            })
            .unwrap_or(0);
        Ok(Self {
            path,
            run_id: run_id.to_owned(),
            sequence,
        })
    }

    pub fn completed_steps(&self) -> std::collections::BTreeSet<String> {
        self.completed_events()
            .filter_map(|event| event.payload["step_id"].as_str().map(str::to_owned))
            .collect()
    }

    pub fn completed_files(&self) -> std::collections::BTreeSet<String> {
        self.completed_events()
            .flat_map(|event| {
                event.payload["files"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|file| file.as_str().map(str::to_owned))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn completed_events(&self) -> impl Iterator<Item = RunEvent> + '_ {
        fs::read_to_string(&self.path)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str::<RunEvent>(line).ok())
            .filter(|event| {
                event.event_type == "step.completed"
                    && event.payload["approved"].as_bool().unwrap_or(false)
                    && event.payload["merged"].as_bool().unwrap_or(false)
            })
            .collect::<Vec<_>>()
            .into_iter()
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
        log.append(
            "step.completed",
            serde_json::json!({
                "step_id": "implement",
                "approved": true,
                "merged": true,
                "files": ["src/lib.rs"],
            }),
        )
        .expect("second event");
        let lines = fs::read_to_string(log.path()).expect("read log");
        let events = lines
            .lines()
            .map(|line| serde_json::from_str::<RunEvent>(line).expect("valid event"))
            .collect::<Vec<_>>();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].sequence, 1);
        assert_eq!(events[1].event_type, "step.completed");
        let completed = log.completed_steps();
        assert!(completed.contains("implement"));
        assert!(log.completed_files().contains("src/lib.rs"));
        log.append(
            "step.completed",
            serde_json::json!({"step_id": "unmerged", "approved": true, "merged": false}),
        )
        .expect("third event");
        assert!(!log.completed_steps().contains("unmerged"));
        let _ = fs::remove_dir_all(directory);
    }
}
