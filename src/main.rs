use anyhow::Result;
use crossterm::{
    cursor::Show,
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::env;
use std::io::{self, stdout};
use std::time::Duration;
use stoat_tui::app::App;
use stoat_tui::config::Config;
use stoat_tui::event::EventHandler;
use stoat_tui::ui;

const VERSION: &str = env!("CARGO_PKG_VERSION");

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

#[tokio::main]
async fn main() -> Result<()> {
    // 0. Parse command-line arguments
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

    // 1. Initialize file logger early
    let log_path = stoat_tui::logger::init();
    stoat_tui::log_info!("Starting stoat-tui v{} (PID: {})", VERSION, std::process::id());
    stoat_tui::log_info!("Debug log path: {}", log_path.display());

    // 2. Setup panic hook for terminal restoration safety
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

    // 3. Initialize terminal in raw mode with alternate screen, mouse capture, and bracketed paste
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

    // 4. Load configuration (and apply CLI overrides)
    let mut config = Config::load().unwrap_or_default();
    if let Some(api) = override_api {
        config.api_url = api;
    }
    stoat_tui::log_info!("Loaded config: API={}, HasToken={}", config.api_url, config.token.as_ref().map(|t| !t.trim().is_empty()).unwrap_or(false));

    // 5. Initialize event stream (30ms tick rate for responsive 30+ FPS UI updates)
    let mut event_handler = EventHandler::new(Duration::from_millis(30));

    // 6. Initialize application & run startup authentication verification
    let mut app = App::new(config, event_handler.sender());
    app.startup();

    // 7. Main event loop
    while !app.state.should_quit {
        terminal.draw(|f| ui::render(f, &mut app.state))?;

        tokio::select! {
            Some(event) = event_handler.rx.recv() => {
                app.handle_event(event).await;
            }
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

    Ok(())
}
