use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
#[command(name = "tai", about = "Terminal Agent Interface")]
enum Cli {
    /// Start TAI server with dual TUI (user + model viewports)
    Server {
        /// Unix socket path for FD passing (default: /tmp/tai.sock)
        #[arg(long)]
        socket: Option<String>,

        /// Start kitty hidden
        #[arg(long)]
        hidden: bool,
    },

    /// Run inside Kitty — pass stdin/stdout FD to server
    Client {
        /// Unix socket path to connect to
        #[arg(long)]
        socket: String,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli {
        Cli::Server {
            socket,
            hidden,
        } => {
            if let Err(e) = tai::run_server(socket.map(PathBuf::from), hidden).await {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
        Cli::Client { socket } => {
            if let Err(e) = tai::run_client(PathBuf::from(socket)).await {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
    }
}
