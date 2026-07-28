#[derive(Default)]
pub struct Recorder {
    running: bool,
}

impl Recorder {
    pub fn start(&mut self) {
        self.running = true;
        println!("Recorder started");
    }

    pub fn stop(&mut self) {
        self.running = false;
        println!("Recorder stopped");
    }

    pub fn is_running(&self) -> bool {
        self.running
    }
}
