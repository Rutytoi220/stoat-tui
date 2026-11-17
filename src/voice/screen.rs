use crate::{log_debug, log_error, log_info, log_warn};
use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType};
use ashpd::desktop::PersistMode;
use pipewire as pw;
use pw::{properties::properties, spa};
use spa::param::format::{MediaSubtype, MediaType};
use spa::param::format_utils;
use spa::param::video::VideoFormat;
use spa::pod::Pod;
use std::io::Cursor;
use std::os::fd::OwnedFd;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

pub const MAX_CAPTURE_WIDTH: u32 = 1280;
pub const MAX_CAPTURE_HEIGHT: u32 = 720;
const PIPEWIRE_MIN_FRAME_INTERVAL: Duration = Duration::from_millis(45); // ~22 FPS cap
const CLI_FALLBACK_FRAME_INTERVAL: Duration = Duration::from_millis(80); // ~12 FPS

#[derive(Debug, Clone)]
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

enum PortalError {
    Cancelled,
    Unavailable(String),
}

/// Converts raw PipeWire video buffer rows into packed RGBA (`[R, G, B, A]`).
pub fn convert_pipewire_to_rgba(
    format: VideoFormat,
    src: &[u8],
    stride: usize,
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    let w = width as usize;
    let h = height as usize;
    if w == 0 || h == 0 {
        return None;
    }

    let bpp = match format {
        VideoFormat::RGBA | VideoFormat::RGBx | VideoFormat::BGRA | VideoFormat::BGRx => 4,
        VideoFormat::RGB | VideoFormat::BGR => 3,
        _ => return None,
    };

    let row_bytes = w * bpp;
    let effective_stride = if stride >= row_bytes { stride } else { row_bytes };
    if src.len() < (h - 1) * effective_stride + row_bytes {
        return None;
    }

    let mut out = vec![255u8; w * h * 4];
    for y in 0..h {
        let s_row = &src[y * effective_stride..y * effective_stride + row_bytes];
        let d_row = &mut out[y * w * 4..(y + 1) * w * 4];
        match format {
            VideoFormat::RGBA => {
                d_row.copy_from_slice(s_row);
            }
            VideoFormat::RGBx => {
                d_row.copy_from_slice(s_row);
                for px in d_row.chunks_exact_mut(4) {
                    px[3] = 255;
                }
            }
            VideoFormat::BGRA => {
                for (s, d) in s_row.chunks_exact(4).zip(d_row.chunks_exact_mut(4)) {
                    d[0] = s[2];
                    d[1] = s[1];
                    d[2] = s[0];
                    d[3] = s[3];
                }
            }
            VideoFormat::BGRx => {
                for (s, d) in s_row.chunks_exact(4).zip(d_row.chunks_exact_mut(4)) {
                    d[0] = s[2];
                    d[1] = s[1];
                    d[2] = s[0];
                    d[3] = 255;
                }
            }
            VideoFormat::RGB => {
                for (s, d) in s_row.chunks_exact(3).zip(d_row.chunks_exact_mut(4)) {
                    d[0] = s[0];
                    d[1] = s[1];
                    d[2] = s[2];
                    d[3] = 255;
                }
            }
            VideoFormat::BGR => {
                for (s, d) in s_row.chunks_exact(3).zip(d_row.chunks_exact_mut(4)) {
                    d[0] = s[2];
                    d[1] = s[1];
                    d[2] = s[0];
                    d[3] = 255;
                }
            }
            _ => unreachable!(),
        }
    }
    Some(out)
}

/// Downscales an RGBA buffer to fit inside `max_w x max_h` preserving aspect ratio,
/// snapping dimensions to even integers (`>= 2`) required by I420 chroma subsampling.
pub fn downscale_rgba_even(
    src: &[u8],
    src_w: u32,
    src_h: u32,
    max_w: u32,
    max_h: u32,
) -> (u32, u32, Vec<u8>) {
    if src_w == 0 || src_h == 0 {
        return (0, 0, Vec::new());
    }

    let scale_w = max_w as f64 / src_w as f64;
    let scale_h = max_h as f64 / src_h as f64;
    let scale = scale_w.min(scale_h).min(1.0);

    let mut dst_w = ((src_w as f64 * scale).round() as u32).max(2);
    let mut dst_h = ((src_h as f64 * scale).round() as u32).max(2);
    dst_w &= !1;
    dst_h &= !1;
    let dst_w = dst_w.max(2);
    let dst_h = dst_h.max(2);

    if dst_w == src_w && dst_h == src_h && src.len() == (src_w * src_h * 4) as usize {
        return (dst_w, dst_h, src.to_vec());
    }

    let mut dst = vec![0u8; (dst_w * dst_h * 4) as usize];
    for dy in 0..dst_h {
        let sy = ((dy as u64 * src_h as u64) / dst_h as u64).min(src_h as u64 - 1) as usize;
        let src_row_off = sy * src_w as usize * 4;
        let dst_row_off = dy as usize * dst_w as usize * 4;
        for dx in 0..dst_w {
            let sx = ((dx as u64 * src_w as u64) / dst_w as u64).min(src_w as u64 - 1) as usize;
            let s_idx = src_row_off + sx * 4;
            let d_idx = dst_row_off + dx as usize * 4;
            if s_idx + 4 <= src.len() {
                dst[d_idx..d_idx + 4].copy_from_slice(&src[s_idx..s_idx + 4]);
            }
        }
    }
    (dst_w, dst_h, dst)
}

/// Parses a binary PPM (`P6`) buffer produced by `grim -t ppm -` into `(width, height, rgba)`.
pub fn parse_ppm_p6(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    if !bytes.starts_with(b"P6") {
        return Err("Not a P6 PPM stream".to_string());
    }
    let mut pos = 2usize;
    let mut tokens = Vec::with_capacity(3);

    while tokens.len() < 3 {
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= bytes.len() {
            return Err("Truncated PPM header".to_string());
        }
        if bytes[pos] == b'#' {
            while pos < bytes.len() && bytes[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }
        let start = pos;
        while pos < bytes.len() && !bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        let tok = std::str::from_utf8(&bytes[start..pos])
            .map_err(|e| format!("Invalid UTF-8 in PPM header: {}", e))?;
        tokens.push(tok);
    }

    // Single whitespace byte separates maxval from raw RGB raster data
    if pos >= bytes.len() || !bytes[pos].is_ascii_whitespace() {
        return Err("Missing whitespace separator after PPM maxval".to_string());
    }
    pos += 1;

    let width: u32 = tokens[0]
        .parse()
        .map_err(|_| format!("Invalid PPM width: {}", tokens[0]))?;
    let height: u32 = tokens[1]
        .parse()
        .map_err(|_| format!("Invalid PPM height: {}", tokens[1]))?;
    let maxval: u32 = tokens[2]
        .parse()
        .map_err(|_| format!("Invalid PPM maxval: {}", tokens[2]))?;

    if maxval != 255 || width == 0 || height == 0 {
        return Err(format!("Unsupported PPM dimensions/maxval: {}x{} max={}", width, height, maxval));
    }

    let expected_rgb = (width as usize) * (height as usize) * 3;
    let rgb = &bytes[pos..];
    if rgb.len() < expected_rgb {
        return Err(format!(
            "Incomplete PPM pixel buffer: expected {}, got {}",
            expected_rgb,
            rgb.len()
        ));
    }

    let mut rgba = vec![255u8; (width as usize) * (height as usize) * 4];
    for (s, d) in rgb[..expected_rgb].chunks_exact(3).zip(rgba.chunks_exact_mut(4)) {
        d[0] = s[0];
        d[1] = s[1];
        d[2] = s[2];
        d[3] = 255;
    }
    Ok((width, height, rgba))
}

/// Starts screen capture using Wayland xdg-desktop-portal + PipeWire as primary,
/// falling back cleanly to `grim -t ppm -` or `ffmpeg` if the portal is unavailable.
pub async fn start_screen_capture(
    shutdown: Arc<AtomicBool>,
) -> Result<mpsc::Receiver<CapturedFrame>, String> {
    let (frame_tx, frame_rx) = mpsc::channel::<CapturedFrame>(2);

    match start_portal_screencast(frame_tx.clone(), shutdown.clone()).await {
        Ok(()) => {
            log_info!("ScreenCast: initialized Wayland xdg-desktop-portal + PipeWire stream");
            return Ok(frame_rx);
        }
        Err(PortalError::Cancelled) => {
            return Err("Screen share cancelled in system picker".to_string());
        }
        Err(PortalError::Unavailable(reason)) => {
            log_warn!("ScreenCast portal unavailable ({}); trying CLI frame grabber fallback...", reason);
        }
    }

    start_cli_fallback_capture(frame_tx, shutdown).await?;
    Ok(frame_rx)
}

async fn start_portal_screencast(
    frame_tx: mpsc::Sender<CapturedFrame>,
    shutdown: Arc<AtomicBool>,
) -> Result<(), PortalError> {
    let proxy = Screencast::new()
        .await
        .map_err(|e| PortalError::Unavailable(format!("D-Bus Screencast proxy error: {}", e)))?;

    let session = proxy
        .create_session(Default::default())
        .await
        .map_err(|e| PortalError::Unavailable(format!("create_session error: {}", e)))?;

    proxy
        .select_sources(
            &session,
            SelectSourcesOptions::default()
                .set_cursor_mode(CursorMode::EMBEDDED)
                .set_sources(SourceType::MONITOR | SourceType::WINDOW)
                .set_multiple(false)
                .set_persist_mode(PersistMode::DoNot),
        )
        .await
        .map_err(|e| PortalError::Unavailable(format!("select_sources error: {}", e)))?;

    let start_req = proxy
        .start(&session, None, Default::default())
        .await
        .map_err(|e| PortalError::Unavailable(format!("start error: {}", e)))?;

    let streams = match start_req.response() {
        Ok(resp) => resp,
        Err(ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled)) => {
            let _ = session.close().await;
            return Err(PortalError::Cancelled);
        }
        Err(e) => {
            let _ = session.close().await;
            return Err(PortalError::Unavailable(format!("portal response error: {}", e)));
        }
    };

    let Some(stream_info) = streams.streams().first() else {
        let _ = session.close().await;
        return Err(PortalError::Unavailable("portal returned zero streams".to_string()));
    };
    let node_id = stream_info.pipe_wire_node_id();
    log_info!(
        "ScreenCast portal granted node_id={} size={:?}",
        node_id,
        stream_info.size()
    );

    let fd = proxy
        .open_pipe_wire_remote(&session, Default::default())
        .await
        .map_err(|e| PortalError::Unavailable(format!("open_pipe_wire_remote error: {}", e)))?;

    // Keep portal session alive until shutdown
    let session_shutdown = shutdown.clone();
    tokio::spawn(async move {
        while !session_shutdown.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let _ = session.close().await;
    });

    spawn_pipewire_reader_thread(fd, node_id, frame_tx, shutdown)
        .map_err(PortalError::Unavailable)?;

    Ok(())
}

fn spawn_pipewire_reader_thread(
    fd: OwnedFd,
    node_id: u32,
    frame_tx: mpsc::Sender<CapturedFrame>,
    shutdown: Arc<AtomicBool>,
) -> Result<(), String> {
    let (init_tx, init_rx) = std::sync::mpsc::sync_channel::<Result<(), String>>(1);

    std::thread::spawn(move || {
        pw::init();

        let mainloop = match pw::main_loop::MainLoopRc::new(None) {
            Ok(ml) => ml,
            Err(e) => {
                let _ = init_tx.send(Err(format!("PipeWire MainLoopRc error: {}", e)));
                return;
            }
        };

        let context = match pw::context::ContextRc::new(&mainloop, None) {
            Ok(c) => c,
            Err(e) => {
                let _ = init_tx.send(Err(format!("PipeWire ContextRc error: {}", e)));
                return;
            }
        };

        let core = match context.connect_fd_rc(fd, None) {
            Ok(c) => c,
            Err(e) => {
                let _ = init_tx.send(Err(format!("PipeWire connect_fd_rc error: {}", e)));
                return;
            }
        };

        let stream = match pw::stream::StreamBox::new(
            &core,
            "stoat-tui-screencast",
            properties! {
                *pw::keys::MEDIA_TYPE => "Video",
                *pw::keys::MEDIA_CATEGORY => "Capture",
                *pw::keys::MEDIA_ROLE => "Screen",
            },
        ) {
            Ok(s) => s,
            Err(e) => {
                let _ = init_tx.send(Err(format!("PipeWire StreamBox error: {}", e)));
                return;
            }
        };

        struct PwState {
            format: spa::param::video::VideoInfoRaw,
            last_sent: Instant,
            frame_tx: mpsc::Sender<CapturedFrame>,
        }

        let user_data = PwState {
            format: Default::default(),
            last_sent: Instant::now() - PIPEWIRE_MIN_FRAME_INTERVAL,
            frame_tx,
        };

        let mainloop_err = mainloop.clone();
        let _listener = match stream
            .add_local_listener_with_user_data(user_data)
            .state_changed(move |_, _, old, new| {
                log_debug!("PipeWire screencast state: {:?} -> {:?}", old, new);
                if matches!(new, pw::stream::StreamState::Error(_)) {
                    mainloop_err.quit();
                }
            })
            .param_changed(|_, state, id, param| {
                let Some(param) = param else { return; };
                if id != spa::param::ParamType::Format.as_raw() {
                    return;
                }
                let Ok((media_type, media_subtype)) = format_utils::parse_format(param) else {
                    return;
                };
                if media_type != MediaType::Video || media_subtype != MediaSubtype::Raw {
                    return;
                }
                if state.format.parse(param).is_ok() {
                    log_info!(
                        "PipeWire negotiated video format: {:?} {}x{}",
                        state.format.format(),
                        state.format.size().width,
                        state.format.size().height
                    );
                }
            })
            .process(|stream, state| {
                let Some(mut buffer) = stream.dequeue_buffer() else {
                    return;
                };
                if state.last_sent.elapsed() < PIPEWIRE_MIN_FRAME_INTERVAL {
                    return;
                }
                let datas = buffer.datas_mut();
                if datas.is_empty() {
                    return;
                }
                let width = state.format.size().width;
                let height = state.format.size().height;
                let fmt = state.format.format();
                if width == 0 || height == 0 {
                    return;
                }

                let data = &mut datas[0];
                let chunk_offset = data.chunk().offset() as usize;
                let chunk_size = data.chunk().size() as usize;
                let chunk_stride = data.chunk().stride() as usize;
                let Some(raw) = data.data() else {
                    return;
                };
                let end = (chunk_offset + chunk_size).min(raw.len());
                if chunk_offset >= end {
                    return;
                }

                if let Some(rgba) =
                    convert_pipewire_to_rgba(fmt, &raw[chunk_offset..end], chunk_stride, width, height)
                {
                    let (dw, dh, scaled) = downscale_rgba_even(
                        &rgba,
                        width,
                        height,
                        MAX_CAPTURE_WIDTH,
                        MAX_CAPTURE_HEIGHT,
                    );
                    if dw > 0 && dh > 0 {
                        state.last_sent = Instant::now();
                        let _ = state.frame_tx.try_send(CapturedFrame {
                            width: dw,
                            height: dh,
                            rgba: scaled,
                        });
                    }
                }
            })
            .register()
        {
            Ok(l) => l,
            Err(e) => {
                let _ = init_tx.send(Err(format!("PipeWire listener register error: {}", e)));
                return;
            }
        };

        let obj = spa::pod::object!(
            spa::utils::SpaTypes::ObjectParamFormat,
            spa::param::ParamType::EnumFormat,
            spa::pod::property!(
                spa::param::format::FormatProperties::MediaType,
                Id,
                MediaType::Video
            ),
            spa::pod::property!(
                spa::param::format::FormatProperties::MediaSubtype,
                Id,
                MediaSubtype::Raw
            ),
            spa::pod::property!(
                spa::param::format::FormatProperties::VideoFormat,
                Choice,
                Enum,
                Id,
                VideoFormat::BGRx,
                VideoFormat::BGRx,
                VideoFormat::BGRA,
                VideoFormat::RGBA,
                VideoFormat::RGBx,
                VideoFormat::RGB,
            ),
            spa::pod::property!(
                spa::param::format::FormatProperties::VideoSize,
                Choice,
                Range,
                Rectangle,
                spa::utils::Rectangle {
                    width: 1280,
                    height: 720
                },
                spa::utils::Rectangle {
                    width: 1,
                    height: 1
                },
                spa::utils::Rectangle {
                    width: 4096,
                    height: 4096
                }
            ),
            spa::pod::property!(
                spa::param::format::FormatProperties::VideoFramerate,
                Choice,
                Range,
                Fraction,
                spa::utils::Fraction { num: 20, denom: 1 },
                spa::utils::Fraction { num: 1, denom: 1 },
                spa::utils::Fraction { num: 30, denom: 1 }
            ),
        );
        let values: Vec<u8> = spa::pod::serialize::PodSerializer::serialize(
            Cursor::new(Vec::new()),
            &spa::pod::Value::Object(obj),
        )
        .unwrap()
        .0
        .into_inner();
        let mut params = [Pod::from_bytes(&values).unwrap()];

        if let Err(e) = stream.connect(
            spa::utils::Direction::Input,
            Some(node_id),
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        ) {
            let _ = init_tx.send(Err(format!("PipeWire stream.connect error: {}", e)));
            return;
        }

        let mainloop_weak = mainloop.downgrade();
        let timer = mainloop.loop_().add_timer(move |_| {
            if shutdown.load(Ordering::SeqCst) {
                if let Some(ml) = mainloop_weak.upgrade() {
                    ml.quit();
                }
            }
        });
        let _ = timer.update_timer(
            Some(Duration::from_millis(100)),
            Some(Duration::from_millis(100)),
        );

        let _ = init_tx.send(Ok(()));
        mainloop.run();
        log_debug!("PipeWire screencast loop exited");
    });

    init_rx
        .recv()
        .map_err(|_| "PipeWire initialization thread aborted".to_string())?
}

async fn start_cli_fallback_capture(
    frame_tx: mpsc::Sender<CapturedFrame>,
    shutdown: Arc<AtomicBool>,
) -> Result<(), String> {
    // 1. Test `grim -t ppm -` (standard wlroots/Hyprland fast frame grabber)
    if let Ok(test_frame) = tokio::task::spawn_blocking(grab_grim_frame).await.map_err(|e| e.to_string())? {
        log_info!(
            "ScreenCast fallback: using `grim -t ppm -` ({}x{})",
            test_frame.width,
            test_frame.height
        );
        let _ = frame_tx.try_send(test_frame);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(CLI_FALLBACK_FRAME_INTERVAL);
            loop {
                interval.tick().await;
                if shutdown.load(Ordering::SeqCst) {
                    break;
                }
                match tokio::task::spawn_blocking(grab_grim_frame).await {
                    Ok(Ok(frame)) => {
                        let _ = frame_tx.try_send(frame);
                    }
                    Ok(Err(e)) => {
                        log_warn!("grim capture frame error: {}", e);
                    }
                    Err(e) => {
                        log_warn!("grim blocking task error: {}", e);
                    }
                }
            }
        });
        return Ok(());
    }

    // 2. Test `ffmpeg` X11 fallback if DISPLAY is set
    if let Ok(display) = std::env::var("DISPLAY") {
        if !display.trim().is_empty() {
            if let Ok(test_frame) =
                tokio::task::spawn_blocking(move || grab_ffmpeg_x11_frame(&display))
                    .await
                    .map_err(|e| e.to_string())?
            {
                log_info!(
                    "ScreenCast fallback: using `ffmpeg -f x11grab` ({}x{})",
                    test_frame.width,
                    test_frame.height
                );
                let _ = frame_tx.try_send(test_frame);

                tokio::spawn(async move {
                    let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".to_string());
                    let mut interval = tokio::time::interval(CLI_FALLBACK_FRAME_INTERVAL);
                    loop {
                        interval.tick().await;
                        if shutdown.load(Ordering::SeqCst) {
                            break;
                        }
                        let disp = display.clone();
                        if let Ok(Ok(frame)) =
                            tokio::task::spawn_blocking(move || grab_ffmpeg_x11_frame(&disp)).await
                        {
                            let _ = frame_tx.try_send(frame);
                        }
                    }
                });
                return Ok(());
            }
        }
    }

    let msg = "No Wayland ScreenCast portal, `grim`, or X11 `ffmpeg` capture available".to_string();
    log_error!("ScreenCast fallback failed: {}", msg);
    Err(msg)
}

fn grab_grim_frame() -> Result<CapturedFrame, String> {
    let output = Command::new("grim")
        .args(["-t", "ppm", "-"])
        .output()
        .map_err(|e| format!("Failed to execute grim: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("grim exited with {}: {}", output.status, stderr.trim()));
    }

    let (w, h, rgba) = parse_ppm_p6(&output.stdout)?;
    let (dw, dh, scaled) = downscale_rgba_even(&rgba, w, h, MAX_CAPTURE_WIDTH, MAX_CAPTURE_HEIGHT);
    Ok(CapturedFrame {
        width: dw,
        height: dh,
        rgba: scaled,
    })
}

fn grab_ffmpeg_x11_frame(display: &str) -> Result<CapturedFrame, String> {
    let output = Command::new("ffmpeg")
        .args([
            "-loglevel",
            "error",
            "-f",
            "x11grab",
            "-i",
            display,
            "-vframes",
            "1",
            "-vf",
            "scale='min(1280,iw)':-2",
            "-f",
            "image2pipe",
            "-vcodec",
            "ppm",
            "-",
        ])
        .output()
        .map_err(|e| format!("Failed to execute ffmpeg: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffmpeg exited with {}: {}", output.status, stderr.trim()));
    }

    let (w, h, rgba) = parse_ppm_p6(&output.stdout)?;
    let (dw, dh, scaled) = downscale_rgba_even(&rgba, w, h, MAX_CAPTURE_WIDTH, MAX_CAPTURE_HEIGHT);
    Ok(CapturedFrame {
        width: dw,
        height: dh,
        rgba: scaled,
    })
}
