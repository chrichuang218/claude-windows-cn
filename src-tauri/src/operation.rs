use crate::{assistant, claude, patch, self_update};
use serde::Serialize;
use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationSnapshot {
    pub id: u64,
    pub kind: String,
    pub step: String,
    pub progress: Option<u8>,
    pub logs: Vec<String>,
    pub state: String,
    pub message: String,
    pub update_available: Option<bool>,
    pub latest_version: Option<String>,
}

impl Default for OperationSnapshot {
    fn default() -> Self {
        Self {
            id: 0,
            kind: String::new(),
            step: String::new(),
            progress: None,
            logs: Vec::new(),
            state: "idle".into(),
            message: String::new(),
            update_available: None,
            latest_version: None,
        }
    }
}

#[derive(Clone)]
pub struct OperationState(Arc<Mutex<OperationSnapshot>>);

pub struct OperationOutcome {
    pub message: String,
    pub update_available: Option<bool>,
    pub latest_version: Option<String>,
}

impl OperationOutcome {
    pub fn done(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            update_available: None,
            latest_version: None,
        }
    }

    pub fn update(message: impl Into<String>, available: bool, version: String) -> Self {
        Self {
            message: message.into(),
            update_available: Some(available),
            latest_version: Some(version),
        }
    }
}

impl OperationState {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(OperationSnapshot::default())))
    }

    fn lock(&self) -> MutexGuard<'_, OperationSnapshot> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn snapshot(&self) -> OperationSnapshot {
        self.lock().clone()
    }

    pub fn start(&self, kind: &str, mode: Option<&str>) -> Result<OperationSnapshot, String> {
        const KINDS: &[&str] = &[
            "install_assistant",
            "install_claude",
            "check_claude_update",
            "update_claude",
            "apply_patch",
            "restore_patch",
            "create_claude_shortcut",
            "check_assistant_update",
            "install_assistant_update",
            "uninstall_assistant",
        ];
        if !KINDS.contains(&kind) {
            return Err(format!("不支持的操作：{kind}"));
        }
        if kind == "apply_patch" && !matches!(mode, Some("safe" | "full")) {
            return Err("请选择 Cowork 兼容或完整汉化模式。".into());
        }
        let config = if kind == "install_assistant" {
            Some(assistant::load_config()?)
        } else {
            None
        };
        let mut current = self.lock();
        if current.state == "running" {
            return Err("已有操作正在执行，请等待完成。".into());
        }
        let id = current.id.wrapping_add(1);
        *current = OperationSnapshot {
            id,
            kind: kind.into(),
            step: "准备中".into(),
            state: "running".into(),
            message: "操作已开始。".into(),
            ..OperationSnapshot::default()
        };
        let started = current.clone();
        drop(current);

        let worker = self.clone();
        let kind = kind.to_string();
        let mode = mode.map(str::to_string);
        std::thread::spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(|| match kind.as_str() {
                "install_assistant" => {
                    assistant::install(config.expect("validated config"), &worker)
                }
                "uninstall_assistant" => assistant::uninstall(&worker),
                "install_claude" => claude::install(&worker, false),
                "update_claude" => claude::install(&worker, true),
                "check_claude_update" => claude::check_update(&worker),
                "apply_patch" => patch::apply(mode.as_deref().unwrap_or(""), &worker),
                "restore_patch" => patch::restore(&worker),
                "create_claude_shortcut" => claude::create_shortcut(&worker),
                "check_assistant_update" => self_update::check(&worker),
                "install_assistant_update" => self_update::install(&worker),
                _ => unreachable!(),
            }))
            .unwrap_or_else(|_| Err("后台操作意外中断，请查看执行日志并重试。".into()));
            worker.finish(id, result);
        });
        Ok(started)
    }

    pub fn step(&self, step: &str) {
        let mut current = self.lock();
        current.step = step.into();
        current.progress = None;
        current.logs.push(step.into());
        trim_logs(&mut current.logs);
    }

    pub fn download_progress(&self, downloaded: u64, total: Option<u64>) {
        let mut current = self.lock();
        let mib = 1024.0 * 1024.0;
        let package = if cfg!(target_os = "macos") {
            "官方 macOS 应用"
        } else {
            "官方 x64 MSIX"
        };
        current.step = match total.filter(|total| *total > 0) {
            Some(total) => {
                current.progress = Some(((downloaded.saturating_mul(100) / total).min(100)) as u8);
                format!(
                    "下载{package}：{:.1} / {:.1} MiB",
                    downloaded as f64 / mib,
                    total as f64 / mib
                )
            }
            None => {
                current.progress = None;
                format!("下载{package}：已下载 {:.1} MiB", downloaded as f64 / mib)
            }
        };
    }

    pub fn log(&self, log: impl AsRef<str>) {
        let mut current = self.lock();
        for line in log.as_ref().lines() {
            current.logs.push(line.into());
        }
        trim_logs(&mut current.logs);
    }

    fn finish(&self, id: u64, result: Result<OperationOutcome, String>) {
        let mut current = self.lock();
        if current.id != id {
            return;
        }
        match result {
            Ok(outcome) => {
                current.state = "success".into();
                current.step = "已完成".into();
                current.message = outcome.message;
                current.update_available = outcome.update_available;
                current.latest_version = outcome.latest_version;
                let message = current.message.clone();
                current.logs.push(message);
            }
            Err(error) => {
                current.state = "error".into();
                current.step = "失败".into();
                current.message = error;
                let message = current.message.clone();
                current.logs.push(message);
            }
        }
        trim_logs(&mut current.logs);
    }
}

fn trim_logs(logs: &mut Vec<String>) {
    if logs.len() > 500 {
        logs.drain(..logs.len() - 500);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_invalid_operation_without_changing_state() {
        let state = OperationState::new();
        assert!(state.start("unknown", None).is_err());
        assert_eq!(state.snapshot().state, "idle");
        assert!(state.start("apply_patch", Some("invalid")).is_err());
        assert_eq!(state.snapshot().state, "idle");
    }

    #[test]
    fn poisoned_lock_does_not_hide_operation_failure() {
        let state = OperationState::new();
        let worker = state.clone();
        assert!(std::thread::spawn(move || {
            let _guard = worker.0.lock().unwrap();
            panic!("simulate unexpected worker panic");
        })
        .join()
        .is_err());

        state.finish(0, Err("后台操作意外中断。".into()));
        let snapshot = state.snapshot();
        assert_eq!(snapshot.state, "error");
        assert_eq!(snapshot.message, "后台操作意外中断。");
    }
}
