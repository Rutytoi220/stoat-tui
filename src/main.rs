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

KEYBINDINGS:
    Tab / Shift+Tab      Cycle focus (Servers -> Channels -> Chat -> Input -> Members)
    Alt+1..4             Direct jump to Servers (1), Channels (2), Chat (3), Input (4)
    Ctrl+P               Open quick switcher / fuzzy command palette
    F4                   Toggle right member rail
    Ctrl+C               Quit application
    Enter                Send message (in input) / select channel or server
    Esc                  Unfocus input back to chat feed
    Mouse Left Click     Select server, channel, member, or focus pane
    Mouse Wheel          Scroll chat feed backlog smoothly

For more documentation, visit: https://github.com/stoat-chat/stoat-tui",
        version = VERSION,
        config_dir = Config::config_dir().display(),
        config_path = Config::config_path().display()
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

    // 1. Setup panic hook for terminal restoration safety
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
        default_panic(info);
    }));

    // 2. Initialize terminal in raw mode with alternate screen, mouse capture, and bracketed paste
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

    // 3. Load configuration (and apply CLI overrides)
    let mut config = Config::load().unwrap_or_default();
    if let Some(api) = override_api {
        config.api_url = api;
    }
    let token_clone = config.token.clone();
    let gw_url_clone = config.gateway_url.clone();

    // 4. Initialize event stream (30ms tick rate for responsive 30+ FPS UI updates)
    let mut event_handler = EventHandler::new(Duration::from_millis(30));

    // 5. Initialize application
    let mut app = App::new(config, event_handler.sender());

    if let Some(token) = token_clone {
        app.start_gateway(token, gw_url_clone);
    }

    // 6. Main event loop
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
