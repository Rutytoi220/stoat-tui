# stoat-tui

A fast, keyboard-first, and mouse-accessible Terminal User Interface (TUI) client for **Stoat** (and Revolt-compatible Delta/Bonfire servers), written in Rust with `ratatui`, `tokio`, `crossterm`, `livekit`, and `cpal`.

---

## Features

- **Unified Multi-Instance Account Support:**
  - Connect to `https://api.stoat.chat` and one or more self-hosted Stoat / Delta instances (e.g. `https://alpha-node.<tailnet>.ts.net`) simultaneously.
  - Automatic `/api` base-path discovery when logging into self-hosted servers.
  - All Direct Messages, Groups, and Servers across connected accounts are merged into a single unified sidebar with muted instance tags (e.g. `@ coucou (alpha-node)`), concurrent Bonfire WebSocket gateways, and per-account REST routing.
- **Real-Time LiveKit WebRTC Voice & Screen Sharing:**
  - Native WebRTC voice calls powered by `livekit` + `cpal` (48kHz mono capture & multi-participant audio mixing) with DM/Group ringing (`POST /channels/{id}/ring`), automatic ghost-session recovery (`AlreadyConnected` -> `leave_call` retry), and synthesized join/leave/mute/deafen chimes.
  - Real Linux/Wayland screen sharing (`TrackSource::ScreenShare`) via `xdg-desktop-portal` ScreenCast + PipeWire (`ashpd` / `gstreamer`) with automatic CLI frame-grabber fallback (`grim` / `ffmpeg`) and remote video stream rendering inside the TUI.
  - Dynamic auto-sizing **Live Voice & Video Stage** with PFP participant tiles, green speaking outlines, and `🔇`/`🎧` status badges.
- **True Graphics Protocol Rendering (Kitty / Ghostty / WezTerm / Sixel / Halfblocks):**
  - Renders user avatars (PFPs), server icons, image attachments, and live call video streams directly in the terminal via `ratatui-image`, with fast non-blocking protocol detection and an LRU image cache.
- **Instant Startup (`<15ms` to First Frame):**
  - Renders the first TUI frame before any network I/O using a persistent navigation cache (`~/.config/stoat-tui/nav_cache.json`), then hydrates `GET /`, `GET /users/@me`, `GET /users/dms`, and Bonfire WebSocket connections concurrently in the background.
- **Responsive Layout Down to `10x10` Cells:**
  - Adapts gracefully from full 3-column desktop layouts (resizable Left Sidebar + Main Chat/Call Stage + collapsible Member Rail) down to ultra-compact single-pane layouts at `10x10` terminal cells.

---

## Installation

### 1. One-Line Installer (Linux / macOS — Prebuilt Binary with Cargo Fallback)
```bash
curl -fsSL https://raw.githubusercontent.com/Rutytoi220/stoat-tui/master/scripts/install.sh | sh
```
Downloads the latest prebuilt release binary from [GitHub Releases](https://github.com/Rutytoi220/stoat-tui/releases/latest) (verifying SHA-256 checksums) into `~/.local/bin` (or `/usr/local/bin`), and automatically falls back to `cargo install --git` if no prebuilt binary exists for your architecture.

### 2. Via Cargo Directly from GitHub (No Clone Required)
```bash
cargo install --git https://github.com/Rutytoi220/stoat-tui
```

### 3. Prebuilt Binaries
Download the `.tar.gz` archive for your platform directly from [GitHub Releases](https://github.com/Rutytoi220/stoat-tui/releases/latest), extract it, and place `stoat-tui` in your `$PATH`.

### 4. Arch Linux (`PKGBUILD`)
System dependencies for building with ALSA audio and optional Wayland screen sharing:
```bash
sudo pacman -S --needed base-devel rust alsa-lib gtk3
# Optional for Wayland screen sharing & file picker:
sudo pacman -S --needed wf-recorder grim zenity
cargo install --git https://github.com/Rutytoi220/stoat-tui
```
Or using the included `packaging/PKGBUILD` / `packaging/PKGBUILD-bin`:
```bash
cd packaging
makepkg -si
```

---

## Keybindings

| Shortcut | Context | Action |
|---|---|---|
| `Tab` / `Shift+Tab` | Global | Cycle focus (`Servers` ➔ `Channels` ➔ `Chat` ➔ `Input` ➔ `Members`) |
| `Alt+1` .. `Alt+5` | Global | Jump directly to `Servers`, `Channels`, `Chat`, `Input`, or `Members` |
| `Ctrl+P` | Global | Open Fuzzy Command Palette / Quick Switcher |
| `F2` / `Alt+Shift+L` | Global | Open Login / Add Account Modal (`Email & Password`, `Token`, `Demo`) |
| `Shift+S` (`S`) | Non-Input | Open Settings & Profile Modal (profile, presence, audio chimes, accounts) |
| `Shift+F` (`F`) / `F3` | Non-Input / Global | Open Friends & Friend Requests Modal (add, accept, decline, open DM) |
| `Shift+G` (`G`) / `F4` | Non-Input / Global | Open Create Group Modal (name, description, multi-select friends) |
| `Ctrl+X` / `Delete` / Right-Click | Global / Sidebar | Delete / Leave Group or close DM (with confirmation popup) |
| `Ctrl+H` / `Ctrl+L` | Global | Shrink / expand left sidebar width (`Ctrl+Left` / `Ctrl+Right` also work) |
| `Alt+M` | Non-Call | Toggle Right Member Rail |
| `Ctrl+Alt+J` / `K` | Global | Switch to next / previous server |
| `Alt+Shift+J` / `K` | Global | Jump to next / previous unread channel |
| `c` / `Alt+C` | Non-Input / Global | Join or leave voice call in the active channel |
| `m` / `Alt+M` | Active Call | Toggle microphone mute (`🔇`) |
| `d` / `Alt+D` | Active Call | Toggle audio deafen (`🎧`) |
| `s` / `Alt+S` | Active Call | Toggle Wayland / CLI screen sharing (`🖥`) |
| `v` / `Alt+V` | Active Call | Toggle local screen-share preview in Call Stage |
| `f` / `Alt+F` | Active Call | Toggle fullscreen Call Stage |
| `u` / `Ctrl+U` | Non-Input / Global | Open file picker to upload an attachment (`zenity` / `kdialog`) |
| `i` / `Enter` | Chat Feed | Focus message input bar |
| `Enter` | Input Bar | Send message (with optimistic nonce deduplication) |
| `Backspace` | Empty Input Bar | Remove last pending attachment |
| `Esc` | Modal / Input Bar | Close active modal or return focus from Input to Chat |
| `Ctrl+C` | Global | Cleanly leave any active call and exit application |

---

## Configuration & State Files

- **Configuration File:** `~/.config/stoat-tui/config.toml` (written with `0600` permissions)
- **Navigation Cache:** `~/.config/stoat-tui/nav_cache.json` (instant startup cache of servers, channels, DMs, and Autumn/WebSocket URLs)
- **Diagnostic Log:** `~/.local/state/stoat-tui/debug.log` (structured diagnostic log with tokens and passwords redacted)

Example `~/.config/stoat-tui/config.toml`:
```toml
api_url = "https://api.stoat.chat"
is_bot = false
left_sidebar_width = 28
call_sounds_enabled = true
compact_mode = false
render_avatars = true

[[accounts]]
id = "primary"
label = ""
api_url = "https://api.stoat.chat"
token = "your_primary_session_token"
is_bot = false

[[accounts]]
id = "alpha-node"
label = "alpha-node"
api_url = "https://alpha-node.example.ts.net/api"
token = "your_self_hosted_session_token"
is_bot = false

[theme]
primary = "#FD6671"
accent = "#5970FD"
background = "#191919"
sidebar_bg = "#202020"
rail_bg = "#151515"
text = "#F6F6F6"
muted = "#848484"
```

---

## License

Licensed under the [MIT License](./LICENSE).
