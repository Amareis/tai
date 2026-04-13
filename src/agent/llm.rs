use super::{Agent, AgentError, AgentResponse};
use crate::prompt::{Prompt, TrackedView};
use crate::response::parse_response;
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
use std::collections::HashSet;
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
    async fn step(&self, prompt: &Prompt) -> Result<AgentResponse, AgentError> {
        info!("llm step: building request for model '{}'", self.model);
        let request = CreateChatCompletionRequestArgs::default()
            .model(&self.model)
            .messages(prompt_to_messages(prompt))
            .stream(true)
            .build()?;

        if self.debug {
            let s = toml::to_string(&request).unwrap_or_else(|e| e.to_string());
            info!("Send request: {s}");
        }

        info!("llm step: creating stream...");
        let mut stream: MyStreamingType = match self.client.chat().create_stream_byot(request).await {
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

        info!("llm step: stream complete, text {} bytes, reasoning {} bytes", text.len(), reasoning.len());

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
fn prompt_to_messages(prompt: &Prompt) -> Vec<ChatCompletionRequestMessage> {
    let mut ms: Vec<ChatCompletionRequestMessage> =
        vec![ChatCompletionRequestSystemMessage::from(prompt.system.clone()).into()];

    let prev_titles: HashSet<&str> = prompt
        .previous_response
        .as_ref()
        .map(|resp| {
            resp.segments
                .iter()
                .map(|b| b.window.as_str())
                .collect()
        })
        .unwrap_or_default();

    let tracked_map: std::collections::HashMap<&str, &TrackedView> = prompt
        .tracked
        .iter()
        .map(|v| (v.title.as_str(), v))
        .collect();

    // 1. Persistent windows (not from previous response)
    for view in &prompt.tracked {
        if !prev_titles.contains(view.title.as_str()) {
            ms.push(render_block_assistant(&view.title, BlockMode::View, "", None));
            ms.push(render_result_user(view));
        }
    }

    // 2. Previous response blocks (interleaved assistant/user)
    if let Some(prev) = &prompt.previous_response {
        for block in &prev.segments {
            ms.push(render_block_assistant(
                &block.window,
                block.mode,
                &block.content,
                block.prose.as_deref(),
            ));
            if let Some(view) = tracked_map.get(block.window.as_str()) {
                ms.push(render_result_user(view));
            }
        }

        // 3. Outro (trailing prose)
        if let Some(outro) = &prev.outro {
            ms.push(ChatCompletionRequestAssistantMessage::from(outro.clone()).into());
        }
    }

    // 4. Dashboard
    ms.push(render_dashboard(prompt));

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
            let _ = std::fmt::Write::write_fmt(&mut text, format_args!("```{window}:exec\n{content}\n```"));
        }
        BlockMode::View => {
            let _ = std::fmt::Write::write_fmt(&mut text, format_args!("```{window}\n{content}\n```"));
        }
    }
    ChatCompletionRequestAssistantMessage::from(text).into()
}

fn render_result_user(view: &TrackedView) -> ChatCompletionRequestMessage {
    let mut body = String::new();
    body.push_str(&view.output);
    if !view.output.ends_with('\n') && !view.output.is_empty() {
        body.push('\n');
    }
    let _ = std::fmt::Write::write_fmt(&mut body, format_args!("exit {}", view.exit_code));
    ChatCompletionRequestUserMessage::from(body).into()
}

fn render_dashboard(prompt: &Prompt) -> ChatCompletionRequestMessage {
    let mut body = String::from("== Windows ==\n");
    for view in &prompt.tracked {
        let lines = view.output.lines().count();
        let mode = if view.rerun { "view" } else { "exec" };
        let status = if view.rerun { "rerun" } else { "cached" };
        let _ = std::fmt::Write::write_fmt(
            &mut body,
            format_args!("{}: {} lines, {} ({})\n", view.title, lines, mode, status),
        );
    }
    let _ = std::fmt::Write::write_fmt(&mut body, format_args!("\n== Tick #{} ==", prompt.tick_n));
    ChatCompletionRequestUserMessage::from(body).into()
}
