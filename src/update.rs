//! Self-update from the latest published GitHub release.
//!
//! `releases/latest` only ever returns a published, non-prerelease release.
//! CI creates every release as a draft, so a build nobody has reviewed is
//! never offered here.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

const LATEST_RELEASE: &str =
    "https://api.github.com/repos/krakerz/HyprWindowsRules/releases/latest";
const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// How this copy was installed, which decides what an update replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallKind {
    /// `install.sh` from the release archive: the binary in ~/.local/bin.
    Archive,
    /// An AppImage file.
    AppImage(PathBuf),
    /// Running from a build tree (or installed some other way); updating doesn't apply.
    Source,
}

impl InstallKind {
    pub fn describe(&self) -> &'static str {
        match self {
            InstallKind::Archive => "installed with install.sh",
            InstallKind::AppImage(_) => "AppImage",
            InstallKind::Source => {
                "source build — updates are for installed builds (install.sh or AppImage)"
            }
        }
    }
}

fn installed_binary() -> PathBuf {
    crate::settings::home().join(".local/bin/hypr-rules")
}

pub fn install_kind() -> InstallKind {
    if let Some(appimage) = std::env::var_os("APPIMAGE") {
        return InstallKind::AppImage(PathBuf::from(appimage));
    }
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok());
    if exe.is_some() && exe == installed_binary().canonicalize().ok() {
        return InstallKind::Archive;
    }
    InstallKind::Source
}

pub fn current_version() -> &'static str {
    CURRENT
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub version: String,
    pub asset_name: String,
    asset_url: String,
}

#[derive(Debug, Deserialize)]
struct ReleaseJson {
    tag_name: String,
    #[serde(default)]
    assets: Vec<AssetJson>,
}

#[derive(Debug, Clone, Deserialize)]
struct AssetJson {
    name: String,
    browser_download_url: String,
}

fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(format!("hypr-rules/{CURRENT}"))
        .build()
        .context("couldn't create the HTTP client")
}

/// The newer release for this install, if any.
pub async fn check(kind: InstallKind) -> Result<Option<Update>> {
    let url = std::env::var("HYPR_RULES_UPDATE_URL").unwrap_or_else(|_| LATEST_RELEASE.to_string());
    let release: ReleaseJson = client()?
        .get(&url)
        .send()
        .await
        .context("couldn't reach GitHub")?
        .error_for_status()
        .context("GitHub returned an error")?
        .json()
        .await
        .context("unexpected release data")?;
    Ok(pick_update(&release, &kind, CURRENT))
}

fn pick_update(release: &ReleaseJson, kind: &InstallKind, current: &str) -> Option<Update> {
    let latest = parse_version(&release.tag_name)?;
    if latest <= parse_version(current)? {
        return None;
    }
    let suffix = match kind {
        InstallKind::Archive => "-x86_64.tar.gz",
        InstallKind::AppImage(_) => "-x86_64.AppImage",
        InstallKind::Source => return None,
    };
    let asset = release.assets.iter().find(|a| a.name.ends_with(suffix))?;
    Some(Update {
        version: release.tag_name.trim_start_matches('v').to_string(),
        asset_name: asset.name.clone(),
        asset_url: asset.browser_download_url.clone(),
    })
}

/// `1.2.3` / `v1.2.3` → (1, 2, 3); anything else is ignored.
fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.trim().trim_start_matches('v').splitn(3, '.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts
        .next()?
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()?;
    Some((major, minor, patch))
}

/// How far a download has got, shared with the UI (which polls it).
#[derive(Debug, Default)]
pub struct Progress {
    received: AtomicU64,
    /// 0 until known (the server may not say).
    total: AtomicU64,
}

impl Progress {
    pub fn received(&self) -> u64 {
        self.received.load(Ordering::Relaxed)
    }

    pub fn total(&self) -> Option<u64> {
        Some(self.total.load(Ordering::Relaxed)).filter(|&t| t > 0)
    }

    /// 0..=1, when the size is known.
    pub fn fraction(&self) -> Option<f32> {
        self.total()
            .map(|total| (self.received() as f32 / total as f32).clamp(0.0, 1.0))
    }

    /// "34 % · 3.5 / 10.1 MB", or "3.5 MB" when the size is unknown.
    pub fn describe(&self) -> String {
        let mb = |bytes: u64| bytes as f64 / 1_000_000.0;
        match (self.fraction(), self.total()) {
            (Some(fraction), Some(total)) => {
                format!(
                    "{:.0} % · {:.1} / {:.1} MB",
                    fraction * 100.0,
                    mb(self.received()),
                    mb(total)
                )
            }
            _ => format!("{:.1} MB", mb(self.received())),
        }
    }
}

/// Downloads `update` and swaps it in. The running process keeps its (now
/// unlinked) binary; a restart picks up the new version.
pub async fn apply(kind: InstallKind, update: Update, progress: &Progress) -> Result<()> {
    match kind {
        InstallKind::Archive => apply_archive(&update, progress).await,
        InstallKind::AppImage(path) => apply_appimage(&path, &update, progress).await,
        InstallKind::Source => bail!("updates apply to installed builds only"),
    }
}

/// Streams `url` to `to`, counting into `progress` as it goes.
async fn download(url: &str, to: &Path, progress: &Progress) -> Result<()> {
    use std::io::Write;
    let mut response = client()?
        .get(url)
        .send()
        .await
        .context("download failed")?
        .error_for_status()
        .context("download failed")?;
    progress
        .total
        .store(response.content_length().unwrap_or(0), Ordering::Relaxed);
    progress.received.store(0, Ordering::Relaxed);
    let mut file = std::io::BufWriter::new(
        std::fs::File::create(to).with_context(|| format!("couldn't write {}", to.display()))?,
    );
    while let Some(chunk) = response.chunk().await.context("download interrupted")? {
        file.write_all(&chunk)
            .with_context(|| format!("couldn't write {}", to.display()))?;
        progress
            .received
            .fetch_add(chunk.len() as u64, Ordering::Relaxed);
    }
    file.flush()
        .with_context(|| format!("couldn't write {}", to.display()))
}

/// Unpacks the release archive and runs its own install.sh, so the binary, desktop
/// entry and icon are updated exactly as a manual reinstall would.
async fn apply_archive(update: &Update, progress: &Progress) -> Result<()> {
    let staging = dirs::cache_dir()
        .unwrap_or_else(|| crate::settings::home().join(".cache"))
        .join("hypr-rules/update");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)
        .with_context(|| format!("couldn't create {}", staging.display()))?;
    let result = install_from_archive(update, &staging, progress).await;
    let _ = std::fs::remove_dir_all(&staging);
    result
}

async fn install_from_archive(update: &Update, staging: &Path, progress: &Progress) -> Result<()> {
    let archive = staging.join("update.tar.gz");
    download(&update.asset_url, &archive, progress).await?;
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(staging)
        .status()
        .context("couldn't run tar")?;
    if !status.success() {
        bail!("couldn't unpack {}", update.asset_name);
    }
    let bundle = find_bundle(staging).context("the update archive has no install.sh")?;
    let out = Command::new("bash")
        .arg(bundle.join("install.sh"))
        .output()
        .context("couldn't run install.sh")?;
    if !out.status.success() {
        bail!(
            "install.sh failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// The folder in the unpacked archive holding install.sh and the binary (the archive
/// wraps them in a versioned folder).
fn find_bundle(staging: &Path) -> Option<PathBuf> {
    let is_bundle =
        |dir: &Path| dir.join("install.sh").is_file() && dir.join("hypr-rules").is_file();
    if is_bundle(staging) {
        return Some(staging.to_path_buf());
    }
    std::fs::read_dir(staging)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|dir| is_bundle(dir))
}

async fn apply_appimage(path: &Path, update: &Update, progress: &Progress) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let partial = path.with_extension("AppImage.part");
    download(&update.asset_url, &partial, progress).await?;
    std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o755))?;
    std::fs::rename(&partial, path).context("couldn't replace the AppImage")
}

/// Starts the updated copy; the caller then quits.
pub fn relaunch(kind: &InstallKind) -> Result<()> {
    let program = match kind {
        InstallKind::Archive => installed_binary(),
        InstallKind::AppImage(path) => path.clone(),
        InstallKind::Source => bail!("nothing to relaunch"),
    };
    Command::new(&program)
        .spawn()
        .with_context(|| format!("couldn't start {}", program.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str) -> ReleaseJson {
        let v = tag.trim_start_matches('v');
        ReleaseJson {
            tag_name: tag.into(),
            assets: vec![
                AssetJson {
                    name: format!("hypr-rules-{v}-x86_64.tar.gz"),
                    browser_download_url: "https://x/archive".into(),
                },
                AssetJson {
                    name: format!("hypr-rules-{v}-x86_64.AppImage"),
                    browser_download_url: "https://x/appimage".into(),
                },
            ],
        }
    }

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(parse_version("v0.10.2"), Some((0, 10, 2)));
        assert_eq!(parse_version("1.2.3-rc1"), Some((1, 2, 3)));
        assert_eq!(parse_version("nightly"), None);
        assert!(parse_version("0.10.0") > parse_version("0.9.9"));
    }

    #[test]
    fn picks_the_asset_for_the_install_kind() {
        let update = pick_update(&release("v0.3.0"), &InstallKind::Archive, "0.2.0").unwrap();
        assert_eq!(update.version, "0.3.0");
        assert!(update.asset_name.ends_with(".tar.gz"));
        let update = pick_update(
            &release("v0.3.0"),
            &InstallKind::AppImage("/x".into()),
            "0.2.0",
        )
        .unwrap();
        assert!(update.asset_name.ends_with(".AppImage"));
    }

    #[test]
    fn same_or_older_releases_and_source_builds_get_nothing() {
        assert!(pick_update(&release("v0.2.0"), &InstallKind::Archive, "0.2.0").is_none());
        assert!(pick_update(&release("v0.1.9"), &InstallKind::Archive, "0.2.0").is_none());
        assert!(pick_update(&release("v0.3.0"), &InstallKind::Source, "0.2.0").is_none());
    }

    #[test]
    fn finds_the_bundle_inside_a_versioned_folder() {
        let root = std::env::temp_dir().join(format!("hypr-rules-update-{}", std::process::id()));
        let bundle = root.join("hypr-rules-0.3.0-x86_64");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(bundle.join("install.sh"), b"").unwrap();
        std::fs::write(bundle.join("hypr-rules"), b"").unwrap();
        assert_eq!(find_bundle(&root), Some(bundle));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn progress_describes_known_and_unknown_sizes() {
        let progress = Progress::default();
        progress.received.store(3_500_000, Ordering::Relaxed);
        assert_eq!(progress.describe(), "3.5 MB");
        progress.total.store(10_000_000, Ordering::Relaxed);
        assert_eq!(progress.describe(), "35 % · 3.5 / 10.0 MB");
    }

    /// End-to-end check against a fake release server (not run by default); installs
    /// into ~/.local/bin like the Update button does:
    /// `HYPR_RULES_UPDATE_URL=http://127.0.0.1:8765/latest.json cargo test update_from_fake_release -- --ignored`
    #[test]
    #[ignore]
    fn update_from_fake_release() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let found = runtime
            .block_on(check(InstallKind::Archive))
            .unwrap()
            .expect("the fake release should be newer");
        let progress = Progress::default();
        runtime
            .block_on(apply(InstallKind::Archive, found, &progress))
            .unwrap();
        assert!(progress.received() > 0);
        assert_eq!(progress.fraction(), Some(1.0));
    }
}
