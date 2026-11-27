use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use axum::{Json, extract::Query, http::StatusCode};
use reqwest::{Client, Url};
use ring::digest::{self, SHA256};
use semver::Version;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::routes::{ApiResult, err_resp};

const REPOSITORY: &str = "pan2013e/meta-harness";
const RELEASES_URL: &str = "https://github.com/pan2013e/meta-harness/releases";
const LATEST_URL: &str = "https://api.github.com/repos/pan2013e/meta-harness/releases/latest";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_DOWNLOAD_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Cli,
    Desktop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum UpdateStatus {
    Unsupported,
    NoRelease,
    Current,
    MissingAsset,
    Available,
}

#[derive(Debug, Serialize)]
pub struct UpdateCheck {
    status: UpdateStatus,
    latest_version: Option<String>,
    release_url: String,
}

#[derive(Deserialize)]
pub struct UpdateQuery {
    kind: ArtifactKind,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    size: u64,
    state: String,
    digest: Option<String>,
}

struct Asset {
    name: String,
    url: String,
    size: u64,
    digest: [u8; 32],
}

pub fn is_release_build() -> bool {
    option_env!("MH_RELEASE_BUILD") == Some("1") && !cfg!(debug_assertions)
}

pub async fn check_handler(Query(query): Query<UpdateQuery>) -> ApiResult<UpdateCheck> {
    let report = if is_release_build() {
        check(&github_client().map_err(update_api_error)?, query.kind)
            .await
            .map_err(update_api_error)?
            .0
    } else {
        unsupported_update()
    };
    Ok(Json(report))
}

fn update_api_error(error: anyhow::Error) -> (StatusCode, Json<serde_json::Value>) {
    err_resp(StatusCode::BAD_GATEWAY, error.to_string())
}

fn github_client() -> Result<Client> {
    Ok(mh_acp::outbound_http_builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(10))
        .user_agent(concat!("meta-harness/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let allowed = matches!(
                attempt.url().host_str(),
                Some(
                    "github.com"
                        | "api.github.com"
                        | "release-assets.githubusercontent.com"
                        | "objects.githubusercontent.com"
                )
            );
            if allowed && attempt.url().scheme() == "https" && attempt.previous().len() < 5 {
                attempt.follow()
            } else {
                attempt.error("Unexpected GitHub release redirect")
            }
        }))
        .build()?)
}

fn artifact_name(kind: ArtifactKind, os: &str, arch: &str) -> Option<String> {
    let arch = match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        _ => return None,
    };
    let (artifact, extension) = match (kind, os) {
        (ArtifactKind::Cli, "windows") => ("cli", ".exe"),
        (ArtifactKind::Cli, "macos" | "linux") => ("cli", ""),
        (ArtifactKind::Desktop, "windows") => ("desktop", ".appx"),
        (ArtifactKind::Desktop, "macos") => ("desktop", ".dmg"),
        _ => return None,
    };
    Some(format!("meta-harness-{artifact}-{os}-{arch}{extension}"))
}

fn unsupported_update() -> UpdateCheck {
    UpdateCheck {
        status: UpdateStatus::Unsupported,
        latest_version: None,
        release_url: RELEASES_URL.into(),
    }
}

async fn check(http: &Client, kind: ArtifactKind) -> Result<(UpdateCheck, Option<Asset>)> {
    let Some(filename) = artifact_name(kind, std::env::consts::OS, std::env::consts::ARCH) else {
        return Ok((unsupported_update(), None));
    };
    let response = http
        .get(LATEST_URL)
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .context("Could not reach GitHub Releases")?;
    let release = if response.status() == StatusCode::NOT_FOUND {
        None
    } else {
        Some(
            response
                .error_for_status()
                .context("GitHub release check failed")?
                .json()
                .await
                .context("Could not read the GitHub release")?,
        )
    };
    inspect_release(release, &Version::parse(VERSION)?, &filename)
}

fn inspect_release(
    release: Option<GithubRelease>,
    current: &Version,
    filename: &str,
) -> Result<(UpdateCheck, Option<Asset>)> {
    let Some(release) = release.filter(|release| !release.draft && !release.prerelease) else {
        return Ok((
            UpdateCheck {
                status: UpdateStatus::NoRelease,
                ..unsupported_update()
            },
            None,
        ));
    };
    let version = release
        .tag_name
        .strip_prefix('v')
        .and_then(|tag| Version::parse(tag).ok())
        .filter(|version| version.pre.is_empty() && version.build.is_empty())
        .context("GitHub published a release without a stable version tag")?;
    let release_url = format!("{RELEASES_URL}/tag/{}", release.tag_name);
    let asset = if &version > current {
        let bundle = filename
            .ends_with(".appx")
            .then(|| format!("{filename}bundle"));
        release.assets.into_iter().find_map(|asset| {
            if (asset.name != filename && bundle.as_deref() != Some(&asset.name))
                || asset.state != "uploaded"
                || asset.size == 0
                || asset.size > MAX_DOWNLOAD_BYTES
            {
                return None;
            }
            let digest = hex::decode(asset.digest?.strip_prefix("sha256:")?)
                .ok()?
                .try_into()
                .ok()?;
            Some(Asset {
                url: format!(
                    "https://github.com/{REPOSITORY}/releases/download/{}/{}",
                    release.tag_name, asset.name
                ),
                name: asset.name,
                size: asset.size,
                digest,
            })
        })
    } else {
        None
    };
    let status = if &version <= current {
        UpdateStatus::Current
    } else if asset.is_some() {
        UpdateStatus::Available
    } else {
        UpdateStatus::MissingAsset
    };
    Ok((
        UpdateCheck {
            status,
            latest_version: Some(version.to_string()),
            release_url,
        },
        asset,
    ))
}

async fn download(http: &Client, asset: &Asset, destination: &Path) -> Result<()> {
    let mut response = http
        .get(Url::parse(&asset.url)?)
        .timeout(Duration::from_secs(8 * 60))
        .send()
        .await
        .context("Could not download the release")?
        .error_for_status()
        .context("GitHub rejected the release download")?;
    ensure!(
        response
            .content_length()
            .is_none_or(|size| size == asset.size),
        "Release download size did not match GitHub"
    );
    let mut file = tokio::fs::File::create_new(destination).await?;
    let mut bytes = 0;
    let mut hash = digest::Context::new(&SHA256);
    while let Some(chunk) = response.chunk().await? {
        bytes += chunk.len() as u64;
        ensure!(
            bytes <= asset.size,
            "Release download exceeded its declared size"
        );
        hash.update(&chunk);
        file.write_all(&chunk).await?;
    }
    ensure!(
        bytes == asset.size && hash.finish().as_ref() == asset.digest,
        "Release download failed SHA-256 verification"
    );
    file.sync_all().await?;
    Ok(())
}

pub async fn update_cli(check_only: bool) -> Result<()> {
    require_release_build()?;
    let http = github_client()?;
    let (report, asset) = check(&http, ArtifactKind::Cli).await?;
    let version = report.latest_version.as_deref().unwrap_or(VERSION);
    match report.status {
        UpdateStatus::Current => println!("Meta-Harness {VERSION} is up to date."),
        UpdateStatus::NoRelease => println!(
            "GitHub has no published release yet. {}",
            report.release_url
        ),
        UpdateStatus::Unsupported => bail!("Self-updates are unavailable on this platform."),
        UpdateStatus::MissingAsset => bail!(
            "Release {version} has no verified standalone binary for this platform. {}",
            report.release_url
        ),
        UpdateStatus::Available if check_only => println!(
            "Meta-Harness {version} is available. Run `meta-harness update` to install it."
        ),
        UpdateStatus::Available => {
            let exe = dunce::canonicalize(std::env::current_exe()?)?;
            ensure_standalone_executable(&exe)?;
            let asset = asset.context("Missing update download")?;
            let install_dir = exe
                .parent()
                .context("The executable has no installation directory")?;
            let mut lock_name = exe
                .file_name()
                .context("The executable has no filename")?
                .to_os_string();
            lock_name.push(".update.lock");
            let lock = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(install_dir.join(lock_name))
                .context("Could not open the installation directory for updates")?;
            lock.try_lock().context("Another update is in progress")?;
            let staging = tempfile::Builder::new()
                .prefix(".meta-harness-update-")
                .tempdir_in(install_dir)?;
            println!("Downloading and verifying Meta-Harness {version}…");
            let binary = staging.path().join(&asset.name);
            download(&http, &asset, &binary).await?;
            std::fs::set_permissions(&binary, std::fs::metadata(&exe)?.permissions())?;
            replace_installed_binary(&exe, &binary, staging)?;
            println!(
                "Updated to {version}. Quit and reopen any running Meta-Harness instance to use it."
            );
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_installed_binary(exe: &Path, binary: &Path, _staging: tempfile::TempDir) -> Result<()> {
    std::fs::rename(binary, exe).context("Could not replace the installed executable")
}

#[cfg(windows)]
fn replace_installed_binary(exe: &Path, binary: &Path, staging: tempfile::TempDir) -> Result<()> {
    let backup = staging.path().join("previous.exe");
    std::fs::rename(exe, &backup)
        .context("Could not move the installed executable. Close Meta-Harness and try again")?;
    if let Err(error) = std::fs::rename(binary, exe) {
        if let Err(rollback) = std::fs::rename(&backup, exe) {
            let saved = staging.keep().join("previous.exe");
            bail!(
                "Could not install the update ({error}) or restore the previous executable ({rollback}). It was saved at {}",
                saved.display()
            );
        }
        return Err(error)
            .context("Could not install the update. The previous executable was restored");
    }
    if let Err(error) = self_replace::self_delete_at(&backup) {
        let saved = staging.keep();
        eprintln!(
            "The update succeeded. Windows could not clean up the previous executable at {} ({error}).",
            saved.display()
        );
    }
    Ok(())
}

/// The desktop main process calls this with the version the user selected in About.
pub async fn stage_desktop_update(expected_version: &str) -> Result<PathBuf> {
    require_release_build()?;
    let http = github_client()?;
    let (report, asset) = check(&http, ArtifactKind::Desktop).await?;
    ensure!(
        report.latest_version.as_deref() == Some(expected_version),
        "The GitHub release changed. Check for updates again."
    );
    let asset = asset.context("A verified desktop installer is unavailable")?;
    let staging = tempfile::Builder::new()
        .prefix("meta-harness-update-")
        .tempdir()?;
    download(&http, &asset, &staging.path().join(&asset.name)).await?;
    Ok(staging.keep().join(asset.name))
}

pub fn install_cli(bin_dir: Option<PathBuf>) -> Result<()> {
    require_release_build()?;
    let source = std::env::current_exe()?;
    ensure_standalone_executable(&source)?;
    let dest = install_path(bin_dir)?;
    let dir = dest.parent().context("Invalid installation directory")?;
    std::fs::create_dir_all(dir)?;
    if dest.exists() && dunce::canonicalize(&source)? == dunce::canonicalize(&dest)? {
        println!("Meta-Harness is already installed at {}", dest.display());
        return Ok(());
    }
    let staged = tempfile::NamedTempFile::new_in(dir)?;
    std::fs::copy(&source, staged.path())?;
    staged.persist_noclobber(&dest).with_context(|| {
        format!(
            "Could not install to {}. Use `meta-harness update` for an existing installation.",
            dest.display()
        )
    })?;
    println!("Installed Meta-Harness at {}", dest.display());
    if !std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path)
            .any(|entry| dunce::canonicalize(entry).ok() == dunce::canonicalize(dir).ok())
    }) {
        println!(
            "Add {} to PATH to run `meta-harness` by name.",
            dir.display()
        );
    }
    Ok(())
}

pub fn uninstall_cli(bin_dir: Option<PathBuf>) -> Result<()> {
    require_release_build()?;
    let exe = std::env::current_exe()?;
    ensure_standalone_executable(&exe)?;
    let installed = install_path(bin_dir)?;
    ensure!(
        dunce::canonicalize(&installed).ok().as_ref() == Some(&dunce::canonicalize(&exe)?),
        "Run `uninstall` from the installed executable at {}",
        installed.display()
    );
    self_replace::self_delete().context("Could not remove the installed executable")?;
    println!(
        "Removed Meta-Harness from {}. Sessions and settings were kept.",
        installed.display()
    );
    Ok(())
}

fn install_path(bin_dir: Option<PathBuf>) -> Result<PathBuf> {
    let default_bin = || {
        if cfg!(windows) {
            dirs::data_local_dir().map(|base| base.join("meta-harness-cli").join("bin"))
        } else {
            dirs::home_dir().map(|base| base.join(".local").join("bin"))
        }
    };
    let dir = bin_dir
        .or_else(default_bin)
        .context("Pass --bin-dir to choose a user-writable installation directory")?;
    Ok(dir.join(if cfg!(windows) {
        "meta-harness.exe"
    } else {
        "meta-harness"
    }))
}

fn ensure_standalone_executable(exe: &Path) -> Result<()> {
    // The bundled Rust process is owned by the Electron installer.
    ensure!(
        !exe.parent().and_then(Path::parent).is_some_and(|parent| {
            parent
                .file_name()
                .is_some_and(|part| part.eq_ignore_ascii_case("resources"))
                && parent.join("app.asar").is_file()
        }),
        "Use Settings → About to update a desktop installation."
    );
    Ok(())
}

fn require_release_build() -> Result<()> {
    ensure!(
        is_release_build(),
        "Run this command from a Meta-Harness release build."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_selects_verified_assets_from_a_newer_stable_release() {
        let parse = |tag: &str, filename: &str, digest: Option<&str>| GithubRelease {
            tag_name: tag.into(),
            draft: false,
            prerelease: false,
            assets: vec![GithubAsset {
                name: filename.into(),
                size: 5,
                state: "uploaded".into(),
                digest: digest.map(str::to_string),
            }],
        };
        let name = artifact_name(ArtifactKind::Cli, "windows", "x86_64").unwrap();
        let mac = artifact_name(ArtifactKind::Desktop, "macos", "aarch64").unwrap();
        assert_eq!(mac, "meta-harness-desktop-macos-arm64.dmg");
        let good_digest = format!("sha256:{}", "ab".repeat(32));
        let current = Version::parse("0.5.9").unwrap();
        for (release, expected) in [
            (None, UpdateStatus::NoRelease),
            (
                Some(parse("v0.5.8", &name, Some(&good_digest))),
                UpdateStatus::Current,
            ),
            (
                Some(parse("v0.5.10", &name, None)),
                UpdateStatus::MissingAsset,
            ),
            (
                Some(parse("v0.5.10", &mac, Some(&good_digest))),
                UpdateStatus::MissingAsset,
            ),
            (
                Some(parse("v0.5.10", &name, Some("sha256:invalid"))),
                UpdateStatus::MissingAsset,
            ),
            (
                Some(parse("v0.5.10", &name, Some(&good_digest))),
                UpdateStatus::Available,
            ),
        ] {
            let (report, asset) = inspect_release(release, &current, &name).unwrap();
            assert_eq!(report.status, expected);
            assert_eq!(asset.is_some(), expected == UpdateStatus::Available);
        }
        assert!(
            inspect_release(
                Some(parse("v0.6.0-beta.1", &name, Some(&good_digest))),
                &current,
                &name
            )
            .is_err()
        );
        let appx = artifact_name(ArtifactKind::Desktop, "windows", "x86_64").unwrap();
        let bundle = format!("{appx}bundle");
        let (report, asset) = inspect_release(
            Some(parse("v0.5.10", &bundle, Some(&good_digest))),
            &current,
            &appx,
        )
        .unwrap();
        assert_eq!(report.status, UpdateStatus::Available);
        assert!(asset.unwrap().url.ends_with(&bundle));
    }

    #[test]
    fn failed_replacement_keeps_the_installed_binary() {
        let install_dir = tempfile::tempdir().unwrap();
        let staging = tempfile::tempdir_in(install_dir.path()).unwrap();
        let destination = install_dir.path().join("meta-harness.exe");
        std::fs::write(&destination, "previous release").unwrap();
        let missing_binary = staging.path().join("missing.exe");

        assert!(replace_installed_binary(&destination, &missing_binary, staging).is_err());
        assert_eq!(
            std::fs::read_to_string(destination).unwrap(),
            "previous release"
        );
    }
}
