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

        /// Start kitty hidden
        #[arg(long, short)]
        debug: bool,
    },

    /// Run inside Kitty — pass stdin/stdout FD to server
    Client {
        /// Unix socket path to connect to
        #[arg(long)]
        socket: String,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli {
        Cli::Server {
            socket,
            hidden,
            debug
        } => {
            dotenv::dotenv()?;
            tai::run_server(socket.map(PathBuf::from), hidden, debug).await
        }
        Cli::Client { socket } => {
            tai::run_client(PathBuf::from(socket)).await
        }
    }
}
