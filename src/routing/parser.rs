use crate::backend::BackendCmd;
use clap::error::ErrorKind;
use clap::{Parser};

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("failed to parse command: {0}")]
    Clap(#[from] clap::Error),
    #[error("{0}")]
    Help(String),
}

#[derive(Parser, Debug)]
#[command(name = "tai")]
#[command(multicall = true)]
#[command(disable_help_flag = true)]
struct TaiCli {
    #[command(subcommand)]
    command: BackendCmd,
}

pub fn parse(input: &str) -> Result<BackendCmd, ParseError> {
    let args = shell_words::split(input).map_err(|e| {
        clap::Error::raw(
            ErrorKind::InvalidValue,
            format!("shell parsing failed: {e}"),
        )
    })?;

    let cli = match TaiCli::try_parse_from(args) {
        Ok(cmd) => Ok(cmd),
        Err(e) => Err(if e.kind() == ErrorKind::DisplayHelp {
            ParseError::Help(e.to_string())
        } else {
            ParseError::Clap(e)
        }),
    }?;

    Ok(cli.command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{
        CloseCmd, GetTextCmd, LaunchCmd, SendKeysCmd, SendTextCmd, SetTitleCmd, WindowId,
    };

    #[test]
    fn test_launch_basic() {
        let cmd = parse("launch -- bash").unwrap();
        match cmd {
            BackendCmd::Launch(l) => assert_eq!(l.command, vec!["bash"]),
            _ => panic!("expected Launch"),
        }
    }

    #[test]
    fn test_launch_with_title() {
        let cmd = parse("launch --title mytitle -- bash -c 'echo hello'").unwrap();
        match cmd {
            BackendCmd::Launch(LaunchCmd { title, command }) => {
                assert_eq!(title, Some("mytitle".to_string()));
                assert_eq!(command, vec!["bash", "-c", "echo hello"]);
            }
            _ => panic!("expected Launch"),
        }
    }

    #[test]
    fn test_launch_command_with_dashes() {
        let cmd = parse("launch -- cargo build --release").unwrap();
        match cmd {
            BackendCmd::Launch(LaunchCmd { command, .. }) => {
                assert_eq!(command, vec!["cargo", "build", "--release"]);
            }
            _ => panic!("expected Launch"),
        }
    }

    #[test]
    fn test_send_text() {
        let cmd = parse("send 1 hello world").unwrap();
        match cmd {
            BackendCmd::Send(SendTextCmd { window, text }) => {
                assert_eq!(window, WindowId("1".to_string()));
                assert_eq!(text, vec!["hello", "world"]);
            }
            _ => panic!("expected Send"),
        }
    }

    #[test]
    fn test_send_keys() {
        let cmd = parse("keys 1 Ctrl+c Enter").unwrap();
        match cmd {
            BackendCmd::Keys(SendKeysCmd { window, keys }) => {
                assert_eq!(window, WindowId("1".to_string()));
                assert_eq!(keys, vec!["Ctrl+c", "Enter"]);
            }
            _ => panic!("expected Keys"),
        }
    }

    #[test]
    fn test_get_text() {
        let cmd = parse("get 123").unwrap();
        match cmd {
            BackendCmd::Get(GetTextCmd { window_id: window }) => {
                assert_eq!(window, WindowId("123".to_string()));
            }
            _ => panic!("expected Get"),
        }
    }

    #[test]
    fn test_close() {
        let cmd = parse("close 42").unwrap();
        match cmd {
            BackendCmd::Close(CloseCmd { window_id: window }) => {
                assert_eq!(window, WindowId("42".to_string()));
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
            BackendCmd::Title(SetTitleCmd { window_id: window, title }) => {
                assert_eq!(window, WindowId("1".to_string()));
                assert_eq!(title, vec!["my", "new", "title"]);
            }
            _ => panic!("expected Title"),
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
        assert!(result.is_ok());
    }

    #[test]
    fn test_help_subcommand() {
        let result = parse("help");
        assert!(matches!(result, Err(ParseError::Help(_))));
        let err = result.unwrap_err().to_string();
        assert!(err.contains("help"));
    }

    #[test]
    fn test_help_with_arg() {
        let result = parse("help launch");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("launch"));
    }
}
