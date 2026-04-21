use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditCommand {
    pub start: LineRef,
    pub end: Option<LineRef>,
    pub action: EditAction,
    pub content: String,
    pub start_text: Option<String>,
    pub end_text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineRef {
    Num(usize),
    Last,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditAction {
    Change,
    Delete,
    InsertBefore,
    AppendAfter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    Parse {
        line: usize,
        message: String,
    },
    OverlappingRanges {
        cmd_a: String,
        cmd_b: String,
    },
    LineTextMismatch {
        line_num: usize,
        expected: String,
        got: String,
    },
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse { line, message } => write!(f, "parse error at line {line}: {message}"),
            Self::OverlappingRanges { cmd_a, cmd_b } => {
                write!(f, "overlapping ranges: {cmd_a} vs {cmd_b}")
            }
            Self::LineTextMismatch {
                line_num,
                expected,
                got,
            } => write!(
                f,
                "line {line_num} text mismatch: expected '{expected}', got '{got}'"
            ),
        }
    }
}

impl std::error::Error for EditError {}

impl LineRef {
    #[must_use]
    pub fn resolve(self, total_lines: usize) -> usize {
        match self {
            Self::Num(n) => n,
            Self::Last => total_lines,
        }
    }
}

impl EditCommand {
    #[must_use]
    pub fn line_range(&self, total_lines: usize) -> (usize, usize) {
        let start = self.start.resolve(total_lines);
        let end = self.end.map_or(start, |e| e.resolve(total_lines));
        (start, end)
    }

    #[must_use]
    pub fn to_ex_lines(&self) -> String {
        let range = match (&self.start, &self.end) {
            (LineRef::Num(s), None) => format!("{s}"),
            (LineRef::Last, None) => "$".to_string(),
            (LineRef::Num(s), Some(LineRef::Num(e))) => format!("{s},{e}"),
            (LineRef::Num(s), Some(LineRef::Last)) => format!("{s},$"),
            (LineRef::Last, Some(LineRef::Num(e))) => format!("$,{e}"),
            (LineRef::Last, Some(LineRef::Last)) => "$,$".to_string(),
        };
        let cmd = match self.action {
            EditAction::Change => 'c',
            EditAction::Delete => 'd',
            EditAction::InsertBefore => 'i',
            EditAction::AppendAfter => 'a',
        };
        format!("{range}{cmd}")
    }
}

/// Parse edit commands in the new format:
/// ```text
/// ActionWord
/// L<n>:line text
/// [L<n>:line text]   (optional range end)
/// [content lines]
/// .
/// ```
///
/// `ActionWord` is one of: Change, Delete, `InsertBefore`, `AppendAfter`.
/// Line references are `L<n>:text` or `$:text`.
/// For Change/InsertBefore/AppendAfter, content follows until a lone `.` line.
/// Delete has no content section.
///
/// If `file_content` is provided, line texts are validated against the actual file.
pub fn parse_edit_commands(
    content: &str,
    file_content: Option<&str>,
) -> Result<Vec<EditCommand>, EditError> {
    let file_lines: Vec<String> = file_content
        .map(|s| s.lines().map(ToString::to_string).collect())
        .unwrap_or_default();
    let has_file = file_content.is_some();

    let mut commands = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;

    while let Some(line_raw) = lines.get(i) {
        let line = line_raw.trim();
        if line.is_empty() {
            i += 1;
            continue;
        }

        let action = parse_action_word(line, i + 1)?;
        i += 1;

        let (start, start_text) = parse_line_ref_with_text(lines.get(i).copied(), i + 1)?;
        i += 1;

        let mut end = None;
        let mut end_text: Option<String> = None;
        if let Some(ref_line) = lines.get(i)
            && looks_like_line_ref(ref_line)
        {
            let (end_ref, text) = parse_line_ref_with_text(Some(ref_line), i + 1)?;
            end = Some(end_ref);
            end_text = Some(text);
            i += 1;
        }

        let mut cmd_content = String::new();
        if action != EditAction::Delete {
            while let Some(content_line) = lines.get(i) {
                if content_line.trim() == "." {
                    i += 1;
                    break;
                }
                if !cmd_content.is_empty() {
                    cmd_content.push('\n');
                }
                cmd_content.push_str(content_line);
                i += 1;
            }
        }

        if has_file {
            validate_line_text(&start, &start_text, &file_lines)?;
            if let Some(end_ref) = end
                && let Some(ref text) = end_text {
                    validate_line_text(&end_ref, text, &file_lines)?;
                }
        }

        commands.push(EditCommand {
            start,
            end,
            action,
            content: cmd_content,
            start_text: Some(start_text),
            end_text,
        });
    }

    validate_no_overlaps(&commands)?;
    Ok(commands)
}

fn parse_action_word(line: &str, line_num: usize) -> Result<EditAction, EditError> {
    match line.trim() {
        "Change" => Ok(EditAction::Change),
        "Delete" => Ok(EditAction::Delete),
        "InsertBefore" => Ok(EditAction::InsertBefore),
        "AppendAfter" => Ok(EditAction::AppendAfter),
        other => Err(EditError::Parse {
            line: line_num,
            message: format!(
                "expected action word (Change/Delete/InsertBefore/AppendAfter) in: {other}"
            ),
        }),
    }
}

/// Check if a line looks like a line reference: `L<n>:...` or `$:...`
fn looks_like_line_ref(line: &str) -> bool {
    let trimmed = line.trim();
    if let Some(rest) = trimmed.strip_prefix('L') {
        let delim_pos = rest.find([':', ';']);
        let Some(pos) = delim_pos else { return false };
        let num_part = &rest[..pos];
        !num_part.is_empty() && num_part.chars().all(|c| c.is_ascii_digit())
    } else if let Some(rest) = trimmed.strip_prefix('$') {
        rest.starts_with(':') || rest.starts_with(';')
    } else {
        false
    }
}

/// Parse a line reference with text: `L<n>:text` or `$:text`
fn parse_line_ref_with_text(
    line: Option<&str>,
    line_num: usize,
) -> Result<(LineRef, String), EditError> {
    let line = line.ok_or_else(|| EditError::Parse {
        line: line_num,
        message: "expected line reference after action".to_string(),
    })?;
    let trimmed = line.trim();

    if let Some(rest) = trimmed.strip_prefix('$') {
        if let Some(text) = rest.strip_prefix(':') {
            return Ok((LineRef::Last, text.to_string()));
        }
        if let Some(text) = rest.strip_prefix(';') {
            return Ok((LineRef::Last, text.to_string()));
        }
        return Err(EditError::Parse {
            line: line_num,
            message: format!("expected ':' or ';' after '$' in: {line}"),
        });
    }

    if let Some(rest) = trimmed.strip_prefix('L') {
        let delim_pos = rest.find([':', ';']).ok_or_else(|| EditError::Parse {
            line: line_num,
            message: format!("expected ':' or ';' after line number in: {line}"),
        })?;
        let num_str = &rest[..delim_pos];
        if num_str.is_empty() {
            return Err(EditError::Parse {
                line: line_num,
                message: format!("missing line number in: {line}"),
            });
        }
        let num: usize = num_str.parse().map_err(|_| EditError::Parse {
            line: line_num,
            message: format!("invalid line number in: {line}"),
        })?;
        let text = if rest.as_bytes().get(delim_pos) == Some(&b';') {
            String::new()
        } else {
            rest[delim_pos + 1..].to_string()
        };
        Ok((LineRef::Num(num), text))
    } else {
        Err(EditError::Parse {
            line: line_num,
            message: format!("expected line reference (L<n>: or $:) in: {line}"),
        })
    }
}

fn validate_line_text(
    line_ref: &LineRef,
    provided_text: &str,
    file_lines: &[String],
) -> Result<(), EditError> {
    let total = file_lines.len();
    let line_num = line_ref.resolve(total);

    if line_num == 0 || line_num > total {
        return Ok(());
    }

    let actual = file_lines.get(line_num - 1).ok_or_else(|| EditError::Parse {
        line: line_num,
        message: format!("line {line_num} does not exist in file"),
    })?;
    if actual.trim_end() != provided_text.trim_end() {
        return Err(EditError::LineTextMismatch {
            line_num,
            expected: actual.trim_end().to_string(),
            got: provided_text.trim_end().to_string(),
        });
    }

    Ok(())
}

fn validate_no_overlaps(commands: &[EditCommand]) -> Result<(), EditError> {
    let mut ranges: Vec<(usize, usize, &EditCommand)> = Vec::new();

    for cmd in commands {
        let start = match cmd.start {
            LineRef::Num(n) => n,
            LineRef::Last => continue,
        };
        let end = match cmd.end {
            Some(LineRef::Num(n)) => n,
            Some(LineRef::Last) => continue,
            None => start,
        };

        ranges.push((start, end, cmd));
    }

    for (i, &(s1, e1, cmd_a)) in ranges.iter().enumerate() {
        if let Some(remaining) = ranges.get(i + 1..) {
            for &(s2, e2, cmd_b) in remaining {
                if s1 <= e2 && s2 <= e1 {
                    return Err(EditError::OverlappingRanges {
                        cmd_a: cmd_a.to_ex_lines(),
                        cmd_b: cmd_b.to_ex_lines(),
                    });
                }
            }
        }
    }

    Ok(())
}

pub fn sort_bottom_up(commands: &mut [EditCommand]) {
    commands.sort_by(|a, b| {
        let a_start = match a.start {
            LineRef::Num(n) => n,
            LineRef::Last => usize::MAX,
        };
        let b_start = match b.start {
            LineRef::Num(n) => n,
            LineRef::Last => usize::MAX,
        };
        b_start.cmp(&a_start)
    });
}

#[must_use]
pub fn build_ex_script(path: &str, commands: &[EditCommand]) -> String {
    let mut script = String::new();
    for cmd in commands {
        script.push_str(&cmd.to_ex_lines());
        script.push('\n');
        if cmd.action != EditAction::Delete && !cmd.content.is_empty() {
            script.push_str(&cmd.content);
            script.push('\n');
        }
        if cmd.action != EditAction::Delete {
            script.push_str(".\n");
        }
    }
    script.push_str("wq\n");

    format!("ex {path} <<'TAIEX'\n{script}TAIEX")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_change_single() {
        let cmds = parse_edit_commands("Change\nL5:old line\nnew line\n.", None).unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].start, LineRef::Num(5));
        assert_eq!(cmds[0].end, None);
        assert_eq!(cmds[0].action, EditAction::Change);
        assert_eq!(cmds[0].content, "new line");
        assert_eq!(cmds[0].start_text.as_deref(), Some("old line"));
    }

    #[test]
    fn test_parse_change_range() {
        let cmds =
            parse_edit_commands("Change\nL10:old start\nL15:old end\nfn new() {}", None).unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].start, LineRef::Num(10));
        assert_eq!(cmds[0].end, Some(LineRef::Num(15)));
        assert_eq!(cmds[0].action, EditAction::Change);
        assert_eq!(cmds[0].content, "fn new() {}");
    }

    #[test]
    fn test_parse_delete() {
        let cmds = parse_edit_commands("Delete\nL5:line to delete", None).unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].action, EditAction::Delete);
        assert_eq!(cmds[0].content, "");
        assert_eq!(cmds[0].start_text.as_deref(), Some("line to delete"));
    }

    #[test]
    fn test_parse_delete_range() {
        let cmds = parse_edit_commands("Delete\nL10:first line\nL15:last line", None).unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].start, LineRef::Num(10));
        assert_eq!(cmds[0].end, Some(LineRef::Num(15)));
        assert_eq!(cmds[0].action, EditAction::Delete);
    }

    #[test]
    fn test_parse_insert() {
        let cmds =
            parse_edit_commands("InsertBefore\nL5:existing line\ninserted line\n.", None).unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].action, EditAction::InsertBefore);
        assert_eq!(cmds[0].content, "inserted line");
    }

    #[test]
    fn test_parse_append() {
        let cmds =
            parse_edit_commands("AppendAfter\nL5:existing line\nappended line\n.", None).unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].action, EditAction::AppendAfter);
        assert_eq!(cmds[0].content, "appended line");
    }

    #[test]
    fn test_parse_dollar_append() {
        let cmds = parse_edit_commands("AppendAfter\n$:last line\nat the end\n.", None).unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].start, LineRef::Last);
        assert_eq!(cmds[0].end, None);
        assert_eq!(cmds[0].action, EditAction::AppendAfter);
    }

    #[test]
    fn test_parse_dollar_change() {
        let cmds = parse_edit_commands("Change\n$:old last line\nnew last line\n.", None).unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].start, LineRef::Last);
        assert_eq!(cmds[0].end, None);
        assert_eq!(cmds[0].action, EditAction::Change);
    }

    #[test]
    fn test_parse_multiple_non_overlapping() {
        let cmds = parse_edit_commands(
            "Change\nL5:old five\nnew five\n.\nDelete\nL10:line ten\nChange\nL20:old twenty\nL25:old end\nnew block\n.",
            None,
        )
        .unwrap();
        assert_eq!(cmds.len(), 3);
    }

    #[test]
    fn test_parse_overlapping_fails() {
        let result = parse_edit_commands(
            "Change\nL5:start five\nL10:end five\nnew\n.\nChange\nL8:start eight\nL12:end eight\nother\n.",
            None,
        );
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            EditError::OverlappingRanges { .. }
        ));
    }

    #[test]
    fn test_sort_bottom_up() {
        let mut cmds = parse_edit_commands(
            "Change\nL5:old five\nnew\n.\nChange\nL10:old ten\nL15:old end\nnewer\n.\nDelete\nL3:line three",
            None,
        )
        .unwrap();
        sort_bottom_up(&mut cmds);
        assert_eq!(cmds[0].start, LineRef::Num(10));
        assert_eq!(cmds[1].start, LineRef::Num(5));
        assert_eq!(cmds[2].start, LineRef::Num(3));
    }

    #[test]
    fn test_build_ex_script() {
        let mut cmds = parse_edit_commands(
            "Change\nL10:old ten\nL15:old end\nfn new() {}\n.\nDelete\nL5:old five",
            None,
        )
        .unwrap();
        sort_bottom_up(&mut cmds);
        let script = build_ex_script("src/main.rs", &cmds);
        assert!(script.starts_with("ex src/main.rs <<'TAIEX'"));
        assert!(script.contains("10,15c"));
        assert!(script.contains("fn new() {}"));
        assert!(script.contains("5d"));
        assert!(script.contains("wq"));
        assert!(script.ends_with("TAIEX"));
    }

    #[test]
    fn test_multiline_content() {
        let cmds = parse_edit_commands(
            "Change\nL5:old five\nline one\nline two\nline three\n.",
            None,
        )
        .unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].content, "line one\nline two\nline three");
    }

    #[test]
    fn test_validate_line_text_ok() {
        let file = "first\nsecond\nthird\nfourth\nfifth\n";
        let cmds = parse_edit_commands("Change\nL2:second\nreplaced\n.", Some(file)).unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].start_text.as_deref(), Some("second"));
    }

    #[test]
    fn test_validate_line_text_mismatch() {
        let file = "first\nsecond\nthird\n";
        let result = parse_edit_commands("Change\nL2:wrong text\nreplaced\n.", Some(file));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            EditError::LineTextMismatch { line_num: 2, .. }
        ));
    }

    #[test]
    fn test_validate_line_text_end_range() {
        let file = "line1\nline2\nline3\nline4\nline5\n";
        let cmds =
            parse_edit_commands("Change\nL2:line2\nL4:line4\nnew block\n.", Some(file)).unwrap();
        assert_eq!(cmds[0].end_text.as_deref(), Some("line4"));
    }

    #[test]
    fn test_validate_line_text_end_mismatch() {
        let file = "line1\nline2\nline3\nline4\nline5\n";
        let result = parse_edit_commands("Change\nL2:line2\nL4:wrong\nnew\n.", Some(file));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            EditError::LineTextMismatch { line_num: 4, .. }
        ));
    }

    #[test]
    fn test_validate_no_file_skips() {
        let cmds = parse_edit_commands("Change\nL5:anything\nnew\n.", None).unwrap();
        assert_eq!(cmds.len(), 1);
        // No validation error even though text doesn't match any file
    }

    #[test]
    fn test_validate_dollar_line() {
        let file = "line1\nline2\nlast line\n";
        let cmds = parse_edit_commands("AppendAfter\n$:last line\nadded\n.", Some(file)).unwrap();
        assert_eq!(cmds.len(), 1);
    }

    #[test]
    fn test_validate_dollar_mismatch() {
        let file = "line1\nline2\nactual last\n";
        let result = parse_edit_commands("AppendAfter\n$:wrong last\nadded\n.", Some(file));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            EditError::LineTextMismatch { .. }
        ));
    }

    #[test]
    fn test_parse_range_with_dollar_end() {
        let cmds =
            parse_edit_commands("Change\nL5:line five\n$:last line\nnew ending\n.", None).unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].start, LineRef::Num(5));
        assert_eq!(cmds[0].end, Some(LineRef::Last));
    }

    #[test]
    fn test_parse_dollar_range_with_num_end() {
        let cmds = parse_edit_commands("Change\n$:first ref\nL5:second ref\nnew\n.", None).unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].start, LineRef::Last);
        assert_eq!(cmds[0].end, Some(LineRef::Num(5)));
    }

    #[test]
    fn test_empty_change_content() {
        let cmds = parse_edit_commands("Change\nL5:old line\n.", None).unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].content, "");
    }

    #[test]
    fn test_content_with_blank_line() {
        let cmds = parse_edit_commands("Change\nL5:old\nnew line\n\nafter blank\n.", None).unwrap();
        assert_eq!(cmds[0].content, "new line\n\nafter blank");
    }

    #[test]
    fn test_parse_unknown_action_fails() {
        let result = parse_edit_commands("Replace\nL5:text\nnew\n.", None);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), EditError::Parse { .. }));
    }

    #[test]
    fn test_parse_missing_line_ref_fails() {
        let result = parse_edit_commands("Change\n", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_line_ref_without_colon_fails() {
        let result = parse_edit_commands("Change\nL5 no colon\nnew\n.", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_change_with_dot_in_content() {
        // Content line that is just a dot ends the block — same as ex convention
        let cmds = parse_edit_commands("Change\nL5:old\nnew line\n.", None).unwrap();
        assert_eq!(cmds[0].content, "new line");
    }

    #[test]
    fn test_parse_empty_line_ref() {
        let cmds = parse_edit_commands("Change\nL2;\nnew line\n.", None).unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].start, LineRef::Num(2));
        assert_eq!(cmds[0].start_text.as_deref(), Some(""));
        assert_eq!(cmds[0].content, "new line");
    }

    #[test]
    fn test_parse_dollar_empty_line_ref() {
        let cmds = parse_edit_commands("AppendAfter\n$;\nadded\n.", None).unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].start, LineRef::Last);
        assert_eq!(cmds[0].start_text.as_deref(), Some(""));
        assert_eq!(cmds[0].content, "added");
    }

    #[test]
    fn test_validate_empty_line_text() {
        let file = "first\n\nthird\n";
        let cmds = parse_edit_commands("Change\nL2;\ninserted\n.", Some(file)).unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].start_text.as_deref(), Some(""));
    }

    #[test]
    fn test_parse_change_with_empty_line_range() {
        let cmds = parse_edit_commands(
            "Change\nL1:old start\nL2;\nnew block\n.",
            None,
        )
        .unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].start, LineRef::Num(1));
        assert_eq!(cmds[0].end, Some(LineRef::Num(2)));
        assert_eq!(cmds[0].start_text.as_deref(), Some("old start"));
        assert_eq!(cmds[0].end_text.as_deref(), Some(""));
        assert_eq!(cmds[0].content, "new block");
    }
}
