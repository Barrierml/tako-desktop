use crate::region::{self, Region};
use std::path::PathBuf;
use tauri::AppHandle;
use tokio::fs;

/// Get the Tako Bun installation directory (~/.tako/bun/).
fn tako_bun_dir() -> PathBuf {
    dirs::home_dir()
        .expect("Cannot determine home directory")
        .join(".tako")
        .join("bun")
}

/// Get the Tako Bun binary path.
pub fn tako_bun_bin() -> PathBuf {
    let bun_name = if cfg!(target_os = "windows") {
        "bun.exe"
    } else {
        "bun"
    };
    tako_bun_dir().join("bin").join(bun_name)
}

/// Check if Tako's dedicated Bun is installed.
pub fn is_bun_installed() -> bool {
    tako_bun_bin().exists()
}

/// Ensure Bun is installed, downloading if necessary.
/// Emits `setup-progress` events to the window.
pub async fn ensure_bun_installed(app: &AppHandle) -> Result<(), String> {
    if is_bun_installed() {
        return Ok(());
    }

    emit_progress(app, "bun", "running", "Detecting region...");

    let region = region::detect_region().await;
    let mirror_label = match region {
        Region::Cn => "China mirror (npmmirror.com)",
        Region::Global => "Global (github.com)",
    };

    emit_progress(
        app,
        "bun",
        "running",
        &format!("Downloading Bun runtime via {}...", mirror_label),
    );

    let bun_dir = tako_bun_dir();
    let bin_dir = bun_dir.join("bin");
    fs::create_dir_all(&bin_dir)
        .await
        .map_err(|e| format!("Failed to create bun directory: {}", e))?;

    let url = region::get_bun_download_url(region, region::DEFAULT_BUN_VERSION);

    // Download zip
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Download failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Download failed with status: {}", response.status()));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    emit_progress(app, "bun", "running", "Extracting Bun runtime...");

    // Write zip to temp file and extract
    let zip_path = bun_dir.join("bun-download.zip");
    fs::write(&zip_path, &bytes)
        .await
        .map_err(|e| format!("Failed to write zip: {}", e))?;

    // Extract using the zip crate (sync, run in blocking task)
    let zip_path_clone = zip_path.clone();
    let bun_dir_clone = bun_dir.clone();
    tokio::task::spawn_blocking(move || extract_zip(&zip_path_clone, &bun_dir_clone))
        .await
        .map_err(|e| format!("Extract task failed: {}", e))?
        .map_err(|e| format!("Extraction failed: {}", e))?;

    // Find extracted bun binary and move to bin/
    let bun_name = if cfg!(target_os = "windows") {
        "bun.exe"
    } else {
        "bun"
    };

    let mut found = false;
    let mut entries = fs::read_dir(&bun_dir)
        .await
        .map_err(|e| format!("Failed to read dir: {}", e))?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("Failed to read entry: {}", e))?
    {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("bun-") && entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false)
        {
            let binary_path = entry.path().join(bun_name);
            if binary_path.exists() {
                let dest = bin_dir.join(bun_name);
                fs::rename(&binary_path, &dest)
                    .await
                    .map_err(|e| format!("Failed to move bun binary: {}", e))?;

                // Set executable permission on Unix
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let perms = std::fs::Permissions::from_mode(0o755);
                    std::fs::set_permissions(&dest, perms)
                        .map_err(|e| format!("Failed to set permissions: {}", e))?;
                }

                // Clean up extracted directory
                let _ = fs::remove_dir_all(entry.path()).await;
                found = true;
                break;
            }
        }
    }

    // Clean up zip
    let _ = fs::remove_file(&zip_path).await;

    if !found {
        return Err("Bun binary not found in downloaded archive".to_string());
    }

    Ok(())
}

/// Extract a zip file to a target directory (blocking).
fn extract_zip(zip_path: &std::path::Path, target_dir: &std::path::Path) -> Result<(), String> {
    let file =
        std::fs::File::open(zip_path).map_err(|e| format!("Failed to open zip: {}", e))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("Failed to read zip: {}", e))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read zip entry: {}", e))?;

        let out_path = target_dir.join(
            entry
                .enclosed_name()
                .ok_or_else(|| "Invalid zip entry name".to_string())?,
        );

        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)
                .map_err(|e| format!("Failed to create dir: {}", e))?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create parent dir: {}", e))?;
            }
            let mut outfile = std::fs::File::create(&out_path)
                .map_err(|e| format!("Failed to create file: {}", e))?;
            std::io::copy(&mut entry, &mut outfile)
                .map_err(|e| format!("Failed to write file: {}", e))?;
        }
    }

    Ok(())
}

/// Emit a setup-progress event.
pub fn emit_progress(app: &AppHandle, step: &str, status: &str, message: &str) {
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

use tauri::Emitter;
