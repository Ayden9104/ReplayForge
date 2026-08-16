//! Check GitHub Releases and install Linux tarball updates into ~/.local.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const REPO: &str = "Ayden9104/ReplayForge";
const USER_AGENT: &str = "ReplayForge";

#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub latest: String,
    pub html_url: String,
    pub newer: bool,
    pub tarball_url: String,
    pub sha256sums_url: String,
    pub tarball_name: String,
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Query GitHub for the latest release and compare to this build.
pub fn check_latest() -> Result<UpdateInfo, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let body = github_api_get(&url)?;

    let tag = json_string_field(&body, "tag_name")
        .ok_or_else(|| "Update check failed: missing tag_name in GitHub response".to_string())?;
    let latest = tag.trim().trim_start_matches('v').to_string();
    let html_url = json_string_field(&body, "html_url")
        .unwrap_or_else(|| format!("https://github.com/{REPO}/releases/tag/{}", tag.trim()));
    let html_url = sanitize_release_html_url(&html_url, tag.trim())?;

    let current = current_version();
    let newer = is_newer(&latest, current)?;

    if !newer {
        return Ok(UpdateInfo {
            latest,
            html_url,
            newer: false,
            tarball_url: String::new(),
            sha256sums_url: String::new(),
            tarball_name: String::new(),
        });
    }

    let assets = github_assets(&body);
    let (tarball_name, tarball_url) = assets
        .iter()
        .find(|(name, _)| {
            name.starts_with("replayforge-") && name.ends_with("-linux-x86_64.tar.gz")
        })
        .cloned()
        .ok_or_else(|| {
            "Update check failed: release missing Linux tarball asset (replayforge-*-linux-x86_64.tar.gz)"
                .to_string()
        })?;
    let (_, sha256sums_url) = assets
        .iter()
        .find(|(name, _)| name == "SHA256SUMS")
        .cloned()
        .ok_or_else(|| "Update check failed: release missing SHA256SUMS asset".to_string())?;

    require_https(&tarball_url)?;
    require_https(&sha256sums_url)?;

    Ok(UpdateInfo {
        latest,
        html_url,
        newer: true,
        tarball_url,
        sha256sums_url,
        tarball_name,
    })
}

/// Download, verify, extract, and install a pending update into `$HOME/.local`.
pub fn install_update(info: &UpdateInfo) -> Result<(), String> {
    if is_root() {
        return Err("Refusing to install update as root".into());
    }
    require_https(&info.tarball_url)?;
    require_https(&info.sha256sums_url)?;

    let home = dirs_home()?;
    let prefix = home.join(".local");
    let tmp = unique_temp_dir()?;
    let cleanup = TempDirGuard(tmp.clone());

    let tarball_path = tmp.join(&info.tarball_name);
    let sums_path = tmp.join("SHA256SUMS");

    download_file(&info.tarball_url, &tarball_path)?;
    download_file(&info.sha256sums_url, &sums_path)?;
    verify_sha256(&tmp, &info.tarball_name)?;
    validate_tar_members(&tarball_path)?;

    let extract_dir = tmp.join("extract");
    fs::create_dir_all(&extract_dir).map_err(|e| format!("Failed to create extract dir: {e}"))?;
    run_cmd(
        "tar",
        &[
            "--no-same-owner",
            "-xzf",
            &path_str(&tarball_path),
            "-C",
            &path_str(&extract_dir),
        ],
        None,
    )?;

    let pkg = find_package_dir(&extract_dir)?;
    let bin_src = pkg.join("replayforge");
    let desktop_src = pkg.join("replayforge.desktop");
    let icon_src = pkg.join("replayforge.svg");

    if !bin_src.is_file() || bin_src.is_symlink() {
        return Err("Update package binary missing or is a symlink".into());
    }
    if !desktop_src.is_file() || !icon_src.is_file() {
        return Err("Update package missing desktop entry or icon".into());
    }

    let bin_dst = prefix.join("bin/replayforge");
    let desktop_dst = prefix.join("share/applications/replayforge.desktop");
    let icon_dst = prefix.join("share/icons/hicolor/scalable/apps/replayforge.svg");

    install_file(&bin_src, &bin_dst, true)?;
    install_file(&desktop_src, &desktop_dst, false)?;
    install_file(&icon_src, &icon_dst, false)?;
    rewrite_desktop_exec(&desktop_dst, &bin_dst)?;

    let _ = Command::new("update-desktop-database")
        .arg(prefix.join("share/applications"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = Command::new("gtk-update-icon-cache")
        .args(["-f", "-t"])
        .arg(prefix.join("share/icons/hicolor"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    drop(cleanup);
    Ok(())
}

/// Path where in-app updates install the binary (`$HOME/.local/bin/replayforge`).
pub fn installed_bin_path() -> Result<PathBuf, String> {
    Ok(dirs_home()?.join(".local/bin/replayforge"))
}

/// Start a new process from the installed binary (after an update replace).
pub fn relaunch_installed() -> Result<(), String> {
    let bin = installed_bin_path()?;
    if !bin.is_file() {
        return Err(format!("Installed binary missing: {}", bin.display()));
    }
    Command::new(&bin)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to relaunch {}: {e}", bin.display()))?;
    Ok(())
}

struct TempDirGuard(PathBuf);
impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn dirs_home() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| "HOME is not set".to_string())
}

fn is_root() -> bool {
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<u32>().ok())
        == Some(0)
}

fn unique_temp_dir() -> Result<PathBuf, String> {
    let base = std::env::temp_dir().join(format!(
        "replayforge-update-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&base).map_err(|e| format!("Failed to create temp dir: {e}"))?;
    Ok(base)
}

fn require_https(url: &str) -> Result<(), String> {
    if url.starts_with("https://") {
        Ok(())
    } else {
        Err(format!("Refusing non-https URL: {url}"))
    }
}

fn download_file(url: &str, dest: &Path) -> Result<(), String> {
    require_https(url)?;
    let status = Command::new("curl")
        .args([
            "-fsSL",
            "--max-time",
            "600",
            "-A",
            USER_AGENT,
            "-o",
            &path_str(dest),
            url,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "curl not found — install curl to install updates".to_string()
            } else {
                format!("Failed to run curl: {e}")
            }
        })?;
    if !status.success() {
        return Err(format!("Download failed for {url}"));
    }
    if !dest.is_file() {
        return Err(format!("Download produced no file: {}", dest.display()));
    }
    Ok(())
}

fn verify_sha256(dir: &Path, tarball_name: &str) -> Result<(), String> {
    let sums = fs::read_to_string(dir.join("SHA256SUMS"))
        .map_err(|e| format!("Failed to read SHA256SUMS: {e}"))?;
    let expected = sums
        .lines()
        .find_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (hash, name) = line.split_once(char::is_whitespace)?;
            let name = name.trim().trim_start_matches('*').trim();
            if name == tarball_name || name.ends_with(tarball_name) {
                Some(hash.trim().to_ascii_lowercase())
            } else {
                None
            }
        })
        .ok_or_else(|| format!("SHA256SUMS has no entry for {tarball_name}"))?;

    let out = Command::new("sha256sum")
        .arg(tarball_name)
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "sha256sum not found — needed to verify updates".to_string()
            } else {
                format!("Failed to run sha256sum: {e}")
            }
        })?;
    if !out.status.success() {
        return Err("sha256sum failed while verifying download".into());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let actual = text
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if actual != expected {
        return Err("Update checksum mismatch — refusing to install".into());
    }
    Ok(())
}

fn validate_tar_members(tarball: &Path) -> Result<(), String> {
    let out = Command::new("tar")
        .args(["-tzf", &path_str(tarball)])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("Failed to list archive: {e}"))?;
    if !out.status.success() {
        return Err("Failed to list update archive members".into());
    }
    for member in String::from_utf8_lossy(&out.stdout).lines() {
        let member = member.trim();
        if member.is_empty() {
            continue;
        }
        if member.starts_with('/') || member.contains("..") {
            return Err(format!("Refusing unsafe archive member: {member}"));
        }
    }
    Ok(())
}

fn find_package_dir(extract_dir: &Path) -> Result<PathBuf, String> {
    let mut dirs = Vec::new();
    for entry in
        fs::read_dir(extract_dir).map_err(|e| format!("Failed to read extract dir: {e}"))?
    {
        let entry = entry.map_err(|e| format!("Failed to read extract entry: {e}"))?;
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        }
    }
    if dirs.len() != 1 {
        return Err("Update archive must contain exactly one top-level directory".into());
    }
    let name = dirs[0].file_name().and_then(|n| n.to_str()).unwrap_or("");
    if !(name.starts_with("replayforge-") && name.contains("-linux-")) {
        return Err(format!("Unexpected package directory name: {name}"));
    }
    Ok(dirs[0].clone())
}

fn install_file(src: &Path, dst: &Path, executable: bool) -> Result<(), String> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
    }

    // Write to a sibling temp file then rename over the destination so replacing a
    // currently-running binary does not hit Linux ETXTBSY ("Text file busy").
    let tmp = dst.with_file_name(format!(
        ".{}.new",
        dst.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("replayforge")
    ));
    let _ = fs::remove_file(&tmp);

    let copy_result = (|| -> Result<(), String> {
        fs::copy(src, &tmp).map_err(|e| format!("Failed to stage {}: {e}", tmp.display()))?;
        if executable {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&tmp)
                    .map_err(|e| format!("Failed to stat {}: {e}", tmp.display()))?
                    .permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&tmp, perms)
                    .map_err(|e| format!("Failed to chmod {}: {e}", tmp.display()))?;
            }
        }
        fs::rename(&tmp, dst).map_err(|e| format!("Failed to install {}: {e}", dst.display()))?;
        Ok(())
    })();

    if copy_result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    copy_result
}

fn rewrite_desktop_exec(desktop: &Path, bin: &Path) -> Result<(), String> {
    let text =
        fs::read_to_string(desktop).map_err(|e| format!("Failed to read desktop file: {e}"))?;
    let bin_str = bin.to_string_lossy();
    let rewritten = text
        .lines()
        .map(|line| {
            if line.starts_with("Exec=") {
                format!("Exec={bin_str}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let rewritten = if text.ends_with('\n') {
        rewritten + "\n"
    } else {
        rewritten
    };
    fs::write(desktop, rewritten).map_err(|e| format!("Failed to write desktop file: {e}"))
}

fn run_cmd(program: &str, args: &[&str], cwd: Option<&Path>) -> Result<(), String> {
    let mut cmd = Command::new(program);
    cmd.args(args).stdout(Stdio::null()).stderr(Stdio::piped());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("Failed to run {program}: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "{program} failed: {}",
            err.trim().chars().take(200).collect::<String>()
        ));
    }
    Ok(())
}

fn path_str(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn github_api_get(url: &str) -> Result<String, String> {
    let mut args = vec![
        "-sS".to_string(),
        "--max-time".to_string(),
        "20".to_string(),
        "-w".to_string(),
        "\n%{http_code}".to_string(),
        "-H".to_string(),
        "Accept: application/vnd.github+json".to_string(),
        "-H".to_string(),
        format!("User-Agent: {USER_AGENT}"),
    ];
    if let Ok(token) = std::env::var("GH_TOKEN").or_else(|_| std::env::var("GITHUB_TOKEN")) {
        let token = token.trim().to_string();
        if !token.is_empty() {
            args.push("-H".to_string());
            args.push(format!("Authorization: Bearer {token}"));
        }
    }
    args.push(url.to_string());
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = run_curl(&arg_refs)?;
    let raw = stdout_text(&out);
    let (body, status) = split_http_code(&raw);
    if status != "200" {
        let detail = json_string_field(body, "message").unwrap_or_else(|| {
            if body.is_empty() {
                format!("HTTP {status}")
            } else {
                summarize_body(body)
            }
        });
        if status == "404" {
            return Err(
                "Update check failed: GitHub release not found (is the repo private?)".into(),
            );
        }
        return Err(format!("Update check failed: {detail}"));
    }
    Ok(body.to_string())
}

fn sanitize_release_html_url(url: &str, tag: &str) -> Result<String, String> {
    let prefix = format!("https://github.com/{REPO}/");
    let url = url.trim();
    if url.starts_with(&prefix) {
        return Ok(url.to_string());
    }
    Ok(format!("https://github.com/{REPO}/releases/tag/{tag}"))
}

fn split_http_code(raw: &str) -> (&str, &str) {
    match raw.rsplit_once('\n') {
        Some((body, code)) if code.chars().all(|c| c.is_ascii_digit()) && code.len() == 3 => {
            (body.trim_end(), code)
        }
        _ => (raw, "000"),
    }
}

fn is_newer(latest: &str, current: &str) -> Result<bool, String> {
    Ok(parse_semver(latest)? > parse_semver(current)?)
}

fn parse_semver(s: &str) -> Result<(u32, u32, u32), String> {
    let core = s.split(['-', '+']).next().unwrap_or(s).trim();
    let mut parts = core.split('.');
    let major = parts
        .next()
        .ok_or_else(|| format!("Invalid version: {s}"))?
        .parse::<u32>()
        .map_err(|_| format!("Invalid version: {s}"))?;
    let minor = parts
        .next()
        .unwrap_or("0")
        .parse::<u32>()
        .map_err(|_| format!("Invalid version: {s}"))?;
    let patch = parts
        .next()
        .unwrap_or("0")
        .parse::<u32>()
        .map_err(|_| format!("Invalid version: {s}"))?;
    Ok((major, minor, patch))
}

fn run_curl(args: &[&str]) -> Result<std::process::Output, String> {
    Command::new("curl")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "curl not found — install curl to check for updates".to_string()
            } else {
                format!("Failed to run curl: {e}")
            }
        })
}

fn stdout_text(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn json_string_field(json: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{key}\"");
    let idx = json.find(&pattern)?;
    parse_json_string_after_key(&json[idx + pattern.len()..])
}

fn parse_json_string_after_key(after_key: &str) -> Option<String> {
    let after = after_key.trim_start();
    let after = after.strip_prefix(':')?.trim_start();
    let after = after.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = after.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(n) = chars.next() {
                out.push(n);
            }
        } else if c == '"' {
            break;
        } else {
            out.push(c);
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

/// Collect `(name, browser_download_url)` pairs from a GitHub release JSON body.
///
/// Walks `browser_download_url` first, then looks backward for the nearest `"name"`
/// (GitHub puts a large nested `uploader` object between those fields).
fn github_assets(json: &str) -> Vec<(String, String)> {
    const LOOKBACK: usize = 16 * 1024;
    let mut assets = Vec::new();
    let mut search_from = 0usize;
    let key = "\"browser_download_url\"";

    while let Some(rel) = json[search_from..].find(key) {
        let url_key_at = search_from + rel;
        let after_url = &json[url_key_at + key.len()..];
        let Some(url) = parse_json_string_after_key(after_url) else {
            search_from = url_key_at + key.len();
            continue;
        };

        let lookback_start = url_key_at.saturating_sub(LOOKBACK);
        let before = &json[lookback_start..url_key_at];
        let Some(name) = nearest_preceding_asset_name(before) else {
            search_from = url_key_at + key.len();
            continue;
        };

        assets.push((name, url));
        search_from = url_key_at + key.len();
    }
    assets
}

fn is_release_asset_name(name: &str) -> bool {
    name == "SHA256SUMS"
        || name.ends_with(".tar.gz")
        || (name.starts_with("replayforge-") && name.contains('.'))
}

/// Nearest preceding `"name"` that looks like a release asset (skip uploader display names).
fn nearest_preceding_asset_name(before: &str) -> Option<String> {
    let mut last: Option<String> = None;
    let mut search = before;
    while let Some(name_rel) = search.find("\"name\"") {
        let after_name = &search[name_rel + "\"name\"".len()..];
        if let Some(name) = parse_json_string_after_key(after_name) {
            if is_release_asset_name(&name) {
                last = Some(name);
            }
        }
        search = &search[name_rel + 6..];
    }
    last
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_name_across_large_uploader_gap() {
        let gap = "x".repeat(3000);
        let json = format!(
            r#"{{"assets":[{{"name":"replayforge-0.1.3-linux-x86_64.tar.gz","uploader":{{"login":"x","name":"Not An Asset","pad":"{gap}"}},"browser_download_url":"https://github.com/Ayden9104/ReplayForge/releases/download/v0.1.3/replayforge-0.1.3-linux-x86_64.tar.gz"}},{{"name":"SHA256SUMS","uploader":{{"login":"x","name":"Nope"}},"browser_download_url":"https://github.com/Ayden9104/ReplayForge/releases/download/v0.1.3/SHA256SUMS"}}]}}"#
        );
        let assets = github_assets(&json);
        assert!(
            assets.iter().any(|(n, u)| {
                n == "replayforge-0.1.3-linux-x86_64.tar.gz" && u.starts_with("https://")
            }),
            "assets={assets:?}"
        );
        assert!(
            assets
                .iter()
                .any(|(n, u)| n == "SHA256SUMS" && u.starts_with("https://")),
            "assets={assets:?}"
        );
    }
}

fn summarize_body(body: &str) -> String {
    if let Some(err) = json_string_field(body, "message") {
        return err;
    }
    let trimmed: String = body.chars().take(180).collect();
    if body.chars().count() > 180 {
        format!("{trimmed}…")
    } else if trimmed.is_empty() {
        "empty response".into()
    } else {
        trimmed
    }
}
