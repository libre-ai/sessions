//! The AI provider seam: embeddings + chat completion behind a trait, so the
//! product is decoupled from any single vendor. [`OpenAiCompatible`] talks to any
//! OpenAI-compatible endpoint (Clever AI by default, BYO key); [`FakeAiProvider`]
//! is a deterministic stand-in for tests.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// An AI call failure. Never carries the API key.
#[derive(Debug)]
pub struct AiError(pub String);

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ai provider error: {}", self.0)
    }
}

impl std::error::Error for AiError {}

/// Embeddings and chat completion, abstracted over the vendor.
#[async_trait]
pub trait AiProvider: Send + Sync {
    /// Embed each input text into a vector.
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, AiError>;
    /// Chat completion: a system prompt + a user prompt → the assistant's text.
    async fn complete(&self, system: &str, user: &str) -> Result<String, AiError>;
}

/// A client for any OpenAI-compatible endpoint (Clever AI, Mistral, …).
pub struct OpenAiCompatible {
    base_url: String,
    api_key: String,
    embed_model: String,
    chat_model: String,
    http: reqwest::Client,
}

impl OpenAiCompatible {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        embed_model: impl Into<String>,
        chat_model: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            embed_model: embed_model.into(),
            chat_model: chat_model.into(),
            http: reqwest::Client::new(),
        }
    }

    /// Build from env: `AI_BASE_URL`, `AI_API_KEY`, and optional
    /// `AI_EMBED_MODEL` / `AI_CHAT_MODEL`. The key never appears in logs.
    pub fn from_env() -> Result<Self, AiError> {
        let base = std::env::var("AI_BASE_URL").map_err(|_| AiError("set AI_BASE_URL".into()))?;
        let key = std::env::var("AI_API_KEY").map_err(|_| AiError("set AI_API_KEY".into()))?;
        let embed =
            std::env::var("AI_EMBED_MODEL").unwrap_or_else(|_| "text-embedding-3-small".into());
        let chat = std::env::var("AI_CHAT_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into());
        Ok(Self::new(base, key, embed, chat))
    }
}

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedDatum>,
}

#[derive(Deserialize)]
struct EmbedDatum {
    embedding: Vec<f32>,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatChoiceMessage,
}

#[derive(Deserialize)]
struct ChatChoiceMessage {
    content: String,
}

#[async_trait]
impl AiProvider for OpenAiCompatible {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, AiError> {
        let response: EmbedResponse = self
            .http
            .post(format!("{}/v1/embeddings", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&EmbedRequest {
                model: &self.embed_model,
                input: texts,
            })
            .send()
            .await
            .map_err(|e| AiError(e.to_string()))?
            .error_for_status()
            .map_err(|e| AiError(e.to_string()))?
            .json()
            .await
            .map_err(|e| AiError(e.to_string()))?;
        Ok(response.data.into_iter().map(|d| d.embedding).collect())
    }

    async fn complete(&self, system: &str, user: &str) -> Result<String, AiError> {
        let response: ChatResponse = self
            .http
            .post(format!("{}/v1/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&ChatRequest {
                model: &self.chat_model,
                messages: vec![
                    ChatMessage {
                        role: "system",
                        content: system,
                    },
                    ChatMessage {
                        role: "user",
                        content: user,
                    },
                ],
            })
            .send()
            .await
            .map_err(|e| AiError(e.to_string()))?
            .error_for_status()
            .map_err(|e| AiError(e.to_string()))?
            .json()
            .await
            .map_err(|e| AiError(e.to_string()))?;
        response
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| AiError("no choices returned".into()))
    }
}

/// Deterministic, offline stand-in for tests: embeddings derived from byte
/// buckets (so similar text yields similar vectors), and a canned completion.
pub struct FakeAiProvider;

impl FakeAiProvider {
    pub const DIM: usize = 8;

    fn embed_one(text: &str) -> Vec<f32> {
        let mut v = vec![0f32; Self::DIM];
        for (i, b) in text.bytes().enumerate() {
            v[i % Self::DIM] += f32::from(b) / 255.0;
        }
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }
        v
    }
}

#[async_trait]
impl AiProvider for FakeAiProvider {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, AiError> {
        Ok(texts.iter().map(|t| Self::embed_one(t)).collect())
    }

    async fn complete(&self, _system: &str, user: &str) -> Result<String, AiError> {
        Ok(format!(
            "FAKE: {}",
            user.chars().take(40).collect::<String>()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fake_embed_is_deterministic_and_dimensioned() {
        let p = FakeAiProvider;
        let a = p.embed(&["hello".into(), "world".into()]).await.unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].len(), FakeAiProvider::DIM);
        let again = p.embed(&["hello".into()]).await.unwrap();
        assert_eq!(a[0], again[0], "embeddings are deterministic");
    }

    #[tokio::test]
    async fn fake_complete_returns_nonempty_text() {
        let p = FakeAiProvider;
        let out = p
            .complete("you are a quiz writer", "make a question")
            .await
            .unwrap();
        assert!(!out.is_empty());
    }
}
