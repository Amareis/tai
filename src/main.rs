use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "tai", about = "Terminal Agent Interface")]
struct Cli {
    #[arg(long, short)]
    debug: bool,

    path: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Cli { debug, path } = Cli::try_parse()?;

    dotenv::dotenv().ok();
    tai::run_server(path.as_deref(), debug).await
}
