use std::path::PathBuf;

use clap::Parser;
use tai::backend::kitty::KittyBackend;
use tai::backend::TerminalBackend;
use tai::types::LaunchOpts;

#[derive(Parser)]
#[command(name = "tai", about = "Terminal Agent Interface")]
enum Cli {
    /// Launch TAI with a managed Kitty instance
    Run {
        /// Socket path for kitty (default: temp file)
        #[arg(long)]
        socket: Option<String>,
        /// Start kitty hidden
        #[arg(long)]
        hidden: bool,
    },
    /// Connect to an already running Kitty
    Connect {
        /// Socket path of running kitty
        socket: String,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let backend = match cli {
        Cli::Run { socket, hidden } => {
            let socket_path = socket.map(PathBuf::from);
            KittyBackend::spawn(socket_path, hidden).await
        }
        Cli::Connect { socket } => KittyBackend::connect(PathBuf::from(socket)).await,
    };

    let backend = match backend {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };

    eprintln!("connected to kitty via {}", backend.socket_path().display());

    let windows = match backend.list_windows().await {
        Ok(w) => w,
        Err(e) => {
            eprintln!("error listing windows: {e}");
            std::process::exit(1);
        }
    };

    eprintln!("existing windows: {}", windows.len());
    for w in &windows {
        eprintln!("  [{}] {} (pid={}, at_prompt={})", w.id, w.title, w.pid, w.is_at_prompt);
    }

    let opts = LaunchOpts::new(
        "test-window".to_string(),
        vec!["bash".to_string(), "-c".to_string(), "echo hello && sleep 1 && echo done".to_string()],
    );
    match backend.launch(&opts).await {
        Ok(window_id) => eprintln!("launched window: {window_id}"),
        Err(e) => {
            eprintln!("error launching window: {e}");
            std::process::exit(1);
        }
    }

    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    let windows = backend.list_windows().await.unwrap_or_default();
    for w in &windows {
        eprintln!("  [{}] {} (at_prompt={})", w.id, w.title, w.is_at_prompt);
        if w.title == "test-window" {
            match backend.get_text(&w.id).await {
                Ok(text) => eprintln!("    text: {:?}", text),
                Err(e) => eprintln!("    get_text error: {e}"),
            }
        }
    }

    eprintln!("done. kitty will be killed on exit.");
}
