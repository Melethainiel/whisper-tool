use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub audio: AudioConfig,
    #[serde(default)]
    pub whisper: WhisperConfig,
    /// LLM configuration (supports ollama, openai, anthropic, openrouter)
    /// Note: "ollama" key is supported for backward compatibility
    #[serde(default, alias = "ollama")]
    pub llm: LlmConfig,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub prompts: PromptsConfig,
    #[serde(default)]
    pub shortcuts: ShortcutsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    /// Enhancement mode: raw, clean, code, email, notes
    #[serde(default = "default_mode")]
    pub mode: String,
    /// Microphone gain (0.5 to 10.0)
    #[serde(default = "default_mic_gain")]
    pub mic_gain: f32,
    /// Auto-enhance after transcription
    #[serde(default = "default_true")]
    pub auto_enhance: bool,
    /// Auto-copy result to clipboard
    #[serde(default = "default_true")]
    pub auto_copy: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    /// Sample rate in Hz
    #[serde(default = "default_sample_rate")]
    pub sample_rate: u32,
    /// Audio device name (None = default)
    pub device: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhisperConfig {
    /// Model size: tiny, base, small, medium, large
    #[serde(default = "default_model")]
    pub model: String,
    /// Path to models directory
    #[serde(default = "default_models_dir")]
    pub models_dir: PathBuf,
    /// Language (None = auto-detect)
    pub language: Option<String>,
}

/// LLM provider types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum LlmProvider {
    #[default]
    Ollama,
    OpenAI,
    Anthropic,
    OpenRouter,
}

impl LlmProvider {
    /// Get display name for UI
    #[allow(dead_code)] // Used by GUI binary
    pub fn display_name(&self) -> &'static str {
        match self {
            LlmProvider::Ollama => "Ollama (local)",
            LlmProvider::OpenAI => "OpenAI",
            LlmProvider::Anthropic => "Anthropic",
            LlmProvider::OpenRouter => "OpenRouter",
        }
    }

    /// Get default URL for this provider
    pub fn default_url(&self) -> &'static str {
        match self {
            LlmProvider::Ollama => "http://localhost:11434",
            LlmProvider::OpenAI => "https://api.openai.com",
            LlmProvider::Anthropic => "https://api.anthropic.com",
            LlmProvider::OpenRouter => "https://openrouter.ai/api",
        }
    }

    /// Get default model for this provider
    pub fn default_model(&self) -> &'static str {
        match self {
            LlmProvider::Ollama => "llama3.2",
            LlmProvider::OpenAI => "gpt-4o-mini",
            LlmProvider::Anthropic => "claude-3-haiku-20240307",
            LlmProvider::OpenRouter => "meta-llama/llama-3.2-3b-instruct:free",
        }
    }

    /// Whether this provider requires an API key
    #[allow(dead_code)] // Used by GUI binary
    pub fn requires_api_key(&self) -> bool {
        !matches!(self, LlmProvider::Ollama)
    }

    /// List of all providers for UI iteration
    #[allow(dead_code)] // Used by GUI binary
    pub fn all() -> &'static [LlmProvider] {
        &[
            LlmProvider::Ollama,
            LlmProvider::OpenAI,
            LlmProvider::Anthropic,
            LlmProvider::OpenRouter,
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    /// LLM provider (ollama, openai, anthropic, openrouter)
    #[serde(default)]
    pub provider: LlmProvider,
    /// API URL (provider-specific)
    #[serde(default = "default_llm_url")]
    pub url: String,
    /// Model to use for enhancement
    #[serde(default = "default_llm_model")]
    pub model: String,
    /// API key (required for cloud providers)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

/// Prompt templates for each enhancement mode
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptsConfig {
    /// Map of mode name to prompt template
    #[serde(default = "default_prompts")]
    pub templates: HashMap<String, PromptTemplate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTemplate {
    /// Display name for the mode
    pub name: String,
    /// The system prompt template
    pub prompt: String,
}

/// Keyboard shortcuts configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortcutsConfig {
    /// Map of action name to keybinding (e.g., "toggle_recording" -> "<Control>r")
    #[serde(default = "default_shortcuts")]
    pub bindings: HashMap<String, String>,
}

// Default value functions
fn default_sample_rate() -> u32 {
    16000
}

fn default_model() -> String {
    "base".to_string()
}

fn default_models_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("whisper-tool")
        .join("models")
}

fn default_llm_url() -> String {
    LlmProvider::default().default_url().to_string()
}

fn default_llm_model() -> String {
    LlmProvider::default().default_model().to_string()
}

fn default_mode() -> String {
    "raw".to_string()
}

fn default_mic_gain() -> f32 {
    2.0
}

fn default_true() -> bool {
    true
}

fn default_prompts() -> HashMap<String, PromptTemplate> {
    let mut prompts = HashMap::new();
    
    prompts.insert("clean".to_string(), PromptTemplate {
        name: "Clean (fix grammar)".to_string(),
        prompt: "You are a text editor. Fix spelling, grammar, and punctuation errors. \
                 Keep the original meaning and tone. Output only the corrected text, nothing else.".to_string(),
    });
    
    prompts.insert("code".to_string(), PromptTemplate {
        name: "Code (for coding)".to_string(),
        prompt: "You are a coding assistant. Transform the following voice dictation into a clear, \
                 precise instruction or code comment suitable for code generation. \
                 Be concise and technical. Output only the improved text, nothing else.".to_string(),
    });
    
    prompts.insert("email".to_string(), PromptTemplate {
        name: "Email (professional)".to_string(),
        prompt: "You are a professional email writer. Transform the following voice dictation \
                 into a professional, well-structured email. Keep it concise and polite. \
                 Output only the email text, nothing else.".to_string(),
    });
    
    prompts.insert("notes".to_string(), PromptTemplate {
        name: "Notes (bullet points)".to_string(),
        prompt: "You are a note-taking assistant. Transform the following voice dictation \
                 into clear, organized bullet points. Be concise. \
                 Output only the bullet points, nothing else.".to_string(),
    });
    
    prompts
}

fn default_shortcuts() -> HashMap<String, String> {
    let mut shortcuts = HashMap::new();
    
    // Recording controls
    shortcuts.insert("toggle_recording".to_string(), "<Control>r".to_string());
    shortcuts.insert("cancel_recording".to_string(), "Escape".to_string());
    
    // Copy actions
    shortcuts.insert("copy_transcript".to_string(), "<Control><Shift>c".to_string());
    shortcuts.insert("copy_enhanced".to_string(), "<Control><Shift>e".to_string());
    
    // Window controls
    shortcuts.insert("show_hide_window".to_string(), "<Control><Alt>w".to_string());
    shortcuts.insert("open_settings".to_string(), "<Control>comma".to_string());
    
    shortcuts
}

impl Default for Config {
    fn default() -> Self {
        Self {
            audio: AudioConfig::default(),
            whisper: WhisperConfig::default(),
            llm: LlmConfig::default(),
            ui: UiConfig::default(),
            prompts: PromptsConfig::default(),
            shortcuts: ShortcutsConfig::default(),
        }
    }
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: default_sample_rate(),
            device: None,
        }
    }
}

impl Default for WhisperConfig {
    fn default() -> Self {
        Self {
            model: default_model(),
            models_dir: default_models_dir(),
            language: None,
        }
    }
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: LlmProvider::default(),
            url: default_llm_url(),
            model: default_llm_model(),
            api_key: None,
        }
    }
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            mode: default_mode(),
            mic_gain: default_mic_gain(),
            auto_enhance: default_true(),
            auto_copy: default_true(),
        }
    }
}

impl Default for PromptsConfig {
    fn default() -> Self {
        Self {
            templates: default_prompts(),
        }
    }
}

impl Default for ShortcutsConfig {
    fn default() -> Self {
        Self {
            bindings: default_shortcuts(),
        }
    }
}

/// Load configuration from file or use defaults
pub fn load_config(path: Option<&str>) -> Result<Config> {
    let config_path = match path {
        Some(p) => PathBuf::from(p),
        None => default_config_path(),
    };

    if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read config: {:?}", config_path))?;
        let config: Config =
            toml::from_str(&content).with_context(|| "Failed to parse config file")?;
        Ok(config)
    } else {
        Ok(Config::default())
    }
}

/// Get default config file path
fn default_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("whisper-tool")
        .join("config.toml")
}

/// Get model file path for a given model name
pub fn get_model_path(config: &Config, model_name: &str) -> PathBuf {
    config
        .whisper
        .models_dir
        .join(format!("ggml-{}.bin", model_name))
}

/// Save configuration to file
#[allow(dead_code)] // Used by GUI binary
pub fn save_config(config: &Config) -> Result<()> {
    let config_path = default_config_path();

    // Create parent directory if needed
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create config directory: {:?}", parent))?;
    }

    let content = toml::to_string_pretty(config).with_context(|| "Failed to serialize config")?;

    std::fs::write(&config_path, content)
        .with_context(|| format!("Failed to write config: {:?}", config_path))?;

    Ok(())
}
