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

            let initial = std::fs::read_to_string("tai.md")
                .ok()
                .map(|content| tai::response::parse_response(String::new(), &content));

            tai::run_server(debug, initial).await
        }
    }
}
