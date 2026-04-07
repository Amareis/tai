use crate::types::{BlockMode, ParsedSegment};

/// Парсит ответ модели на сегменты.
///
/// Формат code block: ` ```<window>:<mode>\ncontent\n``` `
/// Режимы: text, keys, cmd (или tai:cmd)
#[must_use]
pub fn parse_response(text: &str) -> Vec<ParsedSegment> {
    let mut segments = Vec::new();
    let mut current_prose = String::new();
    let mut in_block = false;
    let mut block_header: Option<String> = None;
    let mut block_content = String::new();

    for line in text.lines() {
        if line.starts_with("```") {
            if in_block {
                if let Some(header) = block_header.take()
                    && let Some(segment) = parse_block(&header, &block_content)
                {
                    segments.push(segment);
                }
                block_content.clear();
                in_block = false;
            } else {
                if !current_prose.trim().is_empty() {
                    segments.push(ParsedSegment::Prose(current_prose.trim().to_string()));
                    current_prose.clear();
                }
                let header = line.trim_start_matches('`').to_string();
                block_header = Some(header);
                in_block = true;
            }
        } else if in_block {
            block_content.push_str(line);
            block_content.push('\n');
        } else {
            current_prose.push_str(line);
            current_prose.push('\n');
        }
    }

    if !current_prose.trim().is_empty() {
        segments.push(ParsedSegment::Prose(current_prose.trim().to_string()));
    }

    segments
}

fn parse_block(header: &str, content: &str) -> Option<ParsedSegment> {
    let header = header.trim();
    if header.is_empty() {
        return None;
    }

    if header == "tai:cmd" {
        return Some(ParsedSegment::Block {
            window: "tai".to_string(),
            mode: BlockMode::Cmd,
            content: content.trim().to_string(),
        });
    }

    let parts: Vec<&str> = header.splitn(2, ':').collect();

    let [window, mode] = parts.get(..=1)? else {
        return None;
    };

    Some(ParsedSegment::Block {
        window: window.trim().to_string(),
        mode: mode.parse::<BlockMode>().ok()?,
        content: content.trim().to_string(),
    })
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
        let text = "Before\n```build:text\ncargo build\n```\nAfter";
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
    fn test_parse_tai_cmd() {
        let text = "```tai:cmd\nlaunch -- bash\n```";
        let segments = parse_response(text);
        assert_eq!(segments.len(), 1);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Block { window, mode, content }
            if window == "tai" && *mode == BlockMode::Cmd && content == "launch -- bash"
        ));
    }

    #[test]
    fn test_parse_keys_mode() {
        let text = "```vim:keys\nEscape :wq Enter\n```";
        let segments = parse_response(text);
        assert_eq!(segments.len(), 1);
        assert!(matches!(
            &segments[0],
            ParsedSegment::Block { mode, .. } if *mode == BlockMode::Keys
        ));
    }

    #[test]
    fn test_parse_multiple_blocks() {
        let text = r"```1:text
echo hello
```
```2:text
echo world
```";
        let segments = parse_response(text);
        assert_eq!(segments.len(), 2);
    }
}
