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

/// Read the version from the external bundle's package.json.
pub fn get_external_bundle_version() -> Option<String> {
    let pkg_path = external_bundle_dir().join("package.json");
    let content = std::fs::read_to_string(&pkg_path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
    parsed.get("version")?.as_str().map(|s| s.to_string())
}

/// Resolve a bundle file by name.
///
/// Priority:
/// 1. External bundle at ~/.tako/desktop-bundles/node_modules/tako-desktop-bundles/{name}
///    (validated: exists + size > 0 for JS, contains index.html for web-dist)
/// 2. Fallback to Tauri's built-in resources/{name}
pub fn resolve_bundle(app: &tauri::AppHandle, name: &str) -> Result<PathBuf, String> {
    let external = external_bundle_dir().join(name);

    if validate_bundle(&external, name) {
        return Ok(external);
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
