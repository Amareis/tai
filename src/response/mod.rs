use crate::types::{BlockMode, ParsedSegment};

/// Parsed block header result.
struct Header {
    window: String,
    mode: BlockMode,
    /// Heredoc delimiter for :write blocks (from `:write<<DELIM` syntax)
    heredoc_delim: Option<String>,
}

#[must_use]
pub fn parse_response(input: &str) -> Vec<ParsedSegment> {
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

            let (_body_end, close_end, content) =
                parse_block_body(input, header_end, hdr.heredoc_delim.as_deref());

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

fn parse_block_body(
    input: &str,
    from: usize,
    initial_heredoc: Option<&str>,
) -> (usize, usize, String) {
    let mut pos = from;
    let mut content = String::new();
    let mut heredoc_delim: Option<String> = initial_heredoc.map(String::from);

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
    // First extract heredoc delimiter for :write<<DELIM
    let (header_without_delim, heredoc_delim) = if let Some(arrow_pos) = header.find("<<") {
        let before = &header[..arrow_pos];
        let delim_raw = &header[arrow_pos + 2..];
        // Validate that before<< actually makes sense (must have :write)
        if before.ends_with(":write") {
            let delim = delim_raw.trim().to_string();
            if delim.is_empty() {
                (header.to_string(), None)
            } else {
                (before.to_string(), Some(delim))
            }
        } else {
            (header.to_string(), None)
        }
    } else {
        (header.to_string(), None)
    };

    // Then parse window:mode from the remaining header
    if let Some(pos) = header_without_delim.rfind(':') {
        let window = header_without_delim[..pos].to_string();
        let mode_str = &header_without_delim[pos + 1..];
        if let Ok(mode) = mode_str.parse::<BlockMode>() {
            return Header {
                window,
                mode,
                heredoc_delim,
            };
        }
    }

    Header {
        window: header.to_string(),
        mode: BlockMode::Text,
        heredoc_delim: None,
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
        let segments = parse_response(text);
        assert_eq!(segments.len(), 1);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Prose(s) if s.contains("Hello")
        ));
    }

    #[test]
    fn test_parse_single_block() {
        let text = "Before\n```build\ncargo build\n```\nAfter";
        let segments = parse_response(text);
        assert_eq!(segments.len(), 3);
        assert!(matches!(&segments[0], ParsedSegment::Prose(_)));
        assert!(matches!(
            &segments[1],
            ParsedSegment::Block { window, mode, content }
            if window == "build" && *mode == BlockMode::Text && content == "cargo build"
        ));
        assert!(matches!(&segments[2], ParsedSegment::Prose(_)));
    }

    #[test]
    fn test_parse_close_mode() {
        let text = "```build:close\n```";
        let segments = parse_response(text);
        assert_eq!(segments.len(), 1);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Block { window, mode, content }
            if window == "build" && *mode == BlockMode::Close && content.is_empty()
        ));
    }

    #[test]
    fn test_parse_multiple_blocks() {
        let text = "```1\necho hello\n```\n```2\necho world\n```";
        let segments = parse_response(text);
        assert_eq!(segments.len(), 2);
    }

    #[test]
    fn test_parse_heredoc_inside_block() {
        let text = "```build\ncat > config.yaml << 'EOF'\nserver:\n  port: 8080\n  note: \"``` not a closer\"\nEOF\n```";
        let segments = parse_response(text);
        assert_eq!(segments.len(), 1);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Block { window, content, .. }
            if window == "build" && content.contains("``` not a closer")
        ));
    }

    #[test]
    fn test_parse_unquoted_heredoc() {
        let text = "```build\ncat > file << DELIM\ncontent with ``` inside\nDELIM\n```";
        let segments = parse_response(text);
        assert_eq!(segments.len(), 1);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Block { content, .. }
            if content.contains("content with ``` inside")
        ));
    }

    #[test]
    fn test_parse_double_quoted_heredoc() {
        let text = "```build\ncat > file <<\"DELIM\"\ncontent\nDELIM\n```";
        let segments = parse_response(text);
        assert_eq!(segments.len(), 1);
    }

    #[test]
    fn test_parse_no_mode_suffix() {
        let text = "```build\ncargo test\n```";
        let segments = parse_response(text);
        assert_eq!(segments.len(), 1);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Block { window, mode, content }
            if window == "build" && *mode == BlockMode::Text && content == "cargo test"
        ));
    }

    #[test]
    fn test_parse_unclosed_block() {
        let text = "```build\ncargo build";
        let segments = parse_response(text);
        assert_eq!(segments.len(), 1);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Block { window, content, .. }
            if window == "build" && content == "cargo build"
        ));
    }

    #[test]
    fn test_parse_header_with_mode() {
        let h = parse_block_header("build");
        assert_eq!(h.window, "build");
        assert_eq!(h.mode, BlockMode::Text);
        assert!(h.heredoc_delim.is_none());

        let h = parse_block_header("build:close");
        assert_eq!(h.window, "build");
        assert_eq!(h.mode, BlockMode::Close);

        let h = parse_block_header("build:text");
        assert_eq!(h.window, "build");
        assert_eq!(h.mode, BlockMode::Text);

        let h = parse_block_header("my-window:close");
        assert_eq!(h.window, "my-window");
        assert_eq!(h.mode, BlockMode::Close);
    }

    #[test]
    fn test_parse_header_write_with_delim() {
        let h = parse_block_header("RESULT.md:write<<EOF");
        assert_eq!(h.window, "RESULT.md");
        assert_eq!(h.mode, BlockMode::Write);
        assert_eq!(h.heredoc_delim.as_deref(), Some("EOF"));

        let h = parse_block_header("src/lib.rs:write<<END");
        assert_eq!(h.window, "src/lib.rs");
        assert_eq!(h.mode, BlockMode::Write);
        assert_eq!(h.heredoc_delim.as_deref(), Some("END"));
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
        assert_eq!(
            extract_heredoc_delimiter("  cat <<'INNEREOF'"),
            Some("INNEREOF".to_string())
        );
        assert_eq!(extract_heredoc_delimiter("no heredoc here"), None);
    }

    #[test]
    fn test_inline_block_open() {
        let text = "Here we go:```build\ncargo build\n```";
        let segments = parse_response(text);
        assert_eq!(segments.len(), 2);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Prose(s) if s == "Here we go:"
        ));
        assert!(matches!(
            &segments[1],
            ParsedSegment::Block { window, mode, content }
            if window == "build" && *mode == BlockMode::Text && content == "cargo build"
        ));
    }

    #[test]
    fn test_inline_block_with_space() {
        let text = "Some text ```build\ncargo build\n```";
        let segments = parse_response(text);
        assert_eq!(segments.len(), 2);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Prose(s) if s == "Some text"
        ));
        assert!(matches!(
            &segments[1],
            ParsedSegment::Block { window, .. }
            if window == "build"
        ));
    }

    #[test]
    fn test_inline_block_close() {
        let text = "```build\necho done```";
        let segments = parse_response(text);
        assert!(segments.len() >= 1);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Block { window, content, .. }
            if window == "build" && content.contains("echo done")
        ));
    }

    #[test]
    fn test_inline_close_with_prose_after() {
        let text = "```build\necho hi```\nSome prose";
        let segments = parse_response(text);
        assert!(segments.len() >= 1);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Block { window, content, .. }
            if window == "build" && content.contains("echo hi")
        ));
    }

    #[test]
    fn test_heredoc_with_space_after_redirect() {
        let text = "```build\ncat > config.yaml << 'EOF'\nserver:\n  port: 8080\nEOF\n```";
        let segments = parse_response(text);
        assert_eq!(segments.len(), 1);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Block { window, content, .. }
            if window == "build" && content.contains("server:")
        ));
    }

    #[test]
    fn test_consecutive_blocks() {
        let text = "```build\ncargo build\n```\n```test\ncargo test\n```";
        let segments = parse_response(text);
        assert_eq!(segments.len(), 2);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Block { window, .. } if window == "build"
        ));
        assert!(matches!(
            &segments[1],
            ParsedSegment::Block { window, .. } if window == "test"
        ));
    }

    #[test]
    fn test_empty_prose_between_blocks_ignored() {
        let text = "```build\ncargo build\n```\n\n\n```test\ncargo test\n```";
        let segments = parse_response(text);
        assert_eq!(segments.len(), 2);
    }

    #[test]
    fn test_model_munged_format() {
        let text = r"Let me examine the key source files.```shell
cat src/lib.rs
``````shell
cat src/main.rs
```";
        let segments = parse_response(text);
        assert_eq!(segments.len(), 3);
        assert!(matches!(&segments[0], ParsedSegment::Prose(_)));
    }

    #[test]
    fn test_write_block_basic() {
        let text = "```RESULT.md:write<<EOF\nhello world\nEOF\n```";
        let segments = parse_response(text);
        assert_eq!(segments.len(), 1);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Block { window, mode, content }
            if window == "RESULT.md" && *mode == BlockMode::Write && content == "hello world"
        ));
    }

    #[test]
    fn test_write_block_multiline() {
        let text = "```config.toml:write<<DELIM\n[build]\nrelease = true\nDELIM\n```";
        let segments = parse_response(text);
        assert_eq!(segments.len(), 1);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Block { window, mode, content }
            if window == "config.toml" && *mode == BlockMode::Write && content.contains("[build]")
        ));
    }

    #[test]
    fn test_write_block_with_backticks_inside() {
        let text = "```RESULT.md:write<<EOF\nSome ``` backticks inside\nand more\nEOF\n```";
        let segments = parse_response(text);
        assert_eq!(segments.len(), 1);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Block { window, mode, content }
            if window == "RESULT.md" && *mode == BlockMode::Write && content.contains("``` backticks")
        ));
    }

    #[test]
    fn test_write_block_then_text_block() {
        let text =
            "```config.yaml:write<<END\nkey: value\nEND\n```\n```build\ncat config.yaml\n```";
        let segments = parse_response(text);
        assert_eq!(segments.len(), 2);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Block { mode, .. } if *mode == BlockMode::Write
        ));
        assert!(matches!(
            &segments[1],
            ParsedSegment::Block { mode, .. } if *mode == BlockMode::Text
        ));
    }
}
