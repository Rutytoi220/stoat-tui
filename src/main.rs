use anyhow::Result;
use crossterm::{
    cursor::Show,
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, window_size, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::env;
use std::io::{self, stdout, IsTerminal};
use std::time::{Duration, Instant};
use stoat_tui::app::App;
use stoat_tui::config::{Config, NavCache};
use stoat_tui::event::{AppEvent, EventHandler};
use stoat_tui::ui;

const VERSION: &str = env!("CARGO_PKG_VERSION");

struct TermiosSlot(std::cell::UnsafeCell<std::mem::MaybeUninit<libc::termios>>);
unsafe impl Sync for TermiosSlot {}

static ORIG_TERMIOS: TermiosSlot =
    TermiosSlot(std::cell::UnsafeCell::new(std::mem::MaybeUninit::uninit()));
static HAS_ORIG_TERMIOS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn save_original_termios() {
    unsafe {
        let slot = &mut *ORIG_TERMIOS.0.get();
        if libc::tcgetattr(libc::STDIN_FILENO, slot.as_mut_ptr()) == 0 {
            HAS_ORIG_TERMIOS.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

extern "C" fn crash_signal_handler(sig: libc::c_int) {
    unsafe {
        if HAS_ORIG_TERMIOS.load(std::sync::atomic::Ordering::SeqCst) {
            let slot = &*ORIG_TERMIOS.0.get();
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, slot.as_ptr());
        }
        let seq = b"\x1b[?1049l\x1b[?25h\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?2004l";
        let _ = libc::write(
            libc::STDERR_FILENO,
            seq.as_ptr() as *const libc::c_void,
            seq.len(),
        );
        libc::signal(sig, libc::SIG_DFL);
        libc::raise(sig);
    }
}

fn install_crash_signal_handlers() {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = crash_signal_handler as *const () as usize;
        libc::sigemptyset(&mut sa.sa_mask);
        sa.sa_flags = libc::SA_RESETHAND;
        for sig in [libc::SIGSEGV, libc::SIGABRT, libc::SIGTERM, libc::SIGHUP] {
            libc::sigaction(sig, &sa, std::ptr::null_mut());
        }
    }
}

fn print_help() {
    let log_path = stoat_tui::logger::resolve_log_path();
    println!(
        "\
stoat-tui {version}
Fast, keyboard-first, and fully mouse-accessible Terminal User Interface client for Stoat (formerly Revolt).

USAGE:
    stoat-tui [OPTIONS]

OPTIONS:
    -h, -H, --help       Print help information, config paths, and keybindings
    -v, -V, --version    Print version information
    -a, --api <URL>      Override base API URL (e.g. https://api.revolt.chat or https://api.stoat.chat)

CONFIGURATION:
    Config directory:    {config_dir}
    Config file:         {config_path}
    Debug log:           {log_path}

KEYBINDINGS:
    Tab / Shift+Tab      Cycle focus (Servers -> Channels -> Chat -> Input -> Members)
    Alt+1..4             Direct jump to Servers (1), Channels (2), Chat (3), Input (4)
    Ctrl+P               Open quick switcher / fuzzy command palette
    F2 / Ctrl+L          Open login modal / switch account
    F4                   Toggle right member rail
    Alt+J/K              Previous / Next channel
    Ctrl+Alt+J/K         Previous / Next server
    Alt+Shift+J/K        Previous / Next unread channel
    Ctrl+C               Quit application
    Enter                Send message (in input) / select channel or server
    Esc                  Unfocus input back to chat feed
    Mouse Left Click     Select server, channel, member, or focus pane
    Mouse Wheel          Scroll chat feed backlog smoothly

For more documentation, visit: https://github.com/stoat-chat/stoat-tui",
        version = VERSION,
        config_dir = Config::config_dir().display(),
        config_path = Config::config_path().display(),
        log_path = log_path.display()
    );
}

fn print_version() {
    println!("stoat-tui {}", VERSION);
}

fn detect_picker_fast() -> (std::sync::Arc<ratatui_image::picker::Picker>, bool) {
    use ratatui_image::picker::{Picker, ProtocolType};

    // Query cell pixel dimensions via pure TIOCGWINSZ ioctl (~10us, never blocks on stdin)
    let font_size = match window_size() {
        Ok(ws) if ws.columns > 0 && ws.rows > 0 && ws.width > 0 && ws.height > 0 => {
            let cw = (ws.width / ws.columns).clamp(4, 64);
            let ch = (ws.height / ws.rows).clamp(8, 128);
            (cw, ch)
        }
        _ => (10, 20),
    };

    #[allow(deprecated)]
    let mut picker = Picker::from_fontsize(font_size.into());

    let term = env::var("TERM").unwrap_or_default().to_ascii_lowercase();
    let term_program = env::var("TERM_PROGRAM").unwrap_or_default().to_ascii_lowercase();
    let has_kitty_id = env::var("KITTY_WINDOW_ID").map(|v| !v.trim().is_empty()).unwrap_or(false);
    let has_wezterm = env::var("WEZTERM_EXECUTABLE").map(|v| !v.trim().is_empty()).unwrap_or(false);

    if has_kitty_id
        || term.contains("kitty")
        || term.contains("ghostty")
        || term_program.contains("ghostty")
    {
        picker.set_protocol_type(ProtocolType::Kitty);
        return (std::sync::Arc::new(picker), true);
    }

    if has_wezterm || term_program.contains("wezterm") || term_program.contains("iterm") {
        picker.set_protocol_type(ProtocolType::Iterm2);
        return (std::sync::Arc::new(picker), true);
    }

    // Immediate non-blocking fallback before optional background stdio check
    (std::sync::Arc::new(picker), false)
}

#[tokio::main]
async fn main() -> Result<()> {
    // 0. Setup panic hook immediately so any crash restores terminal state
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            LeaveAlternateScreen,
            DisableMouseCapture,
            DisableBracketedPaste,
            Show
        );
        stoat_tui::log_error!("Application panicked: {:?}", info);
        default_panic(info);
    }));
    save_original_termios();
    install_crash_signal_handlers();

    let launch_start = Instant::now();
    // 1. Parse command-line arguments
    let args: Vec<String> = env::args().skip(1).collect();
    let mut override_api: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "--help" | "-h" | "-H" => {
                print_help();
                return Ok(());
            }
            "--version" | "-V" | "-v" => {
                print_version();
                return Ok(());
            }
            "--api" | "-a" => {
                if i + 1 < args.len() {
                    override_api = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!("stoat-tui: error: --api requires a URL parameter\n");
                    print_help();
                    std::process::exit(1);
                }
            }
            s if s.starts_with("--api=") => {
                override_api = Some(s.trim_start_matches("--api=").to_string());
                i += 1;
            }
            s if s.starts_with("-a=") => {
                override_api = Some(s.trim_start_matches("-a=").to_string());
                i += 1;
            }
            unknown => {
                eprintln!("stoat-tui: unrecognized argument '{}'\n", unknown);
                print_help();
                std::process::exit(1);
            }
        }
    }

    // 2. Initialize file logger early
    let log_path = stoat_tui::logger::init();
    stoat_tui::log_info!("Starting stoat-tui v{} (PID: {})", VERSION, std::process::id());
    stoat_tui::log_info!("Debug log path: {}", log_path.display());

    // 3. Load configuration and lightweight navigation cache synchronously (<1ms)
    let mut config = Config::load().unwrap_or_default();
    if let Some(api) = override_api {
        config.api_url = api;
    }
    let has_token = config.token.as_deref().map(|t| !t.trim().is_empty()).unwrap_or(false);
    let nav_cache = if has_token {
        NavCache::load_from(&config.nav_cache_path())
    } else {
        None
    };

    // 4. Instant graphics protocol fast-path (zero stdio blocking)
    let (picker, env_matched_graphics) = detect_picker_fast();

    // 5. Initialize terminal in raw mode with alternate screen, mouse capture, and bracketed paste
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // 6. Initialize deferred event handler + App state hydrated from disk cache
    let mut event_handler = EventHandler::new_deferred(Duration::from_millis(30));
    let tx = event_handler.sender();
    let mut app = App::new(config, tx.clone(), picker.clone());
    if let Some(cache) = nav_cache {
        app.state.hydrate_nav_cache(cache);
    }

    // 7. Paint Frame 1 IMMEDIATELY before any network I/O or stdio queries!
    terminal.draw(|f| ui::render(f, &mut app.state))?;
    let first_frame_ms = launch_start.elapsed().as_secs_f64() * 1000.0;
    stoat_tui::log_info!(
        "First frame painted in {:.2}ms (protocol={:?}, font={}x{}, env_fast_path={})",
        first_frame_ms,
        picker.protocol_type(),
        picker.font_size().width,
        picker.font_size().height,
        env_matched_graphics
    );

    // 8. Start input stream & optional background stdio probe (after Frame 1)
    if env_matched_graphics || !io::stdin().is_terminal() {
        EventHandler::spawn_stdin_reader(tx.clone());
    } else {
        let probe_tx = tx.clone();
        tokio::task::spawn_blocking(move || {
            let mut opts = ratatui_image::picker::cap_parser::QueryStdioOptions::default();
            opts.timeout = Duration::from_millis(80);
            if let Ok(probed) = ratatui_image::picker::Picker::from_query_stdio_with_options(opts) {
                let _ = probe_tx.blocking_send(AppEvent::PickerResolved(std::sync::Arc::new(probed)));
            }
            EventHandler::spawn_stdin_reader(probe_tx);
        });
    }

    // 9. Spawn concurrent network startup tasks AFTER Frame 1 is on screen
    app.startup();

    // 10. Main event loop
    while !app.state.should_quit {
        tokio::select! {
            Some(event) = event_handler.rx.recv() => {
                app.handle_event(event).await;
            }
        }
        if !app.state.should_quit {
            terminal.draw(|f| ui::render(f, &mut app.state))?;
        }
    }

    // 7. Clean terminal restoration on exit
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste,
        Show
    )?;
    terminal.show_cursor()?;

    // Gracefully leave any active voice call and close LiveKit room before exiting
    app.shutdown().await;

    Ok(())
}
