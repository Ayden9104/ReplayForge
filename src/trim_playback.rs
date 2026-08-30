use crate::clips::{build_trim_audio_filter, TrimCompressPreset};
use crate::host::host_command;
use rodio::source::Source;
use rodio::{OutputStream, Sink};
use std::io::Read;
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const VIDEO_WIDTH: usize = 854;
const VIDEO_HEIGHT: usize = 480;
const FRAME_BYTES: usize = VIDEO_WIDTH * VIDEO_HEIGHT * 3;
const PLAYBACK_FPS: f64 = 30.0;
const AUDIO_SAMPLE_RATE: u32 = 48000;
const AUDIO_CHANNELS: u16 = 2;

pub struct TrimFrame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub time_secs: f64,
}

pub struct TrimPlayback {
    handle: TrimPlaybackHandle,
    frame_rx: Receiver<TrimFrame>,
    pending_frame: Option<TrimFrame>,
    pub start_secs: f64,
    pub selection_secs: f64,
    pub audio_enabled: bool,
    pub audio_error: Option<String>,
}

impl TrimPlayback {
    /// Play `[start_secs, end_secs)` from `path` (caller clamps playhead into the keep range).
    pub fn start(
        path: &Path,
        start_secs: f64,
        end_secs: f64,
        audio_gain: f32,
        compress: TrimCompressPreset,
    ) -> Result<Self, String> {
        if end_secs <= start_secs {
            return Err("Invalid play range".into());
        }
        let selection_secs = end_secs - start_secs;
        let path_str = path.to_string_lossy();
        let start = format!("{start_secs:.3}");
        let duration = format!("{selection_secs:.3}");

        let stop = Arc::new(AtomicBool::new(false));
        let video_child: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
        let audio_child: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));

        // Small buffer: producer paces to realtime; consumer syncs by clock.
        let (frame_tx, frame_rx) = mpsc::sync_channel(2);

        spawn_video_thread(
            Arc::clone(&stop),
            Arc::clone(&video_child),
            path_str.to_string(),
            start.clone(),
            duration.clone(),
            start_secs,
            frame_tx,
        )?;

        let (audio_stream, sink, audio_enabled, audio_error) = setup_audio_playback(
            Arc::clone(&stop),
            Arc::clone(&audio_child),
            path_str.to_string(),
            start,
            duration,
            audio_gain,
            compress,
        );

        let handle = TrimPlaybackHandle {
            stop,
            video_child,
            audio_child,
            _audio_stream: audio_stream,
            sink,
        };

        Ok(Self {
            handle,
            frame_rx,
            pending_frame: None,
            start_secs,
            selection_secs,
            audio_enabled,
            audio_error,
        })
    }

    pub fn stop(&mut self) {
        self.handle.stop();
    }

    pub fn is_active(&self) -> bool {
        !self.handle.stop.load(Ordering::Relaxed)
    }

    pub fn set_volume(&self, volume: f32) {
        if let Some(sink) = &self.handle.sink {
            sink.set_volume(volume.clamp(0.0, 1.0));
        }
    }

    /// Return the newest frame due at `target_secs` (media timeline). Hold early frames.
    pub fn take_frame_for_time(&mut self, target_secs: f64) -> Option<TrimFrame> {
        let mut latest: Option<TrimFrame> = None;

        if let Some(frame) = self.pending_frame.take() {
            if frame.time_secs <= target_secs + 0.002 {
                latest = Some(frame);
            } else {
                self.pending_frame = Some(frame);
                return None;
            }
        }

        while let Ok(frame) = self.frame_rx.try_recv() {
            if frame.time_secs <= target_secs + 0.002 {
                latest = Some(frame);
            } else {
                self.pending_frame = Some(frame);
                break;
            }
        }

        latest
    }
}

pub struct TrimPlaybackHandle {
    stop: Arc<AtomicBool>,
    video_child: Arc<Mutex<Option<Child>>>,
    audio_child: Arc<Mutex<Option<Child>>>,
    _audio_stream: Option<OutputStream>,
    sink: Option<Sink>,
}

impl TrimPlaybackHandle {
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(mut child) = self.video_child.lock().unwrap().take() {
            let _ = child.kill();
        }
        if let Some(mut child) = self.audio_child.lock().unwrap().take() {
            let _ = child.kill();
        }
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
    }
}

fn sleep_until(deadline: Instant, stop: &AtomicBool) {
    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        let remaining = deadline.saturating_duration_since(now);
        thread::sleep(remaining.min(Duration::from_millis(5)));
    }
}

fn spawn_video_thread(
    stop: Arc<AtomicBool>,
    video_child: Arc<Mutex<Option<Child>>>,
    path: String,
    start: String,
    duration: String,
    start_secs: f64,
    frame_tx: SyncSender<TrimFrame>,
) -> Result<(), String> {
    let mut child = host_command(
        "ffmpeg",
        &[
            "-nostdin",
            "-loglevel",
            "error",
            "-ss",
            &start,
            "-i",
            &path,
            "-t",
            &duration,
            "-an",
            "-vf",
            "scale=854:480:force_original_aspect_ratio=decrease,pad=854:480:(ow-iw)/2:(oh-ih)/2",
            "-r",
            "30",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-",
        ],
    )
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .map_err(|e| format!("Failed to start ffmpeg for video preview: {e}"))?;

    let stdout = child
        .stdout
        .take()
        .ok_or("ffmpeg video stdout unavailable".to_string())?;
    video_child.lock().unwrap().replace(child);

    thread::spawn(move || {
        let mut stdout = stdout;
        let mut rgb = vec![0u8; FRAME_BYTES];
        let mut rgba = Vec::with_capacity(FRAME_BYTES / 3 * 4);
        let mut frame_idx = 0u64;
        // Pace emission to 1× wall clock so the UI can't drain frames as a timelapse.
        let origin = Instant::now();

        while !stop.load(Ordering::Relaxed) {
            if !read_exact(&mut stdout, &mut rgb, &stop) {
                break;
            }
            rgb24_to_rgba(&rgb, &mut rgba);
            let time_secs = start_secs + frame_idx as f64 / PLAYBACK_FPS;

            let due = origin + Duration::from_secs_f64(frame_idx as f64 / PLAYBACK_FPS);
            sleep_until(due, &stop);
            if stop.load(Ordering::Relaxed) {
                break;
            }

            let frame = TrimFrame {
                rgba,
                width: VIDEO_WIDTH as u32,
                height: VIDEO_HEIGHT as u32,
                time_secs,
            };
            if frame_tx.send(frame).is_err() {
                break;
            }
            rgba = Vec::with_capacity(FRAME_BYTES / 3 * 4);
            frame_idx += 1;
        }
    });

    Ok(())
}

fn setup_audio_playback(
    stop: Arc<AtomicBool>,
    audio_child: Arc<Mutex<Option<Child>>>,
    path: String,
    start: String,
    duration: String,
    audio_gain: f32,
    compress: TrimCompressPreset,
) -> (Option<OutputStream>, Option<Sink>, bool, Option<String>) {
    let (pcm_tx, pcm_rx) = mpsc::sync_channel::<Vec<f32>>(64);

    let mut ffmpeg_args = vec![
        "-nostdin",
        "-loglevel",
        "error",
        "-ss",
        &start,
        "-i",
        &path,
        "-t",
        &duration,
        "-vn",
        "-ac",
        "2",
        "-ar",
        "48000",
    ];
    let audio_filter = build_trim_audio_filter(audio_gain, compress);
    if let Some(af) = &audio_filter {
        ffmpeg_args.push("-af");
        ffmpeg_args.push(af);
    }
    ffmpeg_args.extend(["-f", "f32le", "-"]);

    let mut child = match host_command("ffmpeg", &ffmpeg_args)
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            return (
                None,
                None,
                false,
                Some(format!("ffmpeg audio spawn failed: {e}")),
            );
        }
    };

    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            let _ = child.kill();
            return (
                None,
                None,
                false,
                Some("ffmpeg audio stdout unavailable".into()),
            );
        }
    };
    audio_child.lock().unwrap().replace(child);

    let stop_audio = Arc::clone(&stop);
    thread::spawn(move || {
        let mut stdout = stdout;
        let mut buf = [0u8; 8192];
        loop {
            if stop_audio.load(Ordering::Relaxed) {
                break;
            }
            match stdout.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let samples = bytes_to_f32(&buf[..n]);
                    if samples.is_empty() {
                        continue;
                    }
                    if pcm_tx.send(samples).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    match OutputStream::try_default() {
        Ok((stream, stream_handle)) => match Sink::try_new(&stream_handle) {
            Ok(sink) => {
                let source = PcmStream {
                    receiver: pcm_rx,
                    buffer: Vec::new(),
                    index: 0,
                };
                sink.append(source);
                (Some(stream), Some(sink), true, None)
            }
            Err(e) => {
                drop(pcm_rx);
                if let Some(mut child) = audio_child.lock().unwrap().take() {
                    let _ = child.kill();
                }
                (
                    None,
                    None,
                    false,
                    Some(format!("Audio sink unavailable: {e}")),
                )
            }
        },
        Err(e) => {
            drop(pcm_rx);
            if let Some(mut child) = audio_child.lock().unwrap().take() {
                let _ = child.kill();
            }
            (
                None,
                None,
                false,
                Some(format!("No audio output device: {e}")),
            )
        }
    }
}

struct PcmStream {
    receiver: mpsc::Receiver<Vec<f32>>,
    buffer: Vec<f32>,
    index: usize,
}

impl Iterator for PcmStream {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.index < self.buffer.len() {
                let sample = self.buffer[self.index];
                self.index += 1;
                return Some(sample);
            }
            match self.receiver.recv_timeout(Duration::from_millis(50)) {
                Ok(chunk) => {
                    self.buffer = chunk;
                    self.index = 0;
                }
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return None,
            }
        }
    }
}

impl Source for PcmStream {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        AUDIO_CHANNELS
    }

    fn sample_rate(&self) -> u32 {
        AUDIO_SAMPLE_RATE
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

fn read_exact(stdout: &mut impl Read, buf: &mut [u8], stop: &Arc<AtomicBool>) -> bool {
    let mut filled = 0;
    while filled < buf.len() {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        match stdout.read(&mut buf[filled..]) {
            Ok(0) => return false,
            Ok(n) => filled += n,
            Err(_) => return false,
        }
    }
    true
}

fn rgb24_to_rgba(rgb: &[u8], rgba: &mut Vec<u8>) {
    rgba.clear();
    for chunk in rgb.chunks_exact(3) {
        rgba.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]);
    }
}

fn bytes_to_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}
