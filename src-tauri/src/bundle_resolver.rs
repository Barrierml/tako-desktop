use std::path::PathBuf;
use tauri::Manager;

/// Directory where hot-updated bundles are installed via npm.
/// ~/.tako/desktop-bundles/node_modules/tako-desktop-bundles/
fn external_bundle_dir() -> PathBuf {
    dirs::home_dir()
        .expect("Cannot determine home directory")
        .join(".tako")
        .join("desktop-bundles")
        .join("node_modules")
        .join("tako-desktop-bundles")
}

/// Whether to skip external (hot-updated) bundles and use built-in only.
///
/// - Debug builds (`cargo tauri dev`) always skip external bundles.
/// - Release builds can opt-in via `TAKO_DEV_BUNDLES=1` for local testing.
fn should_skip_external() -> bool {
    if cfg!(debug_assertions) {
        return true;
    }
    std::env::var("TAKO_DEV_BUNDLES").map_or(false, |v| v == "1")
}

/// Read the version from the external bundle's package.json (internal helper).
fn read_external_version() -> Option<String> {
    let pkg_path = external_bundle_dir().join("package.json");
    let content = std::fs::read_to_string(&pkg_path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
    parsed.get("version")?.as_str().map(|s| s.to_string())
}

/// Read the version from the external bundle's package.json.
pub fn get_external_bundle_version() -> Option<String> {
    if should_skip_external() {
        return None;
    }
    read_external_version()
}

/// Compare two semver strings. Returns true if a >= b.
fn is_version_gte(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u32> {
        s.split('.')
            .map(|p| p.parse::<u32>().unwrap_or(0))
            .collect()
    };
    let va = parse(a);
    let vb = parse(b);
    for i in 0..3 {
        let x = va.get(i).copied().unwrap_or(0);
        let y = vb.get(i).copied().unwrap_or(0);
        if x > y {
            return true;
        }
        if x < y {
            return false;
        }
    }
    true // equal
}

/// Check whether external (hot-updated) bundles should be preferred over built-in ones.
///
/// Returns true only when:
/// - External bundles are not skipped (release build, no TAKO_DEV_BUNDLES)
/// - External bundle has a readable version
/// - External version >= app's built-in version
pub fn should_use_external(app: &tauri::AppHandle) -> bool {
    if should_skip_external() {
        return false;
    }
    let ext_ver = match read_external_version() {
        Some(v) => v,
        None => return false,
    };
    match app.config().version.as_deref() {
        Some(app_ver) => {
            let use_ext = is_version_gte(&ext_ver, app_ver);
            if !use_ext {
                eprintln!(
                    "[Tako] External bundle v{} < built-in v{}, using built-in",
                    ext_ver, app_ver
                );
            }
            use_ext
        }
        None => true, // no built-in version to compare, use external
    }
}

/// Resolve a bundle file by name.
///
/// Priority (when external bundles are enabled and version >= built-in):
/// 1. External bundle at ~/.tako/desktop-bundles/node_modules/tako-desktop-bundles/{name}
///    (validated: exists + size > 0 for JS, contains index.html for web-dist)
/// 2. Fallback to Tauri's built-in resources/{name}
///
/// In dev mode (debug builds or TAKO_DEV_BUNDLES=1), external bundles are skipped
/// so the app always uses the freshly-built built-in bundles.
pub fn resolve_bundle(app: &tauri::AppHandle, name: &str) -> Result<PathBuf, String> {
    if should_use_external(app) {
        let external = external_bundle_dir().join(name);
        if validate_bundle(&external, name) {
            return Ok(external);
        }
    }

    // Fallback to app-bundled resources
    let resource = app
        .path()
        .resource_dir()
        .map(|dir| dir.join("resources").join(name))
        .map_err(|e| format!("Cannot resolve resource dir: {}", e))?;

    if resource.exists() {
        return Ok(resource);
    }

    Err(format!(
        "Bundle '{}' not found in external or built-in resources",
        name
    ))
}

/// Validate that a bundle path is usable.
fn validate_bundle(path: &PathBuf, name: &str) -> bool {
    if !path.exists() {
        return false;
    }

    if name == "web-dist" {
        // Directory must contain index.html
        return path.join("index.html").exists();
    }

    // JS files: must exist and be non-empty
    if let Ok(meta) = std::fs::metadata(path) {
        return meta.len() > 0;
    }

    false
}
