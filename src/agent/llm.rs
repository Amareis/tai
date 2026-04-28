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
use tracing::{error, info, warn};

pub struct LlmAgent {
    model: String,
    client: Client<OpenAIConfig>,
    pub tui: bool,
}

fn check_edit_violations(
    segments: &[ParsedBlock],
    outputs: &std::collections::HashMap<String, crate::backend::CmdOutput>,
) -> Vec<String> {
    let mut violations = Vec::new();
    for block in segments {
        if let BlockMode::Edit(ref cmd_opt) = block.mode {
            if cmd_opt.is_none() {
                violations.push(format!(
                    "edit:{} — edit command not parsed.",
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
            if let Some(cmd) = cmd_opt.as_ref()
                && let Err(e) = edit_command::validate_texts_against_output(cmd, &output.stdout)
            {
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
            tui: false,
        }
    }
}

type MyStreamingType = Pin<Box<dyn Stream<Item = Result<Value, OpenAIError>> + Send>>;

const MAX_RETRIES: usize = 3;

#[async_trait]
impl Agent for LlmAgent {
    #[allow(clippy::too_many_lines)]
    async fn step(&self, state: &State) -> Result<AgentResponse, AgentError> {
        let messages = state_to_messages(state);
        let (text, reasoning) = self.call_llm(&messages).await?;

        let mut resp = parse_response(&text).with_reasoning(&reasoning);

        info!(
            "llm step: parsed {} segments, task {} bytes",
            resp.segments.len(),
            resp.task.len()
        );
        let mut re_messages = messages;
        re_messages.push(ChatCompletionRequestAssistantMessage::from(text.as_str()).into());

        for attempt in 1..=MAX_RETRIES {
            let edit_violations = check_edit_violations(&resp.segments, &state.outputs);
            let has_heredoc_violations = !resp.heredoc_violations.is_empty();
            let has_edit_violations =
                !edit_violations.is_empty() || !resp.edit_parse_errors.is_empty();

            if !has_heredoc_violations && !has_edit_violations {
                break;
            }

            let mut complaint = String::new();
            if has_heredoc_violations {
                complaint.push_str(
                    "Write and edit blocks MUST use heredoc syntax: \
                     the content must start with <<'TAIDELIM' and end with TAIDELIM on its own line. \
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

            info!(
                "llm step: re-prompt attempt {}/{}, parsed {} segments",
                attempt,
                MAX_RETRIES,
                re_resp.segments.len()
            );

            if !re_resp.task.is_empty() {
                resp.task = re_resp.task;
            }

            // Collect windows that still have parse/heredoc errors in the re-response
            let bad_re_windows: std::collections::HashSet<String> = re_resp
                .heredoc_violations
                .iter()
                .chain(&re_resp.edit_parse_errors)
                .filter_map(|e| {
                    // e.g. "write:readme.md — ..." or "edit:src/main.rs — ..."
                    let prefix = e.split_once(" — ")?.0;
                    let (_, window) = prefix.split_once(':')?;
                    Some(window.to_string())
                })
                .collect();

            // Remove original segments for windows addressed by the re-response
            let re_windows: std::collections::HashSet<String> =
                re_resp.segments.iter().map(|s| s.window.clone()).collect();
            resp.segments.retain(|s| !re_windows.contains(&s.window));

            // Add clean segments from the re-response
            resp.segments.extend(
                re_resp
                    .segments
                    .into_iter()
                    .filter(|s| !bad_re_windows.contains(&s.window)),
            );

            // Update violations for next check
            resp.heredoc_violations = re_resp.heredoc_violations;
            resp.edit_parse_errors = re_resp.edit_parse_errors;

            re_messages.push(ChatCompletionRequestAssistantMessage::from(re_text.as_str()).into());
        }

        // Final validation after all retries
        let final_edit_violations = check_edit_violations(&resp.segments, &state.outputs);
        let final_has_heredoc = !resp.heredoc_violations.is_empty();
        let final_has_edit =
            !final_edit_violations.is_empty() || !resp.edit_parse_errors.is_empty();

        if final_has_heredoc || final_has_edit {
            let mut all_violations: Vec<String> = Vec::new();
            all_violations.extend(resp.heredoc_violations);
            all_violations.extend(resp.edit_parse_errors);
            all_violations.extend(final_edit_violations);
            return Err(AgentError::InvalidResponse(all_violations));
        }

        if !resp.complete {
            info!("llm step: generating task summary...");
            let task_messages = build_task_messages(state, &resp);
            match self.call_llm(&task_messages).await {
                Ok((task_text, _)) => {
                    info!("llm step: task summary generated ({} bytes)", task_text.len());
                    resp.task = task_text.trim().to_string();
                }
                Err(e) => {
                    warn!("failed to generate task summary: {e}");
                }
            }
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
                        if self.tui {
                            print!("{content}");
                        }
                        reasoning.push_str(content);
                    }

                    if let Some(content) = res["choices"][0]["delta"]["content"].as_str()
                        && !content.is_empty()
                    {
                        if thinks {
                            thinks = false;
                            if self.tui {
                                println!("\n\nTHINK END\n");
                            }
                        }
                        if self.tui {
                            print!("{content}");
                        }
                        text.push_str(content);
                    }
                }
                Err(e) => {
                    warn!("llm: stream error: {e:#?}");
                }
            }
        }
        if self.tui {
            println!("\nDONE");
        }

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

    let (regular, dashboard): (Vec<_>, Vec<_>) =
        state.segments.iter().partition(|s| !s.dashboard);

    for segment in regular {
        let agent_text = crate::response::serialize_blocks(std::slice::from_ref(segment));
        if !agent_text.trim().is_empty() {
            ms.push(ChatCompletionRequestAssistantMessage::from(agent_text).into());
        }
        ms.push(build_user_window_message(segment, state));
    }

    for segment in dashboard {
        let agent_text = crate::response::serialize_blocks(std::slice::from_ref(segment));
        if !agent_text.trim().is_empty() {
            ms.push(ChatCompletionRequestAssistantMessage::from(agent_text).into());
        }
        ms.push(build_user_window_message(segment, state));
    }

    let mut body = String::new();
    if !state.task.is_empty() {
        body.push_str("## Task\n");
        body.push_str(&state.task);
        body.push_str("\n\n");
    }
    body.push_str(&render_window_summary(state));
    body.push_str("\n\n");
    body.push_str(&state.instructions);

    ms.push(ChatCompletionRequestUserMessage::from(body).into());
    ms
}

fn build_user_window_message(segment: &ParsedBlock, state: &State) -> ChatCompletionRequestMessage {
    let mut text = String::new();
    writeln!(text, "## [{}]", segment.window).ok();
    if let Some(output) = state.outputs.get(&segment.window) {
        if !output.stdout.is_empty() {
            write!(text, "{}", output.stdout).ok();
            if !output.stdout.ends_with('\n') {
                writeln!(text).ok();
            }
        }
        write!(text, "exit {}\n\n", output.exit_code).ok();
    } else {
        writeln!(text, "UNKNOWN OUTPUT\n").ok();
    }
    ChatCompletionRequestUserMessage::from(text).into()
}

fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

fn render_window_summary(state: &State) -> String {
    let mut body = String::from("== Windows ==\n");
    let mut total_tokens: usize = estimate_tokens(&state.system);
    total_tokens += estimate_tokens(&state.instructions);
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

#[must_use]
fn build_task_messages(state: &State, resp: &AgentResponse) -> Vec<ChatCompletionRequestMessage> {
    let mut context = String::new();

    if !state.outputs.is_empty() {
        let _ = writeln!(context, "## Known results from previous ticks");
        for (key, out) in &state.outputs {
            let status = if out.exit_code == 0 { "OK" } else { "FAIL" };
            let preview: String = out.stdout.chars().take(80).collect();
            let _ = writeln!(
                context,
                "- {}: exit {} ({}, {} chars) — {}",
                key, out.exit_code, status, out.stdout.len(), preview
            );
        }
        let _ = writeln!(context);
    }

    if !state.task.is_empty() {
        let _ = writeln!(context, "## Previous task (for context only)\n{}\n", state.task);
    }

    if !resp.reasoning.is_empty() {
        let _ = writeln!(context, "## Agent reasoning this tick\n{}\n", resp.reasoning);
    }

    if !resp.segments.is_empty() {
        let _ = writeln!(
            context,
            "## Agent's full raw response (prose + commands)\n``````markdown\n{}\n``````\n",
            crate::response::serialize_blocks(&resp.segments)
        );
    }

    let prompt = r#"You are the navigator in a pair programming session with an AI agent (the driver). The driver has zero memory between ticks and relies entirely on your task note.

The driver operates in ticks:
1. Wakes up with ZERO memory. Sees only your task note + current system state.
2. Reasons and decides what to do.
3. Sends commands.
4. Commands execute. Results appear on the NEXT tick.
5. You see the driver's full response (prose + commands) and known results.

Your job: write the task/status note that the driver will see at step 1 of the next tick.

Format (use ALL sections):
Goal: [original goal from Previous task — preserve exactly]
Plan:
[x] completed subtask
[ ] pending subtask
[ ] pending subtask
Log: [last 3-5 actions across ticks; collapse older ones into summaries like "Ticks 2-4: explored codebase"]
Done: [what was accomplished this tick]
Known: [confirmed facts and state]
Next: [concrete next step]

Rules:
- Plan is your todo list / roadmap. Update it every tick. Mark completed items [x], add new items as they emerge, remove irrelevant ones.
- Log is a rolling window of recent driver actions. Keep 3-5 most recent entries. When older entries become irrelevant, collapse them into a single summary line.
- Detect circular trajectories using Log. If the driver repeats an action already logged, call it out explicitly and propose a different angle.
- Question flawed assumptions. If the driver's reasoning is wrong, say so and explain why.
- Commands the driver JUST sent have NOT executed yet. Their results are unknown.
- Be specific. Write enough to capture the essential context, but keep it concise.
- If the original goal has been fully achieved, say so explicitly and set the next step to "Finish — use task:complete"."#;

    let mut messages: Vec<ChatCompletionRequestMessage> = vec![
        ChatCompletionRequestSystemMessage::from("You are the navigator in a pair programming session with an AI agent (the driver). The driver has zero memory between ticks and relies entirely on your task note. You see the driver's reasoning, prose, and commands, plus brief previews of command outputs (not full files or large outputs). Your job is to track the overall strategy, detect circular reasoning, question flawed assumptions, and set a clear task for the next tick. You are NOT the driver. Do not write code or commands.").into(),
        ChatCompletionRequestUserMessage::from(context).into(),
    ];

    if !resp.segments.is_empty() {
        let raw = crate::response::serialize_blocks(&resp.segments);
        messages.push(ChatCompletionRequestAssistantMessage::from(raw.as_str()).into());
    }

    messages.push(ChatCompletionRequestUserMessage::from(prompt).into());
    messages
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
            mode: BlockMode::Edit(None),
            content: "Change\nL1:foo\nbar\n.".into(),
            prose: None,
            dashboard: false,
        }];
        let outputs = HashMap::new();
        let violations = check_edit_violations(&segments, &outputs);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("edit command not parsed"));
    }

    #[test]
    fn test_check_edit_violations_ok() {
        let cmd = edit_command::parse_edit_command("Exactly L2:line two\n<<'TAIDELIM'\nREPLACED\nTAIDELIM").unwrap();
        let segments = vec![ParsedBlock {
            window: "main.rs".into(),
            mode: BlockMode::Edit(Some(cmd)),
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
        let cmd = edit_command::parse_edit_command("Exactly L2:wrong line\n<<'TAIDELIM'\nREPLACED\nTAIDELIM").unwrap();
        let segments = vec![ParsedBlock {
            window: "main.rs".into(),
            mode: BlockMode::Edit(Some(cmd)),
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
