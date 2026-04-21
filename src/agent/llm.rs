use super::{Agent, AgentError, AgentResponse};
use crate::response::{edit_command, parse_response};
use crate::state::State;
use crate::types::{BlockMode, ParsedBlock};
use async_openai::error::OpenAIError;
use async_openai::types::chat::{
    ChatCompletionRequestAssistantMessage, ChatCompletionRequestMessage,
    ChatCompletionRequestSystemMessage, ChatCompletionRequestUserMessage,
};
use async_openai::{Client, config::OpenAIConfig, types::chat::CreateChatCompletionRequestArgs};
use async_trait::async_trait;
use futures_util::Stream;
use futures_util::stream::StreamExt;
use serde_json::Value;
use std::fmt::Write;
use std::pin::Pin;
use tracing::{debug, error, info, warn};

pub struct LlmAgent {
    model: String,
    client: Client<OpenAIConfig>,
    pub debug: bool,
}

fn check_edit_violations(
    segments: &[ParsedBlock],
    outputs: &std::collections::HashMap<String, crate::backend::CmdOutput>,
) -> Vec<String> {
    let mut violations = Vec::new();
    for block in segments {
        if let BlockMode::Edit(ref cmds) = block.mode {
            if cmds.is_empty() {
                violations.push(format!(
                    "edit:{} — edit commands not parsed.",
                    block.window
                ));
                continue;
            }
            let Some(output) = outputs.get(&block.window) else {
                violations.push(format!(
                    "edit:{} — no file output available. View the file first with a file block.",
                    block.window
                ));
                continue;
            };
            if let Err(e) = edit_command::validate_texts_against_output(cmds, &output.stdout) {
                violations.push(format!("edit:{} — {e}", block.window));
            }
        }
    }
    violations
}

impl LlmAgent {
    #[must_use]
    pub fn new(model: String) -> Self {
        Self {
            model,
            client: Client::default(),
            debug: false,
        }
    }
}

type MyStreamingType = Pin<Box<dyn Stream<Item = Result<Value, OpenAIError>> + Send>>;

#[async_trait]
impl Agent for LlmAgent {
    #[allow(clippy::indexing_slicing)]
    async fn step(&self, state: &State) -> Result<AgentResponse, AgentError> {
        let messages = state_to_messages(state);
        let (text, reasoning) = self.call_llm(&messages).await?;

        let mut resp = parse_response(&text).with_reasoning(&reasoning);

        info!(
            "llm step: parsed {} segments, task {} bytes",
            resp.segments.len(),
            resp.task.len()
        );

        let edit_violations = check_edit_violations(&resp.segments, &state.outputs);
        let needs_task = resp.task.is_empty();
        let has_heredoc_violations = !resp.heredoc_violations.is_empty();
        let has_edit_violations = !edit_violations.is_empty() || !resp.edit_parse_errors.is_empty();

        if needs_task || has_heredoc_violations || has_edit_violations {
            let mut re_messages = messages;
            re_messages.push(ChatCompletionRequestAssistantMessage::from(text.as_str()).into());
            let mut complaint = String::new();
            if needs_task {
                complaint.push_str(
                    "You forgot to include a `task` block. \
                     You MUST reply with a ```task block containing your current status and plan for the next tick. \
                     Include: Known, Resolved, Context, Do. \
                     It will be APPENDED to your previous answer, so you don't need to duplicate all other commands.",
                );
            }
            if has_heredoc_violations {
                complaint.push_str(
                    "Write and edit blocks MUST use heredoc syntax: \
                     the content must start with <<'TAI' and end with TAI on its own line. \
                     Violations: ",
                );
                complaint.push_str(&resp.heredoc_violations.join(", "));
                complaint.push_str(". ");
            }
            if has_edit_violations {
                complaint.push_str(
                    "Edit block line references do not match the current file output, \
                     or edit commands could not be parsed. \
                     Each line reference must exactly match a line from the file view output. \
                     Violations: ",
                );
                let mut all: Vec<String> = resp.edit_parse_errors.clone();
                all.extend(edit_violations);
                complaint.push_str(&all.join(", "));
                complaint.push_str(". ");
            }
            complaint.push_str("You may also include action blocks if needed.");
            re_messages.push(ChatCompletionRequestUserMessage::from(complaint.as_str()).into());
            let (re_text, re_reasoning) = self.call_llm(&re_messages).await?;
            let re_resp = parse_response(&re_text).with_reasoning(&re_reasoning);
            if !re_resp.task.is_empty() {
                resp.task = re_resp.task;
            }
            let fixed_segments: Vec<ParsedBlock> = re_resp
                .segments
                .into_iter()
                .filter(|s| !s.mode.requires_heredoc() || !s.content.contains("```") || re_resp.heredoc_violations.is_empty())
                .collect();
            resp.segments.extend(fixed_segments);
            info!(
                "llm step: re-prompt done, task {} bytes",
                resp.task.len()
            );
        }

        if self.debug {
            let s = toml::to_string_pretty(&resp).unwrap_or_else(|e| e.to_string());
            debug!("Response: {s}");
        }

        Ok(resp)
    }
}

impl LlmAgent {
    #[allow(clippy::indexing_slicing)]
    async fn call_llm(
        &self,
        messages: &[ChatCompletionRequestMessage],
    ) -> Result<(String, String), AgentError> {
        info!("llm: building request for model '{}'", self.model);
        let request = CreateChatCompletionRequestArgs::default()
            .model(&self.model)
            .messages(messages.to_vec())
            .stream(true)
            .build()?;

        if self.debug {
            let s = toml::to_string(&request).unwrap_or_else(|e| e.to_string());
            info!("Send request: {s}");
        }

        info!("llm: creating stream...");
        let mut stream: MyStreamingType = match self.client.chat().create_stream_byot(request).await
        {
            Ok(s) => s,
            Err(e) => {
                error!("llm: failed to create stream: {e}");
                return Err(AgentError::Llm(e));
            }
        };

        let mut text = String::new();
        let mut reasoning = String::new();
        let mut thinks = false;

        while let Some(result) = stream.next().await {
            match result {
                Ok(res) => {
                    if let Some(content) = res["choices"][0]["delta"]["reasoning_content"].as_str()
                        && !content.is_empty()
                    {
                        thinks = true;
                        print!("{content}");
                        reasoning.push_str(content);
                    }

                    if let Some(content) = res["choices"][0]["delta"]["content"].as_str()
                        && !content.is_empty()
                    {
                        if thinks {
                            thinks = false;
                            println!("\n\nTHINK END\n");
                        }
                        print!("{content}");
                        text.push_str(content);
                    }
                }
                Err(e) => {
                    warn!("llm: stream error: {e:#?}");
                }
            }
        }
        println!("\nDONE");

        info!(
            "llm: stream complete, text {} bytes, reasoning {} bytes",
            text.len(),
            reasoning.len()
        );

        Ok((text, reasoning))
    }
}

#[must_use]
fn state_to_messages(state: &State) -> Vec<ChatCompletionRequestMessage> {
    let mut ms: Vec<ChatCompletionRequestMessage> =
        vec![ChatCompletionRequestSystemMessage::from(state.system.clone()).into()];

    let mut body = String::new();
    let (regular, dashboard): (Vec<_>, Vec<_>) =
        state.segments.iter().partition(|s| !s.dashboard);

    write_blocks(&mut body, &regular, state);

    write_blocks(&mut body, &dashboard, state);

    if !state.task.is_empty() {
        body.push_str("## Task\n");
        body.push_str(&state.task);
        body.push_str("\n\n");
    }

    body.push_str(&render_window_summary(state));

    ms.push(ChatCompletionRequestUserMessage::from(body).into());
    ms
}

fn write_blocks(to: &mut impl Write, segments: &[&ParsedBlock], state: &State) {
    for segment in segments {
        writeln!(to, "## [{}]", segment.window).ok();
        if let Some(output) = state.outputs.get(&segment.window) {
            if !output.stdout.is_empty() {
                write!(to, "{}", output.stdout).ok();
                if !output.stdout.ends_with('\n') {
                    writeln!(to).ok();
                }
            }
            write!(to, "exit {}\n\n", output.exit_code).ok();
        } else {
            writeln!(to, "UNKNOWN OUTPUT\n").ok();
        }
    }
}

fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

fn render_window_summary(state: &State) -> String {
    let mut body = String::from("== Windows ==\n");
    let mut total_tokens: usize = estimate_tokens(&state.system);
    if !state.task.is_empty() {
        total_tokens += estimate_tokens(&state.task);
    }

    for segment in &state.segments {
        if let Some(output) = state.outputs.get(&segment.window) {
            let tokens = estimate_tokens(&output.stdout);
            total_tokens += tokens;
            let is_dashboard = segment.dashboard;
            let tag = if is_dashboard { "dashboard" } else { "active" };
            Write::write_fmt(
                &mut body,
                format_args!("{}: ~{} tok ({})\n", segment.window, tokens, tag),
            ).ok();
        }
    }

    Write::write_fmt(
        &mut body,
        format_args!(
            "\n== Context: ~{total_tokens} tokens | Tick #{} ==",
            state.tick_n
        ),
    ).ok();
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::CmdOutput;
    use crate::types::{BlockMode, ParsedBlock};
    use std::collections::HashMap;

    #[test]
    fn test_check_edit_violations_no_edit_blocks() {
        let segments = vec![ParsedBlock {
            window: "build".into(),
            mode: BlockMode::Watch,
            content: "echo ok".into(),
            prose: None,
            dashboard: false,
        }];
        let outputs = HashMap::new();
        assert!(check_edit_violations(&segments, &outputs).is_empty());
    }

    #[test]
    fn test_check_edit_violations_missing_output() {
        let segments = vec![ParsedBlock {
            window: "main.rs".into(),
            mode: BlockMode::Edit(vec![]),
            content: "Change\nL1:foo\nbar\n.".into(),
            prose: None,
            dashboard: false,
        }];
        let outputs = HashMap::new();
        let violations = check_edit_violations(&segments, &outputs);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("edit commands not parsed"));
    }

    #[test]
    fn test_check_edit_violations_ok() {
        let cmds = edit_command::parse_edit_commands("Change\nL2:line two\nREPLACED\n.", None).unwrap();
        let segments = vec![ParsedBlock {
            window: "main.rs".into(),
            mode: BlockMode::Edit(cmds),
            content: String::new(),
            prose: None,
            dashboard: false,
        }];
        let mut outputs = HashMap::new();
        outputs.insert(
            "main.rs".into(),
            CmdOutput {
                exit_code: 0,
                stdout: "L1:line one\nL2:line two\nL3:line three\n".into(),
            },
        );
        assert!(check_edit_violations(&segments, &outputs).is_empty());
    }

    #[test]
    fn test_check_edit_violations_text_mismatch() {
        let cmds = edit_command::parse_edit_commands("Change\nL2:wrong line\nREPLACED\n.", None).unwrap();
        let segments = vec![ParsedBlock {
            window: "main.rs".into(),
            mode: BlockMode::Edit(cmds),
            content: String::new(),
            prose: None,
            dashboard: false,
        }];
        let mut outputs = HashMap::new();
        outputs.insert(
            "main.rs".into(),
            CmdOutput {
                exit_code: 0,
                stdout: "L1:line one\nL2:line two\nL3:line three\n".into(),
            },
        );
        let violations = check_edit_violations(&segments, &outputs);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("wrong line"));
    }
}
