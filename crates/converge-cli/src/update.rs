//! `converge update` — self-update from the signed release layout
//! (`converge-<target>.tar.gz` + `SHA256SUMS` + `SHA256SUMS.minisig`).
//!
//! Verification is **mandatory** here, unlike the bootstrap install: the
//! manifest signature must check out against the public key baked into
//! this binary before anything touches disk state — each installed
//! version vouches for its successor, so a compromised release channel
//! alone can't push code. The swap is atomic-with-rollback: the running
//! binary is kept as `converge-previous` (`--rollback` swaps back), and a
//! staged binary must pass a `--version` sanity run before it replaces
//! anything.
//!
//! Closed contours: `--from <dir>` verifies the same signed layout from a
//! local directory — no egress needed to update.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use minisign_verify::{PublicKey, Signature};
use sha2::{Digest, Sha256};

/// The release-signing public key (minisign, one base64 line). The
/// placeholder is replaced when the project's key is generated (see
/// RELEASING.md); until then `CONVERGE_UPDATE_PUBKEY` overrides — which
/// is also the test seam.
const PUBKEY: &str = "__MINISIGN_PUBKEY__";

const REPO: &str = "converge-lab/converge";

pub async fn run(
    version: Option<String>,
    from: Option<PathBuf>,
    rollback: bool,
    force: bool,
) -> Result<()> {
    let exe = std::env::current_exe().context("resolve own path")?;
    let bin_dir = exe
        .parent()
        .context("the binary has no parent directory")?
        .to_path_buf();

    if rollback {
        return swap_back(&bin_dir);
    }

    let key = key()?;
    let fetched = match from {
        Some(dir) => Fetched::local(&dir, &target()?)?,
        None => fetch(version.as_deref(), &target()?).await?,
    };

    // Authenticity first: the signed manifest vouches for everything else.
    let signature = Signature::decode(&fetched.signature)
        .context("SHA256SUMS.minisig is not a minisign signature")?;
    key.verify(&fetched.manifest, &signature, false)
        .map_err(|e| anyhow::anyhow!("manifest signature verification failed: {e}"))?;

    // Integrity: the artifact must match its manifest line.
    let expected = manifest_hash(
        std::str::from_utf8(&fetched.manifest).context("manifest is not UTF-8")?,
        &fetched.artifact_name,
    )?;
    let actual = format!("{:x}", Sha256::digest(&fetched.artifact));
    if actual != expected {
        bail!("checksum mismatch for {}", fetched.artifact_name);
    }

    if !force && fetched.version.trim_start_matches('v') == crate::skew::CLI {
        println!(
            "already at {} — nothing to do (--force to reinstall)",
            fetched.version
        );
        return Ok(());
    }

    // Stage, sanity-run, then swap. Staging lives beside the binary so
    // the final renames stay on one filesystem (atomic).
    let staging = bin_dir.join(".update-staging");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    let tarball = staging.join(&fetched.artifact_name);
    std::fs::write(&tarball, &fetched.artifact)?;
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(&tarball)
        .arg("-C")
        .arg(&staging)
        .status()
        .context("run tar")?;
    if !status.success() {
        bail!("unpacking {} failed", fetched.artifact_name);
    }
    let staged = staging.join("converge");
    let sane = Command::new(&staged)
        .arg("--version")
        .output()
        .with_context(|| format!("run the staged binary ({})", staged.display()))?;
    if !sane.status.success() {
        bail!("the staged binary failed its --version sanity run — not installing");
    }

    let previous = bin_dir.join("converge-previous");
    std::fs::rename(&exe, &previous).context("keep the current binary as converge-previous")?;
    std::fs::rename(&staged, &exe).context("move the new binary into place")?;
    let _ = std::fs::remove_dir_all(&staging);

    println!(
        "updated to {} ({} kept as rollback — `converge update --rollback`)",
        String::from_utf8_lossy(&sane.stdout).trim(),
        previous.display()
    );
    Ok(())
}

/// `--rollback`: swap `converge` and `converge-previous`, whichever of
/// them is running.
fn swap_back(bin_dir: &Path) -> Result<()> {
    let current = bin_dir.join("converge");
    let previous = bin_dir.join("converge-previous");
    if !previous.exists() {
        bail!("no {} to roll back to", previous.display());
    }
    let parked = bin_dir.join(".rollback-tmp");
    std::fs::rename(&current, &parked)?;
    std::fs::rename(&previous, &current)?;
    std::fs::rename(&parked, &previous)?;
    println!("rolled back — the replaced binary is now converge-previous");
    Ok(())
}

/// The verification key: the env seam wins (tests, pre-keygen interim),
/// else the baked-in constant; a placeholder build refuses to update.
fn key() -> Result<PublicKey> {
    let base64 = match std::env::var("CONVERGE_UPDATE_PUBKEY") {
        Ok(key) => key,
        Err(_) if PUBKEY.starts_with("__") => bail!(
            "this build has no release key baked in and CONVERGE_UPDATE_PUBKEY \
             is not set — updates would be unverifiable, refusing"
        ),
        Err(_) => PUBKEY.to_string(),
    };
    PublicKey::from_base64(base64.trim()).context("the release public key is malformed")
}

/// This machine's release target (matching the artifacts we publish;
/// Linux ships musl-static).
fn target() -> Result<String> {
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-musl",
        ("linux", "aarch64") => "aarch64-unknown-linux-musl",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        (os, arch) => bail!("no release artifacts for {os}/{arch}"),
    };
    Ok(target.to_string())
}

/// One release's relevant files, wherever they came from.
struct Fetched {
    version: String,
    artifact_name: String,
    artifact: Vec<u8>,
    manifest: Vec<u8>,
    signature: String,
}

impl Fetched {
    /// A local, already-transported release directory (closed contours).
    fn local(dir: &Path, target: &str) -> Result<Self> {
        let artifact_name = format!("converge-{target}.tar.gz");
        let read = |name: &str| {
            std::fs::read(dir.join(name))
                .with_context(|| format!("read {} from {}", name, dir.display()))
        };
        Ok(Self {
            // Local layouts carry no tag; the artifact decides via the
            // sanity run, and skip-if-same is checked post-verification.
            version: "local".into(),
            artifact: read(&artifact_name)?,
            artifact_name,
            manifest: read("SHA256SUMS")?,
            signature: String::from_utf8(read("SHA256SUMS.minisig")?)
                .context("SHA256SUMS.minisig is not UTF-8")?,
        })
    }
}

/// Download a release (a pinned tag, or the latest) from GitHub — or a
/// mirror via `CONVERGE_DOWNLOAD_BASE` (which then serves the flat
/// release layout regardless of tag).
async fn fetch(version: Option<&str>, target: &str) -> Result<Fetched> {
    let http = reqwest::Client::builder()
        .user_agent("converge-cli")
        .build()?;
    let version = match version {
        Some(tag) => tag.to_string(),
        None => latest(&http).await?,
    };
    let base = match std::env::var("CONVERGE_DOWNLOAD_BASE") {
        Ok(base) => base.trim_end_matches('/').to_string(),
        Err(_) => format!("https://github.com/{REPO}/releases/download/{version}"),
    };
    let artifact_name = format!("converge-{target}.tar.gz");
    let get = |name: String| {
        let http = http.clone();
        let url = format!("{base}/{name}");
        async move {
            let response = http.get(&url).send().await?.error_for_status()?;
            anyhow::Ok(response.bytes().await?.to_vec())
        }
    };
    Ok(Fetched {
        artifact: get(artifact_name.clone())
            .await
            .with_context(|| format!("download {artifact_name}"))?,
        manifest: get("SHA256SUMS".into())
            .await
            .context("download SHA256SUMS")?,
        signature: String::from_utf8(
            get("SHA256SUMS.minisig".into())
                .await
                .context("download SHA256SUMS.minisig")?,
        )
        .context("SHA256SUMS.minisig is not UTF-8")?,
        artifact_name,
        version,
    })
}

/// Resolve the newest release tag.
async fn latest(http: &reqwest::Client) -> Result<String> {
    #[derive(serde::Deserialize)]
    struct Release {
        tag_name: String,
    }
    let release: Release = http
        .get(format!(
            "https://api.github.com/repos/{REPO}/releases/latest"
        ))
        .send()
        .await?
        .error_for_status()
        .context("resolve the latest release")?
        .json()
        .await?;
    Ok(release.tag_name)
}

/// The manifest line for `name` → its sha256 hex.
fn manifest_hash(manifest: &str, name: &str) -> Result<String> {
    manifest
        .lines()
        .find_map(|line| {
            let (hash, file) = line.split_once("  ")?;
            (file.trim() == name).then(|| hash.trim().to_string())
        })
        .with_context(|| format!("{name} is not in SHA256SUMS"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_lines_resolve_by_name() {
        let manifest = "abc123  converge-x86_64-unknown-linux-musl.tar.gz\n\
                        def456  converge-aarch64-apple-darwin.tar.gz\n";
        assert_eq!(
            manifest_hash(manifest, "converge-aarch64-apple-darwin.tar.gz").unwrap(),
            "def456"
        );
        assert!(manifest_hash(manifest, "converge-nope.tar.gz").is_err());
    }

    #[test]
    fn placeholder_key_refuses_without_override() {
        // The baked-in key is still the placeholder in this tree.
        unsafe { std::env::remove_var("CONVERGE_UPDATE_PUBKEY") };
        assert!(key().is_err());
    }
}
