use crate::log_debug;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, OutputCallbackInfo, SampleFormat, SizedSample, StreamConfig};
use std::f32::consts::PI;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

pub const SOUND_SAMPLE_RATE: u32 = 48_000;

static CALL_SOUNDS_ENABLED: AtomicBool = AtomicBool::new(true);

pub fn set_enabled(enabled: bool) {
    CALL_SOUNDS_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn is_enabled() -> bool {
    CALL_SOUNDS_ENABLED.load(Ordering::Relaxed)
}

/// Synthesizes a single polyphonic chime tone (fundamental + gentle 2nd harmonic)
/// with a smooth sine-shaped attack/decay envelope to avoid clicks.
fn append_tone(buf: &mut Vec<f32>, freq_hz: f32, duration_ms: u32, gain: f32) {
    let sample_count = ((SOUND_SAMPLE_RATE as f32) * (duration_ms as f32 / 1000.0)) as usize;
    if sample_count == 0 {
        return;
    }
    let attack_samples = (sample_count / 8).max(1);
    for i in 0..sample_count {
        let t = i as f32 / SOUND_SAMPLE_RATE as f32;
        let progress = i as f32 / sample_count as f32;

        // Smooth attack & exponential-like sine decay envelope
        let env = if i < attack_samples {
            let a = i as f32 / attack_samples as f32;
            (a * PI * 0.5).sin()
        } else {
            let d = (progress - (attack_samples as f32 / sample_count as f32))
                / (1.0 - (attack_samples as f32 / sample_count as f32)).max(0.001);
            ((1.0 - d) * PI * 0.5).sin().powf(1.4)
        };

        let fundamental = (2.0 * PI * freq_hz * t).sin();
        let overtone = 0.25 * (2.0 * PI * (freq_hz * 2.0) * t).sin();
        let sample = (fundamental + overtone) * 0.8 * gain * env;
        buf.push(sample.clamp(-0.95, 0.95));
    }
}

/// Mixes a secondary note starting at `offset_ms` for richer polyphonic chime overlap.
fn mix_tone_at(buf: &mut Vec<f32>, offset_ms: u32, freq_hz: f32, duration_ms: u32, gain: f32) {
    let offset_samples = ((SOUND_SAMPLE_RATE as f32) * (offset_ms as f32 / 1000.0)) as usize;
    let mut note = Vec::new();
    append_tone(&mut note, freq_hz, duration_ms, gain);
    let required_len = offset_samples + note.len();
    if buf.len() < required_len {
        buf.resize(required_len, 0.0);
    }
    for (i, s) in note.into_iter().enumerate() {
        buf[offset_samples + i] = (buf[offset_samples + i] + s).clamp(-0.95, 0.95);
    }
}

/// Join Call: rising two-note major chime (C5 523Hz -> G5 784Hz, ~180ms total).
pub fn synthesize_join_sound() -> Vec<f32> {
    let mut buf = Vec::new();
    append_tone(&mut buf, 523.25, 95, 0.30);
    mix_tone_at(&mut buf, 75, 783.99, 105, 0.32);
    buf
}

/// Leave Call: falling two-note chime (G5 784Hz -> C5 523Hz, ~180ms total).
pub fn synthesize_leave_sound() -> Vec<f32> {
    let mut buf = Vec::new();
    append_tone(&mut buf, 783.99, 95, 0.28);
    mix_tone_at(&mut buf, 75, 523.25, 105, 0.28);
    buf
}

/// Mute / Unmute: subtle low/high double-tap click (330Hz vs 440Hz, ~80ms total).
pub fn synthesize_mute_sound(muted: bool) -> Vec<f32> {
    let mut buf = Vec::new();
    if muted {
        append_tone(&mut buf, 440.0, 38, 0.22);
        mix_tone_at(&mut buf, 38, 330.0, 42, 0.24);
    } else {
        append_tone(&mut buf, 330.0, 38, 0.22);
        mix_tone_at(&mut buf, 38, 440.0, 42, 0.24);
    }
    buf
}

/// Deafen / Undeafen: distinct two-tone ping (294Hz vs 587Hz, ~110ms total).
pub fn synthesize_deafen_sound(deafened: bool) -> Vec<f32> {
    let mut buf = Vec::new();
    if deafened {
        append_tone(&mut buf, 587.33, 52, 0.26);
        mix_tone_at(&mut buf, 50, 293.66, 60, 0.28);
    } else {
        append_tone(&mut buf, 293.66, 52, 0.26);
        mix_tone_at(&mut buf, 50, 587.33, 60, 0.28);
    }
    buf
}

pub fn play_join_sound() {
    play_samples_48k(synthesize_join_sound());
}

pub fn play_leave_sound() {
    play_samples_48k(synthesize_leave_sound());
}

pub fn play_mute_sound(muted: bool) {
    play_samples_48k(synthesize_mute_sound(muted));
}

pub fn play_deafen_sound(deafened: bool) {
    play_samples_48k(synthesize_deafen_sound(deafened));
}

/// Resamples a 48kHz mono f32 buffer to `target_rate` using linear interpolation.
pub fn resample_mono_f32(input: &[f32], src_rate: u32, dst_rate: u32) -> Vec<f32> {
    if input.is_empty() || src_rate == 0 || dst_rate == 0 {
        return Vec::new();
    }
    if src_rate == dst_rate {
        return input.to_vec();
    }
    let ratio = src_rate as f64 / dst_rate as f64;
    let out_len = ((input.len() as f64) / ratio).ceil() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_pos = i as f64 * ratio;
        let idx = src_pos.floor() as usize;
        let frac = (src_pos - idx as f64) as f32;
        let s0 = input.get(idx).copied().unwrap_or(0.0);
        let s1 = input.get(idx + 1).copied().unwrap_or(s0);
        out.push(s0 + (s1 - s0) * frac);
    }
    out
}

/// Plays a 48kHz mono f32 buffer asynchronously on the default `cpal` output device.
/// Fails silently if no audio output device is available.
fn play_samples_48k(samples_48k: Vec<f32>) {
    if !is_enabled() || cfg!(test) || std::env::var("STOAT_TUI_DISABLE_AUDIO").is_ok() {
        return;
    }

    std::thread::spawn(move || {
        let host = cpal::default_host();
        let Some(device) = host.default_output_device() else {
            log_debug!("Call SFX: No default output audio device available");
            return;
        };

        let Ok(supported) = device.default_output_config() else {
            log_debug!("Call SFX: Failed to query default output config");
            return;
        };

        let sample_format = supported.sample_format();
        let stream_config: StreamConfig = supported.into();
        let resampled = resample_mono_f32(&samples_48k, SOUND_SAMPLE_RATE, stream_config.sample_rate);
        if resampled.is_empty() {
            return;
        }

        let duration_ms = ((resampled.len() as f64 / stream_config.sample_rate as f64) * 1000.0) as u64;
        let samples_arc = Arc::new(resampled);

        let result = match sample_format {
            SampleFormat::F32 => run_oneshot_output::<f32>(&device, stream_config, samples_arc, duration_ms),
            SampleFormat::I16 => run_oneshot_output::<i16>(&device, stream_config, samples_arc, duration_ms),
            SampleFormat::I32 => run_oneshot_output::<i32>(&device, stream_config, samples_arc, duration_ms),
            SampleFormat::U16 => run_oneshot_output::<u16>(&device, stream_config, samples_arc, duration_ms),
            SampleFormat::F64 => run_oneshot_output::<f64>(&device, stream_config, samples_arc, duration_ms),
            other => {
                log_debug!("Call SFX: Unsupported output sample format {:?}", other);
                Ok(())
            }
        };

        if let Err(e) = result {
            log_debug!("Call SFX playback error: {}", e);
        }
    });
}

fn run_oneshot_output<T>(
    device: &cpal::Device,
    config: StreamConfig,
    samples: Arc<Vec<f32>>,
    duration_ms: u64,
) -> Result<(), String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels.max(1) as usize;
    let cursor = Arc::new(AtomicUsize::new(0));
    let cursor_cb = cursor.clone();
    let samples_cb = samples.clone();

    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &OutputCallbackInfo| {
                let mut pos = cursor_cb.load(Ordering::Relaxed);
                for frame in data.chunks_mut(channels) {
                    let s = if pos < samples_cb.len() {
                        let val = samples_cb[pos];
                        pos += 1;
                        val
                    } else {
                        0.0
                    };
                    let converted = T::from_sample(s);
                    for ch_sample in frame.iter_mut() {
                        *ch_sample = converted;
                    }
                }
                cursor_cb.store(pos, Ordering::Relaxed);
            },
            |err| {
                log_debug!("Call SFX stream error: {}", err);
            },
            None,
        )
        .map_err(|e| format!("build_output_stream: {}", e))?;

    stream.play().map_err(|e| format!("stream.play: {}", e))?;
    std::thread::sleep(std::time::Duration::from_millis(duration_ms + 60));
    Ok(())
}
