//! Short UI sound effects (clip saved cue).
use rodio::source::{SineWave, Source};
use rodio::{OutputStream, Sink};
use std::thread;
use std::time::Duration;

/// Play a short two-tone “clip saved” beep. Best-effort; never panics.
pub fn play_clip_saved() {
    thread::spawn(|| {
        let Ok((_stream, handle)) = OutputStream::try_default() else {
            return;
        };
        let Ok(sink) = Sink::try_new(&handle) else {
            return;
        };

        let first = SineWave::new(880.0)
            .take_duration(Duration::from_millis(70))
            .amplify(0.18);
        let gap = SineWave::new(1.0)
            .take_duration(Duration::from_millis(30))
            .amplify(0.0);
        let second = SineWave::new(1174.7)
            .take_duration(Duration::from_millis(110))
            .amplify(0.16);

        sink.append(first);
        sink.append(gap);
        sink.append(second);
        sink.sleep_until_end();
    });
}
