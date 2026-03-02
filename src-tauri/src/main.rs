// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::Emitter;

fn main() {
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let shutdown_flag_setup = shutdown_flag.clone();
    let shutdown_flag_event = shutdown_flag.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let handle = app.handle().clone();

            // Spawn async setup: Bun install → Hub loop (with hot-restart)
            tauri::async_runtime::spawn(async move {
                let flag = shutdown_flag_setup.clone();
                if let Err(e) =
                    tako_desktop_lib::run_setup(handle.clone(), shutdown_flag_setup).await
                {
                    // Don't report errors if we're shutting down
                    if !flag.load(Ordering::Relaxed) {
                        eprintln!("[Tako] Setup failed: {}", e);

                        #[derive(serde::Serialize, Clone)]
                        struct SetupProgress {
                            step: String,
                            status: String,
                            message: String,
                        }

                        let _ = handle.emit(
                            "setup-progress",
                            SetupProgress {
                                step: "error".to_string(),
                                status: "error".to_string(),
                                message: e,
                            },
                        );
                    }
                }
            });

            Ok(())
        })
        .on_window_event(move |_window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                // Signal the Hub loop to stop
                shutdown_flag_event.store(true, Ordering::Relaxed);
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Tako Desktop");
}
