use crate::agent::AgentResponse;
use crate::types::{BlockMode, ParsedBlock};

pub mod edit_command;

#[derive(Debug)]
struct Header {
    window: String,
    mode: BlockMode,
    dashboard: bool,
}

#[derive(Debug)]
enum ParsedSegment {
    Block {
        window: String,
        mode: BlockMode,
        dashboard: bool,
        content: String,
    },
    Prose(String),
}

fn mode_prefix(mode: &BlockMode, dashboard: bool) -> String {
    let base = mode.to_string();
    if dashboard {
        format!("{base}.dashboard")
    } else {
        base
    }
}

#[must_use]
pub fn serialize_blocks(segments: &[ParsedBlock]) -> String {
    let mut text = String::new();
    for block in segments {
        if let Some(prose) = &block.prose {
            text.push_str(prose);
            text.push('\n');
        }
        let prefix = mode_prefix(&block.mode, block.dashboard);
        match block.mode {
            BlockMode::Close | BlockMode::File => {
                let _ = std::fmt::Write::write_fmt(
                    &mut text,
                    format_args!("```{}:{}\n```\n", prefix, block.window),
                );
            }
            BlockMode::Watch | BlockMode::Exec | BlockMode::Ask | BlockMode::Delegate => {
                let _ = std::fmt::Write::write_fmt(
                    &mut text,
                    format_args!("```{}:{}\n{}\n```\n", prefix, block.window, block.content),
                );
            }
            BlockMode::Write => {
                let _ = std::fmt::Write::write_fmt(
                    &mut text,
                    format_args!(
                        "```{}:{}\n<<'TAIDELIM'\n{}\nTAIDELIM\n```\n",
                        prefix, block.window, block.content
                    ),
                );
            }
            BlockMode::Edit(ref cmd_opt) => {
                let content = if let Some(cmd) = cmd_opt {
                    edit_command::serialize_edit_command(cmd)
                } else {
                    block.content.clone()
                };
                let _ = std::fmt::Write::write_fmt(
                    &mut text,
                    format_args!(
                        "```{}:{}\n<<'TAIDELIM'\n{}\nTAIDELIM\n```\n",
                        prefix, block.window, content
                    ),
                );
            }
            BlockMode::Task => {
                let _ = std::fmt::Write::write_fmt(
                    &mut text,
                    format_args!("```task\n{}\n```\n", block.content),
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
    let mut task = String::new();
    let mut complete = false;
    let mut heredoc_violations: Vec<String> = Vec::new();

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
                dashboard,
                content,
            } => {
                if matches!(mode, BlockMode::Task) {
                    if window == "complete" {
                        complete = true;
                    }
                    task = content;
                    prose_buf = None;
                    continue;
                }
                if mode.requires_heredoc() {
                    if matches!(mode, BlockMode::Edit(_)) {
                        // Edit blocks keep raw content with inner heredoc
                        if !content.contains("<<") {
                            heredoc_violations.push(format!(
                                "{mode}:{window} — content must use heredoc (<<'TAIDELIM' ... TAIDELIM)"
                            ));
                        }
                        blocks.push(ParsedBlock {
                            window,
                            mode,
                            content,
                            prose: prose_buf.take(),
                            dashboard,
                        });
                    } else if let Some(extracted) = extract_heredoc_content(&content) {
                        blocks.push(ParsedBlock {
                            window,
                            mode,
                            content: extracted,
                            prose: prose_buf.take(),
                            dashboard,
                        });
                    } else {
                        heredoc_violations.push(format!(
                            "{mode}:{window} — content must use heredoc (<<'TAIDELIM' ... TAIDELIM)"
                        ));
                        blocks.push(ParsedBlock {
                            window,
                            mode,
                            content,
                            prose: prose_buf.take(),
                            dashboard,
                        });
                    }
                } else {
                    blocks.push(ParsedBlock {
                        window,
                        mode,
                        content,
                        prose: prose_buf.take(),
                        dashboard,
                    });
                }
            }
        }
    }

    let mut edit_parse_errors: Vec<String> = Vec::new();
    for block in &mut blocks {
        if let BlockMode::Edit(ref mut cmd_opt) = block.mode && cmd_opt.is_none() {
            match edit_command::parse_edit_command(&block.content, None) {
                Ok(cmd) => *cmd_opt = Some(cmd),
                Err(e) => edit_parse_errors.push(format!("edit:{} — {e}", block.window)),
            }
        }
    }

    AgentResponse {
        reasoning: String::new(),
        segments: blocks,
        task,
        complete,
        heredoc_violations,
        edit_parse_errors,
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
            let (header_end, hdr_opt) = parse_header(input, header_start);

            let (_body_end, close_end, content) = parse_block_body(input, header_end);

            if let Some(hdr) = hdr_opt
                && (!hdr.window.is_empty() || hdr.mode == BlockMode::Task)
            {
                segments.push(ParsedSegment::Block {
                    window: hdr.window,
                    mode: hdr.mode,
                    dashboard: hdr.dashboard,
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

fn parse_header(input: &str, from: usize) -> (usize, Option<Header>) {
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

fn parse_block_header(header: &str) -> Option<Header> {
    if let Some(pos) = header.find(':') {
        let mode_str = &header[..pos];
        let window = header[pos + 1..].to_string();
        let (mode, dashboard) = parse_mode_spec(mode_str)?;
        return Some(Header {
            window,
            mode,
            dashboard,
        });
    }

    let (mode, dashboard) = parse_mode_spec(header)?;
    Some(Header {
        window: String::new(),
        mode,
        dashboard,
    })
}

fn parse_mode_spec(spec: &str) -> Option<(BlockMode, bool)> {
    if let Some(base) = spec.strip_suffix(".dashboard") {
        base.parse::<BlockMode>().ok().map(|m| (m, true))
    } else {
        spec.parse::<BlockMode>().ok().map(|m| (m, false))
    }
}

fn extract_heredoc_content(raw: &str) -> Option<String> {
    let trimmed = raw.trim_start();
    let after = trimmed.strip_prefix("<<")?;
    let after = after.trim_start();
    let (delim, after_delim) = if let Some(rest) = after.strip_prefix('\'') {
        let end = rest.find('\'')?;
        (rest[..end].to_string(), &rest[end + 1..])
    } else if let Some(rest) = after.strip_prefix('"') {
        let end = rest.find('"')?;
        (rest[..end].to_string(), &rest[end + 1..])
    } else {
        let delim: String = after
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        if delim.is_empty() {
            return None;
        }
        let delim_len = delim.len();
        (delim, &after[delim_len..])
    };

    let body = after_delim.strip_prefix('\n')?;

    let end_marker = format!("\n{delim}");
    if let Some(end_pos) = body.find(&end_marker) {
        Some(body[..end_pos].to_string())
    } else if body.trim_end().ends_with(&delim) {
        let end_pos = body.len() - delim.len();
        Some(body[..end_pos].trim_end_matches('\n').to_string())
    } else {
        Some(body.trim_end().to_string())
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
        if delim.is_empty() { None } else { Some(delim) }
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
            ParsedSegment::Block { window, mode, content, .. }
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
            ParsedSegment::Block { window, mode, content, .. }
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
    fn test_parse_no_mode_suffix_skipped() {
        let text = "```build\ncargo test\n```";
        let segments = parse_response_segments(text);
        assert!(
            segments.is_empty(),
            "blocks without mode prefix should be skipped"
        );
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
        assert!(
            parse_block_header("build").is_none(),
            "no mode prefix should return None"
        );

        let h = parse_block_header("close:build").unwrap();
        assert_eq!(h.window, "build");
        assert_eq!(h.mode, BlockMode::Close);
        assert!(!h.dashboard);

        let h = parse_block_header("watch:build").unwrap();
        assert_eq!(h.window, "build");
        assert_eq!(h.mode, BlockMode::Watch);

        let h = parse_block_header("watch.dashboard:tree").unwrap();
        assert_eq!(h.window, "tree");
        assert_eq!(h.mode, BlockMode::Watch);
        assert!(h.dashboard);

        let h = parse_block_header("file.dashboard:mind.md").unwrap();
        assert_eq!(h.window, "mind.md");
        assert_eq!(h.mode, BlockMode::File);
        assert!(h.dashboard);

        let h = parse_block_header("exec:install").unwrap();
        assert_eq!(h.window, "install");
        assert_eq!(h.mode, BlockMode::Exec);

        let h = parse_block_header("file:src/main.rs").unwrap();
        assert_eq!(h.window, "src/main.rs");
        assert_eq!(h.mode, BlockMode::File);

        let h = parse_block_header("edit:src/main.rs").unwrap();
        assert_eq!(h.window, "src/main.rs");
        assert!(matches!(h.mode, BlockMode::Edit(_)));

        let h = parse_block_header("write:src/main.rs").unwrap();
        assert_eq!(h.window, "src/main.rs");
        assert_eq!(h.mode, BlockMode::Write);

        let h = parse_block_header("task").unwrap();
        assert_eq!(h.window, "");
        assert!(matches!(h.mode, BlockMode::Task));
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
            ParsedSegment::Block { window, mode, content, .. }
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
            ParsedSegment::Block { window, mode, content, .. }
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
            ParsedSegment::Block { window, mode, content, .. }
            if window == "src/main.rs" && *mode == BlockMode::File && content.is_empty()
        ));
    }

    #[test]
    fn test_edit_block() {
        let text = "```edit:src/main.rs\nChange\nL10:old start\nL15:old end\nfn new() {}\n.\n```";
        let segments = parse_response_segments(text);
        assert_eq!(segments.len(), 1);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Block { window, mode, content, .. }
            if window == "src/main.rs" && matches!(mode, BlockMode::Edit(_)) && content.contains("Change")
        ));
    }

    #[test]
    fn test_write_block() {
        let text = "```write:src/main.rs\nfn main() {}\n```";
        let segments = parse_response_segments(text);
        assert_eq!(segments.len(), 1);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Block { window, mode, content, .. }
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
                dashboard: false,
            },
            ParsedBlock {
                window: "install".into(),
                mode: BlockMode::Exec,
                content: "cargo add serde".into(),
                prose: None,
                dashboard: false,
            },
        ];

        let text = serialize_blocks(&blocks);
        let parsed = parse_response(&text);
        assert_eq!(parsed.segments.len(), 2);
        assert_eq!(parsed.segments[0].window, "mind");
        assert_eq!(parsed.segments[0].mode, BlockMode::Watch);
        assert_eq!(parsed.segments[0].content, "cat mind.md");
        assert_eq!(parsed.segments[0].prose.as_deref(), Some("checking state"));
        assert!(!parsed.segments[0].dashboard);
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
                dashboard: false,
            },
            ParsedBlock {
                window: "src/lib.rs".into(),
                mode: BlockMode::Edit(None),
                content: "Change Start L10:old start\nEnd L15:old end\n<<'TAIDELIM'\nfn new() {}\nTAIDELIM".into(),
                prose: None,
                dashboard: false,
            },
            ParsedBlock {
                window: "config.toml".into(),
                mode: BlockMode::Write,
                content: "[build]\nrelease = true\n".into(),
                prose: None,
                dashboard: false,
            },
        ];

        let text = serialize_blocks(&blocks);
        let parsed = parse_response(&text);
        assert_eq!(parsed.segments.len(), 3);
        assert_eq!(parsed.segments[0].mode, BlockMode::File);
        assert!(matches!(parsed.segments[1].mode, BlockMode::Edit(_)));
        assert_eq!(parsed.segments[2].mode, BlockMode::Write);
    }

    #[test]
    fn test_roundtrip_dashboard() {
        let blocks = vec![
            ParsedBlock {
                window: "tree".into(),
                mode: BlockMode::Watch,
                content: "tree -l --gitignore".into(),
                prose: None,
                dashboard: true,
            },
            ParsedBlock {
                window: "mind.md".into(),
                mode: BlockMode::File,
                content: String::new(),
                prose: None,
                dashboard: true,
            },
        ];

        let text = serialize_blocks(&blocks);
        assert!(text.contains("watch.dashboard:tree"));
        assert!(text.contains("file.dashboard:mind.md"));
        let parsed = parse_response(&text);
        assert_eq!(parsed.segments.len(), 2);
        assert!(parsed.segments[0].dashboard);
        assert!(parsed.segments[1].dashboard);
    }

    #[test]
    fn test_parse_dashboard_block() {
        let text = "```watch.dashboard:tree\ntree -l\n```";
        let resp = parse_response(text);
        assert_eq!(resp.segments.len(), 1);
        assert_eq!(resp.segments[0].window, "tree");
        assert_eq!(resp.segments[0].mode, BlockMode::Watch);
        assert!(resp.segments[0].dashboard);
    }

    #[test]
    fn test_parse_file_dashboard_block() {
        let text = "```file.dashboard:mind.md\n```";
        let resp = parse_response(text);
        assert_eq!(resp.segments.len(), 1);
        assert_eq!(resp.segments[0].window, "mind.md");
        assert_eq!(resp.segments[0].mode, BlockMode::File);
        assert!(resp.segments[0].dashboard);
    }

    #[test]
    fn test_parse_task_block_with_other() {
        let text = "```task\n1. Check build\n2. Fix errors\n```\n```watch:build\ncargo build\n```";
        let resp = parse_response(text);
        assert_eq!(resp.task, "1. Check build\n2. Fix errors");
        assert_eq!(resp.segments.len(), 1);
        assert_eq!(resp.segments[0].window, "build");
        assert_eq!(resp.segments[0].mode, BlockMode::Watch);
    }

    #[test]
    fn test_parse_task_only() {
        let text = "Some reasoning\n\n```task\nWait for user input then proceed\n```";
        let resp = parse_response(text);
        assert_eq!(resp.task, "Wait for user input then proceed");
        assert!(resp.segments.is_empty());
    }

    #[test]
    fn test_parse_no_task() {
        let text = "```watch:build\ncargo build\n```";
        let resp = parse_response(text);
        assert!(resp.task.is_empty());
        assert_eq!(resp.segments.len(), 1);
    }

    #[test]
    fn test_task_not_in_segments() {
        let text = "```task\nmy plan\n```\n```watch:build\ncargo build\n```";
        let resp = parse_response(text);
        assert_eq!(resp.task, "my plan");
        assert_eq!(resp.segments.len(), 1);
        assert!(resp.segments.iter().all(|s| !matches!(s.mode, BlockMode::Task)));
    }

    #[test]
    fn test_write_with_heredoc() {
        let text = "```write:readme.md\n<<'TAIDELIM'\n# Hello\n```rust\nfn main() {}\n```\nTAIDELIM\n```";
        let resp = parse_response(text);
        assert_eq!(resp.segments.len(), 1);
        assert_eq!(resp.segments[0].window, "readme.md");
        assert_eq!(resp.segments[0].mode, BlockMode::Write);
        assert!(resp.segments[0].content.contains("# Hello"));
        assert!(resp.segments[0].content.contains("```rust"));
        assert!(resp.segments[0].content.contains("fn main() {}"));
        assert!(resp.heredoc_violations.is_empty());
    }

    #[test]
    fn test_edit_with_heredoc() {
        let text = "```edit:src/main.rs\n<<'TAIDELIM'\nChange\nL10:old line\nfn new() {}\n.\nTAIDELIM\n```";
        let resp = parse_response(text);
        assert_eq!(resp.segments.len(), 1);
        assert_eq!(resp.segments[0].window, "src/main.rs");
        assert!(matches!(resp.segments[0].mode, BlockMode::Edit(_)));
        assert!(resp.segments[0].content.contains("Change"));
        assert!(resp.heredoc_violations.is_empty());
    }

    #[test]
    fn test_write_without_heredoc_flagged() {
        let text = "```write:readme.md\n# Hello\n```";
        let resp = parse_response(text);
        assert_eq!(resp.segments.len(), 1);
        assert_eq!(resp.segments[0].content, "# Hello");
        assert_eq!(resp.heredoc_violations.len(), 1);
        assert!(resp.heredoc_violations[0].contains("write:readme.md"));
    }

    #[test]
    fn test_edit_without_heredoc_flagged() {
        let text = "```edit:src/main.rs\n10c\nREPLACED\n```";
        let resp = parse_response(text);
        assert_eq!(resp.heredoc_violations.len(), 1);
        assert!(resp.heredoc_violations[0].contains("edit:src/main.rs"));
    }

    #[test]
    fn test_serialize_write_uses_heredoc() {
        let block = ParsedBlock {
            window: "readme.md".into(),
            mode: BlockMode::Write,
            content: "# Hello\n```rust\nfn main() {}\n```".into(),
            prose: None,
            dashboard: false,
        };
        let text = serialize_blocks(&[block]);
        assert!(text.contains("<<'TAIDELIM'"));
        assert!(text.contains("\nTAIDELIM\n"));
        let parsed = parse_response(&text);
        assert!(parsed.heredoc_violations.is_empty());
        assert!(parsed.segments[0].content.contains("```rust"));
    }
    #[test]
    fn test_task_complete_sets_flag() {
        let text = "```task:complete\nTask finished\n```";
        let resp = parse_response(text);
        assert!(resp.complete);
        assert_eq!(resp.task, "Task finished");
        assert!(resp.segments.is_empty());
    }

    #[test]
    fn test_task_complete_with_blocks() {
        let text = "```watch:build\ncargo build\n```\n```task:complete\nAll done\n```";
        let resp = parse_response(text);
        assert!(resp.complete);
        assert_eq!(resp.task, "All done");
        assert_eq!(resp.segments.len(), 1);
    }

    #[test]
    fn test_task_without_complete_no_flag() {
        let text = "```task\nStill working\n```";
        let resp = parse_response(text);
        assert!(!resp.complete);
        assert_eq!(resp.task, "Still working");
    }

    #[test]
    fn test_parse_task_block() {
        let text = "```task\nRefactor auth module\n```";
        let resp = parse_response(text);
        assert_eq!(resp.segments.len(), 0);
        assert_eq!(resp.task, "Refactor auth module");
        assert!(!resp.complete);
    }

    #[test]
    fn test_parse_task_complete() {
        let text = "```task:complete\nDone refactoring.\n```";
        let resp = parse_response(text);
        assert_eq!(resp.segments.len(), 0);
        assert_eq!(resp.task, "Done refactoring.");
        assert!(resp.complete);
    }

    #[test]
    fn test_serialize_edit_uses_heredoc() {
        let block = ParsedBlock {
            window: "src/main.rs".into(),
            mode: BlockMode::Edit(None),
            content: "Change Exactly L10:old line\n<<'TAIDELIM'\nfn new() {}\nTAIDELIM".into(),
            prose: None,
            dashboard: false,
        };
        let text = serialize_blocks(&[block]);
        assert!(text.contains("<<'TAIDELIM'"));
        let parsed = parse_response(&text);
        assert!(parsed.heredoc_violations.is_empty());
    }
}
