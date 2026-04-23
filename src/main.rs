use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "tai", about = "Terminal Agent Interface")]
struct Cli {
    #[arg(long, short)]
    debug: bool,

    #[arg(long, short)]
    task: Option<String>,

    #[arg(long)]
    max_ticks: Option<u64>,

    #[arg(long)]
    tui: bool,

    #[arg(long)]
    no_delegate: bool,

    path: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Cli {
        debug,
        task,
        max_ticks,
        tui,
        no_delegate,
        path,
    } = Cli::try_parse()?;

    dotenv::dotenv().ok();
    tai::run_server(
        path.as_deref(),
        task.as_deref(),
        debug,
        max_ticks,
        tui,
        no_delegate,
    )
    .await
}
