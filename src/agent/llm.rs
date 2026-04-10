use super::{Agent, AgentError, AgentResponse};
use crate::prompt::{Prompt, WindowView};
use crate::response::parse_response;
use crate::types::ParsedSegment;
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
use tracing::error;

pub struct LlmAgent {
    client: Client<OpenAIConfig>,
}

impl Default for LlmAgent {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmAgent {
    #[must_use]
    pub fn new() -> Self {
        Self {
            client: Client::default(),
        }
    }
}

type MyStreamingType = Pin<Box<dyn Stream<Item = Result<Value, OpenAIError>> + Send>>;

#[async_trait]
impl Agent for LlmAgent {
    #[allow(clippy::indexing_slicing)]
    async fn step(&self, prompt: &Prompt) -> Result<AgentResponse, AgentError> {
        let request = CreateChatCompletionRequestArgs::default()
            .model("glm-5.1")
            .messages(prompt_to_messages(prompt))
            .stream(true)
            .build()?;

        let mut stream: MyStreamingType = self.client.chat().create_stream_byot(request).await?;

        let mut text = String::new();
        let mut thinks = true;

        while let Some(result) = stream.next().await {
            match result {
                Ok(res) => {
                    if let Some(content) = res["choices"][0]["delta"]["reasoning_content"].as_str()
                    {
                        print!("{content}");
                    }

                    if let Some(content) = res["choices"][0]["delta"]["content"].as_str() {
                        if thinks {
                            thinks = false;
                            println!("\n\nTHINK END\n");
                        }
                        print!("{content}");
                        text.push_str(content);
                    }
                }
                Err(e) => {
                    error!("{e:#?}");
                }
            }
        }

        Ok(AgentResponse {
            segments: parse_response(&text),
        })
    }
}

#[must_use]
fn prompt_to_messages(prompt: &Prompt) -> Vec<ChatCompletionRequestMessage> {
    let mut ms: Vec<ChatCompletionRequestMessage> =
        vec![ChatCompletionRequestSystemMessage::from(prompt.system.clone()).into()];

    for w in &prompt.focused_windows {
        ms.push(render_window(w));
    }

    if let Some(prev) = &prompt.previous_response {
        let mut parts: Vec<String> = vec![];
        parts.push("## Your previous response".to_string());
        parts.push(
            prev.segments
                .iter()
                .map(|s| match s {
                    ParsedSegment::Block {
                        window,
                        content,
                        mode,
                    } => {
                        format!("```{window}:{mode}\n{content}```")
                    }
                    ParsedSegment::Prose(t) => t.clone(),
                    ParsedSegment::Reasoning(t) => format!("```REASONING\n{t}```"),
                })
                .collect(),
        );
        ms.push(ChatCompletionRequestAssistantMessage::from(parts.join("\n")).into());
    }

    ms.push(render_dashboard(prompt));

    ms
}

fn render_dashboard(prompt: &Prompt) -> ChatCompletionRequestMessage {
    let mut lines = vec!["## Dashboard".to_string()];

    if prompt.dashboard.is_empty() {
        lines.push("No windows.".to_string());
    } else {
        lines.push(format!("Opened {} terminals: ", { prompt.dashboard.len() }));
        lines.extend(prompt.dashboard.iter().map(|w| {
            format!(
                "id {} | {} | pid {} | prompt: {}",
                w.id, w.title, w.pid, w.is_at_prompt
            )
        }));
    }
    ChatCompletionRequestUserMessage::from(lines.join("\n")).into()
}

fn render_window(w: &WindowView) -> ChatCompletionRequestMessage {
    let exit_info = w
        .exit_code
        .map_or(String::new(), |c| format!("\n**Exit code: {c}**"));
    ChatCompletionRequestUserMessage::from(format!(
        "## Window [{}] {} (focused)\n{}{}\n```\n{}\n```",
        w.id,
        w.title,
        exit_info,
        if exit_info.is_empty() { "" } else { "\n" },
        w.content.trim()
    ))
    .into()
}
