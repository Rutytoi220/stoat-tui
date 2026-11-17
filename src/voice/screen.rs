use crate::{log_debug, log_error, log_info, log_warn};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;

pub const MAX_CAPTURE_WIDTH: u32 = 1280;
pub const MAX_CAPTURE_HEIGHT: u32 = 720;
const X11_FALLBACK_FRAME_INTERVAL: Duration = Duration::from_millis(33);

#[derive(Debug, Clone)]
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenShareTargetKind {
    EntireDesktop,
    Monitor {
        name: String,
        width: u32,
        height: u32,
    },
    Window {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        class: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenShareTarget {
    pub label: String,
    pub kind: ScreenShareTargetKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenCaptureError {
    Cancelled,
    NeedTargetSelection(Vec<ScreenShareTarget>),
    Failed(String),
}

pub fn grim_args_for_target(target: &ScreenShareTargetKind) -> Vec<String> {
    match target {
        ScreenShareTargetKind::EntireDesktop => {
            vec!["-t".to_string(), "ppm".to_string(), "-".to_string()]
        }
        ScreenShareTargetKind::Monitor { name, .. } => vec![
            "-o".to_string(),
            name.clone(),
            "-t".to_string(),
            "ppm".to_string(),
            "-".to_string(),
        ],
        ScreenShareTargetKind::Window {
            x,
            y,
            width,
            height,
            ..
        } => vec![
            "-g".to_string(),
            format!("{},{} {}x{}", x, y, width, height),
            "-t".to_string(),
            "ppm".to_string(),
            "-".to_string(),
        ],
    }
}

/// Builds `wf-recorder` CLI arguments for raw RGBA stdout streaming at 60 FPS,
/// returning `(args, src_width, src_height)` when exact dimensions are known.
pub fn wf_recorder_args_for_target(
    target: &ScreenShareTargetKind,
) -> Option<(Vec<String>, u32, u32)> {
    match target {
        ScreenShareTargetKind::Monitor {
            name,
            width,
            height,
        } if *width > 0 && *height > 0 => Some((
            vec![
                "--output".to_string(),
                name.clone(),
                "-c".to_string(),
                "rawvideo".to_string(),
                "-m".to_string(),
                "rawvideo".to_string(),
                "-x".to_string(),
                "rgba".to_string(),
                "-f".to_string(),
                "pipe:1".to_string(),
            ],
            *width,
            *height,
        )),
        ScreenShareTargetKind::Window {
            x,
            y,
            width,
            height,
            ..
        } if *width > 0 && *height > 0 => Some((
            vec![
                "--geometry".to_string(),
                format!("{},{} {}x{}", x, y, width, height),
                "-c".to_string(),
                "rawvideo".to_string(),
                "-m".to_string(),
                "rawvideo".to_string(),
                "-x".to_string(),
                "rgba".to_string(),
                "-f".to_string(),
                "pipe:1".to_string(),
            ],
            *width,
            *height,
        )),
        _ => None,
    }
}

pub fn parse_hyprctl_monitors_json(json_str: &str) -> Vec<ScreenShareTarget> {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) else {
        return Vec::new();
    };
    let Some(arr) = val.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in arr {
        let Some(name) = item.get("name").and_then(|v| v.as_str()).map(str::trim) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let width = item.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let height = item.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let label = if width > 0 && height > 0 {
            format!("Monitor: {} ({}x{})", name, width, height)
        } else {
            format!("Monitor: {}", name)
        };
        out.push(ScreenShareTarget {
            label,
            kind: ScreenShareTargetKind::Monitor {
                name: name.to_string(),
                width,
                height,
            },
        });
    }
    out
}

pub fn parse_hyprctl_clients_json(json_str: &str) -> Vec<ScreenShareTarget> {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) else {
        return Vec::new();
    };
    let Some(arr) = val.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in arr {
        let mapped = item.get("mapped").and_then(|v| v.as_bool()).unwrap_or(true);
        let hidden = item.get("hidden").and_then(|v| v.as_bool()).unwrap_or(false);
        if !mapped || hidden {
            continue;
        }
        let Some(at) = item.get("at").and_then(|v| v.as_array()) else {
            continue;
        };
        let Some(size) = item.get("size").and_then(|v| v.as_array()) else {
            continue;
        };
        if at.len() < 2 || size.len() < 2 {
            continue;
        }
        let x = at[0].as_i64().unwrap_or(0) as i32;
        let y = at[1].as_i64().unwrap_or(0) as i32;
        let width = size[0].as_u64().unwrap_or(0) as u32;
        let height = size[1].as_u64().unwrap_or(0) as u32;
        if width == 0 || height == 0 {
            continue;
        }
        let class = item
            .get("class")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        let raw_title = item
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("");
        let title_preview: String = raw_title.chars().take(36).collect();
        let display_name = if !title_preview.is_empty() && !class.is_empty() {
            format!("{} [{}]", title_preview, class)
        } else if !title_preview.is_empty() {
            title_preview
        } else if !class.is_empty() {
            class.clone()
        } else {
            "Window".to_string()
        };
        out.push(ScreenShareTarget {
            label: format!("Window: {} ({}x{})", display_name, width, height),
            kind: ScreenShareTargetKind::Window {
                x,
                y,
                width,
                height,
                class,
            },
        });
    }
    out
}

pub fn build_hyprland_targets(
    monitors_json: Option<&str>,
    clients_json: Option<&str>,
) -> Vec<ScreenShareTarget> {
    let mut targets = Vec::new();
    if let Some(m) = monitors_json {
        targets.extend(parse_hyprctl_monitors_json(m));
    }
    if let Some(c) = clients_json {
        targets.extend(parse_hyprctl_clients_json(c));
    }
    targets.push(ScreenShareTarget {
        label: "Entire Desktop (All Monitors)".to_string(),
        kind: ScreenShareTargetKind::EntireDesktop,
    });
    targets
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

    if dst_w == src_w && dst_h == src_h && src.len() == (src_w as usize) * (src_h as usize) * 4 {
        return (dst_w, dst_h, src.to_vec());
    }

    let mut dst = vec![0u8; (dst_w as usize) * (dst_h as usize) * 4];
    let src_w_usize = src_w as usize;
    let src_h_usize = src_h as usize;
    let dst_w_usize = dst_w as usize;
    let dst_h_usize = dst_h as usize;
    let src_stride = src_w_usize * 4;
    let dst_stride = dst_w_usize * 4;

    if src.len() < src_h_usize * src_stride {
        return (dst_w, dst_h, dst);
    }

    // Precompute integer byte offsets per column once so the inner pixel loop has zero divisions/floats
    let x_offsets: Vec<usize> = (0..dst_w_usize)
        .map(|dx| ((dx * src_w_usize) / dst_w_usize).min(src_w_usize - 1) * 4)
        .collect();

    for (dy, dst_row) in dst.chunks_exact_mut(dst_stride).enumerate() {
        let sy = ((dy * src_h_usize) / dst_h_usize).min(src_h_usize - 1);
        let src_row = &src[sy * src_stride..(sy + 1) * src_stride];
        for (dst_px, &s_off) in dst_row.chunks_exact_mut(4).zip(x_offsets.iter()) {
            dst_px.copy_from_slice(&src_row[s_off..s_off + 4]);
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
        return Err(format!(
            "Unsupported PPM dimensions/maxval: {}x{} max={}",
            width, height, maxval
        ));
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
    for (s, d) in rgb[..expected_rgb]
        .chunks_exact(3)
        .zip(rgba.chunks_exact_mut(4))
    {
        d[0] = s[0];
        d[1] = s[1];
        d[2] = s[2];
        d[3] = 255;
    }
    Ok((width, height, rgba))
}

/// Starts pure-Rust subprocess screen capture (zero PipeWire C FFI).
/// On Wayland (`WAYLAND_DISPLAY` set):
/// 1. Queries `hyprctl monitors -j` and `hyprctl clients -j` and opens `ScreenPickerModal`
///    whenever multiple targets exist (when `selected_target` is `None`).
/// 2. Streams at 30–60 FPS via `wf-recorder` raw RGBA pipe if available, falling back
///    cleanly to a back-to-back async `grim` loop.
pub async fn start_screen_capture(
    shutdown: Arc<AtomicBool>,
    selected_target: Option<ScreenShareTargetKind>,
) -> Result<mpsc::Receiver<CapturedFrame>, ScreenCaptureError> {
    let (frame_tx, frame_rx) = mpsc::channel::<CapturedFrame>(2);

    if let Some(target) = selected_target {
        if start_wf_recorder_capture(frame_tx.clone(), shutdown.clone(), &target)
            .await
            .is_ok()
        {
            return Ok(frame_rx);
        }
        start_grim_capture_target(frame_tx, shutdown, target)
            .await
            .map_err(ScreenCaptureError::Failed)?;
        return Ok(frame_rx);
    }

    let has_wayland = std::env::var("WAYLAND_DISPLAY")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);

    if has_wayland {
        let targets = list_grim_targets().await;
        if targets.len() > 1 {
            return Err(ScreenCaptureError::NeedTargetSelection(targets));
        }
        let target = targets
            .into_iter()
            .next()
            .map(|t| t.kind)
            .unwrap_or(ScreenShareTargetKind::EntireDesktop);

        if start_wf_recorder_capture(frame_tx.clone(), shutdown.clone(), &target)
            .await
            .is_ok()
        {
            return Ok(frame_rx);
        }
        start_grim_capture_target(frame_tx, shutdown, target)
            .await
            .map_err(ScreenCaptureError::Failed)?;
        return Ok(frame_rx);
    }

    // X11 fallback
    start_ffmpeg_x11_capture(frame_tx, shutdown)
        .await
        .map_err(ScreenCaptureError::Failed)?;
    Ok(frame_rx)
}

pub async fn list_grim_targets() -> Vec<ScreenShareTarget> {
    let monitors_json = match tokio::process::Command::new("hyprctl")
        .args(["monitors", "-j"])
        .output()
        .await
    {
        Ok(out) if out.status.success() => String::from_utf8(out.stdout).ok(),
        _ => None,
    };

    let clients_json = match tokio::process::Command::new("hyprctl")
        .args(["clients", "-j"])
        .output()
        .await
    {
        Ok(out) if out.status.success() => String::from_utf8(out.stdout).ok(),
        _ => None,
    };

    build_hyprland_targets(monitors_json.as_deref(), clients_json.as_deref())
}

async fn start_wf_recorder_capture(
    frame_tx: mpsc::Sender<CapturedFrame>,
    shutdown: Arc<AtomicBool>,
    target: &ScreenShareTargetKind,
) -> Result<(), String> {
    let Some((args, src_w, src_h)) = wf_recorder_args_for_target(target) else {
        return Err("Target does not have fixed dimensions for wf-recorder".to_string());
    };

    let mut child = tokio::process::Command::new("wf-recorder")
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("wf-recorder unavailable: {}", e))?;

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "wf-recorder missing stdout pipe".to_string())?;

    let frame_bytes = (src_w as usize) * (src_h as usize) * 4;
    let mut raw_buf = vec![0u8; frame_bytes];

    // Verify first frame arrives cleanly before committing to wf-recorder
    tokio::time::timeout(Duration::from_millis(1200), stdout.read_exact(&mut raw_buf))
        .await
        .map_err(|_| "wf-recorder timed out waiting for first frame".to_string())?
        .map_err(|e| format!("wf-recorder exited before first frame: {}", e))?;

    let (dw, dh, scaled) =
        downscale_rgba_even(&raw_buf, src_w, src_h, MAX_CAPTURE_WIDTH, MAX_CAPTURE_HEIGHT);
    let _ = frame_tx.try_send(CapturedFrame {
        width: dw,
        height: dh,
        rgba: scaled,
    });

    log_info!(
        "ScreenCast: streaming via `wf-recorder` ({}x{} -> {}x{})",
        src_w,
        src_h,
        dw,
        dh
    );

    tokio::spawn(async move {
        let mut child = child;
        loop {
            if shutdown.load(Ordering::SeqCst) {
                let _ = child.kill().await;
                break;
            }
            match stdout.read_exact(&mut raw_buf).await {
                Ok(_) => {
                    let (dw, dh, scaled) = downscale_rgba_even(
                        &raw_buf,
                        src_w,
                        src_h,
                        MAX_CAPTURE_WIDTH,
                        MAX_CAPTURE_HEIGHT,
                    );
                    if dw > 0 && dh > 0 {
                        let _ = frame_tx.try_send(CapturedFrame {
                            width: dw,
                            height: dh,
                            rgba: scaled,
                        });
                    }
                }
                Err(e) => {
                    log_debug!("wf-recorder stdout stream ended: {}", e);
                    let _ = child.kill().await;
                    break;
                }
            }
        }
    });

    Ok(())
}

async fn start_grim_capture_target(
    frame_tx: mpsc::Sender<CapturedFrame>,
    shutdown: Arc<AtomicBool>,
    target: ScreenShareTargetKind,
) -> Result<(), String> {
    let test_frame = grab_grim_frame_async(&target).await?;
    log_info!(
        "ScreenCast: using safe `grim` back-to-back capture ({:?}, {}x{})",
        target,
        test_frame.width,
        test_frame.height
    );
    let _ = frame_tx.try_send(test_frame);

    tokio::spawn(async move {
        loop {
            if shutdown.load(Ordering::SeqCst) {
                break;
            }
            match grab_grim_frame_async(&target).await {
                Ok(frame) => {
                    let _ = frame_tx.try_send(frame);
                }
                Err(e) => {
                    log_warn!("grim capture frame error: {}", e);
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
            tokio::task::yield_now().await;
        }
    });
    Ok(())
}

async fn start_ffmpeg_x11_capture(
    frame_tx: mpsc::Sender<CapturedFrame>,
    shutdown: Arc<AtomicBool>,
) -> Result<(), String> {
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
                    let mut interval = tokio::time::interval(X11_FALLBACK_FRAME_INTERVAL);
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

    let msg = "No Wayland `wf-recorder`/`grim` or X11 `ffmpeg` capture available".to_string();
    log_error!("ScreenCast failed: {}", msg);
    Err(msg)
}

async fn grab_grim_frame_async(target: &ScreenShareTargetKind) -> Result<CapturedFrame, String> {
    let args = grim_args_for_target(target);
    let output = tokio::process::Command::new("grim")
        .args(&args)
        .output()
        .await
        .map_err(|e| format!("Failed to execute grim: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("grim exited with {}: {}", output.status, stderr.trim()));
    }

    let (w, h, rgba) = parse_ppm_p6(&output.stdout)?;
    let (dw, dh, scaled) = downscale_rgba_even(&rgba, w, h, MAX_CAPTURE_WIDTH, MAX_CAPTURE_HEIGHT);
    if dw == 0 || dh == 0 || scaled.len() != (dw as usize) * (dh as usize) * 4 {
        return Err("Invalid scaled frame dimensions from grim".to_string());
    }
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
