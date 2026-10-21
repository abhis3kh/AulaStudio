use crate::{
    audio_capture::LoopbackCapture,
    music_dsp::{self, Analyzer, AudioFrame, Settings},
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub type Shared = Arc<Mutex<MusicState>>;
#[derive(Clone)]
pub struct MusicState {
    pub active: bool,
    pub ready: bool,
    pub error: Option<String>,
    pub frame: AudioFrame,
    pub colors: [[u8; 3]; 126],
    pub frames: u64,
    pub usb_frames: u64,
    pub sample_rate: u32,
    pub settings: Settings,
}
impl MusicState {
    pub fn new(settings: Settings) -> Self {
        Self {
            active: true,
            ready: false,
            error: None,
            frame: AudioFrame::default(),
            colors: [[0; 3]; 126],
            frames: 0,
            usb_frames: 0,
            sample_rate: 0,
            settings,
        }
    }
}

pub struct MusicSession {
    pub shared: Shared,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl MusicSession {
    pub fn start(
        context: eframe::egui::Context,
        settings: Settings,
        keys: Vec<(usize, f32, f32)>,
    ) -> Result<Self, String> {
        let shared = Arc::new(Mutex::new(MusicState::new(settings)));
        let stop = Arc::new(AtomicBool::new(false));
        let state = shared.clone();
        let cancelled = stop.clone();
        let handle = thread::Builder::new()
            .name("f75-audio".into())
            .spawn(move || {
                let result = run(&state, &cancelled, &keys, &context);
                if let Ok(mut state) = state.lock() {
                    state.active = false;
                    if let Err(error) = result {
                        state.error = Some(error);
                    }
                }
                context.request_repaint();
            })
            .map_err(|e| format!("Cannot start audio capture: {e}"))?;
        Ok(Self {
            shared,
            stop,
            thread: Some(handle),
        })
    }
    pub fn snapshot(&self) -> Result<MusicState, String> {
        self.shared
            .lock()
            .map(|state| state.clone())
            .map_err(|_| "Audio worker state is unavailable".into())
    }
    #[cfg(test)]
    pub fn from_state(state: MusicState) -> Self {
        Self {
            shared: Arc::new(Mutex::new(state)),
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        }
    }
    pub fn update(&self, settings: Settings) {
        if let Ok(mut state) = self.shared.lock() {
            state.settings = settings;
        }
    }
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Ok(mut state) = self.shared.lock() {
            state.active = false;
        }
        // Capture may still be initializing inside Windows. Cancellation stays set;
        // never wait for an audio driver on the UI thread.
        if self
            .thread
            .as_ref()
            .is_some_and(|handle| handle.is_finished())
            && let Some(handle) = self.thread.take()
        {
            let _ = handle.join();
        }
    }
}
impl Drop for MusicSession {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run(
    state: &Shared,
    cancelled: &AtomicBool,
    keys: &[(usize, f32, f32)],
    context: &eframe::egui::Context,
) -> Result<(), String> {
    let mut capture = LoopbackCapture::open()?;
    let mut analyzer = Analyzer::new(capture.sample_rate());
    {
        let mut state = state.lock().map_err(|_| "Audio state lock failed")?;
        if cancelled.load(Ordering::Acquire) {
            return Ok(());
        }
        state.ready = true;
        state.sample_rate = capture.sample_rate();
    }
    context.request_repaint();
    let start = Instant::now();
    let mut next = Instant::now();
    let mut samples = Vec::with_capacity(8192);
    while !cancelled.load(Ordering::Acquire) {
        capture.read_mono(&mut samples)?;
        if next <= Instant::now() {
            let settings = state
                .lock()
                .map_err(|_| "Audio state lock failed")?
                .settings;
            let frame = analyzer.update(&samples, settings);
            samples.clear();
            let colors = music_dsp::colors(frame, settings, keys, start.elapsed().as_secs_f32());
            {
                let mut state = state.lock().map_err(|_| "Audio state lock failed")?;
                state.frame = frame;
                state.colors = colors;
                state.frames += 1;
            }
            context.request_repaint();
            next = Instant::now() + Duration::from_millis(33);
        }
        // Keep the sample backlog bounded if the device delivers an unusually large packet burst.
        if samples.len() > 16384 {
            samples.drain(..samples.len() - 16384);
        }
        thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}
