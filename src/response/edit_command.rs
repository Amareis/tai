use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditCommand {
    pub start: LineRef,
    pub end: Option<LineRef>,
    pub action: EditAction,
    pub content: String,
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
    Parse { line: usize, message: String },
    OverlappingRanges { cmd_a: String, cmd_b: String },
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse { line, message } => write!(f, "parse error at line {line}: {message}"),
            Self::OverlappingRanges { cmd_a, cmd_b } => {
                write!(f, "overlapping ranges: {cmd_a} vs {cmd_b}")
            }
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

pub fn parse_edit_commands(content: &str) -> Result<Vec<EditCommand>, EditError> {
    let mut commands = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;

    while let Some(line_raw) = lines.get(i) {
        let line = line_raw.trim();
        if line.is_empty() {
            i += 1;
            continue;
        }

        let (start, end, rest) = parse_line_range(line, i + 1)?;
        let action_char = rest.chars().next().ok_or_else(|| EditError::Parse {
            line: i + 1,
            message: format!("expected action after range in: {line}"),
        })?;

        let action = match action_char {
            'c' => EditAction::Change,
            'd' => EditAction::Delete,
            'i' => EditAction::InsertBefore,
            'a' => EditAction::AppendAfter,
            _ => {
                return Err(EditError::Parse {
                    line: i + 1,
                    message: format!("unknown action '{action_char}' in: {line}"),
                });
            }
        };

        i += 1;

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

        commands.push(EditCommand {
            start,
            end,
            action,
            content: cmd_content,
        });
    }

    validate_no_overlaps(&commands)?;
    Ok(commands)
}

fn parse_line_range(
    line: &str,
    line_num: usize,
) -> Result<(LineRef, Option<LineRef>, &str), EditError> {
    let trimmed = line.trim();

    if let Some(rest) = trimmed.strip_prefix('$') {
        if let Some(rest) = rest.strip_prefix(',') {
            let (end, action_rest) = parse_single_line_ref(rest, line_num, line)?;
            return Ok((LineRef::Last, Some(end), action_rest));
        }
        let action_rest = parse_action_start(rest);
        return Ok((LineRef::Last, None, action_rest));
    }

    let (first_num, after_first) = parse_leading_number(trimmed, line_num, line)?;

    if let Some(rest) = after_first.strip_prefix(',') {
        if let Some(rest) = rest.strip_prefix('$') {
            let action_rest = parse_action_start(rest);
            return Ok((first_num, Some(LineRef::Last), action_rest));
        }
        let (second_num, action_rest) = parse_single_line_ref(rest, line_num, line)?;
        return Ok((first_num, Some(second_num), action_rest));
    }

    let action_rest = parse_action_start(after_first);
    Ok((first_num, None, action_rest))
}

fn parse_single_line_ref<'a>(
    s: &'a str,
    line_num: usize,
    original: &str,
) -> Result<(LineRef, &'a str), EditError> {
    if let Some(rest) = s.strip_prefix('$') {
        Ok((LineRef::Last, rest))
    } else {
        let (num, rest) = parse_leading_number(s, line_num, original)?;
        Ok((num, rest))
    }
}

fn parse_leading_number<'a>(
    s: &'a str,
    line_num: usize,
    original: &str,
) -> Result<(LineRef, &'a str), EditError> {
    let s = s.strip_prefix('L').unwrap_or(s);
    let num_str_end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let num_str = &s[..num_str_end];
    if num_str.is_empty() {
        return Err(EditError::Parse {
            line: line_num,
            message: format!("expected line number in: {original}"),
        });
    }
    let num: usize = num_str.parse().map_err(|_| EditError::Parse {
        line: line_num,
        message: format!("invalid line number in: {original}"),
    })?;
    let rest = &s[num_str.len()..];
    Ok((LineRef::Num(num), rest))
}

fn parse_action_start(s: &str) -> &str {
    s
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
        let cmds = parse_edit_commands("5c\nnew line").unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].start, LineRef::Num(5));
        assert_eq!(cmds[0].end, None);
        assert_eq!(cmds[0].action, EditAction::Change);
        assert_eq!(cmds[0].content, "new line");
    }

    #[test]
    fn test_parse_change_range() {
        let cmds = parse_edit_commands("10,15c\nfn new() {}").unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].start, LineRef::Num(10));
        assert_eq!(cmds[0].end, Some(LineRef::Num(15)));
        assert_eq!(cmds[0].action, EditAction::Change);
    }

    #[test]
    fn test_parse_delete() {
        let cmds = parse_edit_commands("5d").unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].action, EditAction::Delete);
        assert_eq!(cmds[0].content, "");
    }

    #[test]
    fn test_parse_delete_range() {
        let cmds = parse_edit_commands("10,15d").unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].start, LineRef::Num(10));
        assert_eq!(cmds[0].end, Some(LineRef::Num(15)));
        assert_eq!(cmds[0].action, EditAction::Delete);
    }

    #[test]
    fn test_parse_insert() {
        let cmds = parse_edit_commands("5i\ninserted line").unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].action, EditAction::InsertBefore);
        assert_eq!(cmds[0].content, "inserted line");
    }

    #[test]
    fn test_parse_append() {
        let cmds = parse_edit_commands("5a\nappended line").unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].action, EditAction::AppendAfter);
    }

    #[test]
    fn test_parse_dollar_append() {
        let cmds = parse_edit_commands("$a\nat the end").unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].start, LineRef::Last);
        assert_eq!(cmds[0].end, None);
        assert_eq!(cmds[0].action, EditAction::AppendAfter);
    }

    #[test]
    fn test_parse_multiple_non_overlapping() {
        let cmds = parse_edit_commands("5c\nnew five\n.\n10d\n20,25c\nnew block").unwrap();
        assert_eq!(cmds.len(), 3);
    }

    #[test]
    fn test_parse_overlapping_fails() {
        let result = parse_edit_commands("5,10c\nnew\n.\n8,12c\nother");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            EditError::OverlappingRanges { .. }
        ));
    }

    #[test]
    fn test_sort_bottom_up() {
        let mut cmds = parse_edit_commands("5c\nnew\n.\n10,15c\nnewer\n.\n3d").unwrap();
        sort_bottom_up(&mut cmds);
        assert_eq!(cmds[0].start, LineRef::Num(10));
        assert_eq!(cmds[1].start, LineRef::Num(5));
        assert_eq!(cmds[2].start, LineRef::Num(3));
    }

    #[test]
    fn test_build_ex_script() {
        let mut cmds = parse_edit_commands("10,15c\nfn new() {}\n.\n5d").unwrap();
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
        let cmds = parse_edit_commands("5c\nline one\nline two\nline three\n.").unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].content, "line one\nline two\nline three");
    }
    #[test]
    fn test_parse_l_prefix_single() {
        let cmds = parse_edit_commands("L5c\nnew line").unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].start, LineRef::Num(5));
        assert_eq!(cmds[0].end, None);
        assert_eq!(cmds[0].action, EditAction::Change);
        assert_eq!(cmds[0].content, "new line");
    }

    #[test]
    fn test_parse_l_prefix_range() {
        let cmds = parse_edit_commands("L10,L15c\nfn new() {}").unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].start, LineRef::Num(10));
        assert_eq!(cmds[0].end, Some(LineRef::Num(15)));
        assert_eq!(cmds[0].action, EditAction::Change);
    }

    #[test]
    fn test_parse_l_prefix_delete() {
        let cmds = parse_edit_commands("L5d").unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].action, EditAction::Delete);
        assert_eq!(cmds[0].start, LineRef::Num(5));
    }

    #[test]
    fn test_parse_l_prefix_mixed_with_dollar() {
        let cmds = parse_edit_commands("L5,$c\n till end").unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].start, LineRef::Num(5));
        assert_eq!(cmds[0].end, Some(LineRef::Last));
        assert_eq!(cmds[0].action, EditAction::Change);
    }

    #[test]
    fn test_parse_dollar_comma_l_prefix() {
        let cmds = parse_edit_commands("$,L5c\nstuff").unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].start, LineRef::Last);
        assert_eq!(cmds[0].end, Some(LineRef::Num(5)));
        assert_eq!(cmds[0].action, EditAction::Change);
    }

    #[test]
    fn test_parse_l_prefix_append() {
        let cmds = parse_edit_commands("L5a\nafter five").unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].start, LineRef::Num(5));
        assert_eq!(cmds[0].action, EditAction::AppendAfter);
    }

    #[test]
    fn test_parse_l_prefix_insert() {
        let cmds = parse_edit_commands("L5i\nbefore five").unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].start, LineRef::Num(5));
        assert_eq!(cmds[0].action, EditAction::InsertBefore);
    }
}
