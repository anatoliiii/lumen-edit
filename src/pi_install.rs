//! Install the Pi coding agent into the current user's data directory with a
//! private Node.js runtime. No system packages or existing Node installation
//! are required.
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    thread,
};

const NODE_VERSION: &str = "v22.19.0";
const PI_VERSION: &str = "1.3.0";
const PACKAGE: &str = "@earendil-works/pi-coding-agent";

#[derive(Clone, Debug)]
pub enum InstallState {
    Running(String),
    Finished(Result<InstalledPi, String>),
}

#[derive(Clone, Debug)]
pub struct InstalledPi {
    pub node: PathBuf,
    pub entry: PathBuf,
    pub runtime_dir: PathBuf,
}

/// Start installation outside the UI thread; the callback state is read by the
/// settings page and `ctx` wakes it as progress changes.
pub fn start(ctx: eframe::egui::Context) -> Arc<Mutex<InstallState>> {
    let state = Arc::new(Mutex::new(InstallState::Running("Preparing installation…".into())));
    let worker_state = Arc::clone(&state);
    thread::Builder::new()
        .name("pi-install".into())
        .spawn(move || {
            let result = install(|message| {
                *worker_state.lock().unwrap_or_else(|e| e.into_inner()) = InstallState::Running(message);
                ctx.request_repaint();
            });
            *worker_state.lock().unwrap_or_else(|e| e.into_inner()) = InstallState::Finished(result);
            ctx.request_repaint();
        })
        .map_err(|error| {
            *state.lock().unwrap_or_else(|e| e.into_inner()) = InstallState::Finished(Err(error.to_string()));
        })
        .ok();
    state
}

fn data_dir() -> Result<PathBuf, String> {
    dirs::data_local_dir()
        .or_else(dirs::home_dir)
        .map(|home| home.join("lumen-edit").join("pi"))
        .ok_or_else(|| "Could not find a user data directory".into())
}

fn platform_archive() -> Result<(String, String, String), String> {
    let os = if cfg!(target_os = "windows") {
        "win"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        return Err("Pi's private runtime is not available for this operating system".into());
    };
    let arch = if cfg!(target_arch = "x86_64") {
        "x64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        return Err("Pi's private runtime is available only on x64 and ARM64".into());
    };
    let ext = if os == "win" { "zip" } else { "tar.gz" };
    let directory = format!("node-{NODE_VERSION}-{os}-{arch}");
    let archive = format!("{directory}.{ext}");
    Ok((os.to_string(), arch.to_string(), archive))
}

fn install(mut progress: impl FnMut(String)) -> Result<InstalledPi, String> {
    let base = data_dir()?;
    std::fs::create_dir_all(&base).map_err(|e| format!("{}: {e}", base.display()))?;
    let (platform, arch, archive_name) = platform_archive()?;
    let runtime_dir = base.join(format!("node-{NODE_VERSION}-{platform}-{arch}"));
    let node = if cfg!(target_os = "windows") { runtime_dir.join("node.exe") } else { runtime_dir.join("bin/node") };

    if !node.is_file() {
        progress(format!("Downloading Node.js {NODE_VERSION}…"));
        let scratch = base.join(format!(".install-{}-{}", std::process::id(), unix_time()));
        std::fs::create_dir(&scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
        let result: Result<(), String> = (|| {
            let archive = scratch.join(&archive_name);
            let checksums = scratch.join("SHASUMS256.txt");
            let base_url = format!("https://nodejs.org/dist/{NODE_VERSION}");
            download(&format!("{base_url}/{archive_name}"), &archive)?;
            download(&format!("{base_url}/SHASUMS256.txt"), &checksums)?;
            verify_sha256(&archive, &checksums, &archive_name)?;
            let extract = scratch.join("extract");
            std::fs::create_dir(&extract).map_err(|e| e.to_string())?;
            progress("Extracting the private Node.js runtime…".into());
            extract_archive(&archive, &extract)?;
            let extracted = extract.join(format!("node-{NODE_VERSION}-{platform}-{arch}"));
            std::fs::rename(&extracted, &runtime_dir).map_err(|e| format!("Could not install Node.js runtime: {e}"))?;
            Ok(())
        })();
        let _ = std::fs::remove_dir_all(&scratch);
        result?;
    }
    if !node.is_file() {
        return Err(format!("Node.js runtime was not found at {}", node.display()));
    }

    let prefix = base.join(format!("pi-{PI_VERSION}"));
    let package_dir = prefix.join("node_modules").join("@earendil-works").join("pi-coding-agent");
    let package_json = package_dir.join("package.json");
    let entry = package_dir.join("dist/bundle/cli.js");
    if !package_json.is_file() || !entry.is_file() {
        progress(format!("Installing Pi {PI_VERSION} in the user directory…"));
        std::fs::create_dir_all(&prefix).map_err(|e| format!("{}: {e}", prefix.display()))?;
        let npm_cli = if cfg!(target_os = "windows") {
            runtime_dir.join("node_modules/npm/bin/npm-cli.js")
        } else {
            runtime_dir.join("lib/node_modules/npm/bin/npm-cli.js")
        };
        if !npm_cli.is_file() {
            return Err(format!("Bundled npm was not found at {}", npm_cli.display()));
        }
        let output = Command::new(&node)
            .arg(&npm_cli)
            .args(["install", "--global", "--ignore-scripts", "--no-audit", "--no-fund", "--prefix"])
            .arg(&prefix)
            .arg(format!("{PACKAGE}@{PI_VERSION}"))
            .env("PATH", prepend_path(node.parent().unwrap_or(&runtime_dir)))
            .output()
            .map_err(|e| format!("Could not run bundled npm: {e}"))?;
        if !output.status.success() {
            let detail = if output.stderr.is_empty() { &output.stdout } else { &output.stderr };
            let detail = String::from_utf8_lossy(detail);
            let detail = detail.lines().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
            return Err(format!("Pi installation failed ({}): {}", output.status, detail.trim()));
        }
    }
    if !entry.is_file() {
        return Err(format!("Pi's CLI entry point was not found at {}", entry.display()));
    }
    let runtime_bin = node.parent().unwrap_or(&runtime_dir).to_path_buf();
    Ok(InstalledPi { node, entry, runtime_dir: runtime_bin })
}

fn prepend_path(directory: &Path) -> std::ffi::OsString {
    let mut paths = vec![directory.to_path_buf()];
    if let Some(path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&path));
    }
    std::env::join_paths(paths).unwrap_or_else(|_| directory.as_os_str().to_owned())
}

fn download(url: &str, destination: &Path) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    let response = runtime
        .block_on(async {
            reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(15))
                .timeout(std::time::Duration::from_secs(300))
                .build()?
                .get(url)
                .send()
                .await?
                .error_for_status()
        })
        .map_err(|e| format!("Download failed from {url}: {e}"))?;
    let bytes = runtime.block_on(response.bytes()).map_err(|e| format!("Download failed from {url}: {e}"))?;
    if bytes.is_empty() {
        return Err(format!("Empty download from {url}"));
    }
    std::fs::write(destination, bytes).map_err(|e| format!("{}: {e}", destination.display()))
}

fn verify_sha256(archive: &Path, manifest: &Path, name: &str) -> Result<(), String> {
    let contents = std::fs::read_to_string(manifest).map_err(|e| e.to_string())?;
    let expected = contents
        .lines()
        .find_map(|line| line.split_once("  ").filter(|(_, file)| *file == name).map(|(hash, _)| hash.trim()))
        .ok_or_else(|| format!("Node.js checksum for {name} is missing"))?;
    let actual = sha256(archive)?;
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(format!("Node.js archive checksum mismatch for {name}"));
    }
    Ok(())
}

fn sha256(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};

    let mut archive = std::fs::File::open(path).map_err(|e| format!("Could not verify Node.js download: {e}"))?;
    let mut digest = Sha256::new();
    std::io::copy(&mut archive, &mut digest).map_err(|e| format!("Could not read Node.js archive: {e}"))?;
    Ok(format!("{:x}", digest.finalize()))
}

fn extract_archive(archive: &Path, destination: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let archive = archive.to_string_lossy().replace('\'', "''");
        let destination = destination.to_string_lossy().replace('\'', "''");
        let output = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!("Expand-Archive -LiteralPath '{archive}' -DestinationPath '{destination}'"),
            ])
            .output()
            .map_err(|e| format!("Could not extract Node.js archive: {e}"))?;
        if !output.status.success() {
            return Err(format!("Could not extract Node.js archive: {}", String::from_utf8_lossy(&output.stderr).trim()));
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let output = Command::new("tar")
            .arg("-xzf")
            .arg(archive)
            .arg("-C")
            .arg(destination)
            .output()
            .map_err(|e| format!("Could not extract Node.js archive (tar is required): {e}"))?;
        if !output.status.success() {
            return Err(format!("Could not extract Node.js archive: {}", String::from_utf8_lossy(&output.stderr).trim()));
        }
    }
    Ok(())
}

fn unix_time() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_name_matches_supported_platform() {
        let (platform, arch, archive) = platform_archive().unwrap();
        assert!(matches!(platform.as_str(), "linux" | "darwin" | "win"));
        assert!(matches!(arch.as_str(), "x64" | "arm64"));
        assert!(archive.ends_with(".zip") || archive.ends_with(".tar.gz"));
    }

    #[test]
    fn checksum_verification_requires_matching_archive_name() {
        let root = std::env::temp_dir().join(format!("pi-install-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let archive = root.join("fixture with spaces 'quoted'.tar.gz");
        let manifest = root.join("SHASUMS256.txt");
        std::fs::write(&archive, b"fixture").unwrap();
        // Known digest: do not generate the expected value with the code under test.
        let expected = "f16d05ec6b29248d2c61adb1e9263f78e4f7bace1b955014a2d17872cfe4064d";
        assert_eq!(sha256(&archive).unwrap(), expected);
        std::fs::write(&manifest, format!("{expected}  fixture.tar.gz\n")).unwrap();
        assert!(verify_sha256(&archive, &manifest, "fixture.tar.gz").is_ok());
        assert!(verify_sha256(&archive, &manifest, "other.tar.gz").is_err());
        std::fs::write(&archive, b"corrupted").unwrap();
        assert!(verify_sha256(&archive, &manifest, "fixture.tar.gz").unwrap_err().contains("mismatch"));
        std::fs::remove_file(&archive).unwrap();
        assert!(verify_sha256(&archive, &manifest, "fixture.tar.gz").is_err());
        let _ = std::fs::remove_dir_all(root);
    }
}
