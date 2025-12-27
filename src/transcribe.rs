use anyhow::{anyhow, Context, Result};
use std::io::{Read, Write};
use std::path::Path;
use tracing::{debug, info};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::audio::AudioData;
use crate::config::{get_model_path, Config, GpuBackend};

/// Model download URLs (Hugging Face)
const MODEL_BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

/// Valid model names
const VALID_MODELS: [&str; 5] = ["tiny", "base", "small", "medium", "large"];

/// Cached Whisper engine that holds the model in memory
#[allow(dead_code)] // Used by GUI binary
pub struct WhisperEngine {
    ctx: WhisperContext,
    model_name: String,
    gpu_backend: GpuBackend,
}

/// Create WhisperContextParameters with GPU configuration
fn create_context_params(config: &Config) -> WhisperContextParameters<'_> {
    let mut params = WhisperContextParameters::default();
    let backend = config.whisper.gpu.resolve();
    
    // Enable GPU if the resolved backend uses it
    if backend.uses_gpu() {
        params.use_gpu(true);
        debug!("GPU acceleration enabled (backend: {:?})", backend);
    } else {
        params.use_gpu(false);
        debug!("Using CPU inference");
    }
    
    params
}

/// Get a description of the active GPU backend for logging
fn describe_gpu_backend(config: &Config) -> String {
    let backend = config.whisper.gpu.resolve();
    match backend {
        GpuBackend::Auto => "auto (resolved to CPU)".to_string(),
        GpuBackend::Cpu => "CPU".to_string(),
        GpuBackend::Vulkan => {
            if cfg!(feature = "vulkan") {
                "Vulkan GPU".to_string()
            } else {
                "Vulkan (not compiled, using CPU)".to_string()
            }
        }
        GpuBackend::Cuda => {
            if cfg!(feature = "cuda") {
                "CUDA GPU".to_string()
            } else {
                "CUDA (not compiled, using CPU)".to_string()
            }
        }
    }
}

#[allow(dead_code)] // Used by GUI binary
impl WhisperEngine {
    /// Create a new WhisperEngine with the model loaded and cached
    pub fn new(config: &Config) -> Result<Self> {
        let model_name = &config.whisper.model;
        
        // Ensure model exists (lazy download)
        ensure_model_exists(model_name, config)?;
        
        let model_path = get_model_path(config, model_name);
        let gpu_backend = config.whisper.gpu.resolve();
        let gpu_desc = describe_gpu_backend(config);
        
        info!(
            "Loading Whisper model '{}' into memory (backend: {})...",
            model_name, gpu_desc
        );
        
        let ctx_params = create_context_params(config);
        let ctx = WhisperContext::new_with_params(
            model_path.to_str().unwrap(),
            ctx_params,
        )
        .context("Failed to load Whisper model")?;
        
        info!(
            "Whisper model '{}' loaded and cached (backend: {})",
            model_name, gpu_desc
        );
        
        Ok(Self {
            ctx,
            model_name: model_name.clone(),
            gpu_backend,
        })
    }
    
    /// Get the name of the loaded model
    pub fn model_name(&self) -> &str {
        &self.model_name
    }
    
    /// Get the GPU backend being used
    #[allow(dead_code)] // Used by GUI binary
    pub fn gpu_backend(&self) -> &GpuBackend {
        &self.gpu_backend
    }
    
    /// Transcribe audio using the cached model
    pub fn transcribe(&self, audio: &AudioData, config: &Config) -> Result<String> {
        let mut state = self.ctx.create_state().context("Failed to create state")?;
        
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        
        // Configure parameters
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);
        params.set_single_segment(false);
        
        // Set language if specified
        if let Some(ref lang) = config.whisper.language {
            params.set_language(Some(lang));
        }
        
        // Set initial prompt from custom words for vocabulary hints
        if !config.whisper.custom_words.is_empty() {
            let prompt = config.whisper.custom_words.join(", ");
            params.set_initial_prompt(&prompt);
            debug!("Using custom words for vocabulary hints: {}", prompt);
        }
        
        debug!(
            "Transcribing {} samples at {}Hz (cached model: {})",
            audio.samples.len(),
            audio.sample_rate,
            self.model_name
        );
        
        // Run inference
        state
            .full(params, &audio.samples)
            .context("Failed to run Whisper inference")?;
        
        // Collect segments
        let num_segments = state.full_n_segments().context("Failed to get segments")?;
        let mut transcript = String::new();
        
        for i in 0..num_segments {
            if let Ok(segment) = state.full_get_segment_text(i) {
                transcript.push_str(&segment);
                transcript.push(' ');
            }
        }
        
        Ok(transcript.trim().to_string())
    }
}

/// Ensure the model exists, downloading it if necessary
pub fn ensure_model_exists(model_name: &str, config: &Config) -> Result<()> {
    if !VALID_MODELS.contains(&model_name) {
        return Err(anyhow!(
            "Invalid model: {}. Valid options: {:?}",
            model_name,
            VALID_MODELS
        ));
    }

    let model_path = get_model_path(config, model_name);

    if model_path.exists() {
        debug!("Model already exists: {:?}", model_path);
        return Ok(());
    }

    info!("Model '{}' not found, downloading...", model_name);
    download_model(model_name, config)
}

/// Transcribe audio data to text using Whisper
/// NOTE: This function reloads the model each time. For repeated transcriptions,
/// use WhisperEngine instead to cache the model in memory.
#[allow(dead_code)] // Used by CLI
pub fn transcribe(audio: &AudioData, config: &Config) -> Result<String> {
    // Ensure model exists (lazy download)
    ensure_model_exists(&config.whisper.model, config)?;

    let model_path = get_model_path(config, &config.whisper.model);
    let gpu_desc = describe_gpu_backend(config);
    debug!(
        "Loading Whisper model from {:?} (backend: {})",
        model_path, gpu_desc
    );

    let ctx_params = create_context_params(config);
    let ctx = WhisperContext::new_with_params(
        model_path.to_str().unwrap(),
        ctx_params,
    )
    .context("Failed to load Whisper model")?;

    let mut state = ctx.create_state().context("Failed to create state")?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

    // Configure parameters
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_suppress_blank(true);
    params.set_single_segment(false);

    // Set language if specified
    if let Some(ref lang) = config.whisper.language {
        params.set_language(Some(lang));
    }

    // Set initial prompt from custom words for vocabulary hints
    if !config.whisper.custom_words.is_empty() {
        let prompt = config.whisper.custom_words.join(", ");
        params.set_initial_prompt(&prompt);
        debug!("Using custom words for vocabulary hints: {}", prompt);
    }

    debug!(
        "Transcribing {} samples at {}Hz",
        audio.samples.len(),
        audio.sample_rate
    );

    // Run inference
    state
        .full(params, &audio.samples)
        .context("Failed to run Whisper inference")?;

    // Collect segments
    let num_segments = state.full_n_segments().context("Failed to get segments")?;
    let mut transcript = String::new();

    for i in 0..num_segments {
        if let Ok(segment) = state.full_get_segment_text(i) {
            transcript.push_str(&segment);
            transcript.push(' ');
        }
    }

    Ok(transcript.trim().to_string())
}

/// Download a Whisper model (can be called explicitly via CLI)
pub fn download_model(model_name: &str, config: &Config) -> Result<()> {
    if !VALID_MODELS.contains(&model_name) {
        return Err(anyhow!(
            "Invalid model: {}. Valid options: {:?}",
            model_name,
            VALID_MODELS
        ));
    }

    let model_path = get_model_path(config, model_name);
    let model_url = format!("{}/ggml-{}.bin", MODEL_BASE_URL, model_name);

    if model_path.exists() {
        info!("Model already exists: {:?}", model_path);
        return Ok(());
    }

    // Create models directory
    if let Some(parent) = model_path.parent() {
        std::fs::create_dir_all(parent).context("Failed to create models directory")?;
    }

    info!("Downloading model '{}' from Hugging Face...", model_name);
    info!("URL: {}", model_url);
    info!("Destination: {:?}", model_path);

    download_file(&model_url, &model_path)?;

    info!("Model '{}' downloaded successfully!", model_name);
    Ok(())
}

/// Download a file with progress indication
fn download_file(url: &str, path: &Path) -> Result<()> {
    let client = reqwest::blocking::Client::new();
    let mut response = client
        .get(url)
        .send()
        .context("Failed to connect to model server")?;

    if !response.status().is_success() {
        return Err(anyhow!("Download failed: HTTP {}", response.status()));
    }

    let total_size = response.content_length().unwrap_or(0);
    let size_mb = total_size as f64 / 1_000_000.0;
    info!("Download size: {:.1} MB", size_mb);

    // Create temporary file for download
    let temp_path = path.with_extension("bin.tmp");
    let mut file = std::fs::File::create(&temp_path)
        .context("Failed to create temporary file")?;

    // Download with progress
    let mut downloaded: u64 = 0;
    let mut buffer = [0u8; 8192];
    let mut last_progress = 0;

    loop {
        let bytes_read = response.read(&mut buffer)
            .context("Failed to read from download stream")?;
        
        if bytes_read == 0 {
            break;
        }

        file.write_all(&buffer[..bytes_read])
            .context("Failed to write to file")?;
        
        downloaded += bytes_read as u64;

        // Log progress every 10%
        if total_size > 0 {
            let progress = (downloaded * 100 / total_size) as u32;
            if progress >= last_progress + 10 {
                info!("Download progress: {}%", progress);
                last_progress = progress;
            }
        }
    }

    file.flush().context("Failed to flush file")?;
    drop(file);

    // Rename temp file to final destination
    std::fs::rename(&temp_path, path)
        .context("Failed to finalize model file")?;

    Ok(())
}
