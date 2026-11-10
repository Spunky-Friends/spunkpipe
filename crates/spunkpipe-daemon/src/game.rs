use std::sync::Mutex;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

/// Answers "is the game running right now?".
pub trait GameProbe: Send + Sync {
    fn running(&self) -> bool;
}

pub struct NeverRunning;

impl GameProbe for NeverRunning {
    fn running(&self) -> bool {
        false
    }
}

/// Looks for the game's processes by name.
pub struct ProcessProbe {
    names: Vec<String>,
    sys: Mutex<System>,
}

impl ProcessProbe {
    pub fn new(names: &[String]) -> Self {
        ProcessProbe { names: names.iter().map(|n| n.to_lowercase()).collect(), sys: Mutex::new(System::new()) }
    }

    fn matches(&self, name: &str) -> bool {
        let name = name.to_lowercase();
        self.names.iter().any(|want| {
            *want == name
                // Linux cuts process names to 15 characters.
                || (name.len() >= 15 && want.starts_with(&name))
        })
    }
}

impl GameProbe for ProcessProbe {
    fn running(&self) -> bool {
        let mut sys = self.sys.lock().unwrap();
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
        );
        sys.processes().values().any(|p| {
            self.matches(&p.name().to_string_lossy())
                || p.exe()
                    .and_then(|e| e.file_name())
                    .is_some_and(|n| self.matches(&n.to_string_lossy()))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_matching() {
        let p = ProcessProbe::new(&["7DaysToDie.x86_64".into(), "7DaysToDie.exe".into()]);
        assert!(p.matches("7DaysToDie.exe"));
        assert!(p.matches("7daystodie.x86_"), "linux truncated name");
        assert!(!p.matches("7Days"));
        assert!(!p.matches("spunkpipe"));
    }

    #[test]
    fn detects_nothing_for_an_absent_process() {
        assert!(!ProcessProbe::new(&["definitely-not-running-zzz".into()]).running());
    }
}
