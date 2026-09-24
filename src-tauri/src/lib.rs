mod assistant;
mod claude;
mod operation;
mod patch;
mod self_update;
mod util;

use assistant::{AssistantConfig, AssistantStatus};
use claude::ClaudeStatus;
use operation::{OperationSnapshot, OperationState};

#[tauri::command]
fn get_status() -> Result<ClaudeStatus, String> {
    claude::status()
}

#[tauri::command]
fn get_assistant_status() -> Result<AssistantStatus, String> {
    assistant::status()
}

#[tauri::command]
fn get_config() -> Result<AssistantConfig, String> {
    assistant::load_config()
}

#[tauri::command]
fn save_config(config: AssistantConfig) -> Result<AssistantConfig, String> {
    assistant::save_config(config)
}

#[tauri::command]
fn get_operation_snapshot(state: tauri::State<'_, OperationState>) -> OperationSnapshot {
    state.snapshot()
}

#[tauri::command]
fn start_operation(
    kind: String,
    mode: Option<String>,
    state: tauri::State<'_, OperationState>,
) -> Result<OperationSnapshot, String> {
    state.start(&kind, mode.as_deref())
}

#[tauri::command]
fn open_claude() -> Result<(), String> {
    claude::launch()
}

#[tauri::command]
fn choose_assistant_install_path() -> Result<Option<String>, String> {
    assistant::choose_install_path()
}

pub fn run() {
    if util::run_helper_if_requested() {
        return;
    }
    if std::env::args().any(|arg| arg == "--self-test") {
        std::process::exit(if self_update::self_test() { 0 } else { 1 });
    }
    if std::env::args().any(|arg| arg == "--uninstall-assistant") {
        let state = OperationState::new();
        std::process::exit(if assistant::uninstall(&state).is_ok() {
            0
        } else {
            1
        });
    }

    self_update::start_daily_scheduler();
    tauri::Builder::default()
        .manage(OperationState::new())
        .invoke_handler(tauri::generate_handler![
            get_status,
            get_assistant_status,
            get_config,
            save_config,
            get_operation_snapshot,
            start_operation,
            open_claude,
            choose_assistant_install_path,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run app");
}
