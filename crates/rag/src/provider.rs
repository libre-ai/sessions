//! The AI provider seam: embeddings + chat completion behind a trait, so the
//! product is decoupled from any single vendor. [`OpenAiCompatible`] is limited
//! to loopback development or explicitly enabled Clever AI routing;
//! [`FakeAiProvider`] is a deterministic stand-in for tests.

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

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
    /// Chat completion that asks for a JSON-object response (structured output).
    /// Defaults to [`Self::complete`]; HTTP providers set `response_format`.
    async fn complete_json(&self, system: &str, user: &str) -> Result<String, AiError> {
        self.complete(system, user).await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProviderRoute {
    LocalLoopback,
    CleverAi { contract_ref: String },
}

/// OpenAI-wire-compatible transport with a closed provider policy.
///
/// Hosted routing is Clever AI only and remains disabled unless explicitly
/// enabled with a contract reference. Local development accepts loopback only.
pub struct OpenAiCompatible {
    base_url: String,
    api_key: String,
    embed_model: String,
    chat_model: String,
    /// Request `response_format: json_object` on JSON completions. Hosted mode
    /// keeps it enabled; loopback development may set `LOCAL_AI_JSON_MODE=0`.
    json_mode: bool,
    route: ProviderRoute,
    http: reqwest::Client,
}

#[derive(Debug, Clone, Copy)]
enum ProviderKind {
    Local,
    CleverAi,
}

fn validated_origin(raw: String, kind: ProviderKind) -> Result<String, AiError> {
    let url = reqwest::Url::parse(raw.trim())
        .map_err(|_| AiError("provider endpoint is not a valid URL".into()))?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
    {
        return Err(AiError("provider endpoint must be an origin only".into()));
    }
    let host = url
        .host_str()
        .ok_or_else(|| AiError("provider endpoint has no host".into()))?;
    match kind {
        ProviderKind::Local => {
            let loopback = host.eq_ignore_ascii_case("localhost")
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|address| address.is_loopback());
            if !loopback || !matches!(url.scheme(), "http" | "https") {
                return Err(AiError("local AI endpoint must be loopback".into()));
            }
        }
        ProviderKind::CleverAi => {
            let host = host.to_ascii_lowercase();
            let approved_domain =
                host == "clever-cloud.com" || host.ends_with(".clever-cloud.com");
            if url.scheme() != "https"
                || host.parse::<std::net::IpAddr>().is_ok()
                || !approved_domain
            {
                return Err(AiError(
                    "hosted AI endpoint must be an HTTPS Clever AI host".into(),
                ));
            }
        }
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn safe_policy_reference(value: &str) -> bool {
    (1..=256).contains(&value.len())
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'/' | b'.')
        })
        && !value.contains("..")
        && !value.contains("://")
}

fn non_empty_key(key: String) -> Result<String, AiError> {
    if key.trim().is_empty() {
        Err(AiError("provider credential is empty".into()))
    } else {
        Ok(key)
    }
}

fn provider_http_client() -> Result<reqwest::Client, AiError> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|_| AiError("provider HTTP client could not be built".into()))
}

async fn decode_bounded_json<T: DeserializeOwned>(
    mut response: reqwest::Response,
) -> Result<T, AiError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(AiError("provider response exceeds 4 MiB".into()));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| AiError("provider response body failed".into()))?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(AiError("provider response exceeds 4 MiB".into()));
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| AiError("provider response is invalid JSON".into()))
}

fn non_empty_model(model: String) -> Result<String, AiError> {
    if model.trim().is_empty() {
        Err(AiError("provider model identifier is empty".into()))
    } else {
        Ok(model)
    }
}

impl OpenAiCompatible {
    pub fn new_local(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        embed_model: impl Into<String>,
        chat_model: impl Into<String>,
    ) -> Result<Self, AiError> {
        let base_url = validated_origin(base_url.into(), ProviderKind::Local)?;
        Ok(Self {
            base_url,
            api_key: non_empty_key(api_key.into())?,
            embed_model: non_empty_model(embed_model.into())?,
            chat_model: non_empty_model(chat_model.into())?,
            json_mode: true,
            route: ProviderRoute::LocalLoopback,
            http: provider_http_client()?,
        })
    }

    /// Build a loopback-only development provider. This route cannot be used
    /// for a hosted endpoint.
    pub fn from_local_env() -> Result<Self, AiError> {
        if std::env::var("LOCAL_AI_ENABLED").as_deref() != Ok("1") {
            return Err(AiError("local AI routing is disabled".into()));
        }
        let base = std::env::var("LOCAL_AI_BASE_URL")
            .map_err(|_| AiError("set LOCAL_AI_BASE_URL".into()))?;
        let key = std::env::var("LOCAL_AI_API_KEY").unwrap_or_else(|_| "local-only".into());
        let embed = std::env::var("LOCAL_AI_EMBED_MODEL")
            .map_err(|_| AiError("set LOCAL_AI_EMBED_MODEL".into()))?;
        let chat = std::env::var("LOCAL_AI_CHAT_MODEL")
            .map_err(|_| AiError("set LOCAL_AI_CHAT_MODEL".into()))?;
        let mut provider = Self::new_local(base, key, embed, chat)?;
        provider.json_mode = !matches!(
            std::env::var("LOCAL_AI_JSON_MODE").as_deref(),
            Ok("0") | Ok("false") | Ok("no")
        );
        Ok(provider)
    }

    /// Build the only hosted route from environment. No network call occurs
    /// during construction. `CLEVER_AI_ENABLED=1` and a non-secret, versioned
    /// `CLEVER_AI_CONTRACT_REF` are mandatory kill-switch inputs.
    pub fn from_env() -> Result<Self, AiError> {
        if std::env::var("CLEVER_AI_ENABLED").as_deref() != Ok("1") {
            return Err(AiError("Clever AI hosted routing is disabled".into()));
        }
        let base = std::env::var("CLEVER_AI_BASE_URL")
            .map_err(|_| AiError("set CLEVER_AI_BASE_URL".into()))?;
        let key = std::env::var("CLEVER_AI_API_KEY")
            .map_err(|_| AiError("set CLEVER_AI_API_KEY".into()))?;
        let contract_ref = std::env::var("CLEVER_AI_CONTRACT_REF")
            .map_err(|_| AiError("set CLEVER_AI_CONTRACT_REF".into()))?;
        if !safe_policy_reference(&contract_ref) {
            return Err(AiError("Clever AI contract reference is invalid".into()));
        }
        let embed = std::env::var("CLEVER_AI_EMBED_MODEL")
            .map_err(|_| AiError("set CLEVER_AI_EMBED_MODEL".into()))?;
        let chat = std::env::var("CLEVER_AI_CHAT_MODEL")
            .map_err(|_| AiError("set CLEVER_AI_CHAT_MODEL".into()))?;
        Ok(Self {
            base_url: validated_origin(base, ProviderKind::CleverAi)?,
            api_key: non_empty_key(key)?,
            embed_model: non_empty_model(embed)?,
            chat_model: non_empty_model(chat)?,
            json_mode: true,
            route: ProviderRoute::CleverAi { contract_ref },
            http: provider_http_client()?,
        })
    }

    pub fn provider_policy_ref(&self) -> &str {
        match &self.route {
            ProviderRoute::LocalLoopback => "provider:local-loopback:v1",
            ProviderRoute::CleverAi { contract_ref } => contract_ref,
        }
    }

    async fn chat(&self, system: &str, user: &str, json_object: bool) -> Result<String, AiError> {
        if system.len().saturating_add(user.len()) > 1024 * 1024 {
            return Err(AiError("completion input exceeds 1 MiB".into()));
        }
        let response_format = json_object.then_some(ResponseFormat {
            kind: "json_object",
        });
        let response = self
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
                response_format,
            })
            .send()
            .await
            .map_err(|e| AiError(e.to_string()))?
            .error_for_status()
            .map_err(|e| AiError(e.to_string()))?;
        let decoded: ChatResponse = decode_bounded_json(response).await?;
        decoded
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| AiError("no choices returned".into()))
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
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
}

#[derive(Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    kind: &'static str,
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
        let input_bytes = texts
            .iter()
            .try_fold(0usize, |total, text| total.checked_add(text.len()))
            .ok_or_else(|| AiError("embedding input size overflow".into()))?;
        if texts.len() > 256 || input_bytes > 1024 * 1024 {
            return Err(AiError("embedding input exceeds policy budget".into()));
        }
        let response = self
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
            .map_err(|e| AiError(e.to_string()))?;
        let decoded: EmbedResponse = decode_bounded_json(response).await?;
        Ok(decoded.data.into_iter().map(|d| d.embedding).collect())
    }

    async fn complete(&self, system: &str, user: &str) -> Result<String, AiError> {
        self.chat(system, user, false).await
    }

    async fn complete_json(&self, system: &str, user: &str) -> Result<String, AiError> {
        self.chat(system, user, self.json_mode).await
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

    #[test]
    fn endpoint_policy_rejects_non_loopback_local_and_direct_providers() {
        assert!(OpenAiCompatible::new_local("https://example.eu", "key", "embed", "chat").is_err());
        assert!(
            validated_origin("https://api.mistral.ai".to_string(), ProviderKind::CleverAi).is_err()
        );
        assert!(
            validated_origin("https://api.openai.com".to_string(), ProviderKind::CleverAi).is_err()
        );
        assert!(
            validated_origin(
                "https://notclever.example".to_string(),
                ProviderKind::CleverAi
            )
            .is_err()
        );
        assert_eq!(
            validated_origin(
                "https://ai.clever-cloud.com".to_string(),
                ProviderKind::CleverAi
            )
            .unwrap(),
            "https://ai.clever-cloud.com"
        );
    }

    #[test]
    fn local_provider_exposes_non_secret_policy_reference() {
        let provider = OpenAiCompatible::new_local(
            "http://127.0.0.1:8080",
            "secret-not-logged",
            "embed",
            "chat",
        )
        .expect("loopback route");
        assert_eq!(provider.provider_policy_ref(), "provider:local-loopback:v1");
    }

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
