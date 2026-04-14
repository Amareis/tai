use super::{Agent, AgentError, AgentResponse};
use crate::backend::CmdOutput;
use crate::response::parse_response;
use crate::state::State;
use crate::types::BlockMode;
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
use std::pin::Pin;
use tracing::{debug, error, info, warn};

pub struct LlmAgent {
    model: String,
    client: Client<OpenAIConfig>,
    pub debug: bool,
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
        info!("llm step: building request for model '{}'", self.model);
        let request = CreateChatCompletionRequestArgs::default()
            .model(&self.model)
            .messages(state_to_messages(state))
            .stream(true)
            .build()?;

        if self.debug {
            let s = toml::to_string(&request).unwrap_or_else(|e| e.to_string());
            info!("Send request: {s}");
        }

        info!("llm step: creating stream...");
        let mut stream: MyStreamingType = match self.client.chat().create_stream_byot(request).await
        {
            Ok(s) => s,
            Err(e) => {
                error!("llm step: failed to create stream: {e}");
                return Err(AgentError::Llm(e));
            }
        };

        let mut text = String::new();
        let mut reasoning = String::new();
        let mut thinks = false;

        info!("llm step: reading stream...");
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
                    warn!("llm step: stream error: {e:#?}");
                }
            }
        }
        println!("\nDONE");

        info!(
            "llm step: stream complete, text {} bytes, reasoning {} bytes",
            text.len(),
            reasoning.len()
        );

        let resp = parse_response(reasoning, &text);

        info!("llm step: parsed {} segments", resp.segments.len());

        if self.debug {
            let s = toml::to_string_pretty(&resp).unwrap_or_else(|e| e.to_string());
            debug!("Response: {s}");
        }

        Ok(resp)
    }
}

#[must_use]
fn state_to_messages(state: &State) -> Vec<ChatCompletionRequestMessage> {
    let mut ms: Vec<ChatCompletionRequestMessage> =
        vec![ChatCompletionRequestSystemMessage::from(state.system.clone()).into()];

    for segment in &state.segments {
        ms.push(render_block_assistant(
            &segment.window,
            segment.mode,
            &segment.content,
            segment.prose.as_deref(),
        ));

        if let Some(output) = state.outputs.get(&segment.window) {
            ms.push(render_output_user(output));
        }
    }

    ms.push(render_dashboard(state));

    ms
}

fn render_block_assistant(
    window: &str,
    mode: BlockMode,
    content: &str,
    prose: Option<&str>,
) -> ChatCompletionRequestMessage {
    let mut text = String::new();
    if let Some(p) = prose {
        text.push_str(p);
        text.push('\n');
    }
    match mode {
        BlockMode::Close => {
            let _ = std::fmt::Write::write_fmt(&mut text, format_args!("```{window}:close\n```"));
        }
        BlockMode::Exec => {
            let _ = std::fmt::Write::write_fmt(
                &mut text,
                format_args!("```{window}:exec\n{content}\n```"),
            );
        }
        BlockMode::View => {
            let _ =
                std::fmt::Write::write_fmt(&mut text, format_args!("```{window}\n{content}\n```"));
        }
        BlockMode::Ask => {
            let _ = std::fmt::Write::write_fmt(
                &mut text,
                format_args!("```{window}:ask\n{content}\n```"),
            );
        }
    }
    ChatCompletionRequestAssistantMessage::from(text).into()
}

fn render_output_user(output: &CmdOutput) -> ChatCompletionRequestMessage {
    let mut body = String::new();
    body.push_str(&output.stdout);
    if !output.stdout.ends_with('\n') && !output.stdout.is_empty() {
        body.push('\n');
    }
    let _ = std::fmt::Write::write_fmt(&mut body, format_args!("exit {}", output.exit_code));
    ChatCompletionRequestUserMessage::from(body).into()
}

fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

fn render_dashboard(state: &State) -> ChatCompletionRequestMessage {
    let mut body = String::from("== Windows ==\n");
    let mut total_tokens: usize = estimate_tokens(&state.system);

    for segment in &state.segments {
        if let Some(output) = state.outputs.get(&segment.window) {
            let lines = output.stdout.lines().count();
            let tokens = estimate_tokens(&output.stdout);
            total_tokens += tokens;
            let mode = match segment.mode {
                BlockMode::View => "view",
                BlockMode::Exec => "exec",
                BlockMode::Close => "close",
                BlockMode::Ask => "ask",
            };
            let cached = segment.mode == BlockMode::Exec || segment.mode == BlockMode::Ask;
            let status = if cached { "cached" } else { "rerun" };
            let _ = std::fmt::Write::write_fmt(
                &mut body,
                format_args!(
                    "{}: {} lines (~{} tok), {} ({})\n",
                    segment.window, lines, tokens, mode, status
                ),
            );
        } else {
            total_tokens += estimate_tokens(&segment.content);
            if let Some(prose) = &segment.prose {
                total_tokens += estimate_tokens(prose);
            }
        }
    }

    let _ = std::fmt::Write::write_fmt(
        &mut body,
        format_args!(
            "\n== Context: ~{total_tokens} tokens | Tick #{} ==",
            state.tick_n
        ),
    );
    ChatCompletionRequestUserMessage::from(body).into()
}
