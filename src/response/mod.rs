use crate::agent::AgentResponse;
use crate::types::{BlockMode, ParsedBlock};

pub mod edit_command;

struct Header {
    window: String,
    mode: BlockMode,
}

enum ParsedSegment {
    Block {
        window: String,
        mode: BlockMode,
        content: String,
    },
    Prose(String),
}

#[must_use]
pub fn serialize_blocks(segments: &[ParsedBlock]) -> String {
    let mut text = String::new();
    for block in segments {
        if let Some(prose) = &block.prose {
            text.push_str(prose);
            text.push('\n');
        }
        match block.mode {
            BlockMode::Close => {
                let _ = std::fmt::Write::write_fmt(
                    &mut text,
                    format_args!("```close:{}\n```\n", block.window),
                );
            }
            BlockMode::Watch => {
                let _ = std::fmt::Write::write_fmt(
                    &mut text,
                    format_args!("```watch:{}\n{}\n```\n", block.window, block.content),
                );
            }
            BlockMode::Exec => {
                let _ = std::fmt::Write::write_fmt(
                    &mut text,
                    format_args!("```exec:{}\n{}\n```\n", block.window, block.content),
                );
            }
            BlockMode::Ask => {
                let _ = std::fmt::Write::write_fmt(
                    &mut text,
                    format_args!("```ask:{}\n{}\n```\n", block.window, block.content),
                );
            }
            BlockMode::File => {
                let _ = std::fmt::Write::write_fmt(
                    &mut text,
                    format_args!("```file:{}\n```\n", block.window),
                );
            }
            BlockMode::Edit => {
                let _ = std::fmt::Write::write_fmt(
                    &mut text,
                    format_args!("```edit:{}\n{}\n```\n", block.window, block.content),
                );
            }
            BlockMode::Write => {
                let _ = std::fmt::Write::write_fmt(
                    &mut text,
                    format_args!("```write:{}\n{}\n```\n", block.window, block.content),
                );
            }
        }
    }
    text
}

#[must_use]
pub fn parse_response(input: &str) -> AgentResponse {
    let raw = parse_response_segments(input);

    let mut blocks: Vec<ParsedBlock> = Vec::new();
    let mut prose_buf: Option<String> = None;

    for segment in raw {
        match segment {
            ParsedSegment::Prose(text) => {
                prose_buf = Some(match prose_buf {
                    Some(prev) => format!("{prev}\n{text}"),
                    None => text,
                });
            }
            ParsedSegment::Block {
                window,
                mode,
                content,
            } => {
                blocks.push(ParsedBlock {
                    window,
                    mode,
                    content,
                    prose: prose_buf.take(),
                });
            }
        }
    }

    let outro = prose_buf.filter(|s| !s.is_empty());

    AgentResponse {
        reasoning: String::new(),
        segments: blocks,
        outro,
    }
}

#[must_use]
fn parse_response_segments(input: &str) -> Vec<ParsedSegment> {
    let mut segments = Vec::new();
    let mut pos = 0usize;

    while pos < input.len() {
        if let Some(block_start) = find_next_block(input, pos) {
            if block_start > pos {
                let prose = input[pos..block_start].trim();
                if !prose.is_empty() {
                    segments.push(ParsedSegment::Prose(prose.to_string()));
                }
            }
            let header_start = block_start + 3;
            let (header_end, hdr) = parse_header(input, header_start);

            let (_body_end, close_end, content) = parse_block_body(input, header_end);

            if !hdr.window.is_empty() {
                segments.push(ParsedSegment::Block {
                    window: hdr.window,
                    mode: hdr.mode,
                    content: content.trim_end().to_string(),
                });
            }

            pos = close_end;
        } else {
            let prose = input[pos..].trim();
            if !prose.is_empty() {
                segments.push(ParsedSegment::Prose(prose.to_string()));
            }
            break;
        }
    }

    segments
}

fn find_next_block(input: &str, from: usize) -> Option<usize> {
    input[from..].find("```").map(|p| from + p)
}

fn parse_header(input: &str, from: usize) -> (usize, Header) {
    let newline_pos = input[from..].find('\n').map_or(input.len(), |p| from + p);
    let header_raw = input[from..newline_pos].trim();
    let header = parse_block_header(header_raw);
    let end = if newline_pos < input.len() {
        newline_pos + 1
    } else {
        input.len()
    };
    (end, header)
}

fn parse_block_body(input: &str, from: usize) -> (usize, usize, String) {
    let mut pos = from;
    let mut content = String::new();
    let mut heredoc_delim: Option<String> = None;

    while pos < input.len() {
        if let Some(ref delim) = heredoc_delim {
            let line_end = input[pos..].find('\n').map_or(input.len(), |p| pos + p);
            let line = &input[pos..line_end.min(input.len())];
            if line.trim() == delim.as_str() {
                heredoc_delim = None;
                pos = input[pos..].find('\n').map_or(input.len(), |p| pos + p + 1);
                continue;
            }
            content.push_str(line);
            content.push('\n');
            pos = input[pos..].find('\n').map_or(input.len(), |p| pos + p + 1);
            continue;
        }

        if let Some(close_offset) = input[pos..].find("```") {
            let close_pos = pos + close_offset;
            let before_close = &input[pos..close_pos];

            if let Some(delim) = extract_heredoc_delimiter(before_close) {
                let line_end = input[pos..].find('\n').map_or(input.len(), |p| pos + p);
                content.push_str(&input[pos..line_end]);
                content.push('\n');
                heredoc_delim = Some(delim);
                pos = input[pos..].find('\n').map_or(input.len(), |p| pos + p + 1);
                continue;
            }

            content.push_str(before_close);
            let close_end = close_pos + 3;
            let after_close =
                if close_end < input.len() && input.get(close_end..close_end + 1) == Some("\n") {
                    close_end + 1
                } else {
                    close_end
                };
            return (pos, after_close, content);
        }

        content.push_str(&input[pos..]);
        return (from, input.len(), content);
    }

    (from, input.len(), content)
}

fn parse_block_header(header: &str) -> Header {
    if let Some(pos) = header.find(':') {
        let mode_str = &header[..pos];
        let window = header[pos + 1..].to_string();
        if let Ok(mode) = mode_str.parse::<BlockMode>() {
            return Header { window, mode };
        }
    }

    Header {
        window: header.to_string(),
        mode: BlockMode::Watch,
    }
}

fn extract_heredoc_delimiter(content: &str) -> Option<String> {
    let pos = content.find("<<")?;
    let after = content[pos + 2..].trim_start();

    if let Some(rest) = after.strip_prefix('\'') {
        let end = rest.find('\'')?;
        Some(rest[..end].to_string())
    } else if let Some(rest) = after.strip_prefix('"') {
        let end = rest.find('"')?;
        Some(rest[..end].to_string())
    } else {
        let delim: String = after
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        if delim.is_empty() {
            None
        } else {
            Some(delim)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_prose_only() {
        let text = "Hello, this is prose.\nNo blocks here.";
        let segments = parse_response_segments(text);
        assert_eq!(segments.len(), 1);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Prose(s) if s.contains("Hello")
        ));
    }

    #[test]
    fn test_parse_single_watch_block() {
        let text = "Before\n```watch:build\ncargo build\n```\nAfter";
        let segments = parse_response_segments(text);
        assert_eq!(segments.len(), 3);
        assert!(matches!(&segments[0], ParsedSegment::Prose(_)));
        assert!(matches!(
            &segments[1],
            ParsedSegment::Block { window, mode, content }
            if window == "build" && *mode == BlockMode::Watch && content == "cargo build"
        ));
        assert!(matches!(&segments[2], ParsedSegment::Prose(_)));
    }

    #[test]
    fn test_parse_close_mode() {
        let text = "```close:build\n```";
        let segments = parse_response_segments(text);
        assert_eq!(segments.len(), 1);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Block { window, mode, content }
            if window == "build" && *mode == BlockMode::Close && content.is_empty()
        ));
    }

    #[test]
    fn test_parse_multiple_blocks() {
        let text = "```watch:1\necho hello\n```\n```watch:2\necho world\n```";
        let segments = parse_response_segments(text);
        assert_eq!(segments.len(), 2);
    }

    #[test]
    fn test_parse_heredoc_inside_block() {
        let text = "```watch:build\ncat > config.yaml << 'EOF'\nserver:\n  port: 8080\n  note: \"``` not a closer\"\nEOF\n```";
        let segments = parse_response_segments(text);
        assert_eq!(segments.len(), 1);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Block { window, content, .. }
            if window == "build" && content.contains("``` not a closer")
        ));
    }

    #[test]
    fn test_parse_unquoted_heredoc() {
        let text = "```watch:build\ncat > file << DELIM\ncontent with ``` inside\nDELIM\n```";
        let segments = parse_response_segments(text);
        assert_eq!(segments.len(), 1);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Block { content, .. }
            if content.contains("content with ``` inside")
        ));
    }

    #[test]
    fn test_parse_no_mode_suffix() {
        let text = "```build\ncargo test\n```";
        let segments = parse_response_segments(text);
        assert_eq!(segments.len(), 1);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Block { window, mode, content }
            if window == "build" && *mode == BlockMode::Watch && content == "cargo test"
        ));
    }

    #[test]
    fn test_parse_unclosed_block() {
        let text = "```watch:build\ncargo build";
        let segments = parse_response_segments(text);
        assert_eq!(segments.len(), 1);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Block { window, content, .. }
            if window == "build" && content == "cargo build"
        ));
    }

    #[test]
    fn test_parse_header_prefix_format() {
        let h = parse_block_header("build");
        assert_eq!(h.window, "build");
        assert_eq!(h.mode, BlockMode::Watch);

        let h = parse_block_header("close:build");
        assert_eq!(h.window, "build");
        assert_eq!(h.mode, BlockMode::Close);

        let h = parse_block_header("watch:build");
        assert_eq!(h.window, "build");
        assert_eq!(h.mode, BlockMode::Watch);

        let h = parse_block_header("exec:install");
        assert_eq!(h.window, "install");
        assert_eq!(h.mode, BlockMode::Exec);

        let h = parse_block_header("file:src/main.rs");
        assert_eq!(h.window, "src/main.rs");
        assert_eq!(h.mode, BlockMode::File);

        let h = parse_block_header("edit:src/main.rs");
        assert_eq!(h.window, "src/main.rs");
        assert_eq!(h.mode, BlockMode::Edit);

        let h = parse_block_header("write:src/main.rs");
        assert_eq!(h.window, "src/main.rs");
        assert_eq!(h.mode, BlockMode::Write);
    }

    #[test]
    fn test_extract_heredoc_delimiter() {
        assert_eq!(
            extract_heredoc_delimiter("  <<'EOF'"),
            Some("EOF".to_string())
        );
        assert_eq!(
            extract_heredoc_delimiter("  <<\"MYDELIM\""),
            Some("MYDELIM".to_string())
        );
        assert_eq!(
            extract_heredoc_delimiter("  <<DELIM"),
            Some("DELIM".to_string())
        );
        assert_eq!(extract_heredoc_delimiter("no heredoc here"), None);
    }

    #[test]
    fn test_inline_block_open() {
        let text = "Here we go:```watch:build\ncargo build\n```";
        let segments = parse_response_segments(text);
        assert_eq!(segments.len(), 2);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Prose(s) if s == "Here we go:"
        ));
    }

    #[test]
    fn test_inline_block_close() {
        let text = "```watch:build\necho done```";
        let segments = parse_response_segments(text);
        assert!(!segments.is_empty());
        assert!(matches!(
            &segments[0],
            ParsedSegment::Block { window, content, .. }
            if window == "build" && content.contains("echo done")
        ));
    }

    #[test]
    fn test_consecutive_blocks() {
        let text = "```watch:build\ncargo build\n```\n```watch:test\ncargo test\n```";
        let segments = parse_response_segments(text);
        assert_eq!(segments.len(), 2);
    }

    #[test]
    fn test_empty_prose_between_blocks_ignored() {
        let text = "```watch:build\ncargo build\n```\n\n\n```watch:test\ncargo test\n```";
        let segments = parse_response_segments(text);
        assert_eq!(segments.len(), 2);
    }

    #[test]
    fn test_exec_block_basic() {
        let text = "```exec:install\ncargo add serde\n```";
        let segments = parse_response_segments(text);
        assert_eq!(segments.len(), 1);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Block { window, mode, content }
            if window == "install" && *mode == BlockMode::Exec && content == "cargo add serde"
        ));
    }

    #[test]
    fn test_exec_block_with_heredoc_inside() {
        let text =
            "```exec:write-config\ncat > config.toml << 'EOF'\n[build]\nrelease = true\nEOF\n```";
        let segments = parse_response_segments(text);
        assert_eq!(segments.len(), 1);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Block { window, mode, content }
            if window == "write-config" && *mode == BlockMode::Exec && content.contains("[build]")
        ));
    }

    #[test]
    fn test_file_block() {
        let text = "```file:src/main.rs\n```";
        let segments = parse_response_segments(text);
        assert_eq!(segments.len(), 1);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Block { window, mode, content }
            if window == "src/main.rs" && *mode == BlockMode::File && content.is_empty()
        ));
    }

    #[test]
    fn test_edit_block() {
        let text = "```edit:src/main.rs\n10,15c\nfn new() {}\n```";
        let segments = parse_response_segments(text);
        assert_eq!(segments.len(), 1);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Block { window, mode, content }
            if window == "src/main.rs" && *mode == BlockMode::Edit && content.contains("10,15c")
        ));
    }

    #[test]
    fn test_write_block() {
        let text = "```write:src/main.rs\nfn main() {}\n```";
        let segments = parse_response_segments(text);
        assert_eq!(segments.len(), 1);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Block { window, mode, content }
            if window == "src/main.rs" && *mode == BlockMode::Write && content == "fn main() {}"
        ));
    }

    #[test]
    fn test_roundtrip_serialize_parse() {
        let blocks = vec![
            ParsedBlock {
                window: "mind".into(),
                mode: BlockMode::Watch,
                content: "cat mind.md".into(),
                prose: Some("checking state".into()),
            },
            ParsedBlock {
                window: "install".into(),
                mode: BlockMode::Exec,
                content: "cargo add serde".into(),
                prose: None,
            },
        ];

        let text = serialize_blocks(&blocks);
        let parsed = parse_response(&text);
        assert_eq!(parsed.segments.len(), 2);
        assert_eq!(parsed.segments[0].window, "mind");
        assert_eq!(parsed.segments[0].mode, BlockMode::Watch);
        assert_eq!(parsed.segments[0].content, "cat mind.md");
        assert_eq!(parsed.segments[0].prose.as_deref(), Some("checking state"));
        assert_eq!(parsed.segments[1].window, "install");
        assert_eq!(parsed.segments[1].mode, BlockMode::Exec);
    }

    #[test]
    fn test_roundtrip_file_edit_write() {
        let blocks = vec![
            ParsedBlock {
                window: "src/main.rs".into(),
                mode: BlockMode::File,
                content: String::new(),
                prose: None,
            },
            ParsedBlock {
                window: "src/lib.rs".into(),
                mode: BlockMode::Edit,
                content: "10,15c\nfn new() {}\n".into(),
                prose: None,
            },
            ParsedBlock {
                window: "config.toml".into(),
                mode: BlockMode::Write,
                content: "[build]\nrelease = true\n".into(),
                prose: None,
            },
        ];

        let text = serialize_blocks(&blocks);
        let parsed = parse_response(&text);
        assert_eq!(parsed.segments.len(), 3);
        assert_eq!(parsed.segments[0].mode, BlockMode::File);
        assert_eq!(parsed.segments[1].mode, BlockMode::Edit);
        assert_eq!(parsed.segments[2].mode, BlockMode::Write);
    }
}
