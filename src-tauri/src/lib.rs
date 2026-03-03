pub mod bun_manager;
pub mod bundle_resolver;
pub mod hub_manager;
pub mod region;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::Manager;

/// Shared state holding the Hub child process PID.
/// Updated whenever Hub starts (including restarts). Used to kill Hub on quit.
pub struct HubPidState(pub Mutex<Option<u32>>);

/// Hub restart exit code — Hub exits with 42 to request a restart with new bundles.
const HUB_RESTART_EXIT_CODE: i32 = 42;

/// Maximum consecutive crash restarts before giving up.
const MAX_CRASH_RESTARTS: u32 = 3;

/// Delay before restarting after a crash (seconds).
const CRASH_RESTART_DELAY_SECS: u64 = 2;

/// Main setup logic: install Bun → run Hub loop (with hot-restart support).
pub async fn run_setup(
    app: tauri::AppHandle,
    shutdown_flag: Arc<AtomicBool>,
) -> Result<(), String> {
    // 1. Ensure Bun is installed
    bun_manager::emit_progress(&app, "bun", "running", "Checking Bun runtime...");
    if let Err(e) = bun_manager::ensure_bun_installed(&app).await {
        bun_manager::emit_progress(&app, "bun", "error", &e);
        return Err(e);
    }
    bun_manager::emit_progress(&app, "bun", "done", "Bun runtime ready");

    // 2. Enter Hub loop — restarts Hub on crash or code 42 (update restart)
    if let Err(e) = run_hub_loop(&app, shutdown_flag).await {
        bun_manager::emit_progress(&app, "hub", "error", &e);
        return Err(e);
    }
    Ok(())
}

/// Hub loop with keepalive.
///
/// Each iteration:
/// 1. Resolve bundle paths (external hot-update preferred, app built-in fallback)
/// 2. Start Hub process
/// 3. Navigate WebView to localhost:19837
/// 4. Wait for Hub to exit
/// 5. If exit code == 42 → restart immediately (hot-update)
/// 6. If crash → restart with delay (up to MAX_CRASH_RESTARTS times)
/// 7. If shutdown_flag → clean exit
async fn run_hub_loop(
    app: &tauri::AppHandle,
    shutdown_flag: Arc<AtomicBool>,
) -> Result<(), String> {
    let mut crash_count: u32 = 0;

    loop {
        if shutdown_flag.load(Ordering::Relaxed) {
            return Ok(());
        }

        // Resolve bundles (external preferred, built-in fallback)
        let hub_bundle = bundle_resolver::resolve_bundle(app, "hub-bundle.js")?;
        let cli_bundle = bundle_resolver::resolve_bundle(app, "cli-bundle.js").ok();
        let web_dist = bundle_resolver::resolve_bundle(app, "web-dist")?;
        let catalog = bundle_resolver::resolve_bundle(app, "catalog.json").ok();
        // Desktop version: use external bundle version only if external bundles are active
        let app_version = app.config().version.clone();
        let version = if bundle_resolver::should_use_external(app) {
            bundle_resolver::get_external_bundle_version().or(app_version)
        } else {
            app_version
        };

        if let Some(ref v) = version {
            bun_manager::emit_progress(
                app,
                "hub",
                "running",
                &format!("Starting Hub (bundle v{})...", v),
            );
        }

        // Start Hub
        let child = hub_manager::start_hub(
            app,
            &hub_bundle,
            &web_dist,
            cli_bundle.as_ref(),
            catalog.as_ref(),
            version.as_deref(),
        )
        .await;

        let mut child = match child {
            Ok(c) => {
                crash_count = 0; // Reset on successful start
                // Store PID so CloseRequested handler can kill Hub synchronously on quit
                if let Some(state) = app.try_state::<HubPidState>() {
                    *state.0.lock().unwrap() = Some(c.id());
                }
                c
            }
            Err(e) => {
                crash_count += 1;
                if crash_count > MAX_CRASH_RESTARTS {
                    return Err(format!(
                        "Hub failed to start after {} attempts: {}",
                        MAX_CRASH_RESTARTS, e
                    ));
                }
                eprintln!(
                    "[Tako] Hub failed to start (attempt {}/{}): {}. Retrying in {}s...",
                    crash_count, MAX_CRASH_RESTARTS, e, CRASH_RESTART_DELAY_SECS
                );
                tokio::time::sleep(std::time::Duration::from_secs(CRASH_RESTART_DELAY_SECS)).await;
                continue;
            }
        };

        // Navigate window to Hub
        if let Some(window) = app.get_webview_window("main") {
            let hub_url = format!("http://localhost:{}", hub_manager::HUB_PORT);
            let url: url::Url = hub_url
                .parse()
                .map_err(|e| format!("Invalid URL: {}", e))?;
            let _ = window.navigate(url);
        }

        // Wait for Hub to exit
        let exit_code =
            hub_manager::wait_for_exit(&mut child, &shutdown_flag).await;

        if shutdown_flag.load(Ordering::Relaxed) {
            hub_manager::kill_child(&mut child);
            return Ok(());
        }

        match exit_code {
            Some(HUB_RESTART_EXIT_CODE) => {
                crash_count = 0;
                eprintln!(
                    "[Tako] Hub exited with code 42, restarting with updated bundles..."
                );
                continue;
            }
            Some(code) => {
                crash_count += 1;
                if crash_count > MAX_CRASH_RESTARTS {
                    return Err(format!(
                        "Hub crashed {} times (last exit code: {}), giving up",
                        MAX_CRASH_RESTARTS, code
                    ));
                }
                eprintln!(
                    "[Tako] Hub crashed with code {} (attempt {}/{}). Restarting in {}s...",
                    code, crash_count, MAX_CRASH_RESTARTS, CRASH_RESTART_DELAY_SECS
                );
                tokio::time::sleep(std::time::Duration::from_secs(CRASH_RESTART_DELAY_SECS)).await;
                continue;
            }
            None => {
                crash_count += 1;
                if crash_count > MAX_CRASH_RESTARTS {
                    return Err(format!(
                        "Hub terminated by signal {} times, giving up",
                        MAX_CRASH_RESTARTS
                    ));
                }
                eprintln!(
                    "[Tako] Hub terminated by signal (attempt {}/{}). Restarting in {}s...",
                    crash_count, MAX_CRASH_RESTARTS, CRASH_RESTART_DELAY_SECS
                );
                tokio::time::sleep(std::time::Duration::from_secs(CRASH_RESTART_DELAY_SECS)).await;
                continue;
            }
        }
    }
}
