use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "tai", about = "Terminal Agent Interface")]
enum Cli {
    Server {
        #[arg(long, short)]
        debug: bool,

        path: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli {
        Cli::Server { debug, path } => {
            dotenv::dotenv().ok();
            tai::run_server(path.as_deref(), debug).await
        }
    }
}
