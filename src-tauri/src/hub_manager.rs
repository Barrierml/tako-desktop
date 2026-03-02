use crate::bun_manager;
use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;
use tauri::Emitter;

/// The port Hub listens on in desktop mode.
pub const HUB_PORT: u16 = 3006;

/// Start the Hub server process using Bun.
/// Returns the Child process handle directly (no longer wrapped in HubState).
pub async fn start_hub(
    app: &AppHandle,
    hub_bundle: &PathBuf,
    web_dist: &PathBuf,
    cli_bundle: Option<&PathBuf>,
    catalog: Option<&PathBuf>,
    desktop_version: Option<&str>,
) -> Result<Child, String> {
    let bun = bun_manager::tako_bun_bin();
    if !bun.exists() {
        return Err("Bun is not installed".to_string());
    }

    emit_progress(app, "hub", "running", "Starting Hub server...");

    let mut cmd = std::process::Command::new(&bun);
    cmd.arg(hub_bundle)
        .env("TAKO_WEB_DIST", web_dist)
        .env("TAKO_LISTEN_PORT", HUB_PORT.to_string())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());

    // Pass CLI bundle path so autoRunner uses it
    if let Some(cli) = cli_bundle {
        cmd.env("TAKO_CLI_BUNDLE", cli);
    }

    // Pass catalog path so marketplace loads from it
    if let Some(cat) = catalog {
        cmd.env("TAKO_CATALOG_PATH", cat);
    }

    // Pass desktop bundle version for update comparisons
    if let Some(ver) = desktop_version {
        cmd.env("TAKO_DESKTOP_VERSION", ver);
    }

    let child = cmd
        .spawn()
        .map_err(|e| format!("Failed to start Hub: {}", e))?;

    // Wait for Hub to become healthy
    let health_url = format!("http://localhost:{}/health", HUB_PORT);
    wait_for_health(&health_url, Duration::from_secs(30)).await?;

    emit_progress(app, "hub", "done", "Hub server ready");
    Ok(child)
}

/// Poll the health endpoint until it responds with 200.
async fn wait_for_health(url: &str, timeout: Duration) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let start = std::time::Instant::now();

    loop {
        if start.elapsed() > timeout {
            return Err("Hub server did not become healthy within timeout".to_string());
        }

        match client.get(url).send().await {
            Ok(resp) if resp.status().is_success() => return Ok(()),
            _ => {}
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Wait for the Hub child process to exit.
/// Returns the exit code (Some(code)) or None if killed by signal.
/// Checks the shutdown_flag to allow early termination.
pub async fn wait_for_exit(child: &mut Child, shutdown_flag: &Arc<AtomicBool>) -> Option<i32> {
    loop {
        if shutdown_flag.load(Ordering::Relaxed) {
            return None;
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                return status.code();
            }
            Ok(None) => {
                // Still running, poll again
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(e) => {
                eprintln!("[Tako] Error waiting for Hub process: {}", e);
                return None;
            }
        }
    }
}

/// Force-kill a child process.
pub fn kill_child(child: &mut Child) {
    let pid = child.id();

    #[cfg(unix)]
    {
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }

    // Wait up to 5 seconds for graceful exit
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < Duration::from_secs(5) => {
                std::thread::sleep(Duration::from_millis(100));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
}

/// Emit a setup-progress event.
fn emit_progress(app: &AppHandle, step: &str, status: &str, message: &str) {
    #[derive(serde::Serialize, Clone)]
    struct SetupProgress {
        step: String,
        status: String,
        message: String,
    }

    let _ = app.emit(
        "setup-progress",
        SetupProgress {
            step: step.to_string(),
            status: status.to_string(),
            message: message.to_string(),
        },
    );
}
