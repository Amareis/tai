use crate::types::{BlockMode, ParsedSegment};

/// Парсит ответ модели на сегменты.
///
/// Формат:
/// - ` ```title\ncontent\n``` ` — send text to window (auto-launch если не существует)
/// - ` ```title:close\n``` ` — close window
/// - Текст вне блоков — prose.
///
/// Парсер heredoc-aware: внутри блока отслеживает `<<'DELIM'`, `<<"DELIM"`, `<<DELIM`
/// и не закрывает блок на `` ``` ``, пока heredoc не закрыт.
#[must_use]
pub fn parse_response(text: &str) -> Vec<ParsedSegment> {
    let mut segments = Vec::new();
    let mut current_prose = String::new();
    let mut in_block = false;
    let mut block_window: Option<String> = None;
    let mut block_mode: Option<BlockMode> = None;
    let mut block_content = String::new();
    let mut heredoc_delimiter: Option<String> = None;

    for line in text.lines() {
        if in_block {
            if let Some(ref delim) = heredoc_delimiter {
                block_content.push_str(line);
                block_content.push('\n');
                if line.trim() == delim.as_str() {
                    heredoc_delimiter = None;
                }
            } else if line.starts_with("```") {
                if let Some(window) = block_window.take()
                    && !window.is_empty()
                {
                    segments.push(ParsedSegment::Block {
                        window,
                        mode: block_mode.unwrap_or(BlockMode::Text),
                        content: block_content.trim_end().to_string(),
                    });
                }
                block_content.clear();
                heredoc_delimiter = None;
                in_block = false;
            } else {
                block_content.push_str(line);
                block_content.push('\n');

                if let Some(delim) = extract_heredoc_delimiter(line) {
                    heredoc_delimiter = Some(delim);
                }
            }
        } else if line.starts_with("```") {
            if !current_prose.trim().is_empty() {
                segments.push(ParsedSegment::Prose(current_prose.trim().to_string()));
                current_prose.clear();
            }
            let header = line.trim_start_matches('`').trim();
            let (window, mode) = parse_block_header(header);
            block_window = Some(window);
            block_mode = Some(mode);
            in_block = true;
            heredoc_delimiter = None;
        } else {
            current_prose.push_str(line);
            current_prose.push('\n');
        }
    }

    if in_block {
        if let Some(window) = block_window
            && !window.is_empty()
        {
            segments.push(ParsedSegment::Block {
                window,
                mode: block_mode.unwrap_or(BlockMode::Text),
                content: block_content.trim_end().to_string(),
            });
        }
    } else if !current_prose.trim().is_empty() {
        segments.push(ParsedSegment::Prose(current_prose.trim().to_string()));
    }

    segments
}

/// Parse block header into (window, mode).
///
/// `build` → ("build", Text)
/// `build:close` → ("build", Close)
/// `build:text` → ("build", Text)
fn parse_block_header(header: &str) -> (String, BlockMode) {
    if let Some(pos) = header.rfind(':') {
        let window = header[..pos].to_string();
        let mode_str = &header[pos + 1..];
        if let Ok(mode) = mode_str.parse::<BlockMode>() {
            return (window, mode);
        }
    }
    (header.to_string(), BlockMode::Text)
}

/// Extracts heredoc delimiter from a line.
///
/// Recognizes:
/// - `<<'DELIM'` — quoted (no expansion)
/// - `<<"DELIM"` — quoted (no expansion)
/// - `<<DELIM` — unquoted
///
/// Returns the delimiter word (without quotes) if found.
fn extract_heredoc_delimiter(line: &str) -> Option<String> {
    let line = line.trim_start();

    let heredoc_pos = line.find("<<")?;
    let after = &line[heredoc_pos + 2..];

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
        assert_eq!(parse_block_header("build"), ("build".to_string(), BlockMode::Text));
        assert_eq!(parse_block_header("build:close"), ("build".to_string(), BlockMode::Close));
        assert_eq!(parse_block_header("build:text"), ("build".to_string(), BlockMode::Text));
        assert_eq!(parse_block_header("my-window:close"), ("my-window".to_string(), BlockMode::Close));
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
}