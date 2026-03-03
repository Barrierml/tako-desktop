use crate::bun_manager;
use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;
use tauri::Emitter;

/// The port Hub listens on in desktop mode (separate from CLI's default 19836).
pub const HUB_PORT: u16 = 19837;

/// Kill any existing process listening on the Hub port.
/// Prevents "port already in use" errors from stale processes.
fn kill_existing_on_port(port: u16) {
    #[cfg(unix)]
    {
        // lsof -ti :PORT returns PIDs of processes using the port
        let output = std::process::Command::new("lsof")
            .args(["-ti", &format!(":{}", port)])
            .output();

        if let Ok(output) = output {
            let pids = String::from_utf8_lossy(&output.stdout);
            for line in pids.trim().lines() {
                if let Ok(pid) = line.trim().parse::<i32>() {
                    eprintln!("[Tako] Killing stale process {} on port {}", pid, port);
                    unsafe {
                        libc::kill(pid, libc::SIGTERM);
                    }
                }
            }
            // Brief wait for processes to release the port
            if !pids.trim().is_empty() {
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }

    #[cfg(not(unix))]
    {
        // Windows: netstat + taskkill
        let output = std::process::Command::new("cmd")
            .args(["/C", &format!("netstat -ano | findstr :{}", port)])
            .output();

        if let Ok(output) = output {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                if let Some(pid_str) = line.split_whitespace().last() {
                    if let Ok(_pid) = pid_str.parse::<u32>() {
                        let _ = std::process::Command::new("taskkill")
                            .args(["/F", "/PID", pid_str])
                            .output();
                    }
                }
            }
            if !text.trim().is_empty() {
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }
}

/// Start the Hub server process using Bun.
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

    // Kill any stale process on the port before starting
    kill_existing_on_port(HUB_PORT);

    emit_progress(app, "hub", "running", "Starting Hub server...");

    // Resolve Desktop-specific DB path: ~/.tako/desktop.db
    let home = dirs::home_dir().ok_or("Cannot determine home directory")?;
    let desktop_db = home.join(".tako").join("desktop.db");

    let mut cmd = std::process::Command::new(&bun);
    cmd.arg(hub_bundle)
        .env("TAKO_WEB_DIST", web_dist)
        .env("TAKO_LISTEN_PORT", HUB_PORT.to_string())
        .env("DB_PATH", &desktop_db)
        .env("TAKO_HUB_LOG_NAME", "desktop-hub.log")
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

    // Spawn Hub in its own process group so that killing the group on app quit
    // also terminates all Hub's children (autoRunner, Claude Code, etc.).
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
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

/// Kill the Hub process and all its children (same process group).
pub fn kill_child(child: &mut Child) {
    let pid = child.id();

    #[cfg(unix)]
    {
        // Hub was spawned with process_group(0), so its PGID == its PID.
        // killpg terminates the entire group: Hub + autoRunner + Claude Code, etc.
        unsafe {
            libc::killpg(pid as i32, libc::SIGTERM);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }

    // Wait up to 5 seconds for graceful exit, then force-kill
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < Duration::from_secs(5) => {
                std::thread::sleep(Duration::from_millis(100));
            }
            _ => {
                #[cfg(unix)]
                unsafe {
                    libc::killpg(pid as i32, libc::SIGKILL);
                }
                #[cfg(not(unix))]
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
