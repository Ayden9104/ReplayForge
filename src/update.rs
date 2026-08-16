//! Check GitHub Releases for a newer ReplayForge version.
use std::process::{Command, Stdio};

const REPO: &str = "Ayden9104/ReplayForge";
const USER_AGENT: &str = "ReplayForge";

#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub latest: String,
    pub html_url: String,
    pub newer: bool,
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Query GitHub for the latest release and compare to this build.
pub fn check_latest() -> Result<UpdateInfo, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
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
    // Optional: `GH_TOKEN` / `GITHUB_TOKEN` so private-repo checks work when set.
    if let Ok(token) = std::env::var("GH_TOKEN").or_else(|_| std::env::var("GITHUB_TOKEN")) {
        let token = token.trim().to_string();
        if !token.is_empty() {
            args.push("-H".to_string());
            args.push(format!("Authorization: Bearer {token}"));
        }
    }
    args.push(url);

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

    let tag = json_string_field(body, "tag_name")
        .ok_or_else(|| "Update check failed: missing tag_name in GitHub response".to_string())?;
    let latest = tag.trim().trim_start_matches('v').to_string();
    let html_url = json_string_field(body, "html_url")
        .unwrap_or_else(|| format!("https://github.com/{REPO}/releases/tag/{}", tag.trim()));

    let current = current_version();
    let newer = is_newer(&latest, current)?;

    Ok(UpdateInfo {
        latest,
        html_url,
        newer,
    })
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
    let latest_v = parse_semver(latest)?;
    let current_v = parse_semver(current)?;
    Ok(latest_v > current_v)
}

/// Parse `major.minor.patch`, ignoring any `-pre` / `+build` suffix.
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
    let after = json[idx + pattern.len()..].trim_start();
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
