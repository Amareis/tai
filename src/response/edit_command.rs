use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditCommand {
    pub search: String,
    pub replace: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EditError {
    Parse {
        line: usize,
        message: String,
    },
    OverlappingRanges {
        cmd_a: String,
        cmd_b: String,
    },
    SearchNotFound {
        search: String,
    },
    AmbiguousSearch {
        search: String,
        count: usize,
    },
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse { line, message } => write!(f, "parse error at line {line}: {message}"),
            Self::OverlappingRanges { cmd_a, cmd_b } => {
                write!(f, "overlapping ranges: {cmd_a} vs {cmd_b}")
            }
            Self::SearchNotFound { search } => {
                write!(f, "search text not found: {search:?}")
            }
            Self::AmbiguousSearch { search, count } => {
                write!(
                    f,
                    "search text found {count} times (must be unique): {search:?}"
                )
            }
        }
    }
}

impl std::error::Error for EditError {}

/// Parse edit command with two heredocs: first = search, second = replace.
/// Delimiter names are not significant — only order matters.
pub fn parse_edit_command(content: &str) -> Result<EditCommand, EditError> {
    let heredocs = extract_heredocs(content)?;
    if heredocs.len() < 2 {
        return Err(EditError::Parse {
            line: 1,
            message:
                "edit block must contain two heredocs: first for search, second for replace"
                    .to_string(),
        });
    }
    if heredocs.len() > 2 {
        return Err(EditError::Parse {
            line: 1,
            message: format!(
                "edit block contains {} heredocs, expected exactly 2",
                heredocs.len()
            ),
        });
    }
    Ok(EditCommand {
        search: heredocs.first().cloned().ok_or_else(|| EditError::Parse {
            line: 1,
            message: "edit block missing search heredoc".to_string(),
        })?,
        replace: heredocs.get(1).cloned().ok_or_else(|| EditError::Parse {
            line: 1,
            message: "edit block missing replace heredoc".to_string(),
        })?,
    })
}

fn extract_heredocs(content: &str) -> Result<Vec<String>, EditError> {
    let lines: Vec<&str> = content.lines().collect();
    let mut heredocs = Vec::new();
    let mut i = 0;

    while let Some(line) = lines.get(i) {
        if let Some(delim) = extract_heredoc_delimiter(line) {
            i += 1;
            let start = i;
            while lines.get(i).is_some_and(|l| l.trim() != delim) {
                i += 1;
            }
            if lines.get(i).is_none() {
                return Err(EditError::Parse {
                    line: start,
                    message: format!("unclosed heredoc: {delim}"),
                });
            }
            let body = lines.get(start..i).unwrap_or(&[]).join("\n");
            heredocs.push(body);
            i += 1; // skip closing delimiter
        } else {
            i += 1;
        }
    }

    Ok(heredocs)
}

fn extract_heredoc_delimiter(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let after = trimmed.strip_prefix("<<")?;
    let after = after.trim_start();
    if let Some(rest) = after.strip_prefix('\'') {
        let end = rest.find('\'')?;
        Some(&rest[..end])
    } else if let Some(rest) = after.strip_prefix('"') {
        let end = rest.find('"')?;
        Some(&rest[..end])
    } else {
        let end = after.find(|c: char| c.is_whitespace())?;
        Some(&after[..end])
    }
}

/// Validate that the edit's search text appears exactly once in the file.
/// Returns the byte position of the match.
pub fn validate_search_unique(cmd: &EditCommand, file_content: &str) -> Result<usize, EditError> {
    let mut matches = file_content.match_indices(&cmd.search).peekable();
    let first = matches.next();
    if first.is_none() {
        return Err(EditError::SearchNotFound {
            search: cmd.search.clone(),
        });
    }
    if matches.peek().is_some() {
        let count = 1 + matches.count();
        return Err(EditError::AmbiguousSearch {
            search: cmd.search.clone(),
            count,
        });
    }
    if let Some((pos, _)) = first {
        Ok(pos)
    } else {
        unreachable!()
    }
}

/// Validate that edit ranges do not overlap.
/// Takes `(start, end)` pairs where `end = start + search.len()`.
pub fn validate_no_overlaps(ranges: &[(usize, usize)]) -> Result<(), EditError> {
    let mut sorted: Vec<(usize, usize)> = ranges.to_vec();
    sorted.sort_by_key(|(s, _)| *s);

    for window in sorted.windows(2) {
        if let [first, second] = window {
            let (_, e1) = *first;
            let (s2, _) = *second;
            if s2 < e1 {
                return Err(EditError::OverlappingRanges {
                    cmd_a: format!("search at byte {}..{}", first.0, e1),
                    cmd_b: format!("search at byte {}..{}", s2, second.1),
                });
            }
        }
    }
    Ok(())
}

/// Apply edits to file content. Edits are applied from end to start so that
/// earlier positions are not shifted by later replacements.
#[must_use]
pub fn apply_edits(content: &str, edits: &[(usize, &str, &str)]) -> String {
    let mut sorted: Vec<(usize, &str, &str)> = edits.to_vec();
    sorted.sort_by_key(|(pos, _, _)| *pos);
    sorted.reverse();

    let mut result = content.to_string();
    for (pos, search, replace) in sorted {
        result.replace_range(pos..pos + search.len(), replace);
    }
    result
}

#[must_use]
pub fn serialize_edit_command(cmd: &EditCommand) -> String {
    format!(
        "<<'TAIDELIM'\n{}\nTAIDELIM\n<<'TAIDELIM'\n{}\nTAIDELIM",
        cmd.search, cmd.replace
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_two_heredocs() {
        let cmd = parse_edit_command(
            "<<'SEARCH'\nold line\nSEARCH\n<<'REPLACE'\nnew line\nREPLACE",
        )
        .unwrap();
        assert_eq!(cmd.search, "old line");
        assert_eq!(cmd.replace, "new line");
    }

    #[test]
    fn test_parse_with_arbitrary_delimiters() {
        let cmd = parse_edit_command(
            "<<'FOO'\nsearch text\nFOO\n<<'BAR'\nreplace text\nBAR",
        )
        .unwrap();
        assert_eq!(cmd.search, "search text");
        assert_eq!(cmd.replace, "replace text");
    }

    #[test]
    fn test_parse_multiline() {
        let cmd = parse_edit_command(
            "<<'A'\nline one\nline two\nA\n<<'B'\nline three\nline four\nB",
        )
        .unwrap();
        assert_eq!(cmd.search, "line one\nline two");
        assert_eq!(cmd.replace, "line three\nline four");
    }

    #[test]
    fn test_parse_empty_replace() {
        let cmd = parse_edit_command("<<'A'\nold\nA\n<<'B'\nB").unwrap();
        assert_eq!(cmd.search, "old");
        assert_eq!(cmd.replace, "");
    }

    #[test]
    fn test_parse_only_one_heredoc_fails() {
        let result = parse_edit_command("<<'A'\nold\nA");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_no_heredoc_fails() {
        let result = parse_edit_command("old line");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_three_heredocs_fails() {
        let result = parse_edit_command(
            "<<'A'\nold\nA\n<<'B'\nnew\nB\n<<'C'\nextra\nC",
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_unique_found() {
        let cmd = EditCommand {
            search: "hello".into(),
            replace: "world".into(),
        };
        assert_eq!(validate_search_unique(&cmd, "say hello there").unwrap(), 4);
    }

    #[test]
    fn test_validate_not_found() {
        let cmd = EditCommand {
            search: "missing".into(),
            replace: "x".into(),
        };
        let result = validate_search_unique(&cmd, "say hello there");
        assert!(matches!(result, Err(EditError::SearchNotFound { .. })));
    }

    #[test]
    fn test_validate_ambiguous() {
        let cmd = EditCommand {
            search: "a".into(),
            replace: "x".into(),
        };
        let result = validate_search_unique(&cmd, "aba");
        assert!(matches!(
            result,
            Err(EditError::AmbiguousSearch { count: 2, .. })
        ));
    }

    #[test]
    fn test_validate_no_overlaps_ok() {
        let ranges = vec![(0, 5), (10, 15)];
        assert!(validate_no_overlaps(&ranges).is_ok());
    }

    #[test]
    fn test_validate_overlaps_fails() {
        let ranges = vec![(0, 10), (5, 15)];
        let result = validate_no_overlaps(&ranges);
        assert!(matches!(result, Err(EditError::OverlappingRanges { .. })));
    }

    #[test]
    fn test_validate_adjacent_ok() {
        // Adjacent ranges do not overlap: [0,5) and [5,10)
        let ranges = vec![(0, 5), (5, 10)];
        assert!(validate_no_overlaps(&ranges).is_ok());
    }

    #[test]
    fn test_apply_single_edit() {
        let result = apply_edits("hello world", &[(6, "world", "Rust")]);
        assert_eq!(result, "hello Rust");
    }

    #[test]
    fn test_apply_multiple_edits() {
        let edits = vec![(6, "world", "Rust"), (0, "hello", "hi")];
        let result = apply_edits("hello world", &edits);
        assert_eq!(result, "hi Rust");
    }

    #[test]
    fn test_apply_delete() {
        let result = apply_edits("hello world", &[(6, "world", "")]);
        assert_eq!(result, "hello ");
    }

    #[test]
    fn test_serialize_roundtrip() {
        let cmd = EditCommand {
            search: "old".into(),
            replace: "new".into(),
        };
        let serialized = serialize_edit_command(&cmd);
        let parsed = parse_edit_command(&serialized).unwrap();
        assert_eq!(parsed, cmd);
    }
}
