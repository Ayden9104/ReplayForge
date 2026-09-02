use crate::config::{Config, SystemAudioMode};
use crate::host::host_output;

fn volume_percent(fraction: f32) -> u32 {
    (fraction.clamp(0.0, 1.0) * 100.0).round() as u32
}

fn wpctl_set_volume(target: &str, fraction: f32) -> Result<(), String> {
    let pct = format!("{}%", volume_percent(fraction));
    let output = host_output("wpctl", &["set-volume", target, &pct])
        .map_err(|e| format!("wpctl failed: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("wpctl set-volume failed: {stderr}"))
    }
}

fn pactl_set_source_volume(target: &str, fraction: f32) -> Result<(), String> {
    let pct = format!("{}%", volume_percent(fraction));
    let output = host_output("pactl", &["set-source-volume", target, &pct])
        .map_err(|e| format!("pactl failed: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("pactl set-source-volume failed: {stderr}"))
    }
}

fn pactl_default_sink_monitor() -> Result<String, String> {
    let output = host_output("pactl", &["get-default-sink"])
        .map_err(|e| format!("pactl failed: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("pactl get-default-sink failed: {stderr}"));
    }
    let sink = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if sink.is_empty() {
        return Err("pactl returned empty default sink".into());
    }
    Ok(format!("{sink}.monitor"))
}

fn set_volume_wpctl_or_pactl(wpctl_target: &str, pactl_target: &str, fraction: f32) -> Result<(), String> {
    if wpctl_set_volume(wpctl_target, fraction).is_ok() {
        return Ok(());
    }
    pactl_set_source_volume(pactl_target, fraction)
}

/// Set default mic (input) volume via PipeWire.
pub fn set_mic_volume(fraction: f32) -> Result<(), String> {
    set_volume_wpctl_or_pactl("@DEFAULT_AUDIO_SOURCE@", "@DEFAULT_SOURCE@", fraction)
}

/// Set default sink monitor volume (desktop capture tap) via PipeWire.
pub fn set_desktop_volume(fraction: f32) -> Result<(), String> {
    if wpctl_set_volume("@DEFAULT_AUDIO_SINK@.monitor", fraction).is_ok() {
        return Ok(());
    }
    let monitor = pactl_default_sink_monitor()?;
    pactl_set_source_volume(&monitor, fraction)
}

/// Apply capture volume sliders from config. Returns human-readable errors (non-fatal).
pub fn apply_config_volumes(config: &Config) -> Vec<String> {
    let mut errors = Vec::new();

    if config.capture_microphone {
        if let Err(error) = set_mic_volume(config.mic_volume) {
            errors.push(format!("Mic volume: {error}"));
        }
    }

    if config.capture_system_audio && config.system_audio_mode == SystemAudioMode::All {
        if let Err(error) = set_desktop_volume(config.desktop_audio_volume) {
            errors.push(format!("Desktop audio volume: {error}"));
        }
    }

    errors
}
