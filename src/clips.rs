use crate::detect::clip_duration_secs;
use crate::host::host_command;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;

const MIN_TRIM_SECS: f64 = 0.5;

/// Build an ffmpeg `-af` volume filter string when gain differs from unity.
pub fn build_trim_volume_filter(gain: f32) -> Option<String> {
    let gain = gain.clamp(0.0, 2.0);
    if (gain - 1.0).abs() > 0.001 {
        Some(format!("volume={gain:.3}"))
    } else {
        None
    }
}

fn run_ffmpeg_status(args: &[&str]) -> bool {
    host_command("ffmpeg", args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Path string safe to pass as an ffmpeg argv (never looks like a flag).
fn argv_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    if s.starts_with('-') {
        format!("./{s}")
    } else {
        s.into_owned()
    }
}

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
    let input = argv_path(path);

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
    let input = argv_path(path);

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

/// Peak count for waveform overlay (scales with timeline width).
pub fn waveform_peak_count(timeline_width: f32) -> usize {
    ((timeline_width / 3.0).floor() as usize).clamp(128, 512)
}

/// Decode mono PCM and downsample to `peak_count` RMS peaks in `0.0..=1.0`.
pub fn extract_waveform_peaks(
    path: &Path,
    duration_secs: f64,
    peak_count: usize,
) -> Result<Vec<f32>, String> {
    let peak_count = peak_count.max(1);
    let input = argv_path(path);
    // Cap decode length to avoid huge buffers on very long clips.
    let decode_secs = duration_secs.clamp(0.1, 600.0);
    let duration = format!("{decode_secs:.3}");

    let mut child = host_command(
        "ffmpeg",
        &[
            "-nostdin",
            "-loglevel",
            "error",
            "-i",
            &input,
            "-t",
            &duration,
            "-vn",
            "-ac",
            "1",
            "-ar",
            "8000",
            "-f",
            "f32le",
            "-",
        ],
    )
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .map_err(|e| format!("Failed to run ffmpeg for waveform: {e}"))?;

    let mut pcm_bytes = Vec::new();
    if let Some(mut stdout) = child.stdout.take() {
        stdout
            .read_to_end(&mut pcm_bytes)
            .map_err(|e| format!("Failed to read waveform PCM: {e}"))?;
    }

    let status = child
        .wait()
        .map_err(|e| format!("ffmpeg waveform wait failed: {e}"))?;

    if !status.success() {
        return Err("Waveform extraction failed. Is ffmpeg installed?".into());
    }
    if pcm_bytes.len() < 4 {
        return Ok(vec![0.0; peak_count]);
    }

    let samples: Vec<f32> = pcm_bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();

    if samples.is_empty() {
        return Ok(vec![0.0; peak_count]);
    }

    let bucket = (samples.len() as f64 / peak_count as f64).max(1.0);
    let mut peaks = Vec::with_capacity(peak_count);
    let mut max_peak = 0.0_f32;

    for i in 0..peak_count {
        let start = (i as f64 * bucket).floor() as usize;
        let end = (((i + 1) as f64 * bucket).floor() as usize).min(samples.len());
        if start >= end {
            peaks.push(0.0);
            continue;
        }
        let mut sum_sq = 0.0_f64;
        let mut n = 0usize;
        for &s in &samples[start..end] {
            sum_sq += (s as f64) * (s as f64);
            n += 1;
        }
        let rms = if n > 0 {
            (sum_sq / n as f64).sqrt() as f32
        } else {
            0.0
        };
        max_peak = max_peak.max(rms);
        peaks.push(rms);
    }

    if max_peak > 1e-6 {
        for p in &mut peaks {
            *p = (*p / max_peak).clamp(0.0, 1.0);
        }
    }

    Ok(peaks)
}

/// Generate a sidecar `.png` thumbnail for a clip (first frame).
pub fn generate_clip_thumbnail(path: &Path) -> Result<(), String> {
    let thumbnail = path.with_extension("png");
    let input = argv_path(path);
    let output = argv_path(&thumbnail);

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrimSaveMode {
    ReplaceOriginal,
    SaveCopy,
}

const MAX_TRIM_COPY_SUFFIX: u32 = 999;

/// Unique `{stem}_trim.mp4` path in the same folder as `source`.
pub fn unique_trim_copy_path(source: &Path) -> Result<PathBuf, String> {
    let parent = source
        .parent()
        .ok_or_else(|| "Clip has no parent directory".to_string())?;
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| "Invalid clip filename".to_string())?;

    let first = parent.join(format!("{stem}_trim.mp4"));
    if !first.exists() {
        return Ok(first);
    }

    for n in 2..=MAX_TRIM_COPY_SUFFIX {
        let candidate = parent.join(format!("{stem}_trim_{n}.mp4"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(format!(
        "Too many trim copies for {stem} (max {MAX_TRIM_COPY_SUFFIX})"
    ))
}

/// Trim a clip: keep `[start_secs, end_secs)` and write to original or a new copy path.
pub fn trim_clip(
    path: &Path,
    start_secs: f64,
    end_secs: f64,
    audio_gain: f32,
    mode: TrimSaveMode,
) -> Result<PathBuf, String> {
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

    let input = argv_path(path);
    let temp_str = argv_path(&temp);
    let start = format!("{start_secs:.3}");
    let end = format!("{end_secs:.3}");

    let audio_filter = build_trim_volume_filter(audio_gain);
    let mut ok = false;

    // If gain is non-unity, try re-encoding audio with the volume filter first.
    if let Some(af) = &audio_filter {
        ok = run_ffmpeg_status(&[
            "-y", "-ss", &start, "-to", &end, "-i", &input, "-c:v", "copy", "-c:a", "aac",
            "-b:a", "192k", "-af", af, &temp_str,
        ]);
        if !ok {
            if temp.exists() {
                let _ = fs::remove_file(&temp);
            }
            ok = run_ffmpeg_status(&[
                "-y", "-ss", &start, "-to", &end, "-i", &input, "-c:v", "libx264", "-preset",
                "fast", "-crf", "18", "-c:a", "aac", "-b:a", "192k", "-af", af, &temp_str,
            ]);
        }
    }

    // Stream copy (no gain adjustment needed or filter failed).
    if !ok {
        if temp.exists() {
            let _ = fs::remove_file(&temp);
        }
        ok = run_ffmpeg_status(&[
            "-y", "-ss", &start, "-to", &end, "-i", &input, "-c", "copy", &temp_str,
        ]);
    }

    // Full re-encode fallback.
    if !ok {
        if temp.exists() {
            let _ = fs::remove_file(&temp);
        }
        let mut reencode_args = vec![
            "-y", "-ss", &start, "-to", &end, "-i", &input, "-c:v", "libx264", "-preset", "fast",
            "-crf", "18", "-c:a", "aac",
        ];
        if let Some(af) = &audio_filter {
            reencode_args.extend(["-b:a", "192k", "-af", af]);
        }
        reencode_args.push(&temp_str);
        ok = run_ffmpeg_status(&reencode_args);
    }

    if !ok {
        if temp.exists() {
            let _ = fs::remove_file(&temp);
        }
        return Err("Trim failed (stream copy and re-encode). Is ffmpeg installed?".into());
    }

    if !temp.exists() {
        return Err("Trim produced no output file".into());
    }

    let output_path = match mode {
        TrimSaveMode::ReplaceOriginal => path.to_path_buf(),
        TrimSaveMode::SaveCopy => unique_trim_copy_path(path)?,
    };

    fs::rename(&temp, &output_path)
        .map_err(|e| format!("Failed to finalize trim output: {e}"))?;

    if let Err(error) = generate_clip_thumbnail(&output_path) {
        eprintln!("{error}");
    }

    Ok(output_path)
}
