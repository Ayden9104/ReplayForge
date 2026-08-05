use crate::detect::clip_duration_secs;
use crate::host::host_command;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::Stdio;

const MIN_TRIM_SECS: f64 = 0.5;

/// Extract a single frame as PNG bytes at `time_secs` (for trim preview).
pub fn extract_frame_png(path: &Path, time_secs: f64) -> Result<Vec<u8>, String> {
    let path_buf = path.to_path_buf();
    let duration = clip_duration_secs(&path_buf).unwrap_or(time_secs);
    let clamped = if duration > 0.05 {
        time_secs.clamp(0.0, duration - 0.05)
    } else {
        0.0
    };
    let time = format!("{clamped:.3}");
    let input = path.to_string_lossy();

    let mut child = host_command(
        "ffmpeg",
        &[
            "-y",
            "-ss",
            &time,
            "-i",
            &input,
            "-frames:v",
            "1",
            "-f",
            "image2pipe",
            "-vcodec",
            "png",
            "-",
        ],
    )
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .map_err(|e| format!("Failed to run ffmpeg for frame preview: {e}"))?;

    let mut png = Vec::new();
    if let Some(mut stdout) = child.stdout.take() {
        stdout
            .read_to_end(&mut png)
            .map_err(|e| format!("Failed to read frame preview: {e}"))?;
    }

    let status = child
        .wait()
        .map_err(|e| format!("ffmpeg frame preview wait failed: {e}"))?;

    if !status.success() || png.is_empty() {
        return Err("Frame preview failed. Is ffmpeg installed?".into());
    }

    Ok(png)
}

/// Frame count for a timeline filmstrip (8–24 thumbs from width).
pub fn filmstrip_frame_count(timeline_width: f32) -> usize {
    ((timeline_width / 48.0).floor() as usize).clamp(8, 24)
}

/// Extract a horizontal tiled JPEG filmstrip for the trim timeline.
pub fn extract_filmstrip_jpeg(
    path: &Path,
    duration_secs: f64,
    frame_count: usize,
) -> Result<Vec<u8>, String> {
    let frame_count = frame_count.max(1);
    let interval = (duration_secs / frame_count as f64).max(0.05);
    let interval_str = format!("{interval:.3}");
    let tile = format!("{frame_count}x1");
    let vf = format!(
        "fps=1/{interval_str},scale=80:45:force_original_aspect_ratio=decrease,pad=80:45:(ow-iw)/2:(oh-ih)/2,tile={tile}"
    );
    let input = path.to_string_lossy();

    let mut child = host_command(
        "ffmpeg",
        &[
            "-y",
            "-i",
            &input,
            "-vf",
            &vf,
            "-frames:v",
            "1",
            "-q:v",
            "5",
            "-f",
            "image2pipe",
            "-vcodec",
            "mjpeg",
            "-",
        ],
    )
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .map_err(|e| format!("Failed to run ffmpeg for filmstrip: {e}"))?;

    let mut jpeg = Vec::new();
    if let Some(mut stdout) = child.stdout.take() {
        stdout
            .read_to_end(&mut jpeg)
            .map_err(|e| format!("Failed to read filmstrip: {e}"))?;
    }

    let status = child
        .wait()
        .map_err(|e| format!("ffmpeg filmstrip wait failed: {e}"))?;

    if !status.success() || jpeg.is_empty() {
        return Err("Filmstrip generation failed. Is ffmpeg installed?".into());
    }

    Ok(jpeg)
}

/// Generate a sidecar `.png` thumbnail for a clip (first frame).
pub fn generate_clip_thumbnail(path: &Path) -> Result<(), String> {
    let thumbnail = path.with_extension("png");
    let input = path.to_string_lossy();
    let output = thumbnail.to_string_lossy();

    let status = host_command(
        "ffmpeg",
        &[
            "-y",
            "-ss",
            "0",
            "-i",
            &input,
            "-frames:v",
            "1",
            "-update",
            "1",
            &output,
        ],
    )
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .status()
    .map_err(|e| format!("Failed to run ffmpeg for thumbnail: {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Thumbnail generation failed ({status}). Is ffmpeg installed?"
        ))
    }
}

/// Trim a saved clip in place: keep `[start_secs, end_secs)`.
pub fn trim_clip(path: &Path, start_secs: f64, end_secs: f64) -> Result<(), String> {
    let path_buf = path.to_path_buf();
    let Some(duration) = clip_duration_secs(&path_buf) else {
        return Err("Could not read clip duration (is ffprobe installed?)".into());
    };

    if start_secs < 0.0 || end_secs <= start_secs {
        return Err("Invalid trim range: start must be before end".into());
    }
    if end_secs > duration + 0.05 {
        return Err(format!(
            "End time {end_secs:.1}s exceeds clip duration {duration:.1}s"
        ));
    }
    let kept = end_secs - start_secs;
    if kept < MIN_TRIM_SECS {
        return Err(format!(
            "Trimmed clip must be at least {MIN_TRIM_SECS}s (got {kept:.1}s)"
        ));
    }

    let parent = path
        .parent()
        .ok_or_else(|| "Clip has no parent directory".to_string())?;
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| "Invalid clip filename".to_string())?;
    let temp = parent.join(format!("{stem}.trim.tmp.mp4"));

    if temp.exists() {
        let _ = fs::remove_file(&temp);
    }

    let input = path.to_string_lossy();
    let temp_str = temp.to_string_lossy();
    let start = format!("{start_secs:.3}");
    let end = format!("{end_secs:.3}");

    let copy_ok = host_command(
        "ffmpeg",
        &[
            "-y", "-ss", &start, "-to", &end, "-i", &input, "-c", "copy", &temp_str,
        ],
    )
    .stdout(Stdio::null())
    .stderr(Stdio::piped())
    .status()
    .map(|s| s.success())
    .unwrap_or(false);

    if !copy_ok {
        if temp.exists() {
            let _ = fs::remove_file(&temp);
        }
        let reencode_ok = host_command(
            "ffmpeg",
            &[
                "-y", "-ss", &start, "-to", &end, "-i", &input, "-c:v", "libx264", "-preset",
                "fast", "-crf", "18", "-c:a", "aac", &temp_str,
            ],
        )
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

        if !reencode_ok {
            if temp.exists() {
                let _ = fs::remove_file(&temp);
            }
            return Err("Trim failed (stream copy and re-encode). Is ffmpeg installed?".into());
        }
    }

    if !temp.exists() {
        return Err("Trim produced no output file".into());
    }

    fs::rename(&temp, path).map_err(|e| format!("Failed to replace clip: {e}"))?;

    if let Err(error) = generate_clip_thumbnail(path) {
        eprintln!("{error}");
    }

    Ok(())
}
