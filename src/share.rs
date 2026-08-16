//! Upload clips to the ReplayForge share Worker (R2-backed) and return a share URL.
use std::path::Path;
use std::process::{Command, Stdio};

const MAX_BYTES: u64 = 500 * 1024 * 1024;
const USER_AGENT: &str = "ReplayForge/0.1";

/// Upload `path` via `{share_api_base}` and return the public share URL (`/c/:id`).
pub fn upload_share_link(path: &Path, share_api_base: &str) -> Result<String, String> {
    let base = share_api_base.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err("Share is disabled — enable ReplayForge cloud in Settings → Sharing".into());
    }
    if !base.starts_with("https://") {
        return Err("Share API base must start with https://".into());
    }
    if !path.is_file() {
        return Err(format!("File not found: {}", path.display()));
    }

    let size = std::fs::metadata(path)
        .map(|m| m.len())
        .map_err(|e| format!("Cannot read file size: {e}"))?;
    if size > MAX_BYTES {
        return Err(format!(
            "File is {:.0} MB; cloud share max is 500 MB. Trim or lower quality first.",
            size as f64 / (1024.0 * 1024.0)
        ));
    }

    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("clip.mp4");
    let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    let init_body = format!("{{\"size\":{size},\"filename\":{}}}", json_string(filename));
    let init_out = run_curl(&[
        "-sS",
        "--max-time",
        "60",
        "-A",
        USER_AGENT,
        "-H",
        "Content-Type: application/json",
        "-d",
        &init_body,
        &format!("{base}/v1/upload"),
    ])?;
    let init_text = stdout_text(&init_out);
    if !init_out.status.success() {
        return Err(format!("Share init failed: {}", summarize_body(&init_text)));
    }

    let id = json_string_field(&init_text, "id")
        .ok_or_else(|| format!("Share init missing id: {}", summarize_body(&init_text)))?;
    let upload_url = json_string_field(&init_text, "uploadUrl").ok_or_else(|| {
        format!(
            "Share init missing uploadUrl: {}",
            summarize_body(&init_text)
        )
    })?;
    if !upload_url.starts_with("https://") {
        return Err("Share init returned a non-https uploadUrl".into());
    }
    let share_url = json_string_field(&init_text, "shareUrl").ok_or_else(|| {
        format!(
            "Share init missing shareUrl: {}",
            summarize_body(&init_text)
        )
    })?;
    if !share_url.starts_with("https://") {
        return Err("Share init returned a non-https shareUrl".into());
    }
    let put_out = run_curl(&[
        "-sS",
        "--max-time",
        "600",
        "-A",
        USER_AGENT,
        "-X",
        "PUT",
        "-H",
        "Content-Type: video/mp4",
        "--upload-file",
        &abs.display().to_string(),
        &upload_url,
    ])?;
    if !put_out.status.success() {
        let detail = stdout_text(&put_out);
        let err = String::from_utf8_lossy(&put_out.stderr);
        return Err(format!(
            "Upload to storage failed: {}",
            if !detail.is_empty() {
                summarize_body(&detail)
            } else if !err.trim().is_empty() {
                err.trim().to_string()
            } else {
                format!("curl exited with {}", put_out.status)
            }
        ));
    }

    let complete_out = run_curl(&[
        "-sS",
        "--max-time",
        "60",
        "-A",
        USER_AGENT,
        "-X",
        "POST",
        &format!("{base}/v1/upload/{id}/complete"),
    ])?;
    if !complete_out.status.success() {
        return Err(format!(
            "Share complete failed: {}",
            summarize_body(&stdout_text(&complete_out))
        ));
    }

    Ok(share_url)
}

pub fn share_link_note(_url: &str) -> &'static str {
    "cloud share link (expires in ~7 days)"
}

fn run_curl(args: &[&str]) -> Result<std::process::Output, String> {
    Command::new("curl")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "curl not found — install curl to use Share link".to_string()
            } else {
                format!("Failed to run curl: {e}")
            }
        })
}

fn stdout_text(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn json_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Minimal extractor for `"key":"value"` string fields (sufficient for our API).
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
    if let Some(err) = json_string_field(body, "error") {
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
