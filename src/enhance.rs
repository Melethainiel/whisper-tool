use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::config::{Config, LlmProvider};

// ============================================================================
// Ollama API types
// ============================================================================

#[derive(Debug, Serialize)]
struct OllamaRequest {
    model: String,
    prompt: String,
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct OllamaResponse {
    response: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OllamaTagsResponse {
    models: Vec<OllamaModel>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OllamaModel {
    name: String,
}

// ============================================================================
// OpenAI-compatible API types (works with OpenAI, OpenRouter, Groq, etc.)
// ============================================================================

#[derive(Debug, Serialize)]
struct OpenAIRequest {
    model: String,
    messages: Vec<OpenAIMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAIMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct OpenAIResponse {
    choices: Vec<OpenAIChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAIChoice {
    message: OpenAIMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAIModelsResponse {
    data: Vec<OpenAIModelInfo>,
}

#[derive(Debug, Deserialize)]
struct OpenAIModelInfo {
    id: String,
}

// ============================================================================
// Anthropic API types
// ============================================================================

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<AnthropicMessage>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContent>,
}

#[derive(Debug, Deserialize)]
struct AnthropicContent {
    text: String,
}

/// Enhancement mode prompts - returns hardcoded defaults
fn get_default_prompt(mode: &str) -> &'static str {
    match mode {
        "clean" => {
            "You are a text editor. Fix spelling, grammar, and punctuation errors. \
             Keep the original meaning and tone. Output only the corrected text, nothing else."
        }
        "code" => {
            "You are a coding assistant. Transform the following voice dictation into a clear, \
             precise instruction or code comment suitable for code generation. \
             Be concise and technical. Output only the improved text, nothing else."
        }
        "email" => {
            "You are a professional email writer. Transform the following voice dictation \
             into a professional, well-structured email. Keep it concise and polite. \
             Output only the email text, nothing else."
        }
        "notes" => {
            "You are a note-taking assistant. Transform the following voice dictation \
             into clear, organized bullet points. Be concise. \
             Output only the bullet points, nothing else."
        }
        _ => {
            "You are a helpful assistant. Improve the following text while keeping \
             its original meaning. Output only the improved text, nothing else."
        }
    }
}

/// Get system prompt for a mode - uses config if available, falls back to defaults
fn get_system_prompt(mode: &str, config: &Config) -> String {
    config
        .prompts
        .templates
        .get(mode)
        .map(|t| t.prompt.clone())
        .unwrap_or_else(|| get_default_prompt(mode).to_string())
}

/// List available models from the configured provider (blocking)
#[allow(dead_code)]
pub fn list_models_blocking(provider: &LlmProvider, url: &str, api_key: Option<&str>) -> Result<Vec<String>> {
    match provider {
        LlmProvider::Ollama => list_ollama_models(url),
        LlmProvider::OpenAI | LlmProvider::OpenRouter => list_openai_models(url, api_key),
        LlmProvider::Anthropic => list_anthropic_models(),
    }
}

fn list_ollama_models(url: &str) -> Result<Vec<String>> {
    let api_url = format!("{}/api/tags", url.trim_end_matches('/'));
    debug!("Fetching Ollama models from: {}", api_url);

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .context("Failed to create HTTP client")?;

    let response = client
        .get(&api_url)
        .send()
        .context("Failed to connect to Ollama. Is it running?")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(anyhow!("Ollama API error: {} - {}", status, body));
    }

    let tags_response: OllamaTagsResponse = response
        .json()
        .context("Failed to parse Ollama tags response")?;

    let models: Vec<String> = tags_response
        .models
        .into_iter()
        .map(|m| m.name)
        .collect();

    debug!("Found {} Ollama models", models.len());
    Ok(models)
}

fn list_openai_models(url: &str, api_key: Option<&str>) -> Result<Vec<String>> {
    let api_url = format!("{}/v1/models", url.trim_end_matches('/'));
    debug!("Fetching OpenAI-compatible models from: {}", api_url);

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .context("Failed to create HTTP client")?;

    let mut request = client.get(&api_url);
    if let Some(key) = api_key {
        request = request.header("Authorization", format!("Bearer {}", key));
    }

    let response = request
        .send()
        .context("Failed to connect to OpenAI-compatible API")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(anyhow!("API error: {} - {}", status, body));
    }

    let models_response: OpenAIModelsResponse = response
        .json()
        .context("Failed to parse models response")?;

    let models: Vec<String> = models_response
        .data
        .into_iter()
        .map(|m| m.id)
        .collect();

    debug!("Found {} models", models.len());
    Ok(models)
}

fn list_anthropic_models() -> Result<Vec<String>> {
    // Anthropic doesn't have a models list endpoint, return known models
    Ok(vec![
        "claude-3-5-sonnet-20241022".to_string(),
        "claude-3-5-haiku-20241022".to_string(),
        "claude-3-opus-20240229".to_string(),
        "claude-3-sonnet-20240229".to_string(),
        "claude-3-haiku-20240307".to_string(),
    ])
}

/// Enhance text using the configured LLM provider
pub async fn enhance(text: &str, mode: &str, config: &Config) -> Result<String> {
    let system_prompt = get_system_prompt(mode, config);
    
    match config.llm.provider {
        LlmProvider::Ollama => enhance_ollama(text, &system_prompt, config).await,
        LlmProvider::OpenAI | LlmProvider::OpenRouter => {
            enhance_openai(text, &system_prompt, config).await
        }
        LlmProvider::Anthropic => enhance_anthropic(text, &system_prompt, config).await,
    }
}

/// Enhance using Ollama API
async fn enhance_ollama(text: &str, system_prompt: &str, config: &Config) -> Result<String> {
    let full_prompt = format!(
        "{}\n\nText to improve:\n{}\n\nImproved text:",
        system_prompt, text
    );

    let request = OllamaRequest {
        model: config.llm.model.clone(),
        prompt: full_prompt,
        stream: false,
    };

    let url = format!("{}/api/generate", config.llm.url);
    debug!("Calling Ollama API: {}", url);

    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .json(&request)
        .send()
        .await
        .context("Failed to connect to Ollama. Is it running?")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("Ollama API error: {} - {}", status, body));
    }

    let ollama_response: OllamaResponse = response
        .json()
        .await
        .context("Failed to parse Ollama response")?;

    Ok(ollama_response.response.trim().to_string())
}

/// Enhance using OpenAI-compatible API (works with OpenAI, OpenRouter, Groq, etc.)
async fn enhance_openai(text: &str, system_prompt: &str, config: &Config) -> Result<String> {
    let api_key = config.llm.api_key.as_ref()
        .ok_or_else(|| anyhow!("API key required for {}", config.llm.provider.display_name()))?;

    let request = OpenAIRequest {
        model: config.llm.model.clone(),
        messages: vec![
            OpenAIMessage {
                role: "system".to_string(),
                content: system_prompt.to_string(),
            },
            OpenAIMessage {
                role: "user".to_string(),
                content: text.to_string(),
            },
        ],
        max_tokens: Some(4096),
    };

    let url = format!("{}/v1/chat/completions", config.llm.url.trim_end_matches('/'));
    debug!("Calling OpenAI-compatible API: {}", url);

    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await
        .context("Failed to connect to API")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("API error: {} - {}", status, body));
    }

    let api_response: OpenAIResponse = response
        .json()
        .await
        .context("Failed to parse API response")?;

    api_response
        .choices
        .first()
        .map(|c| c.message.content.trim().to_string())
        .ok_or_else(|| anyhow!("No response from API"))
}

/// Enhance using Anthropic API
async fn enhance_anthropic(text: &str, system_prompt: &str, config: &Config) -> Result<String> {
    let api_key = config.llm.api_key.as_ref()
        .ok_or_else(|| anyhow!("API key required for Anthropic"))?;

    let request = AnthropicRequest {
        model: config.llm.model.clone(),
        max_tokens: 4096,
        system: Some(system_prompt.to_string()),
        messages: vec![
            AnthropicMessage {
                role: "user".to_string(),
                content: text.to_string(),
            },
        ],
    };

    let url = format!("{}/v1/messages", config.llm.url.trim_end_matches('/'));
    debug!("Calling Anthropic API: {}", url);

    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await
        .context("Failed to connect to Anthropic API")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("Anthropic API error: {} - {}", status, body));
    }

    let api_response: AnthropicResponse = response
        .json()
        .await
        .context("Failed to parse Anthropic response")?;

    api_response
        .content
        .first()
        .map(|c| c.text.trim().to_string())
        .ok_or_else(|| anyhow!("No response from Anthropic"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_default_prompt() {
        assert!(get_default_prompt("code").contains("coding"));
        assert!(get_default_prompt("email").contains("email"));
        assert!(get_default_prompt("notes").contains("bullet"));
        assert!(get_default_prompt("clean").contains("grammar"));
    }
}
