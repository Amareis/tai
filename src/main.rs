use clap::Parser;

#[derive(Parser)]
#[command(name = "tai", about = "Terminal Agent Interface")]
enum Cli {
    Server {
        #[arg(long, short)]
        debug: bool,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli {
        Cli::Server { debug } => {
            dotenv::dotenv().ok();
            tai::run_server(debug).await
        }
    }
}
