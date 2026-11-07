# stoat-tui

A fast, keyboard-first, and fully mouse-accessible Terminal User Interface (TUI) client for **Stoat** (formerly Revolt) written in 100% Rust with `ratatui`, `tokio`, `crossterm`, and `tokio-tungstenite`.

---

## Installation

### 1. One-Liner Quick Install (Linux x86_64 / ARM64)
Automated POSIX script that detects your CPU architecture, verifies SHA-256 integrity, and installs the static portable binary directly to `/usr/local/bin` (or `~/.local/bin` without sudo):
```bash
curl -fsSL https://raw.githubusercontent.com/stoat-chat/stoat-tui/main/scripts/install.sh | sh
```

### 2. Via Cargo
Install the latest release directly from crates.io or GitHub:
```bash
# From GitHub repository
cargo install --git https://github.com/stoat-chat/stoat-tui

# Or from local clone
cargo install --path .
```

### 3. Arch Linux (AUR / PKGBUILD)
Build from source using standard Arch Linux tooling:
```bash
cd packaging
makepkg -si
```

Or install the precompiled musl binary instantly without compilation:
```bash
cd packaging
makepkg -p PKGBUILD-bin -si
```

### 4. Manual Release Download
Precompiled release tarballs with checksums for `x86_64-unknown-linux-musl`, `x86_64-unknown-linux-gnu`, and `aarch64-unknown-linux-gnu` are available on the [GitHub Releases](https://github.com/stoat-chat/stoat-tui/releases) page.

---

## CLI Options

```
stoat-tui [OPTIONS]

OPTIONS:
    -h, -H, --help       Print help information, config paths, and keybindings
    -v, -V, --version    Print version information
    -a, --api <URL>      Override base API URL (e.g. https://api.revolt.chat or https://api.stoat.chat)
```

Running `stoat-tui` without options launches the fullscreen reactive TUI. You can easily toggle between the official Stoat API (`https://api.stoat.chat`) and Revolt API (`https://api.revolt.chat`) via the `--api` flag or in `config.toml`.

---

## Features

- **Responsive 3-Column + Member Rail Layout:**
  - **Left Rail (6–8 cols):** Direct Messages shortcut (`@`) and server list with unread notification pills.
  - **Channel Sidebar (20–24 cols):** Active server categories, text channels (`# general`), and voice channels.
  - **Main Content Pane:**
    - **Header:** Channel title, topic, connection status pill, and collapsible member rail toggle.
    - **Message Feed:** Scrollable chat buffer with timestamps, role-colored usernames, full markdown formatting (code blocks with language headers, blockquotes, bold, inline code, links, mentions), and reaction pills.
    - **Message Input Bar:** Bottom input box with cursor positioning, bracketed paste support, and real-time character counter (`42/2000`).
  - **Right Member Rail (20 cols):** Guild members grouped into online/offline sections with status circles (`●` Online, `●` Idle, `●` DND, `○` Offline). Collapsible with `F4` or click.
  - **Bottom Status Bar:** Active user handle and status badge, contextual hotkey hints for the active pane, and live WebSocket ping latency (`● 18ms`).
- **Hybrid Ergonomics (Zero Shortcut Overload):**
  - **Pane Cycling:** Press `Tab` or `Shift+Tab` to cycle between `Servers` ➔ `Channels` ➔ `Chat` ➔ `Input` ➔ `Members`.
  - **Direct Focus:** `Alt+1` (Servers), `Alt+2` (Channels), `Alt+3` (Chat), `Alt+4` (Input), `Alt+5` (Members).
  - **Quick Switcher / Fuzzy Command Palette (`Ctrl+P`):** Pop up a fuzzy search dialog over the UI to jump directly to any server, channel, or direct message.
  - **Full Mouse Support:** Click any server, channel, or pane to switch/focus immediately; scroll chat buffer with the mouse wheel; click the `[ 👥 Members ]` button in the header to toggle the member rail.
- **Bonfire & Delta Protocol:**
  - Delta REST client for login (`POST /auth/session/login`), messages, and DMs.
  - Bonfire WebSocket gateway with automated handshake, 20-second heartbeats (`Ping`/`Pong`), round-trip latency tracking, and real-time event streaming (`Message`, `MessageUpdate`, `MessageDelete`, etc.).
- **Terminal Safety:**
  - Uses `std::panic::set_hook` to ensure the terminal restores raw mode, leaves alternate screen, and disables mouse capture even in the event of an unexpected panic.

---

## Keybindings & Controls

| Shortcut | Context | Action |
|---|---|---|
| `Tab` / `Shift+Tab` | Global | Cycle focus through panes sequentially |
| `Alt+1` .. `Alt+4` | Global | Jump directly to Servers, Channels, Chat, or Input |
| `Ctrl+P` | Global | Open Fuzzy Command Palette / Quick Switcher |
| `F4` | Global | Toggle Right Member Rail |
| `Ctrl+C` | Global | Clean exit |
| `Mouse Left Click` | Global | Focus pane / select server / open channel / toggle members |
| `Mouse Wheel` | Chat Feed | Scroll chat history backlog |
| `↑` / `↓` / `j` / `k` | List Panes | Move selection in Server, Channel, or Member list |
| `Enter` | List Panes | Activate selected server, channel, or search result |
| `PgUp` / `PgDn` | Chat Feed | Fast page scroll |
| `Home` / `End` | Chat Feed | Scroll to top / jump to latest message |
| `i` or `Enter` | Chat Feed | Focus input bar to start typing |
| `Enter` | Input Bar | Send message |
| `Esc` | Input Bar | Unfocus input back to chat feed |

---

## Configuration

Configuration is automatically stored in standard XDG directories:
`~/.config/stoat-tui/config.toml`

```toml
api_url = "https://api.stoat.chat"
gateway_url = "wss://ws.stoat.chat" # Optional override (auto-discovered by default)
token = "your_session_or_bot_token"
is_bot = false

[theme]
primary = "#FD6671"      # Stoat vibrant coral
accent = "#5970FD"       # Stoat accent indigo
background = "#191919"
sidebar_bg = "#202020"
rail_bg = "#151515"
text = "#F6F6F6"
muted = "#848484"
```

If launched without a token, `stoat-tui` displays a welcome modal where you can:
1. Log in via Email & Password
2. Paste a Session or Bot Token
3. Explore the fully interactive **Offline Demo Mode** with simulated Bonfire sync!

---

## Building & Testing

```bash
# Build optimized release binary
cargo build --release

# Run
./target/release/stoat-tui

# Run unit and integration tests
cargo test
```
