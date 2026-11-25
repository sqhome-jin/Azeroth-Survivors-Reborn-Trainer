use std::{
    io::{self, Read},
    time::Duration,
};

use serde::Deserialize;

use crate::diagnostic_log::event;

pub const PROJECT: &str = "https://github.com/sqhome-jin/Azeroth-Survivors-Reborn-Trainer";
pub const ASSET_NAME: &str = "Azeroth.Survivor.Reborn.Trainer.macos-arm64.zip";
const API: &str =
    "https://api.github.com/repos/sqhome-jin/Azeroth-Survivors-Reborn-Trainer/releases/latest";

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    #[serde(default)]
    assets: Vec<ReleaseAsset>,
}

#[derive(Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseStatus {
    pub tag: String,
    pub newer: bool,
    pub release_url: String,
    pub asset_url: Option<String>,
}

pub fn version(input: &str) -> Option<[u32; 3]> {
    let input = input.strip_prefix('v').unwrap_or(input);
    let mut fields = input.split('.');
    let mut parts = [0; 3];
    for part in &mut parts {
        let field = fields.next()?;
        if field.is_empty() || !field.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *part = field.parse().ok()?;
    }
    (fields.next().is_none()).then_some(parts)
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(45))
        .redirects(0)
        .build()
}

pub fn check(current: &str) -> io::Result<ReleaseStatus> {
    event("update.check_started", format_args!("installed={current}"));
    let response = agent()
        .get(API)
        .set("User-Agent", "AzerothSurvivorRebornTrainer")
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(io::Error::other)?;
    if response.status() != 200 {
        return Err(io::Error::other(
            "GitHub returned an unexpected release response.",
        ));
    }
    let mut body = Vec::new();
    response
        .into_reader()
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut body)?;
    if body.len() > 2 * 1024 * 1024 {
        return Err(io::Error::other("Release response is too large."));
    }
    let status = from_response(serde_json::from_slice(&body)?, current)?;
    event(
        "update.check_finished",
        format_args!(
            "latest={} newer={} asset={}",
            status.tag,
            status.newer,
            status.asset_url.is_some()
        ),
    );
    Ok(status)
}

fn from_response(release: Release, current: &str) -> io::Result<ReleaseStatus> {
    if release.draft || release.prerelease {
        return Err(io::Error::other("Expected a stable published release."));
    }
    let installed =
        version(current).ok_or_else(|| io::Error::other("Invalid installed version."))?;
    let stable =
        version(&release.tag_name).ok_or_else(|| io::Error::other("Invalid release version."))?;
    let newer = stable > installed;
    let asset_url = release
        .assets
        .into_iter()
        .find(|asset| asset.name == ASSET_NAME && (1024..=64 * 1024 * 1024).contains(&asset.size))
        .map(|asset| asset.browser_download_url);
    Ok(ReleaseStatus {
        tag: release.tag_name.clone(),
        newer,
        release_url: format!("{PROJECT}/releases/tag/{}", release.tag_name),
        asset_url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stable(tag: &str) -> Release {
        Release {
            tag_name: tag.into(),
            draft: false,
            prerelease: false,
            assets: vec![],
        }
    }

    #[test]
    fn recognizes_stable_versions_and_macos_asset() {
        let older = from_response(stable("v0.9.19"), "1.0.0").unwrap();
        assert!(!older.newer && older.asset_url.is_none());
        let mut release = stable("v1.1.0");
        release.assets.push(ReleaseAsset {
            name: ASSET_NAME.into(),
            browser_download_url: format!("{PROJECT}/releases/download/v1.1.0/{ASSET_NAME}"),
            size: 4096,
        });
        let status = from_response(release, "1.0.9").unwrap();
        assert!(status.newer);
        assert!(status.asset_url.is_some());
        assert!(version("v1.1.0-alpha").is_none());
    }
}
