mod capture_manager;

use capture_manager::{CaptureManager, CaptureOptions, CaptureState, CaptureTarget};
use serde::Deserialize;

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
enum CaptureTargetPayload {
    FullDisplay,
    Window { id: String },
}

#[derive(Debug, Deserialize)]
struct StartCapturePayload {
    #[serde(default = "CaptureOptions::default_chunk_ms")]
    chunk_duration_ms: u64,
    #[serde(default)]
    capture_mic: bool,
    #[serde(default)]
    debug_save: bool,
    #[serde(default = "CaptureTargetPayload::default_full_display")]
    target: CaptureTargetPayload,
}

impl CaptureTargetPayload {
    fn default_full_display() -> Self {
        CaptureTargetPayload::FullDisplay
    }

    fn into_target(self) -> CaptureTarget {
        match self {
            CaptureTargetPayload::FullDisplay => CaptureTarget::FullDisplay,
            CaptureTargetPayload::Window { id } => CaptureTarget::Window { id },
        }
    }
}

impl From<StartCapturePayload> for CaptureOptions {
    fn from(payload: StartCapturePayload) -> Self {
        CaptureOptions {
            chunk_duration_ms: payload.chunk_duration_ms,
            capture_mic: payload.capture_mic,
            debug_save: payload.debug_save,
            target: payload.target.into_target(),
        }
    }
}

#[tauri::command]
fn start_capture(
    manager: tauri::State<CaptureManager>,
    payload: StartCapturePayload,
) -> Result<(), String> {
    manager
        .start_capture(payload.into())
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn stop_capture(manager: tauri::State<CaptureManager>) -> Result<(), String> {
    manager.stop_capture().map_err(|err| err.to_string())
}

#[tauri::command]
fn capture_status(manager: tauri::State<CaptureManager>) -> CaptureState {
    manager.status()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(CaptureManager::default())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            greet,
            start_capture,
            stop_capture,
            capture_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
