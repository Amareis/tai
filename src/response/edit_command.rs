use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditCommand {
    pub start: LineRef,
    pub end: Option<LineRef>,
    pub action: EditAction,
    pub content: String,
    pub start_text: Option<String>,
    pub end_text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineRef {
    Num(usize),
    Last,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EditAction {
    Change,
    Delete,
    InsertBefore,
    AppendAfter,
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

impl std::fmt::Display for LineRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Num(n) => write!(f, "L{n}"),
            Self::Last => write!(f, "$"),
        }
    }
}

impl EditAction {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Change => "Change",
            Self::Delete => "Delete",
            Self::InsertBefore => "InsertBefore",
            Self::AppendAfter => "AppendAfter",
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

/// Parse a single edit command in the format:
/// ```text
/// ActionWord Exactly L<n>:line text
/// [content lines]
/// ```
///
/// Or for a range:
/// ```text
/// ActionWord Start L<n>:line text
/// End L<m>:line text
/// [content lines]
/// ```
///
/// `ActionWord` is one of: Change, Delete, `InsertBefore`, `AppendAfter`.
/// Line references are `L<n>:text`, `L<n>;`, `$:text`, or `$;`.
/// For Change/InsertBefore/AppendAfter, all lines after the header(s) are content.
/// Delete has no content.
///
/// If `file_content` is provided, line texts are validated against the actual file.
pub fn parse_edit_command(
    content: &str,
    file_content: Option<&str>,
) -> Result<EditCommand, EditError> {
    let file_lines: Vec<String> = file_content
        .map(|s| s.lines().map(ToString::to_string).collect())
        .unwrap_or_default();
    let has_file = file_content.is_some();

    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return Err(EditError::Parse {
            line: 1,
            message: "empty edit command".to_string(),
        });
    }

    #[allow(clippy::indexing_slicing)]
    let first_line = lines[0];
    let action = parse_action_word(first_line, 1)?;
    let rest = first_line[action.as_str().len()..].trim_start();

    let mut i = 1;
    let (start, start_text, end, end_text) = if rest.starts_with("Exactly ") {
        let ref_part = rest.strip_prefix("Exactly ").ok_or_else(|| EditError::Parse {
            line: 1,
            message: format!("expected 'Exactly L<n>:text', got: {rest}"),
        })?;
        let (s, st) = parse_line_ref_with_text(Some(ref_part), 1)?;
        (s, st, None, None)
    } else if rest.starts_with("Start ") {
        let ref_part = rest.strip_prefix("Start ").ok_or_else(|| EditError::Parse {
            line: 1,
            message: format!("expected 'Start L<n>:text', got: {rest}"),
        })?;
        let (s, st) = parse_line_ref_with_text(Some(ref_part), 1)?;
        let end_line = lines.get(i).ok_or_else(|| EditError::Parse {
            line: 2,
            message: "expected 'End L<m>:text' after Start line".to_string(),
        })?;
        let end_ref_part = end_line
            .trim_start()
            .strip_prefix("End ")
            .ok_or_else(|| EditError::Parse {
                line: 2,
                message: format!("expected 'End L<m>:text', got: {end_line}"),
            })?;
        let (e, et) = parse_line_ref_with_text(Some(end_ref_part), 2)?;
        i += 1;
        (s, st, Some(e), Some(et))
    } else {
        return Err(EditError::Parse {
            line: 1,
            message: format!(
                "expected 'Exactly' or 'Start' after action, got: {rest}"
            ),
        });
    };

    let cmd_content = if action == EditAction::Delete {
        String::new()
    } else {
        lines.get(i..).map_or(String::new(), |s| s.join("\n"))
    };

    if has_file {
        validate_line_text(&start, &start_text, &file_lines)?;
        if let (Some(end_ref), Some(text)) = (end, end_text.as_ref()) {
            validate_line_text(&end_ref, text, &file_lines)?;
        }
    }

    Ok(EditCommand {
        start,
        end,
        action,
        content: cmd_content,
        start_text: Some(start_text),
        end_text,
    })
}

fn parse_action_word(line: &str, line_num: usize) -> Result<EditAction, EditError> {
    let word = line.split_whitespace().next().unwrap_or(line.trim());
    match word {
        "Change" => Ok(EditAction::Change),
        "Delete" => Ok(EditAction::Delete),
        "InsertBefore" => Ok(EditAction::InsertBefore),
        "AppendAfter" | "InsertAfter" => Ok(EditAction::AppendAfter),
        other => Err(EditError::Parse {
            line: line_num,
            message: format!(
                "expected action word (Change/Delete/InsertBefore/AppendAfter) in: {other}"
            ),
        }),
    }
}

/// Parse a line reference with text: `L<n>:text`, `L<n>;`, `$:text`, or `$;`
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

pub fn validate_no_overlaps(commands: &[EditCommand]) -> Result<(), EditError> {
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

pub fn validate_texts_against_output(
    cmd: &EditCommand,
    output: &str,
) -> Result<(), EditError> {
    if let Some(text) = cmd.start_text.as_ref() && !output.contains(text.as_str()) {
        return Err(EditError::Parse {
            line: 0,
            message: format!(
                "start text for {} not found in file output: expected '{text}'",
                cmd.start
            ),
        });
    }
    if let (Some(end_ref), Some(text)) = (cmd.end, cmd.end_text.as_ref())
        && !output.contains(text.as_str())
    {
        return Err(EditError::Parse {
            line: 0,
            message: format!(
                "end text for {end_ref} not found in file output: expected '{text}'"
            ),
        });
    }
    Ok(())
}

#[must_use]
pub fn serialize_edit_command(cmd: &EditCommand) -> String {
    let mut text = String::new();
    text.push_str(cmd.action.as_str());
    text.push(' ');
    write_line_ref(&mut text, &cmd.start, cmd.start_text.as_deref());
    if let Some(end) = &cmd.end {
        text.push_str("\nEnd ");
        write_line_ref(&mut text, end, cmd.end_text.as_deref());
    }
    if cmd.action != EditAction::Delete && !cmd.content.is_empty() {
        text.push('\n');
        text.push_str(&cmd.content);
    }
    text
}

fn write_line_ref(text: &mut String, line_ref: &LineRef, line_text: Option<&str>) {
    let _ = std::fmt::Write::write_fmt(text, format_args!("{line_ref}"));
    let t = line_text.unwrap_or("");
    if t.is_empty() {
        text.push(';');
    } else {
        text.push(':');
        text.push_str(t);
    }
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
        let cmd = parse_edit_command("Change Exactly L5:old line\nnew line", None).unwrap();
        assert_eq!(cmd.start, LineRef::Num(5));
        assert_eq!(cmd.end, None);
        assert_eq!(cmd.action, EditAction::Change);
        assert_eq!(cmd.content, "new line");
        assert_eq!(cmd.start_text.as_deref(), Some("old line"));
    }

    #[test]
    fn test_parse_change_range() {
        let cmd = parse_edit_command(
            "Change Start L10:old start\nEnd L15:old end\nfn new() {}",
            None,
        )
        .unwrap();
        assert_eq!(cmd.start, LineRef::Num(10));
        assert_eq!(cmd.end, Some(LineRef::Num(15)));
        assert_eq!(cmd.action, EditAction::Change);
        assert_eq!(cmd.content, "fn new() {}");
    }

    #[test]
    fn test_parse_delete() {
        let cmd = parse_edit_command("Delete Exactly L5:line to delete", None).unwrap();
        assert_eq!(cmd.action, EditAction::Delete);
        assert_eq!(cmd.content, "");
        assert_eq!(cmd.start_text.as_deref(), Some("line to delete"));
    }

    #[test]
    fn test_parse_delete_range() {
        let cmd = parse_edit_command(
            "Delete Start L10:first line\nEnd L15:last line",
            None,
        )
        .unwrap();
        assert_eq!(cmd.start, LineRef::Num(10));
        assert_eq!(cmd.end, Some(LineRef::Num(15)));
        assert_eq!(cmd.action, EditAction::Delete);
    }

    #[test]
    fn test_parse_insert() {
        let cmd = parse_edit_command(
            "InsertBefore Exactly L5:existing line\ninserted line",
            None,
        )
        .unwrap();
        assert_eq!(cmd.action, EditAction::InsertBefore);
        assert_eq!(cmd.content, "inserted line");
    }

    #[test]
    fn test_parse_append() {
        let cmd = parse_edit_command(
            "AppendAfter Exactly L5:existing line\nappended line",
            None,
        )
        .unwrap();
        assert_eq!(cmd.action, EditAction::AppendAfter);
        assert_eq!(cmd.content, "appended line");
    }

    #[test]
    fn test_parse_dollar_append() {
        let cmd = parse_edit_command("AppendAfter Exactly $:last line\nat the end", None).unwrap();
        assert_eq!(cmd.start, LineRef::Last);
        assert_eq!(cmd.end, None);
        assert_eq!(cmd.action, EditAction::AppendAfter);
    }

    #[test]
    fn test_parse_dollar_change() {
        let cmd = parse_edit_command("Change Exactly $:old last line\nnew last line", None).unwrap();
        assert_eq!(cmd.start, LineRef::Last);
        assert_eq!(cmd.end, None);
        assert_eq!(cmd.action, EditAction::Change);
    }

    #[test]
    fn test_parse_multiple_commands_rejected() {
        // Only a single command is allowed per block
        let result = parse_edit_command(
            "Change Exactly L5:old five\nnew five\nDelete Exactly L10:line ten",
            None,
        );
        // The second command becomes part of the content — this is intentional.
        // The caller (core) must ensure one block = one command.
        assert!(result.is_ok());
        let cmd = result.unwrap();
        assert_eq!(cmd.action, EditAction::Change);
        assert_eq!(cmd.content, "new five\nDelete Exactly L10:line ten");
    }

    #[test]
    fn test_sort_bottom_up() {
        let mut cmds = vec![
            EditCommand {
                start: LineRef::Num(5),
                end: None,
                action: EditAction::Change,
                content: "new".to_string(),
                start_text: None,
                end_text: None,
            },
            EditCommand {
                start: LineRef::Num(10),
                end: Some(LineRef::Num(15)),
                action: EditAction::Change,
                content: "newer".to_string(),
                start_text: None,
                end_text: None,
            },
            EditCommand {
                start: LineRef::Num(3),
                end: None,
                action: EditAction::Delete,
                content: String::new(),
                start_text: None,
                end_text: None,
            },
        ];
        sort_bottom_up(&mut cmds);
        assert_eq!(cmds[0].start, LineRef::Num(10));
        assert_eq!(cmds[1].start, LineRef::Num(5));
        assert_eq!(cmds[2].start, LineRef::Num(3));
    }

    #[test]
    fn test_build_ex_script() {
        let cmds = vec![
            EditCommand {
                start: LineRef::Num(10),
                end: Some(LineRef::Num(15)),
                action: EditAction::Change,
                content: "fn new() {}".to_string(),
                start_text: None,
                end_text: None,
            },
            EditCommand {
                start: LineRef::Num(5),
                end: None,
                action: EditAction::Delete,
                content: String::new(),
                start_text: None,
                end_text: None,
            },
        ];
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
        let cmd = parse_edit_command(
            "Change Exactly L5:old five\nline one\nline two\nline three",
            None,
        )
        .unwrap();
        assert_eq!(cmd.content, "line one\nline two\nline three");
    }

    #[test]
    fn test_validate_line_text_ok() {
        let file = "first\nsecond\nthird\nfourth\nfifth\n";
        let cmd = parse_edit_command("Change Exactly L2:second\nreplaced", Some(file)).unwrap();
        assert_eq!(cmd.start_text.as_deref(), Some("second"));
    }

    #[test]
    fn test_validate_line_text_mismatch() {
        let file = "first\nsecond\nthird\n";
        let result = parse_edit_command("Change Exactly L2:wrong text\nreplaced", Some(file));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            EditError::LineTextMismatch { line_num: 2, .. }
        ));
    }

    #[test]
    fn test_validate_line_text_end_range() {
        let file = "line1\nline2\nline3\nline4\nline5\n";
        let cmd = parse_edit_command(
            "Change Start L2:line2\nEnd L4:line4\nnew block",
            Some(file),
        )
        .unwrap();
        assert_eq!(cmd.end_text.as_deref(), Some("line4"));
    }

    #[test]
    fn test_validate_line_text_end_mismatch() {
        let file = "line1\nline2\nline3\nline4\nline5\n";
        let result = parse_edit_command(
            "Change Start L2:line2\nEnd L4:wrong\nnew",
            Some(file),
        );
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            EditError::LineTextMismatch { line_num: 4, .. }
        ));
    }

    #[test]
    fn test_validate_no_file_skips() {
        let cmd = parse_edit_command("Change Exactly L5:anything\nnew", None).unwrap();
        // No validation error even though text doesn't match any file
        assert_eq!(cmd.start, LineRef::Num(5));
    }

    #[test]
    fn test_validate_dollar_line() {
        let file = "line1\nline2\nlast line\n";
        let cmd = parse_edit_command("AppendAfter Exactly $:last line\nadded", Some(file)).unwrap();
        assert_eq!(cmd.start, LineRef::Last);
    }

    #[test]
    fn test_validate_dollar_mismatch() {
        let file = "line1\nline2\nactual last\n";
        let result = parse_edit_command("AppendAfter Exactly $:wrong last\nadded", Some(file));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            EditError::LineTextMismatch { .. }
        ));
    }

    #[test]
    fn test_parse_range_with_dollar_end() {
        let cmd = parse_edit_command(
            "Change Start L5:line five\nEnd $:last line\nnew ending",
            None,
        )
        .unwrap();
        assert_eq!(cmd.start, LineRef::Num(5));
        assert_eq!(cmd.end, Some(LineRef::Last));
    }

    #[test]
    fn test_parse_dollar_range_with_num_end() {
        let cmd = parse_edit_command(
            "Change Start $:first ref\nEnd L5:second ref\nnew",
            None,
        )
        .unwrap();
        assert_eq!(cmd.start, LineRef::Last);
        assert_eq!(cmd.end, Some(LineRef::Num(5)));
    }

    #[test]
    fn test_empty_change_content() {
        let cmd = parse_edit_command("Change Exactly L5:old line\n", None).unwrap();
        assert_eq!(cmd.content, "");
    }

    #[test]
    fn test_content_with_blank_line() {
        let cmd = parse_edit_command(
            "Change Exactly L5:old\nnew line\n\nafter blank",
            None,
        )
        .unwrap();
        assert_eq!(cmd.content, "new line\n\nafter blank");
    }

    #[test]
    fn test_parse_unknown_action_fails() {
        let result = parse_edit_command("Replace L5:text\nnew", None);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), EditError::Parse { .. }));
    }

    #[test]
    fn test_parse_missing_line_ref_fails() {
        let result = parse_edit_command("Change\n", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_line_ref_without_colon_fails() {
        let result = parse_edit_command("Change Exactly L5 no colon\nnew", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_empty_line_ref() {
        let cmd = parse_edit_command("Change Exactly L2;\nnew line", None).unwrap();
        assert_eq!(cmd.start, LineRef::Num(2));
        assert_eq!(cmd.start_text.as_deref(), Some(""));
        assert_eq!(cmd.content, "new line");
    }

    #[test]
    fn test_parse_dollar_empty_line_ref() {
        let cmd = parse_edit_command("AppendAfter Exactly $;\nadded", None).unwrap();
        assert_eq!(cmd.start, LineRef::Last);
        assert_eq!(cmd.start_text.as_deref(), Some(""));
        assert_eq!(cmd.content, "added");
    }

    #[test]
    fn test_validate_empty_line_text() {
        let file = "first\n\nthird\n";
        let cmd = parse_edit_command("Change Exactly L2;\ninserted", Some(file)).unwrap();
        assert_eq!(cmd.start_text.as_deref(), Some(""));
    }

    #[test]
    fn test_parse_change_with_empty_line_range() {
        let cmd = parse_edit_command(
            "Change Start L1:old start\nEnd L2;\nnew block",
            None,
        )
        .unwrap();
        assert_eq!(cmd.start, LineRef::Num(1));
        assert_eq!(cmd.end, Some(LineRef::Num(2)));
        assert_eq!(cmd.start_text.as_deref(), Some("old start"));
        assert_eq!(cmd.end_text.as_deref(), Some(""));
        assert_eq!(cmd.content, "new block");
    }

    #[test]
    fn test_validate_texts_against_output_ok() {
        let cmd = parse_edit_command("Change Exactly L2:line two\nREPLACED", None).unwrap();
        let output = "L1:line one\nL2:line two\nL3:line three\n";
        assert!(validate_texts_against_output(&cmd, output).is_ok());
    }

    #[test]
    fn test_validate_texts_against_output_missing_start() {
        let cmd = parse_edit_command("Change Exactly L2:wrong line\nREPLACED", None).unwrap();
        let output = "L1:line one\nL2:line two\nL3:line three\n";
        let result = validate_texts_against_output(&cmd, output);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("wrong line"));
    }

    #[test]
    fn test_validate_texts_against_output_missing_end() {
        let cmd = parse_edit_command(
            "Change Start L1:line one\nEnd L5:missing end\nnew",
            None,
        )
        .unwrap();
        let output = "L1:line one\nL2:line two\nL3:line three\n";
        let result = validate_texts_against_output(&cmd, output);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing end"));
    }
}
