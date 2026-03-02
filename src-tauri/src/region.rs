use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Region {
    Cn,
    Global,
}

/// Detect user's region using multiple IP detection services.
/// Defaults to China (conservative strategy — more CN users).
pub async fn detect_region() -> Region {
    let detectors: Vec<fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<Region>> + Send>>> = vec![
        || Box::pin(detect_by_ip_api()),
        || Box::pin(detect_by_ip_info()),
        || Box::pin(detect_by_ip_sb()),
    ];

    for detector in detectors {
        match tokio::time::timeout(Duration::from_secs(3), detector()).await {
            Ok(Some(region)) => return region,
            _ => continue,
        }
    }

    // Default to China mirror (conservative strategy)
    Region::Cn
}

#[derive(Deserialize)]
struct IpApiResponse {
    #[serde(rename = "countryCode")]
    country_code: Option<String>,
}

async fn detect_by_ip_api() -> Option<Region> {
    let client = Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .ok()?;

    let resp: IpApiResponse = client
        .get("http://ip-api.com/json/?fields=countryCode")
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;

    Some(if resp.country_code.as_deref() == Some("CN") {
        Region::Cn
    } else {
        Region::Global
    })
}

#[derive(Deserialize)]
struct IpInfoResponse {
    country: Option<String>,
}

async fn detect_by_ip_info() -> Option<Region> {
    let client = Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .ok()?;

    let resp: IpInfoResponse = client
        .get("https://ipinfo.io/json")
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;

    Some(if resp.country.as_deref() == Some("CN") {
        Region::Cn
    } else {
        Region::Global
    })
}

#[derive(Deserialize)]
struct IpSbResponse {
    country_code: Option<String>,
}

async fn detect_by_ip_sb() -> Option<Region> {
    let client = Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .ok()?;

    let resp: IpSbResponse = client
        .get("https://api.ip.sb/geoip")
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;

    Some(if resp.country_code.as_deref() == Some("CN") {
        Region::Cn
    } else {
        Region::Global
    })
}

/// Get Bun download URL based on region and platform.
pub fn get_bun_download_url(region: Region, version: &str) -> String {
    let os = if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    };

    let arch = if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x64"
    };

    let target = format!("bun-{}-{}", os, arch);

    match region {
        Region::Cn => format!(
            "https://registry.npmmirror.com/-/binary/bun/bun-v{}/{}.zip",
            version, target
        ),
        Region::Global => format!(
            "https://github.com/oven-sh/bun/releases/download/bun-v{}/{}.zip",
            version, target
        ),
    }
}

/// Default Bun version to install.
pub const DEFAULT_BUN_VERSION: &str = "1.2.5";
