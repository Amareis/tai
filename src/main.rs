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
            let initial: Vec<(&str, &str)> = vec![
                ("task", "cat TASK.md"),
                ("tree", "pwd && tree --gitignore"),
                ("mind", "cat mind.md"),
                ("task", "cat AGENTS.md"),
            ];
            tai::run_server(debug, &initial).await
        }
    }
}
