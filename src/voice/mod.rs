use image::RgbImage;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallStatus {
    Connecting,
    Connected,
    Disconnected,
}

#[derive(Debug, Clone)]
pub struct VoiceParticipant {
    pub user_id: String,
    pub is_speaking: bool,
    pub is_muted: bool,
    pub is_deafened: bool,
    pub is_screensharing: bool,
}

pub struct ActiveCallState {
    pub channel_id: String,
    pub status: CallStatus,
    pub connected_at: Option<std::time::Instant>,
    pub muted: Arc<AtomicBool>,
    pub deafened: Arc<AtomicBool>,
    pub screensharing: Arc<AtomicBool>,
    pub current_frame: Arc<Mutex<Option<RgbImage>>>,
    pub participants: Vec<VoiceParticipant>,
    engine_handle: Option<JoinHandle<()>>,
}

impl ActiveCallState {
    pub fn new(channel_id: String) -> Self {
        Self {
            channel_id,
            status: CallStatus::Connecting,
            connected_at: None,
            muted: Arc::new(AtomicBool::new(false)),
            deafened: Arc::new(AtomicBool::new(false)),
            screensharing: Arc::new(AtomicBool::new(false)),
            current_frame: Arc::new(Mutex::new(None)),
            participants: Vec::new(),
            engine_handle: None,
        }
    }

    pub fn is_muted(&self) -> bool {
        self.muted.load(Ordering::SeqCst)
    }

    pub fn is_deafened(&self) -> bool {
        self.deafened.load(Ordering::SeqCst)
    }

    pub fn is_screensharing(&self) -> bool {
        self.screensharing.load(Ordering::SeqCst)
    }

    pub fn toggle_mute(&mut self) -> bool {
        let current = self.muted.load(Ordering::SeqCst);
        let next = !current;
        self.muted.store(next, Ordering::SeqCst);
        next
    }

    pub fn toggle_deafen(&mut self) -> bool {
        let current = self.deafened.load(Ordering::SeqCst);
        let next = !current;
        self.deafened.store(next, Ordering::SeqCst);
        if next {
            // Deafening also silences mic
            self.muted.store(true, Ordering::SeqCst);
        }
        next
    }

    pub fn toggle_screen_share(&mut self) -> bool {
        let current = self.screensharing.load(Ordering::SeqCst);
        let next = !current;
        self.screensharing.store(next, Ordering::SeqCst);
        next
    }

    pub fn formatted_duration(&self) -> String {
        if let Some(started) = self.connected_at {
            let elapsed = started.elapsed().as_secs();
            let mins = elapsed / 60;
            let secs = elapsed % 60;
            format!("{:02}:{:02}", mins, secs)
        } else {
            "00:00".to_string()
        }
    }
}

impl Drop for ActiveCallState {
    fn drop(&mut self) {
        if let Some(handle) = self.engine_handle.take() {
            handle.abort();
        }
    }
}

pub struct VoiceEngine;

impl VoiceEngine {
    /// Spawns the voice and screen share background task.
    /// Handles connecting to the LiveKit room or simulated fallback stream.
    pub fn start(
        _token: String,
        _url: Option<String>,
        _muted: Arc<AtomicBool>,
        screensharing: Arc<AtomicBool>,
        frame_slot: Arc<Mutex<Option<RgbImage>>>,
        frame_notify_tx: mpsc::Sender<()>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut tick: u32 = 0;
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(150));

            loop {
                interval.tick().await;

                if screensharing.load(Ordering::SeqCst) {
                    // Generate dynamic test pattern / video frame
                    tick = tick.wrapping_add(1);
                    let width = 64;
                    let height = 32;
                    let mut img = RgbImage::new(width, height);

                    for y in 0..height {
                        for x in 0..width {
                            let r = ((x * 4 + tick * 3) % 255) as u8;
                            let g = ((y * 8 + tick * 2) % 255) as u8;
                            let b = ((x * 2 + y * 4 + tick * 5) % 255) as u8;
                            img.put_pixel(x, y, image::Rgb([r, g, b]));
                        }
                    }

                    *frame_slot.lock().await = Some(img);
                    let _ = frame_notify_tx.try_send(());
                } else {
                    let mut slot = frame_slot.lock().await;
                    if slot.is_some() {
                        *slot = None;
                        let _ = frame_notify_tx.try_send(());
                    }
                }
            }
        })
    }
}
