// Berger: open-source email triage daemon.
// Copyright (C) 2026 Michel-Marie Maudet
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! LLM client: an OpenAI-compatible chat-completions client (PRD §5.3).

pub mod classifier;
pub mod error;

use serde::{Deserialize, Serialize};

use crate::llm::error::LlmError;

/// One message in an OpenAI-compatible chat completion.
#[derive(Debug, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

/// The chat-completion request body.
#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
}

/// The chat-completion response body — only the fields Berger reads.
#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
    /// Token usage, when the API reports it. Absent on endpoints that do
    /// not return an OpenAI-compatible `usage` block.
    #[serde(default)]
    usage: Option<Usage>,
}

/// One completion choice.
#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

/// The token-usage block of a chat-completion response.
#[derive(Debug, Deserialize)]
struct Usage {
    #[serde(default)]
    prompt_tokens: Option<i64>,
    #[serde(default)]
    completion_tokens: Option<i64>,
}

/// An LLM completion: the assistant's reply and the token usage the API
/// reported, when it reported any.
#[derive(Debug, Clone)]
pub struct Completion {
    /// The assistant's reply text.
    pub content: String,
    /// Prompt tokens billed, when the API reported `usage`.
    pub tokens_input: Option<i64>,
    /// Completion tokens billed, when the API reported `usage`.
    pub tokens_output: Option<i64>,
}

/// A client for an OpenAI-compatible chat-completions API (Mistral,
/// Ollama, …).
///
/// The API key is held inside the HTTP client as a header marked
/// sensitive and never appears in this type's `Debug` output.
pub struct LlmClient {
    http: reqwest::Client,
    endpoint: String,
    model: String,
}

impl std::fmt::Debug for LlmClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmClient")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl LlmClient {
    /// Builds a client for the chat-completions endpoint at `endpoint`,
    /// using `model`. `api_key`, when given, is sent as a Bearer
    /// credential — omit it for a local endpoint that needs no auth.
    ///
    /// # Errors
    /// Returns [`LlmError::Config`] if `api_key` cannot be encoded as an
    /// HTTP header, or [`LlmError::Transport`] if the HTTP client cannot
    /// be built.
    pub fn new(endpoint: &str, model: &str, api_key: Option<&str>) -> Result<Self, LlmError> {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(key) = api_key {
            let mut value = reqwest::header::HeaderValue::from_str(&format!("Bearer {key}"))
                .map_err(|_| {
                    LlmError::Config(
                        "the API key contains characters not valid in an HTTP header".to_string(),
                    )
                })?;
            value.set_sensitive(true);
            headers.insert(reqwest::header::AUTHORIZATION, value);
        }
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .default_headers(headers)
            .build()?;
        Ok(Self {
            http,
            endpoint: endpoint.to_string(),
            model: model.to_string(),
        })
    }

    /// Sends a system + user prompt and returns the assistant's reply,
    /// together with the token usage the API reported (when it reports any).
    ///
    /// # Errors
    /// Returns [`LlmError`] on a transport failure, a non-success status,
    /// an undecodable body, or a response carrying no completion.
    pub async fn complete(&self, system: &str, user: &str) -> Result<Completion, LlmError> {
        let messages = [
            ChatMessage {
                role: "system".to_string(),
                content: system.to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: user.to_string(),
            },
        ];
        let request = ChatRequest {
            model: self.model.as_str(),
            messages: &messages,
        };
        let response = self
            .http
            .post(self.endpoint.as_str())
            .json(&request)
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(LlmError::Api { status, body });
        }
        let parsed: ChatResponse =
            serde_json::from_str(&body).map_err(|error| LlmError::Decode(error.to_string()))?;
        let usage = parsed.usage;
        let content = parsed
            .choices
            .into_iter()
            .next()
            .map(|choice| choice.message.content)
            .ok_or(LlmError::EmptyResponse)?;
        let (tokens_input, tokens_output) = match usage {
            Some(usage) => (usage.prompt_tokens, usage.completion_tokens),
            None => (None, None),
        };
        Ok(Completion {
            content,
            tokens_input,
            tokens_output,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn complete_sends_the_prompt_and_returns_the_reply() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer k"))
            .and(body_json(json!({
                "model": "test-model",
                "messages": [
                    {"role": "system", "content": "system prompt"},
                    {"role": "user", "content": "user prompt"}
                ]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {"role": "assistant", "content": "the reply"}}]
            })))
            .mount(&server)
            .await;

        let endpoint = format!("{}/v1/chat/completions", server.uri());
        let client = LlmClient::new(&endpoint, "test-model", Some("k")).unwrap();
        let reply = client
            .complete("system prompt", "user prompt")
            .await
            .unwrap();

        assert_eq!(reply.content, "the reply");
    }

    #[tokio::test]
    async fn complete_returns_the_token_usage() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {"role": "assistant", "content": "ok"}}],
                "usage": {"prompt_tokens": 123, "completion_tokens": 45}
            })))
            .mount(&server)
            .await;

        let endpoint = format!("{}/v1/chat/completions", server.uri());
        let client = LlmClient::new(&endpoint, "m", None).unwrap();
        let completion = client.complete("s", "u").await.unwrap();

        assert_eq!(completion.content, "ok");
        assert_eq!(completion.tokens_input, Some(123));
        assert_eq!(completion.tokens_output, Some(45));
    }

    #[tokio::test]
    async fn complete_tolerates_a_response_without_usage() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {"role": "assistant", "content": "ok"}}]
            })))
            .mount(&server)
            .await;

        let endpoint = format!("{}/v1/chat/completions", server.uri());
        let client = LlmClient::new(&endpoint, "m", None).unwrap();
        let completion = client.complete("s", "u").await.unwrap();

        assert_eq!(completion.content, "ok");
        assert_eq!(completion.tokens_input, None);
        assert_eq!(completion.tokens_output, None);
    }

    #[tokio::test]
    async fn complete_maps_a_non_success_status() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("upstream error"))
            .mount(&server)
            .await;

        let endpoint = format!("{}/v1/chat/completions", server.uri());
        let client = LlmClient::new(&endpoint, "m", None).unwrap();

        assert!(matches!(
            client.complete("s", "u").await.unwrap_err(),
            LlmError::Api { .. }
        ));
    }

    #[tokio::test]
    async fn complete_reports_an_empty_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"choices": []})))
            .mount(&server)
            .await;

        let endpoint = format!("{}/v1/chat/completions", server.uri());
        let client = LlmClient::new(&endpoint, "m", None).unwrap();

        assert!(matches!(
            client.complete("s", "u").await.unwrap_err(),
            LlmError::EmptyResponse
        ));
    }

    #[test]
    fn new_rejects_a_bad_api_key() {
        let result = LlmClient::new("https://x/v1/chat/completions", "m", Some("bad\nkey"));
        assert!(matches!(result, Err(LlmError::Config(_))));
    }

    #[test]
    fn debug_output_does_not_leak_the_key() {
        let client =
            LlmClient::new("https://api.example/v1/chat", "m", Some("super-secret")).unwrap();
        assert!(!format!("{client:?}").contains("super-secret"));
    }
}
