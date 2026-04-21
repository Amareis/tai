use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "tai", about = "Terminal Agent Interface")]
struct Cli {
    #[arg(long, short)]
    debug: bool,

    #[arg(long, short)]
    task: Option<String>,

    path: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Cli { debug, task, path } = Cli::try_parse()?;

    dotenv::dotenv().ok();
    tai::run_server(path.as_deref(), task.as_deref(), debug).await
}
