pub mod bun_manager;
pub mod bundle_resolver;
pub mod hub_manager;
pub mod region;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::Manager;

/// Hub restart exit code — Hub exits with 42 to request a restart with new bundles.
const HUB_RESTART_EXIT_CODE: i32 = 42;

/// Main setup logic: install Bun → run Hub loop (with hot-restart support).
pub async fn run_setup(
    app: tauri::AppHandle,
    shutdown_flag: Arc<AtomicBool>,
) -> Result<(), String> {
    // 1. Ensure Bun is installed
    bun_manager::emit_progress(&app, "bun", "running", "Checking Bun runtime...");
    bun_manager::ensure_bun_installed(&app).await?;
    bun_manager::emit_progress(&app, "bun", "done", "Bun runtime ready");

    // 2. Enter Hub loop — restarts Hub when it exits with code 42 (update restart)
    run_hub_loop(&app, shutdown_flag).await
}

/// Hub restart loop.
///
/// Each iteration:
/// 1. Resolve bundle paths (external hot-update preferred, app built-in fallback)
/// 2. Start Hub process
/// 3. Navigate WebView to localhost:3006
/// 4. Wait for Hub to exit
/// 5. If exit code == 42 → loop again (hot-update restart)
/// 6. Otherwise → break
async fn run_hub_loop(
    app: &tauri::AppHandle,
    shutdown_flag: Arc<AtomicBool>,
) -> Result<(), String> {
    loop {
        if shutdown_flag.load(Ordering::Relaxed) {
            return Ok(());
        }

        // Resolve bundles (external preferred, built-in fallback)
        let hub_bundle = bundle_resolver::resolve_bundle(app, "hub-bundle.js")?;
        let cli_bundle = bundle_resolver::resolve_bundle(app, "cli-bundle.js").ok();
        let web_dist = bundle_resolver::resolve_bundle(app, "web-dist")?;
        let catalog = bundle_resolver::resolve_bundle(app, "catalog.json").ok();
        // Desktop version: external bundle version preferred, app version as fallback
        let app_version = app.config().version.clone();
        let version = bundle_resolver::get_external_bundle_version()
            .or(app_version);

        if let Some(ref v) = version {
            bun_manager::emit_progress(
                app,
                "hub",
                "running",
                &format!("Starting Hub (bundle v{})...", v),
            );
        }

        // Start Hub
        let mut child = hub_manager::start_hub(
            app,
            &hub_bundle,
            &web_dist,
            cli_bundle.as_ref(),
            catalog.as_ref(),
            version.as_deref(),
        )
        .await?;

        // Navigate window to Hub
        if let Some(window) = app.get_webview_window("main") {
            let hub_url = format!("http://localhost:{}", hub_manager::HUB_PORT);
            let url: url::Url = hub_url
                .parse()
                .map_err(|e| format!("Invalid URL: {}", e))?;
            let _ = window.navigate(url);
        }

        // Wait for Hub to exit
        let exit_code = hub_manager::wait_for_exit(&mut child, &shutdown_flag).await;

        if shutdown_flag.load(Ordering::Relaxed) {
            // App is closing — kill Hub and exit
            hub_manager::kill_child(&mut child);
            return Ok(());
        }

        match exit_code {
            Some(HUB_RESTART_EXIT_CODE) => {
                // Hot-update restart — loop again with potentially new bundles
                eprintln!("[Tako] Hub exited with code 42, restarting with updated bundles...");
                continue;
            }
            Some(code) => {
                return Err(format!("Hub exited with unexpected code: {}", code));
            }
            None => {
                return Err("Hub process terminated by signal".to_string());
            }
        }
    }
}
