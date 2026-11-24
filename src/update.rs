use std::{
    io::{self, Read},
    time::Duration,
};

use serde::Deserialize;
use url::Url;

use crate::diagnostic_log::event;

pub const PROJECT: &str = "https://github.com/nelbin4/Azeroth-Survivors-Reborn-Trainer";
/// Name of the installable release asset for this platform.
#[cfg(windows)]
pub const ASSET_NAME: &str = "Azeroth.Survivor.Reborn.Trainer.exe";
#[cfg(not(windows))]
pub const ASSET_NAME: &str = "Azeroth.Survivor.Reborn.Trainer.macos-arm64.zip";
const API: &str =
    "https://api.github.com/repos/nelbin4/Azeroth-Survivors-Reborn-Trainer/releases/latest";
const MAX_DOWNLOAD: u64 = 64 * 1024 * 1024;

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
    digest: Option<String>,
    size: u64,
}

pub struct ReleaseStatus {
    pub tag: String,
    /// True when the latest stable release is newer than the installed trainer.
    pub newer: bool,
    /// The verified asset that the built-in installer can apply, when supported.
    pub available: Option<Asset>,
}

#[derive(Clone)]
#[cfg_attr(not(windows), allow(dead_code))] // only the Windows installer reads the asset fields
pub struct Asset {
    url: Url,
    digest: String,
    size: u64,
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
            "latest={} available={}",
            status.tag,
            status.available.is_some()
        ),
    );
    Ok(status)
}

fn digest_valid(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
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
    // The built-in installer only replaces the standalone Windows EXE. Other
    // platforms report the newer version and point to the GitHub release page.
    let available = if newer && cfg!(windows) {
        let entry = release
            .assets
            .into_iter()
            .find(|asset| asset.name == ASSET_NAME)
            .ok_or_else(|| io::Error::other("The latest release has no installable asset."))?;
        let expected = format!(
            "{PROJECT}/releases/download/{}/{ASSET_NAME}",
            release.tag_name
        );
        let digest = entry
            .digest
            .as_deref()
            .and_then(|s| s.strip_prefix("sha256:"))
            .filter(|digest| digest_valid(digest))
            .ok_or_else(|| io::Error::other("The release EXE has no valid SHA-256 digest."))?;
        if entry.browser_download_url != expected || !(1024..=MAX_DOWNLOAD).contains(&entry.size) {
            return Err(io::Error::other(
                "The release EXE has invalid download metadata.",
            ));
        }
        Some(Asset {
            url: Url::parse(&expected).map_err(io::Error::other)?,
            digest: digest.into(),
            size: entry.size,
        })
    } else {
        None
    };
    Ok(ReleaseStatus {
        tag: release.tag_name,
        newer,
        available,
    })
}

#[cfg(windows)]
mod installer {
    use std::{
        env, fs,
        io::{self, Read, Write},
        os::windows::process::CommandExt,
        path::{Path, PathBuf},
        process::{Child, Command},
        thread,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use sha2::{Digest, Sha256};
    use url::Url;

    use super::{ASSET_NAME, Asset, MAX_DOWNLOAD, agent, digest_valid};
    use crate::{diagnostic_log::event, process};

    const HELPER: &str = "reborn-update.exe";
    const PAYLOAD: &str = "verified-update.exe";
    const COMPLETE: &str = "complete";
    const READY: &str = "ready";

    fn trusted_download(url: &Url) -> bool {
        url.scheme() == "https"
            && url.port().is_none()
            && url.username().is_empty()
            && url.password().is_none()
            && url.host_str().is_some_and(|host| {
                host == "github.com" || host.ends_with(".githubusercontent.com")
            })
    }

    fn download(asset: &Asset, destination: &Path) -> io::Result<()> {
        let agent = agent();
        let mut url = asset.url.clone();
        for _ in 0..=5 {
            if !trusted_download(&url) {
                return Err(io::Error::other("Unexpected update download redirect."));
            }
            let response = agent
                .get(url.as_str())
                .set("User-Agent", "AzerothSurvivorRebornTrainer")
                .call()
                .map_err(io::Error::other)?;
            if matches!(response.status(), 301 | 302 | 303 | 307 | 308) {
                let next = response.header("Location").ok_or_else(|| {
                    io::Error::other("Update redirect did not specify a location.")
                })?;
                url = url.join(next).map_err(io::Error::other)?;
                continue;
            }
            if response.status() != 200 {
                return Err(io::Error::other(
                    "Unexpected response while downloading the update.",
                ));
            }
            let mut input = response.into_reader().take(asset.size + 1);
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(destination)?;
            let mut hash = Sha256::new();
            let mut buffer = [0; 64 * 1024];
            let mut total = 0u64;
            loop {
                let read = input.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                total += read as u64;
                if total > asset.size {
                    return Err(io::Error::other("Update exceeds its declared size."));
                }
                hash.update(&buffer[..read]);
                output.write_all(&buffer[..read])?;
            }
            if total != asset.size || format!("{:x}", hash.finalize()) != asset.digest {
                return Err(io::Error::other("Update SHA-256 verification failed."));
            }
            output.sync_all()?;
            return verify_file(destination, &asset.digest);
        }
        Err(io::Error::other("Too many update redirects."))
    }

    fn verify_file(path: &Path, digest: &str) -> io::Result<()> {
        if !digest_valid(digest) || !(1024..=MAX_DOWNLOAD).contains(&fs::metadata(path)?.len()) {
            return Err(io::Error::other("Invalid staged update."));
        }
        let file = fs::read(path)?;
        let pe = file
            .get(60..64)
            .map(|raw| u32::from_le_bytes(raw.try_into().unwrap()) as usize);
        if file.get(..2) != Some(b"MZ")
            || pe.and_then(|pe| file.get(pe..pe + 6)) != Some(b"PE\0\0\x64\x86")
            || format!("{:x}", Sha256::digest(&file)) != digest
        {
            return Err(io::Error::other(
                "Staged update is not the verified Windows x64 EXE.",
            ));
        }
        Ok(())
    }

    fn staging(target: &Path) -> io::Result<PathBuf> {
        let root = target
            .parent()
            .ok_or_else(|| io::Error::other("No installation folder."))?
            .join(".updates");
        fs::create_dir_all(&root)?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let stage = root.join(format!("stage-{}-{stamp}", std::process::id()));
        fs::create_dir(&stage)?;
        Ok(stage)
    }

    fn launch_helper(
        target: &Path,
        stage: &Path,
        digest: &str,
        parent_pid: u32,
    ) -> io::Result<Child> {
        let helper = stage.join(HELPER);
        fs::copy(env::current_exe()?, &helper)?;
        Command::new(helper)
            .arg("--install-update")
            .arg(target)
            .arg(digest)
            .arg(parent_pid.to_string())
            .arg(process::current_creation_time()?.to_string())
            .creation_flags(0x08000000)
            .spawn()
    }

    pub fn stage_and_launch(asset: &Asset) -> io::Result<()> {
        let target = env::current_exe()?.canonicalize()?;
        let stage = staging(&target)?;
        event(
            "update.download_started",
            format_args!("bytes={}", asset.size),
        );
        let result = download(asset, &stage.join(PAYLOAD)).and_then(|()| {
            launch_helper(&target, &stage, &asset.digest, std::process::id()).map(|_| ())
        });
        if result.is_err() {
            discard_stage(&stage);
        }
        result
    }

    fn discard_stage(stage: &Path) {
        for file in [PAYLOAD, COMPLETE, HELPER, READY] {
            let _ = fs::remove_file(stage.join(file));
        }
        let _ = fs::remove_dir(stage);
    }

    pub fn cleanup() {
        let Ok(target) = env::current_exe() else {
            return;
        };
        let Some(parent) = target.parent() else {
            return;
        };
        let Ok(entries) = fs::read_dir(parent.join(".updates")) else {
            return;
        };
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|kind| kind.is_dir())
                && entry.file_name().to_string_lossy().starts_with("stage-")
                && entry.path().join(COMPLETE).is_file()
            {
                discard_stage(&entry.path());
            }
        }
    }

    fn reopen(target: &Path, ready: Option<&Path>, smoke: Option<&str>) -> io::Result<Child> {
        let mut child = Command::new(target);
        child.current_dir(target.parent().expect("installer target has parent"));
        if let Some(mode) = smoke {
            child.arg("--smoke-test");
            if mode == "ready"
                && let Some(ready) = ready
            {
                child.arg(ready);
            }
        } else if let Some(ready) = ready {
            child.arg("--update-ready").arg(ready);
        }
        child.spawn()
    }

    pub fn confirm_ready(path: &Path) -> io::Result<()> {
        let target = env::current_exe()?.canonicalize()?;
        let stage = path.parent().and_then(|stage| stage.canonicalize().ok());
        let stage = stage
            .filter(|stage| {
                stage
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("stage-"))
                    && stage.parent().and_then(Path::parent) == target.parent()
                    && stage
                        .parent()
                        .and_then(Path::file_name)
                        .is_some_and(|name| name == ".updates")
                    && path.file_name().is_some_and(|name| name == READY)
            })
            .ok_or_else(|| io::Error::other("Invalid update startup marker."))?;
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(stage.join(READY))?;
        Ok(())
    }

    fn wait_until_ready(child: &mut Child, ready: &Path) -> io::Result<()> {
        for _ in 0..80 {
            if ready.try_exists()? {
                return Ok(());
            }
            if child.try_wait()?.is_some() {
                if ready.try_exists()? {
                    return Ok(());
                }
                return Err(io::Error::other(
                    "The updated trainer exited before startup.",
                ));
            }
            thread::sleep(Duration::from_millis(250));
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "The updated trainer did not finish starting.",
        ))
    }

    fn replace_and_reopen(
        target: &Path,
        payload: &Path,
        ready: &Path,
        smoke: Option<&str>,
    ) -> io::Result<()> {
        let backup = target.with_extension("previous.exe");
        fs::rename(target, &backup)?;
        let result = (|| -> io::Result<()> {
            fs::rename(payload, target)?;
            let mut child = reopen(target, Some(ready), smoke)?;
            let startup = wait_until_ready(&mut child, ready);
            if startup.is_err() {
                if child.kill().is_ok() {
                    let _ = child.wait();
                }
            } else if smoke.is_some() {
                child.wait()?;
            }
            startup
        })();
        if let Err(error) = result {
            fs::rename(&backup, target).map_err(|restore| {
                io::Error::other(format!(
                    "{error} Recovery from {} failed: {restore}",
                    backup.display()
                ))
            })?;
            return Err(error);
        }
        Ok(())
    }

    pub fn install(target: &Path, digest: &str, parent_pid: u32, created: u64) -> io::Result<()> {
        let target = target.canonicalize()?;
        let helper = env::current_exe()?.canonicalize()?;
        let stage = helper.parent().expect("installer helper has parent");
        let root = target.parent().expect("canonical path has parent");
        if helper.file_name().is_none_or(|name| name != HELPER)
            || stage.parent() != Some(root.join(".updates").as_path())
            || stage
                .file_name()
                .is_none_or(|name| !name.to_string_lossy().starts_with("stage-"))
        {
            return Err(io::Error::other(
                "Installer stage does not match the target folder.",
            ));
        }
        if let Err(error) = process::wait_for_update_parent(parent_pid, created, &target) {
            let _ = fs::write(stage.join(COMPLETE), b"");
            return Err(error);
        }
        let smoke = (parent_pid == 0)
            .then(|| env::var("REBORN_UPDATE_SMOKE").ok())
            .flatten();
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.join(".updates").join("install.lock"))?;
        lock.lock()?;
        let payload = stage.join(PAYLOAD);
        let result = if fs::read(&target)? == fs::read(&helper)? {
            verify_file(&payload, digest).and_then(|()| {
                replace_and_reopen(&target, &payload, &stage.join(READY), smoke.as_deref())
            })
        } else {
            let _ = fs::write(stage.join(COMPLETE), b"");
            return Err(io::Error::other(
                "Another updater already changed the installed trainer.",
            ));
        };
        match &result {
            Ok(()) => event("update.installed", format_args!("sha256={digest}")),
            Err(error) => {
                event("update.install_failed", format_args!("{error}"));
                if target.is_file() {
                    match reopen(&target, None, smoke.as_deref()) {
                        Ok(mut child) if smoke.is_some() => {
                            let _ = child.wait();
                        }
                        Ok(_) => {}
                        Err(error) => event("update.restart_failed", format_args!("{error}")),
                    }
                }
            }
        }
        let _ = fs::write(stage.join(COMPLETE), b"");
        result
    }

    pub fn smoke_install() -> io::Result<()> {
        let root = env::temp_dir().join(format!("reborn-update-smoke-{}", std::process::id()));
        fs::create_dir(&root)?;
        let target = root.join(ASSET_NAME);
        let original = fs::read(env::current_exe()?)?;
        fs::write(&target, &original)?;
        let stage = staging(&target)?;
        let mut updated = original.clone();
        updated.push(0);
        fs::write(stage.join(PAYLOAD), &updated)?;
        let digest = format!("{:x}", Sha256::digest(&updated));
        let helper = stage.join(HELPER);
        fs::copy(env::current_exe()?, &helper)?;
        let run = |mode| {
            Command::new(&helper)
                .arg("--install-update")
                .arg(&target)
                .arg(&digest)
                .args(["0", "0"])
                .env("REBORN_UPDATE_SMOKE", mode)
                .status()
        };
        if run("early-exit")?.success() || fs::read(&target)? != original {
            return Err(io::Error::other(
                "Updater did not restore the previous EXE.",
            ));
        }
        fs::remove_file(stage.join(COMPLETE))?;
        fs::write(stage.join(PAYLOAD), &updated)?;
        if !run("ready")?.success()
            || !stage.join(READY).is_file()
            || fs::read(&target)? != updated
            || fs::read(target.with_extension("previous.exe"))? != original
        {
            return Err(io::Error::other(
                "Standalone self-update smoke test failed.",
            ));
        }
        discard_stage(&stage);
        fs::remove_file(target.with_extension("previous.exe"))?;
        fs::remove_file(target)?;
        fs::remove_file(root.join(".updates").join("install.lock"))?;
        fs::remove_dir(root.join(".updates"))?;
        fs::remove_dir(root)?;
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn only_accepts_https_github_download_hosts() {
            for address in [
                "https://github.com/releases",
                "https://release-assets.githubusercontent.com/x",
            ] {
                assert!(trusted_download(&Url::parse(address).unwrap()));
            }
            for address in [
                "http://github.com/releases",
                "https://github.com.example.org/x",
                "https://github.com@example.org/x",
                "https://github.com:8443/x",
            ] {
                assert!(!trusted_download(&Url::parse(address).unwrap()));
            }
        }
    }
}

#[cfg(windows)]
pub use installer::{cleanup, confirm_ready, install, smoke_install, stage_and_launch};

#[cfg(not(windows))]
pub fn stage_and_launch(_asset: &Asset) -> io::Result<()> {
    Err(io::Error::other(
        "Self-update is not available on this platform. Download the release from GitHub.",
    ))
}

#[cfg(not(windows))]
pub fn cleanup() {}

/// Only the Windows installer restarts the trainer with a startup marker.
#[cfg(not(windows))]
pub fn confirm_ready(_path: &std::path::Path) -> io::Result<()> {
    Ok(())
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
    fn recognizes_stable_versions_and_validates_installable_asset() {
        let older = from_response(stable("v0.9.19"), "1.0.0").unwrap();
        assert!(!older.newer && older.available.is_none());
        let same = from_response(stable("v1.0.0"), "1.0.0").unwrap();
        assert!(!same.newer && same.available.is_none());
        let mut release = stable("v1.1.0");
        release.assets.push(ReleaseAsset {
            name: ASSET_NAME.into(),
            browser_download_url: format!("{PROJECT}/releases/download/v1.1.0/{ASSET_NAME}"),
            digest: Some(format!("sha256:{}", "a".repeat(64))),
            size: 4096,
        });
        let status = from_response(release, "1.0.9").unwrap();
        assert!(status.newer);
        // Only the Windows installer can apply the asset; other platforms link to GitHub.
        assert_eq!(status.available.is_some(), cfg!(windows));
        let missing = from_response(stable("v1.1.0"), "1.0.9");
        if cfg!(windows) {
            assert!(missing.is_err());
        } else {
            assert!(missing.unwrap().newer);
        }
        assert!(version("v1.1.0-alpha").is_none());
    }
}
