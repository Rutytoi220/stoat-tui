pub mod screen;
pub mod sounds;

use crate::event::AppEvent;
use crate::{log_debug, log_error, log_info, log_warn};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, InputCallbackInfo, OutputCallbackInfo, Sample, SampleFormat, SizedSample, StreamConfig};
use futures_util::StreamExt;
use image::{DynamicImage, RgbaImage};
use livekit::options::TrackPublishOptions;
use livekit::prelude::*;
use livekit::webrtc::audio_frame::AudioFrame;
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::audio_source::{AudioSourceOptions, RtcAudioSource};
use livekit::webrtc::audio_stream::native::NativeAudioStream;
use livekit::webrtc::native::yuv_helper;
use livekit::webrtc::video_frame::{BoxVideoFrame, I420Buffer, VideoFormatType, VideoFrame, VideoRotation};
use livekit::webrtc::video_source::native::NativeVideoSource;
use livekit::webrtc::video_source::{RtcVideoSource, VideoResolution};
use livekit::webrtc::video_stream::native::NativeVideoStream;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

const WEBRTC_SAMPLE_RATE: u32 = 48_000;
const WEBRTC_CHANNELS: u32 = 1;
const FRAME_SAMPLES_10MS: usize = 480; // 48kHz * 10ms
const MAX_MIXER_BUFFER_SAMPLES: usize = 9_600; // 200ms cap per remote track

/// Extracts the Stoat user ULID from a LiveKit participant identity (`<user_id>` or `<user_id>:<device>`).
pub fn normalize_livekit_identity(identity: &str) -> String {
    identity.split(':').next().unwrap_or(identity).trim().to_string()
}

fn parse_participant_mute_deafen(
    metadata: &str,
    attrs: &HashMap<String, String>,
) -> (Option<bool>, Option<bool>) {
    let mut muted = None;
    let mut deafened = None;

    for key in ["muted", "is_muted"] {
        if let Some(v) = attrs.get(key) {
            if let Ok(b) = v.trim().parse::<bool>() {
                muted = Some(b);
            }
        }
    }
    for key in ["deafened", "deaf", "is_deafened"] {
        if let Some(v) = attrs.get(key) {
            if let Ok(b) = v.trim().parse::<bool>() {
                deafened = Some(b);
            }
        }
    }

    if !metadata.trim().is_empty() {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(metadata) {
            for key in ["muted", "is_muted"] {
                if let Some(b) = val.get(key).and_then(|x| x.as_bool()) {
                    muted = Some(b);
                }
            }
            for key in ["deafened", "deaf", "is_deafened"] {
                if let Some(b) = val.get(key).and_then(|x| x.as_bool()) {
                    deafened = Some(b);
                }
            }
        }
    }

    (muted, deafened)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallStatus {
    Connecting,
    Connected,
    Disconnected,
}

#[derive(Debug, Clone, Default)]
pub struct VoiceParticipant {
    pub user_id: String,
    pub account_id: String,
    pub is_local: bool,
    pub is_speaking: bool,
    pub is_muted: bool,
    pub is_deafened: bool,
    pub is_screensharing: bool,
}

#[derive(Clone)]
pub struct VideoFrameSnapshot {
    pub image: DynamicImage,
    pub owner_id: String,
    pub fps: u32,
    pub is_remote: bool,
}

pub struct ActiveCallState {
    pub channel_id: String,
    pub account_id: String,
    pub local_user_id: String,
    pub status: CallStatus,
    pub connected_at: Option<std::time::Instant>,
    pub muted: Arc<AtomicBool>,
    pub deafened: Arc<AtomicBool>,
    pub screensharing: Arc<AtomicBool>,
    pub pending_screen_target: Arc<StdMutex<Option<screen::ScreenShareTargetKind>>>,
    pub show_local_preview: bool,
    pub local_speaking: bool,
    pub latest_video_frame: Arc<Mutex<Option<VideoFrameSnapshot>>>,
    pub participants: Vec<VoiceParticipant>,
    pub shutdown: Arc<AtomicBool>,
    engine_handle: Option<JoinHandle<()>>,
}

impl ActiveCallState {
    pub fn new(channel_id: String) -> Self {
        Self {
            channel_id,
            account_id: String::new(),
            local_user_id: String::new(),
            status: CallStatus::Connecting,
            connected_at: None,
            muted: Arc::new(AtomicBool::new(false)),
            deafened: Arc::new(AtomicBool::new(false)),
            screensharing: Arc::new(AtomicBool::new(false)),
            pending_screen_target: Arc::new(StdMutex::new(None)),
            show_local_preview: true,
            local_speaking: false,
            latest_video_frame: Arc::new(Mutex::new(None)),
            participants: Vec::new(),
            shutdown: Arc::new(AtomicBool::new(false)),
            engine_handle: None,
        }
    }

    pub fn toggle_local_preview(&mut self) -> bool {
        self.show_local_preview = !self.show_local_preview;
        self.show_local_preview
    }

    pub fn set_engine_handle(&mut self, handle: JoinHandle<()>) {
        if let Some(old) = self.engine_handle.take() {
            old.abort();
        }
        self.engine_handle = Some(handle);
    }

    pub fn take_engine_handle(&mut self) -> Option<JoinHandle<()>> {
        self.shutdown.store(true, Ordering::SeqCst);
        self.engine_handle.take()
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
        self.shutdown.store(true, Ordering::SeqCst);
    }
}

/// Stateful linear resampler converting mono `f32` audio from `in_rate` to 48kHz `i16`.
pub struct Mono48kResampler {
    in_rate: u32,
    phase: f64,
    prev: f32,
}

impl Mono48kResampler {
    pub fn new(in_rate: u32) -> Self {
        Self {
            in_rate: in_rate.max(1),
            phase: 0.0,
            prev: 0.0,
        }
    }

    pub fn push(&mut self, input: &[f32], out: &mut Vec<i16>) {
        if input.is_empty() {
            return;
        }
        if self.in_rate == WEBRTC_SAMPLE_RATE {
            for &s in input {
                out.push((s.clamp(-1.0, 1.0) * 32767.0) as i16);
            }
            self.prev = *input.last().unwrap_or(&0.0);
            return;
        }

        let step = self.in_rate as f64 / WEBRTC_SAMPLE_RATE as f64;
        let len = input.len();

        while self.phase < len as f64 {
            let idx = self.phase.floor() as isize;
            let frac = (self.phase - idx as f64) as f32;
            let s0 = if idx < 0 {
                self.prev
            } else {
                input[idx as usize]
            };
            let s1 = if (idx + 1) as usize >= len {
                input[len - 1]
            } else if idx + 1 < 0 {
                self.prev
            } else {
                input[(idx + 1) as usize]
            };
            let interp = s0 + (s1 - s0) * frac;
            out.push((interp.clamp(-1.0, 1.0) * 32767.0) as i16);
            self.phase += step;
        }

        self.phase -= len as f64;
        self.prev = input[len - 1];
    }
}

type MixerBuffers = Arc<StdMutex<HashMap<String, VecDeque<i16>>>>;

/// Converts packed RGBA (`[R, G, B, A]`) into a WebRTC `I420Buffer`.
/// Returns `None` if dimensions are zero/odd or `rgba.len()` does not match `width * height * 4`.
pub fn rgba_to_i420_buffer(rgba: &[u8], width: u32, height: u32) -> Option<I420Buffer> {
    if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
        return None;
    }
    let expected_len = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;
    if rgba.len() != expected_len {
        return None;
    }

    let mut i420 = I420Buffer::new(width, height);
    let (sy, su, sv) = i420.strides();
    let (dy, du, dv) = i420.data_mut();
    yuv_helper::abgr_to_i420(
        rgba,
        width * 4,
        dy,
        sy,
        du,
        su,
        dv,
        sv,
        width as i32,
        height as i32,
    );
    Some(i420)
}

/// Converts any incoming WebRTC `BoxVideoFrame` into packed RGBA (`[R, G, B, A]`).
pub fn video_frame_to_rgba(frame: &BoxVideoFrame) -> Option<(u32, u32, Vec<u8>)> {
    let w = frame.buffer.width();
    let h = frame.buffer.height();
    if w == 0 || h == 0 {
        return None;
    }
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    frame
        .buffer
        .to_argb(VideoFormatType::ABGR, &mut rgba, w * 4, w as i32, h as i32);
    Some((w, h, rgba))
}

pub struct VoiceEngine;

impl VoiceEngine {
    /// Connects to a LiveKit SFU room, starts cpal mic capture + remote audio playback,
    /// manages real Wayland/CLI screen sharing + remote video streams,
    /// and drives RoomEvent updates back into `event_tx`.
    #[allow(clippy::too_many_arguments)]
    pub fn connect_and_run(
        channel_id: String,
        livekit_url: String,
        jwt_token: String,
        muted: Arc<AtomicBool>,
        deafened: Arc<AtomicBool>,
        screensharing: Arc<AtomicBool>,
        pending_screen_target: Arc<StdMutex<Option<screen::ScreenShareTargetKind>>>,
        frame_slot: Arc<Mutex<Option<VideoFrameSnapshot>>>,
        shutdown: Arc<AtomicBool>,
        event_tx: mpsc::Sender<AppEvent>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            log_info!(
                "VoiceEngine: connecting to LiveKit SFU at {} for channel {}",
                livekit_url,
                channel_id
            );

            let (room, mut room_events) =
                match Room::connect(&livekit_url, &jwt_token, RoomOptions::default()).await {
                    Ok(res) => res,
                    Err(e) => {
                        let err_msg = format!("LiveKit connection failed: {}", e);
                        log_error!("{}", err_msg);
                        let _ = event_tx
                            .send(AppEvent::CallFailed {
                                channel_id,
                                error: err_msg,
                            })
                            .await;
                        return;
                    }
                };

            let room = Arc::new(room);
            if shutdown.load(Ordering::SeqCst) {
                log_info!(
                    "VoiceEngine: shutdown requested before stream setup for channel {}, closing room",
                    channel_id
                );
                let _ = room.close().await;
                return;
            }
            let local_identity = normalize_livekit_identity(room.local_participant().identity().as_str());
            log_info!(
                "VoiceEngine: LiveKit connected! room={}, local_identity={}",
                room.name(),
                local_identity
            );

            if !local_identity.is_empty() {
                let _ = event_tx
                    .send(AppEvent::CallParticipantJoined {
                        channel_id: channel_id.clone(),
                        user_id: local_identity.clone(),
                        is_local: true,
                    })
                    .await;
            }

            // Report any already-present remote participants (deduplicated & excluding local user)
            for (identity, rp) in room.remote_participants() {
                let uid = normalize_livekit_identity(identity.as_str());
                if uid.is_empty() || uid == local_identity {
                    continue;
                }
                let raw_name = rp.name();
                let fallback_name = if raw_name.trim().is_empty() {
                    None
                } else {
                    Some(raw_name)
                };
                let mic_muted = rp
                    .track_publications()
                    .values()
                    .find(|pub_| pub_.source() == TrackSource::Microphone)
                    .map(|pub_| pub_.is_muted());
                let (meta_muted, meta_deafened) =
                    parse_participant_mute_deafen(&rp.metadata(), &rp.attributes());

                let _ = event_tx
                    .send(AppEvent::CallParticipantJoined {
                        channel_id: channel_id.clone(),
                        user_id: uid.clone(),
                        is_local: false,
                    })
                    .await;
                let _ = event_tx
                    .send(AppEvent::CallParticipantState {
                        channel_id: channel_id.clone(),
                        user_id: uid,
                        fallback_name,
                        is_muted: mic_muted.or(meta_muted),
                        is_deafened: meta_deafened,
                    })
                    .await;
            }

            // Transition CallStage to Connected and play join chime
            let _ = event_tx
                .send(AppEvent::CallConnected {
                    channel_id: channel_id.clone(),
                })
                .await;
            sounds::play_join_sound();

            // 1. Start shared cpal output mixer thread for remote audio tracks
            let mixer_buffers: MixerBuffers = Arc::new(StdMutex::new(HashMap::new()));
            start_output_mixer_thread(mixer_buffers.clone(), deafened.clone(), shutdown.clone());

            // 2. Create NativeAudioSource & publish local microphone track
            let audio_source = NativeAudioSource::new(
                AudioSourceOptions {
                    echo_cancellation: true,
                    noise_suppression: true,
                    auto_gain_control: true,
                },
                WEBRTC_SAMPLE_RATE,
                WEBRTC_CHANNELS,
                100,
            );
            let local_audio_track = LocalAudioTrack::create_audio_track(
                "microphone",
                RtcAudioSource::Native(audio_source.clone()),
            );
            let publish_opts = TrackPublishOptions {
                source: TrackSource::Microphone,
                ..Default::default()
            };
            let local_mic_pub = match room
                .local_participant()
                .publish_track(LocalTrack::Audio(local_audio_track.clone()), publish_opts)
                .await
            {
                Ok(publication) => {
                    log_info!(
                        "VoiceEngine: published local microphone track (sid={})",
                        publication.sid()
                    );
                    if muted.load(Ordering::SeqCst) {
                        publication.mute();
                    }
                    Some(publication)
                }
                Err(e) => {
                    log_warn!("VoiceEngine: failed to publish microphone track: {}", e);
                    None
                }
            };
            let mut last_synced_muted = muted.load(Ordering::SeqCst);

            // 3. Start cpal microphone input stream + async feeder task
            let mic_task = start_mic_capture(
                audio_source,
                local_audio_track,
                muted.clone(),
                deafened.clone(),
                shutdown.clone(),
            );

            // 4. Manage real screen share capture/publishing + remote video tracks
            let active_remote_video_tracks: Arc<StdMutex<HashSet<String>>> =
                Arc::new(StdMutex::new(HashSet::new()));
            let mut screen_active = false;
            let mut screen_shutdown: Option<Arc<AtomicBool>> = None;
            let mut screen_task: Option<JoinHandle<()>> = None;
            let mut published_screen_sid: Option<TrackSid> = None;
            let (screen_sid_tx, mut screen_sid_rx) = mpsc::unbounded_channel::<TrackSid>();
            let mut interval = tokio::time::interval(Duration::from_millis(50));

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if shutdown.load(Ordering::SeqCst) {
                            log_info!("VoiceEngine: shutdown requested for channel {}", channel_id);
                            break;
                        }

                        let now_muted = muted.load(Ordering::SeqCst);
                        if now_muted != last_synced_muted {
                            last_synced_muted = now_muted;
                            if let Some(ref pub_) = local_mic_pub {
                                if now_muted {
                                    pub_.mute();
                                } else {
                                    pub_.unmute();
                                }
                            }
                        }

                        while let Ok(sid) = screen_sid_rx.try_recv() {
                            published_screen_sid = Some(sid);
                        }

                        let want_screen = screensharing.load(Ordering::SeqCst);
                        if want_screen && !screen_active {
                            screen_active = true;
                            let share_stop = Arc::new(AtomicBool::new(false));
                            screen_shutdown = Some(share_stop.clone());
                            let task = spawn_local_screen_share(
                                channel_id.clone(),
                                room.clone(),
                                share_stop,
                                screensharing.clone(),
                                pending_screen_target.clone(),
                                frame_slot.clone(),
                                active_remote_video_tracks.clone(),
                                screen_sid_tx.clone(),
                                event_tx.clone(),
                            );
                            screen_task = Some(task);
                        } else if !want_screen && screen_active {
                            screen_active = false;
                            if let Some(stop) = screen_shutdown.take() {
                                stop.store(true, Ordering::SeqCst);
                            }
                            if let Some(task) = screen_task.take() {
                                task.abort();
                            }
                            if let Some(sid) = published_screen_sid.take() {
                                log_info!("VoiceEngine: unpublishing local screen share track {}", sid);
                                let _ = room.local_participant().unpublish_track(&sid).await;
                            }
                            let mut slot = frame_slot.lock().await;
                            if slot.as_ref().map(|s| !s.is_remote).unwrap_or(false) {
                                *slot = None;
                                let _ = event_tx.try_send(AppEvent::VideoFrameReady);
                            }
                        }
                    }
                    maybe_ev = room_events.recv() => {
                        let Some(ev) = maybe_ev else {
                            log_info!("VoiceEngine: LiveKit room event stream ended");
                            break;
                        };
                        match ev {
                            RoomEvent::ParticipantConnected(participant) => {
                                let uid = normalize_livekit_identity(participant.identity().as_str());
                                if uid.is_empty() || uid == local_identity {
                                    continue;
                                }
                                log_info!("LiveKit participant connected: {}", uid);
                                sounds::play_join_sound();
                                let raw_name = participant.name();
                                let fallback_name = if raw_name.trim().is_empty() {
                                    None
                                } else {
                                    Some(raw_name)
                                };
                                let mic_muted = participant
                                    .track_publications()
                                    .values()
                                    .find(|pub_| pub_.source() == TrackSource::Microphone)
                                    .map(|pub_| pub_.is_muted());
                                let (meta_muted, meta_deafened) = parse_participant_mute_deafen(
                                    &participant.metadata(),
                                    &participant.attributes(),
                                );

                                let _ = event_tx
                                    .send(AppEvent::CallParticipantJoined {
                                        channel_id: channel_id.clone(),
                                        user_id: uid.clone(),
                                        is_local: false,
                                    })
                                    .await;
                                let _ = event_tx
                                    .send(AppEvent::CallParticipantState {
                                        channel_id: channel_id.clone(),
                                        user_id: uid,
                                        fallback_name,
                                        is_muted: mic_muted.or(meta_muted),
                                        is_deafened: meta_deafened,
                                    })
                                    .await;
                            }
                            RoomEvent::ParticipantDisconnected(participant) => {
                                let uid = normalize_livekit_identity(participant.identity().as_str());
                                if uid.is_empty() || uid == local_identity {
                                    continue;
                                }
                                let still_present = room.remote_participants().keys().any(|id| {
                                    normalize_livekit_identity(id.as_str()) == uid
                                });
                                if !still_present {
                                    log_info!("LiveKit participant disconnected: {}", uid);
                                    sounds::play_leave_sound();
                                    let _ = event_tx
                                        .send(AppEvent::CallParticipantLeft {
                                            channel_id: channel_id.clone(),
                                            user_id: uid,
                                        })
                                        .await;
                                }
                            }
                            RoomEvent::TrackMuted {
                                participant,
                                publication,
                            } => {
                                if publication.source() == TrackSource::Microphone {
                                    let uid = normalize_livekit_identity(participant.identity().as_str());
                                    if !uid.is_empty() && uid != local_identity {
                                        let raw_name = participant.name();
                                        let fallback_name = (!raw_name.trim().is_empty()).then_some(raw_name);
                                        let _ = event_tx
                                            .send(AppEvent::CallParticipantState {
                                                channel_id: channel_id.clone(),
                                                user_id: uid,
                                                fallback_name,
                                                is_muted: Some(true),
                                                is_deafened: None,
                                            })
                                            .await;
                                    }
                                }
                            }
                            RoomEvent::TrackUnmuted {
                                participant,
                                publication,
                            } => {
                                if publication.source() == TrackSource::Microphone {
                                    let uid = normalize_livekit_identity(participant.identity().as_str());
                                    if !uid.is_empty() && uid != local_identity {
                                        let raw_name = participant.name();
                                        let fallback_name = (!raw_name.trim().is_empty()).then_some(raw_name);
                                        let _ = event_tx
                                            .send(AppEvent::CallParticipantState {
                                                channel_id: channel_id.clone(),
                                                user_id: uid,
                                                fallback_name,
                                                is_muted: Some(false),
                                                is_deafened: None,
                                            })
                                            .await;
                                    }
                                }
                            }
                            RoomEvent::ParticipantMetadataChanged {
                                participant,
                                metadata,
                                ..
                            } => {
                                let uid = normalize_livekit_identity(participant.identity().as_str());
                                if !uid.is_empty() && uid != local_identity {
                                    let (m, d) = parse_participant_mute_deafen(&metadata, &participant.attributes());
                                    if m.is_some() || d.is_some() {
                                        let _ = event_tx
                                            .send(AppEvent::CallParticipantState {
                                                channel_id: channel_id.clone(),
                                                user_id: uid,
                                                fallback_name: None,
                                                is_muted: m,
                                                is_deafened: d,
                                            })
                                            .await;
                                    }
                                }
                            }
                            RoomEvent::ParticipantAttributesChanged {
                                participant,
                                ..
                            } => {
                                let uid = normalize_livekit_identity(participant.identity().as_str());
                                if !uid.is_empty() && uid != local_identity {
                                    let (m, d) = parse_participant_mute_deafen(&participant.metadata(), &participant.attributes());
                                    if m.is_some() || d.is_some() {
                                        let _ = event_tx
                                            .send(AppEvent::CallParticipantState {
                                                channel_id: channel_id.clone(),
                                                user_id: uid,
                                                fallback_name: None,
                                                is_muted: m,
                                                is_deafened: d,
                                            })
                                            .await;
                                    }
                                }
                            }
                            RoomEvent::TrackSubscribed {
                                track: RemoteTrack::Audio(audio_track),
                                publication: _,
                                participant,
                            } => {
                                let sid = audio_track.sid().to_string();
                                log_info!(
                                    "LiveKit TrackSubscribed (Audio): track={} participant={}",
                                    sid,
                                    participant.identity()
                                );
                                spawn_remote_audio_reader(
                                    sid,
                                    audio_track,
                                    mixer_buffers.clone(),
                                    deafened.clone(),
                                    shutdown.clone(),
                                );
                            }
                            RoomEvent::TrackSubscribed {
                                track: RemoteTrack::Video(video_track),
                                publication,
                                participant,
                            } => {
                                let sid = video_track.sid().to_string();
                                let owner_id = normalize_livekit_identity(participant.identity().as_str());
                                log_info!(
                                    "LiveKit TrackSubscribed (Video {:?}): track={} participant={}",
                                    publication.source(),
                                    sid,
                                    owner_id
                                );
                                if let Ok(mut set) = active_remote_video_tracks.lock() {
                                    set.insert(sid.clone());
                                }
                                spawn_remote_video_reader(
                                    sid,
                                    owner_id,
                                    video_track,
                                    frame_slot.clone(),
                                    active_remote_video_tracks.clone(),
                                    shutdown.clone(),
                                    event_tx.clone(),
                                );
                            }
                            RoomEvent::TrackUnsubscribed {
                                track: RemoteTrack::Audio(audio_track),
                                ..
                            } => {
                                let sid = audio_track.sid().to_string();
                                log_debug!("LiveKit TrackUnsubscribed (Audio): {}", sid);
                                if let Ok(mut guard) = mixer_buffers.lock() {
                                    guard.remove(&sid);
                                }
                            }
                            RoomEvent::TrackUnsubscribed {
                                track: RemoteTrack::Video(video_track),
                                ..
                            } => {
                                let sid = video_track.sid().to_string();
                                log_debug!("LiveKit TrackUnsubscribed (Video): {}", sid);
                                let no_remote_video = active_remote_video_tracks
                                    .lock()
                                    .map(|mut set| {
                                        set.remove(&sid);
                                        set.is_empty()
                                    })
                                    .unwrap_or(true);
                                if no_remote_video {
                                    let mut slot = frame_slot.lock().await;
                                    if slot.as_ref().map(|s| s.is_remote).unwrap_or(false) {
                                        *slot = None;
                                        let _ = event_tx.try_send(AppEvent::VideoFrameReady);
                                    }
                                }
                            }
                            RoomEvent::ActiveSpeakersChanged { speakers } => {
                                let speaker_ids: Vec<String> = speakers
                                    .into_iter()
                                    .map(|p| normalize_livekit_identity(p.identity().as_str()))
                                    .collect();
                                let _ = event_tx
                                    .send(AppEvent::CallActiveSpeakers {
                                        channel_id: channel_id.clone(),
                                        speaker_ids,
                                    })
                                    .await;
                            }
                            RoomEvent::Disconnected { reason } => {
                                let reason_str = format!("{:?}", reason);
                                log_warn!("LiveKit room disconnected: {}", reason_str);
                                let _ = event_tx
                                    .send(AppEvent::CallDisconnected {
                                        channel_id: channel_id.clone(),
                                        reason: reason_str,
                                    })
                                    .await;
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }

            if let Some(stop) = screen_shutdown.take() {
                stop.store(true, Ordering::SeqCst);
            }
            if let Some(task) = screen_task.take() {
                task.abort();
            }
            if let Some(sid) = published_screen_sid.take() {
                let _ = room.local_participant().unpublish_track(&sid).await;
            }
            shutdown.store(true, Ordering::SeqCst);
            mic_task.abort();
            let _ = room.close().await;
            log_info!("VoiceEngine: LiveKit room closed cleanly for channel {}", channel_id);
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_local_screen_share(
    channel_id: String,
    room: Arc<Room>,
    share_stop: Arc<AtomicBool>,
    screensharing_flag: Arc<AtomicBool>,
    pending_screen_target: Arc<StdMutex<Option<screen::ScreenShareTargetKind>>>,
    frame_slot: Arc<Mutex<Option<VideoFrameSnapshot>>>,
    active_remote_video_tracks: Arc<StdMutex<HashSet<String>>>,
    screen_sid_tx: mpsc::UnboundedSender<TrackSid>,
    event_tx: mpsc::Sender<AppEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let selected_target = pending_screen_target
            .lock()
            .ok()
            .and_then(|mut g| g.take());
        let mut capture_rx = match screen::start_screen_capture(share_stop.clone(), selected_target).await {
            Ok(rx) => rx,
            Err(screen::ScreenCaptureError::Cancelled) => {
                log_info!("Screen share cancelled by user for {}", channel_id);
                screensharing_flag.store(false, Ordering::SeqCst);
                let _ = event_tx
                    .send(AppEvent::ScreenShareCancelled { channel_id })
                    .await;
                return;
            }
            Err(screen::ScreenCaptureError::NeedTargetSelection(targets)) => {
                log_info!(
                    "Screen share requesting source picker ({} targets) for {}",
                    targets.len(),
                    channel_id
                );
                screensharing_flag.store(false, Ordering::SeqCst);
                let _ = event_tx
                    .send(AppEvent::ScreenShareSelectTarget {
                        channel_id,
                        targets,
                    })
                    .await;
                return;
            }
            Err(screen::ScreenCaptureError::Failed(err)) => {
                log_error!("Screen share failed to initialize for {}: {}", channel_id, err);
                screensharing_flag.store(false, Ordering::SeqCst);
                let _ = event_tx
                    .send(AppEvent::ScreenShareFailed {
                        channel_id,
                        error: err,
                    })
                    .await;
                return;
            }
        };

        let mut video_source: Option<NativeVideoSource> = None;
        let started_at = Instant::now();
        let mut fps_window_start = Instant::now();
        let mut fps_frames = 0u32;
        let mut current_fps = 0u32;
        let mut last_preview_push = Instant::now() - Duration::from_millis(40);

        while let Some(captured) = capture_rx.recv().await {
            if share_stop.load(Ordering::SeqCst) {
                break;
            }

            let source = match video_source {
                Some(ref src) => src,
                None => {
                    let src = NativeVideoSource::new(
                        VideoResolution {
                            width: captured.width,
                            height: captured.height,
                        },
                        true,
                    );
                    let track = LocalVideoTrack::create_video_track(
                        "screen",
                        RtcVideoSource::Native(src.clone()),
                    );
                    let opts = TrackPublishOptions {
                        source: TrackSource::Screenshare,
                        ..Default::default()
                    };
                    match room
                        .local_participant()
                        .publish_track(LocalTrack::Video(track), opts)
                        .await
                    {
                        Ok(pub_ref) => {
                            let sid = pub_ref.sid();
                            log_info!(
                                "VoiceEngine: published ScreenShare track sid={} ({}x{})",
                                sid,
                                captured.width,
                                captured.height
                            );
                            let _ = screen_sid_tx.send(sid);
                        }
                        Err(e) => {
                            let err_msg = format!("Failed to publish ScreenShare track: {}", e);
                            log_error!("{}", err_msg);
                            screensharing_flag.store(false, Ordering::SeqCst);
                            let _ = event_tx
                                .send(AppEvent::ScreenShareFailed {
                                    channel_id,
                                    error: err_msg,
                                })
                                .await;
                            return;
                        }
                    }
                    video_source = Some(src);
                    video_source.as_ref().unwrap()
                }
            };

            let Some(i420_buf) = rgba_to_i420_buffer(&captured.rgba, captured.width, captured.height) else {
                log_warn!(
                    "Skipping invalid captured screen frame ({}x{}, {} bytes)",
                    captured.width,
                    captured.height,
                    captured.rgba.len()
                );
                continue;
            };
            let video_frame = VideoFrame {
                rotation: VideoRotation::VideoRotation0,
                timestamp_us: started_at.elapsed().as_micros() as i64,
                frame_metadata: None,
                buffer: i420_buf,
            };
            source.capture_frame(&video_frame);

            fps_frames += 1;
            let elapsed = fps_window_start.elapsed();
            if elapsed >= Duration::from_secs(1) {
                current_fps = ((fps_frames as f64 / elapsed.as_secs_f64()).round() as u32).max(1);
                fps_frames = 0;
                fps_window_start = Instant::now();
            } else if current_fps == 0 {
                current_fps = 1;
            }

            let has_remote_video = active_remote_video_tracks
                .lock()
                .map(|s| !s.is_empty())
                .unwrap_or(false);
            if !has_remote_video && last_preview_push.elapsed() >= Duration::from_millis(33) {
                if let Some(rgba_img) =
                    RgbaImage::from_raw(captured.width, captured.height, captured.rgba)
                {
                    last_preview_push = Instant::now();
                    *frame_slot.lock().await = Some(VideoFrameSnapshot {
                        image: DynamicImage::ImageRgba8(rgba_img),
                        owner_id: "You".to_string(),
                        fps: current_fps,
                        is_remote: false,
                    });
                    let _ = event_tx.try_send(AppEvent::VideoFrameReady);
                }
            }
        }
    })
}

fn spawn_remote_video_reader(
    track_sid: String,
    owner_id: String,
    video_track: RemoteVideoTrack,
    frame_slot: Arc<Mutex<Option<VideoFrameSnapshot>>>,
    active_remote_video_tracks: Arc<StdMutex<HashSet<String>>>,
    shutdown: Arc<AtomicBool>,
    event_tx: mpsc::Sender<AppEvent>,
) {
    tokio::spawn(async move {
        let mut stream = NativeVideoStream::new(video_track.rtc_track());
        let mut fps_window_start = Instant::now();
        let mut fps_frames = 0u32;
        let mut current_fps = 0u32;
        let mut last_ui_push = Instant::now() - Duration::from_millis(50);

        while let Some(frame) = stream.next().await {
            if shutdown.load(Ordering::SeqCst) {
                break;
            }

            fps_frames += 1;
            let elapsed = fps_window_start.elapsed();
            if elapsed >= Duration::from_secs(1) {
                current_fps = ((fps_frames as f64 / elapsed.as_secs_f64()).round() as u32).max(1);
                fps_frames = 0;
                fps_window_start = Instant::now();
            } else if current_fps == 0 {
                current_fps = 1;
            }

            // Throttle UI frame conversions to ~20 FPS
            if last_ui_push.elapsed() < Duration::from_millis(45) {
                continue;
            }
            last_ui_push = Instant::now();

            if let Some((w, h, rgba)) = video_frame_to_rgba(&frame) {
                if let Some(rgba_img) = RgbaImage::from_raw(w, h, rgba) {
                    *frame_slot.lock().await = Some(VideoFrameSnapshot {
                        image: DynamicImage::ImageRgba8(rgba_img),
                        owner_id: owner_id.clone(),
                        fps: current_fps,
                        is_remote: true,
                    });
                    let _ = event_tx.try_send(AppEvent::VideoFrameReady);
                }
            }
        }

        if let Ok(mut set) = active_remote_video_tracks.lock() {
            set.remove(&track_sid);
        }
    });
}

fn spawn_remote_audio_reader(
    track_sid: String,
    audio_track: RemoteAudioTrack,
    mixer_buffers: MixerBuffers,
    deafened: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
) {
    tokio::spawn(async move {
        let mut stream = NativeAudioStream::new(
            audio_track.rtc_track(),
            WEBRTC_SAMPLE_RATE as i32,
            WEBRTC_CHANNELS as i32,
        );

        while let Some(frame) = stream.next().await {
            if shutdown.load(Ordering::SeqCst) {
                break;
            }
            if deafened.load(Ordering::SeqCst) {
                if let Ok(mut guard) = mixer_buffers.lock() {
                    if let Some(q) = guard.get_mut(&track_sid) {
                        q.clear();
                    }
                }
                continue;
            }

            if let Ok(mut guard) = mixer_buffers.lock() {
                let q = guard.entry(track_sid.clone()).or_default();
                for &sample in frame.data.iter() {
                    if q.len() >= MAX_MIXER_BUFFER_SAMPLES {
                        q.pop_front();
                    }
                    q.push_back(sample);
                }
            }
        }

        if let Ok(mut guard) = mixer_buffers.lock() {
            guard.remove(&track_sid);
        }
    });
}

fn start_output_mixer_thread(
    mixer_buffers: MixerBuffers,
    deafened: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
) {
    if cfg!(test) || std::env::var("STOAT_TUI_DISABLE_AUDIO").is_ok() {
        return;
    }

    std::thread::spawn(move || {
        let host = cpal::default_host();
        let Some(device) = host.default_output_device() else {
            log_warn!("VoiceEngine: No default cpal output device found");
            return;
        };
        let dev_desc = device
            .description()
            .map(|d| d.to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        let Ok(supported) = device.default_output_config() else {
            log_warn!("VoiceEngine: Failed to query default output config for {}", dev_desc);
            return;
        };

        let sample_format = supported.sample_format();
        let stream_config: StreamConfig = supported.into();
        log_info!(
            "VoiceEngine: Initialized output audio device '{}' (rate={}Hz, ch={}, fmt={:?})",
            dev_desc,
            stream_config.sample_rate,
            stream_config.channels,
            sample_format
        );

        let res = match sample_format {
            SampleFormat::F32 => {
                run_output_mixer::<f32>(&device, stream_config, mixer_buffers, deafened, shutdown)
            }
            SampleFormat::I16 => {
                run_output_mixer::<i16>(&device, stream_config, mixer_buffers, deafened, shutdown)
            }
            SampleFormat::I32 => {
                run_output_mixer::<i32>(&device, stream_config, mixer_buffers, deafened, shutdown)
            }
            SampleFormat::U16 => {
                run_output_mixer::<u16>(&device, stream_config, mixer_buffers, deafened, shutdown)
            }
            SampleFormat::F64 => {
                run_output_mixer::<f64>(&device, stream_config, mixer_buffers, deafened, shutdown)
            }
            other => {
                log_warn!("VoiceEngine: Unsupported output sample format {:?}", other);
                Ok(())
            }
        };

        if let Err(e) = res {
            log_warn!("VoiceEngine: Output audio stream failed: {}", e);
        }
    });
}

fn pop_mixed_48k_sample(mixer_buffers: &MixerBuffers, deafened: &AtomicBool) -> i16 {
    if deafened.load(Ordering::Relaxed) {
        return 0;
    }
    let Ok(mut guard) = mixer_buffers.lock() else {
        return 0;
    };
    let mut sum: i32 = 0;
    for q in guard.values_mut() {
        if let Some(s) = q.pop_front() {
            sum += s as i32;
        }
    }
    sum.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

fn run_output_mixer<T>(
    device: &cpal::Device,
    config: StreamConfig,
    mixer_buffers: MixerBuffers,
    deafened: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
) -> Result<(), String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels.max(1) as usize;
    let step = WEBRTC_SAMPLE_RATE as f64 / config.sample_rate.max(1) as f64;
    let mut frac: f64 = 0.0;
    let mut prev: i16 = 0;
    let mut cur: i16 = 0;

    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &OutputCallbackInfo| {
                for frame in data.chunks_mut(channels) {
                    while frac >= 1.0 {
                        prev = cur;
                        cur = pop_mixed_48k_sample(&mixer_buffers, &deafened);
                        frac -= 1.0;
                    }
                    let interp = prev as f32 + (cur - prev) as f32 * frac as f32;
                    let sample_f32 = (interp / 32768.0).clamp(-1.0, 1.0);
                    let converted = T::from_sample(sample_f32);
                    for ch_sample in frame.iter_mut() {
                        *ch_sample = converted;
                    }
                    frac += step;
                }
            },
            |err| {
                log_debug!("VoiceEngine output stream error: {}", err);
            },
            None,
        )
        .map_err(|e| format!("build_output_stream: {}", e))?;

    stream.play().map_err(|e| format!("stream.play: {}", e))?;

    while !shutdown.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    Ok(())
}

fn start_mic_capture(
    audio_source: NativeAudioSource,
    local_track: LocalAudioTrack,
    muted: Arc<AtomicBool>,
    deafened: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
) -> JoinHandle<()> {
    let (pcm_tx, mut pcm_rx) = mpsc::unbounded_channel::<(u32, Vec<f32>)>();

    if !cfg!(test) && std::env::var("STOAT_TUI_DISABLE_AUDIO").is_err() {
        let shutdown_thread = shutdown.clone();
        std::thread::spawn(move || {
            let host = cpal::default_host();
            let Some(device) = host.default_input_device() else {
                log_warn!("VoiceEngine: No default cpal input device found");
                return;
            };
            let dev_desc = device
                .description()
                .map(|d| d.to_string())
                .unwrap_or_else(|_| "unknown".to_string());
            let Ok(supported) = device.default_input_config() else {
                log_warn!("VoiceEngine: Failed to query default input config for {}", dev_desc);
                return;
            };

            let sample_format = supported.sample_format();
            let stream_config: StreamConfig = supported.into();
            log_info!(
                "VoiceEngine: Initialized input audio device '{}' (rate={}Hz, ch={}, fmt={:?})",
                dev_desc,
                stream_config.sample_rate,
                stream_config.channels,
                sample_format
            );

            let res = match sample_format {
                SampleFormat::F32 => {
                    run_input_stream::<f32>(&device, stream_config, pcm_tx, shutdown_thread)
                }
                SampleFormat::I16 => {
                    run_input_stream::<i16>(&device, stream_config, pcm_tx, shutdown_thread)
                }
                SampleFormat::I32 => {
                    run_input_stream::<i32>(&device, stream_config, pcm_tx, shutdown_thread)
                }
                SampleFormat::U16 => {
                    run_input_stream::<u16>(&device, stream_config, pcm_tx, shutdown_thread)
                }
                SampleFormat::F64 => {
                    run_input_stream::<f64>(&device, stream_config, pcm_tx, shutdown_thread)
                }
                other => {
                    log_warn!("VoiceEngine: Unsupported input sample format {:?}", other);
                    Ok(())
                }
            };

            if let Err(e) = res {
                log_warn!("VoiceEngine: Input audio stream failed: {}", e);
            }
        });
    }

    tokio::spawn(async move {
        let mut resampler: Option<Mono48kResampler> = None;
        let mut pending_48k: Vec<i16> = Vec::with_capacity(FRAME_SAMPLES_10MS * 4);
        let silence_frame = [0i16; FRAME_SAMPLES_10MS];
        let mut was_silenced = false;

        while let Some((in_rate, mono_samples)) = pcm_rx.recv().await {
            if shutdown.load(Ordering::SeqCst) {
                break;
            }

            let is_silenced = muted.load(Ordering::SeqCst) || deafened.load(Ordering::SeqCst);
            if is_silenced != was_silenced {
                if is_silenced {
                    local_track.mute();
                } else {
                    local_track.unmute();
                }
                was_silenced = is_silenced;
            }

            let rs = resampler.get_or_insert_with(|| Mono48kResampler::new(in_rate));
            if rs.in_rate != in_rate {
                *rs = Mono48kResampler::new(in_rate);
            }
            rs.push(&mono_samples, &mut pending_48k);

            while pending_48k.len() >= FRAME_SAMPLES_10MS {
                let frame_samples: Vec<i16> = pending_48k.drain(..FRAME_SAMPLES_10MS).collect();
                let slice: &[i16] = if is_silenced {
                    &silence_frame
                } else {
                    &frame_samples
                };
                let frame = AudioFrame {
                    data: Cow::Borrowed(slice),
                    sample_rate: WEBRTC_SAMPLE_RATE,
                    num_channels: WEBRTC_CHANNELS,
                    samples_per_channel: FRAME_SAMPLES_10MS as u32,
                };
                if let Err(e) = audio_source.capture_frame(&frame).await {
                    log_debug!("VoiceEngine capture_frame error: {}", e);
                }
            }
        }
    })
}

fn run_input_stream<T>(
    device: &cpal::Device,
    config: StreamConfig,
    pcm_tx: mpsc::UnboundedSender<(u32, Vec<f32>)>,
    shutdown: Arc<AtomicBool>,
) -> Result<(), String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let in_rate = config.sample_rate;
    let channels = config.channels.max(1) as usize;

    let stream = device
        .build_input_stream(
            config,
            move |data: &[T], _: &InputCallbackInfo| {
                let mut mono = Vec::with_capacity(data.len() / channels + 1);
                for frame in data.chunks(channels) {
                    let mut sum = 0.0f32;
                    for &sample in frame {
                        sum += f32::from_sample(sample);
                    }
                    mono.push(sum / frame.len().max(1) as f32);
                }
                if !mono.is_empty() {
                    let _ = pcm_tx.send((in_rate, mono));
                }
            },
            |err| {
                log_debug!("VoiceEngine input stream error: {}", err);
            },
            None,
        )
        .map_err(|e| format!("build_input_stream: {}", e))?;

    stream.play().map_err(|e| format!("stream.play: {}", e))?;

    while !shutdown.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    Ok(())
}
