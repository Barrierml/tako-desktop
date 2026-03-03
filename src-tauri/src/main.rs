// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{Listener, Manager};
use tako_desktop_lib::HubPidState;

fn main() {
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let shutdown_flag_setup = shutdown_flag.clone();
    let shutdown_flag_event = shutdown_flag.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(HubPidState(Mutex::new(None)))
        .setup(|app| {
            // Create the main window programmatically so we can attach on_navigation.
            // Intercept navigation: localhost stays in-app, everything else opens in the system browser.
            tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App("index.html".into()))
                .title("Tako")
                .inner_size(1280.0, 800.0)
                .on_navigation(|url: &url::Url| {
                    let is_local = matches!(url.host_str(), Some("localhost") | Some("127.0.0.1"))
                        || matches!(url.scheme(), "tauri" | "https+ipc" | "http+ipc")
                        || url.as_str() == "about:blank";

                    if !is_local {
                        #[cfg(target_os = "macos")]
                        let _ = std::process::Command::new("open").arg(url.as_str()).spawn();
                        #[cfg(target_os = "linux")]
                        let _ = std::process::Command::new("xdg-open").arg(url.as_str()).spawn();
                        #[cfg(target_os = "windows")]
                        let _ = std::process::Command::new("cmd").args(["/c", "start", "", url.as_str()]).spawn();
                        return false; // prevent in-app navigation
                    }
                    true
                })
                .build()
                .map_err(|e| format!("Failed to create window: {}", e))?;

            let handle = app.handle().clone();

            // Spawn async setup: wait for frontend-ready → Bun install → Hub loop
            tauri::async_runtime::spawn(async move {
                let flag = shutdown_flag_setup.clone();

                // Wait for the loading page to signal it's ready to receive events.
                // This prevents setup-progress events from being emitted before the
                // JS event listener is registered (race condition on fast machines).
                let (tx, rx) = tokio::sync::oneshot::channel::<()>();
                let tx = std::sync::Mutex::new(Some(tx));
                handle.once("frontend-ready", move |_| {
                    if let Ok(mut guard) = tx.lock() {
                        if let Some(tx) = guard.take() {
                            let _ = tx.send(());
                        }
                    }
                });
                // Timeout fallback: start anyway after 3s in case the event never fires
                tokio::select! {
                    _ = rx => {}
                    _ = tokio::time::sleep(std::time::Duration::from_secs(3)) => {
                        eprintln!("[Tako] frontend-ready timeout, starting setup anyway");
                    }
                }

                if let Err(e) =
                    tako_desktop_lib::run_setup(handle.clone(), shutdown_flag_setup).await
                {
                    // Don't report errors if we're shutting down.
                    // The specific step (bun/hub) already emitted its error state via emit_progress.
                    if !flag.load(Ordering::Relaxed) {
                        eprintln!("[Tako] Setup failed: {}", e);
                    }
                }
            });

            Ok(())
        })
        .on_window_event(move |window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                // Signal the async Hub loop to stop
                shutdown_flag_event.store(true, Ordering::Relaxed);

                // Synchronously kill the Hub process so it doesn't outlive the app.
                // CloseRequested fires before the process exits, giving us time to clean up.
                let pid = window
                    .app_handle()
                    .state::<HubPidState>()
                    .0
                    .lock()
                    .unwrap()
                    .take();

                if let Some(pid) = pid {
                    eprintln!("[Tako] Killing Hub process group (pgid={}) on app quit", pid);
                    // Hub was spawned with process_group(0), so PGID == Hub's PID.
                    // killpg kills Hub + all its children (autoRunner, Claude Code, etc.)
                    #[cfg(unix)]
                    unsafe {
                        libc::killpg(pid as i32, libc::SIGTERM);
                    }
                    #[cfg(not(unix))]
                    {
                        let _ = std::process::Command::new("taskkill")
                            .args(["/F", "/T", "/PID", &pid.to_string()])
                            .output();
                    }
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Tako Desktop");
}
