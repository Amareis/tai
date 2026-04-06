use clap::{Parser, Subcommand};

use crate::backend::{
    BackendCmd, CloseCmd, GetTextCmd, LaunchCmd, SendKeysCmd, SendTextCmd, SetTitleCmd, WindowId,
};

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("failed to parse command: {0}")]
    Clap(#[from] clap::Error),
}

#[derive(Parser, Debug)]
#[command(name = "tai", no_binary_name = true)]
struct TaiCli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Launch {
        #[arg(short, long)]
        title: Option<String>,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    Send {
        window: WindowId,
        #[arg(trailing_var_arg = true)]
        text: Vec<String>,
    },
    Keys {
        window: WindowId,
        #[arg(trailing_var_arg = true)]
        keys: Vec<String>,
    },
    Get {
        window: WindowId,
    },
    Close {
        window: WindowId,
    },
    List,
    Title {
        window: WindowId,
        #[arg(trailing_var_arg = true)]
        title: Vec<String>,
    },
}

pub fn parse(input: &str) -> Result<BackendCmd, ParseError> {
    let args = shell_words::split(input).map_err(|e| {
        clap::Error::raw(
            clap::error::ErrorKind::InvalidValue,
            format!("shell parsing failed: {e}"),
        )
    })?;

    let cli = TaiCli::try_parse_from(args)?;

    Ok(match cli.command {
        Commands::Launch { title, command } => {
            if command.is_empty() {
                return Err(ParseError::Clap(clap::Error::raw(
                    clap::error::ErrorKind::MissingRequiredArgument,
                    "launch requires a command after --",
                )));
            }
            BackendCmd::Launch(LaunchCmd { title, command })
        }
        Commands::Send { window, text } => BackendCmd::SendText(SendTextCmd {
            window,
            text: text.join(" "),
        }),
        Commands::Keys { window, keys } => BackendCmd::SendKeys(SendKeysCmd {
            window,
            keys: keys.join(" "),
        }),
        Commands::Get { window } => BackendCmd::GetText(GetTextCmd { window }),
        Commands::Close { window } => BackendCmd::Close(CloseCmd { window }),
        Commands::List => BackendCmd::List,
        Commands::Title { window, title } => BackendCmd::SetTitle(SetTitleCmd {
            window,
            title: title.join(" "),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_launch_basic() {
        let cmd = parse("launch -- bash").unwrap();
        assert!(matches!(cmd, BackendCmd::Launch(l) if l.command == vec!["bash"]));
    }

    #[test]
    fn test_launch_with_title() {
        let cmd = parse("launch --title mytitle -- bash -c 'echo hello'").unwrap();
        match cmd {
            BackendCmd::Launch(l) => {
                assert_eq!(l.title, Some("mytitle".to_string()));
                assert_eq!(l.command, vec!["bash", "-c", "echo hello"]);
            }
            _ => panic!("expected Launch"),
        }
    }

    #[test]
    fn test_launch_command_with_dashes() {
        let cmd = parse("launch -- cargo build --release").unwrap();
        match cmd {
            BackendCmd::Launch(l) => {
                assert_eq!(l.command, vec!["cargo", "build", "--release"]);
            }
            _ => panic!("expected Launch"),
        }
    }

    #[test]
    fn test_send_text() {
        let cmd = parse("send 1 hello world").unwrap();
        match cmd {
            BackendCmd::SendText(s) => {
                assert_eq!(s.window, WindowId("1".to_string()));
                assert_eq!(s.text, "hello world");
            }
            _ => panic!("expected SendText"),
        }
    }

    #[test]
    fn test_send_keys() {
        let cmd = parse("keys 1 Ctrl+c Enter").unwrap();
        match cmd {
            BackendCmd::SendKeys(k) => {
                assert_eq!(k.window, WindowId("1".to_string()));
                assert_eq!(k.keys, "Ctrl+c Enter");
            }
            _ => panic!("expected SendKeys"),
        }
    }

    #[test]
    fn test_get_text() {
        let cmd = parse("get 123").unwrap();
        match cmd {
            BackendCmd::GetText(g) => {
                assert_eq!(g.window, WindowId("123".to_string()));
            }
            _ => panic!("expected GetText"),
        }
    }

    #[test]
    fn test_close() {
        let cmd = parse("close 42").unwrap();
        match cmd {
            BackendCmd::Close(c) => {
                assert_eq!(c.window, WindowId("42".to_string()));
            }
            _ => panic!("expected Close"),
        }
    }

    #[test]
    fn test_list() {
        let cmd = parse("list").unwrap();
        assert!(matches!(cmd, BackendCmd::List));
    }

    #[test]
    fn test_set_title() {
        let cmd = parse("title 1 my new title").unwrap();
        match cmd {
            BackendCmd::SetTitle(t) => {
                assert_eq!(t.window, WindowId("1".to_string()));
                assert_eq!(t.title, "my new title");
            }
            _ => panic!("expected SetTitle"),
        }
    }

    #[test]
    fn test_invalid_command() {
        let result = parse("invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_launch_missing_command() {
        let result = parse("launch --title foo");
        assert!(result.is_err());
    }
}
