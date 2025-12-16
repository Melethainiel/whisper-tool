mod audio;
mod config;
mod enhance;
mod floating;
mod notifier;
mod output;
mod transcribe;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, Button, CheckButton, ComboBoxText, Dialog, DrawingArea,
    Entry, Label, Orientation, ResponseType, Scale, ScrolledWindow, TextView,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use config::Config;
use notifier::Notifier;
use transcribe::WhisperEngine;

const APP_ID: &str = "com.whisper.tool";

// Nord theme colors
mod nord {
    pub const NORD0: &str = "#2e3440";  // Darkest
    pub const NORD1: &str = "#3b4252";  // Dark
    pub const NORD2: &str = "#434c5e";  // Medium
    pub const NORD3: &str = "#4c566a";  // Light dark
    pub const NORD4: &str = "#d8dee9";  // Dark white
    pub const NORD6: &str = "#eceff4";  // Bright white
    pub const NORD8: &str = "#88c0d0";  // Light blue
    pub const NORD9: &str = "#81a1c1";  // Blue
    pub const NORD10: &str = "#5e81ac"; // Dark blue
    pub const NORD11: &str = "#bf616a"; // Red
    pub const NORD13: &str = "#ebcb8b"; // Yellow
    pub const NORD14: &str = "#a3be8c"; // Green
}

// Global state for tray icon
static WINDOW_VISIBLE: Mutex<Option<Arc<Mutex<bool>>>> = Mutex::new(None);
static NOTIFIER: Mutex<Option<Notifier>> = Mutex::new(None);
static RECORDING_STATE: Mutex<Option<Arc<Mutex<TrayState>>>> = Mutex::new(None);

// Global state for floating window (to handle SIGUSR1 toggle)
static FLOATING_ACTIVE: AtomicBool = AtomicBool::new(false);
static FLOATING_STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Default)]
struct TrayState {
    recording: bool,
    processing: bool,
    start_requested: bool,
    stop_requested: bool,
    floating_requested: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum AppState {
    Idle,
    Recording,
}

struct RecorderState {
    app_state: AppState,
    start_time: Option<Instant>,
    config: Config,
    input_stream: Option<cpal::Stream>,
    samples: Arc<Mutex<Vec<f32>>>,
    waveform_history: Arc<Mutex<Vec<f32>>>,
    input_level: Arc<Mutex<f32>>,
    mic_gain: Arc<Mutex<f32>>,
    selected_mode: String,
    // Final enhanced result
    enhanced_result: String,
    // Settings
    auto_enhance: bool,
    auto_copy: bool,
    running: Arc<AtomicBool>,
    // Cached Whisper engine (shared across threads)
    whisper_engine: Arc<WhisperEngine>,
}

impl RecorderState {
    fn new() -> Self {
        let config = config::load_config(None).unwrap_or_default();
        
        // Load UI settings from config
        let mic_gain = config.ui.mic_gain;
        let selected_mode = config.ui.mode.clone();
        let auto_enhance = config.ui.auto_enhance;
        let auto_copy = config.ui.auto_copy;
        
        // Initialize WhisperEngine (loads model once at startup)
        let whisper_engine = match WhisperEngine::new(&config) {
            Ok(engine) => Arc::new(engine),
            Err(e) => {
                eprintln!("Failed to initialize Whisper engine: {}", e);
                panic!("Cannot start without Whisper engine: {}", e);
            }
        };
        
        Self {
            app_state: AppState::Idle,
            start_time: None,
            config,
            input_stream: None,
            samples: Arc::new(Mutex::new(Vec::new())),
            waveform_history: Arc::new(Mutex::new(vec![0.0; 60])),
            input_level: Arc::new(Mutex::new(0.0)),
            mic_gain: Arc::new(Mutex::new(mic_gain)),
            selected_mode,
            enhanced_result: String::new(),
            auto_enhance,
            auto_copy,
            running: Arc::new(AtomicBool::new(false)),
            whisper_engine,
        }
    }

    fn start_recording(&mut self) {
        self.samples.lock().unwrap().clear();
        self.enhanced_result.clear();
        self.start_time = Some(Instant::now());
        self.running.store(true, Ordering::SeqCst);

        let host = cpal::default_host();
        let device = host.default_input_device().expect("No input device");

        let stream_config = cpal::StreamConfig {
            channels: 1,
            sample_rate: cpal::SampleRate(self.config.audio.sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        let samples = Arc::clone(&self.samples);
        let input_level = Arc::clone(&self.input_level);
        let waveform_history = Arc::clone(&self.waveform_history);
        let mic_gain = Arc::clone(&self.mic_gain);
        let running = Arc::clone(&self.running);

        let stream = device
            .build_input_stream(
                &stream_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if !running.load(Ordering::SeqCst) {
                        return;
                    }

                    let gain = *mic_gain.lock().unwrap();
                    let gained_data: Vec<f32> = data
                        .iter()
                        .map(|&s| (s * gain).clamp(-1.0, 1.0))
                        .collect();

                    let sum: f32 = gained_data.iter().map(|&s| s * s).sum();
                    let rms = (sum / gained_data.len().max(1) as f32).sqrt();
                    let level = (rms * 5.0).min(1.0);
                    *input_level.lock().unwrap() = level;

                    let mut history = waveform_history.lock().unwrap();
                    history.remove(0);
                    history.push(level);

                    samples.lock().unwrap().extend_from_slice(&gained_data);
                },
                |err| eprintln!("Stream error: {}", err),
                None,
            )
            .expect("Failed to build stream");

        stream.play().expect("Failed to start stream");
        self.input_stream = Some(stream);
        self.app_state = AppState::Recording;

        println!("Recording started");
    }

    fn stop_recording(&mut self) -> Vec<f32> {
        self.running.store(false, Ordering::SeqCst);

        if let Some(stream) = self.input_stream.take() {
            drop(stream);
        }

        let samples = self.samples.lock().unwrap().clone();
        self.app_state = AppState::Idle;

        self.waveform_history
            .lock()
            .unwrap()
            .iter_mut()
            .for_each(|v| *v = 0.0);
        *self.input_level.lock().unwrap() = 0.0;

        println!("Recording stopped: {} samples", samples.len());
        samples
    }
}

fn show_notification(title: &str, body: &str) {
    if let Ok(guard) = NOTIFIER.lock() {
        if let Some(notifier) = guard.as_ref() {
            notifier.notify(title, body);
        }
    }
}

/// Format a keypress into GTK accelerator format (e.g., "<Control><Shift>c")
fn format_keypress(key_name: &str, modifier: gtk4::gdk::ModifierType) -> String {
    let mut result = String::new();

    if modifier.contains(gtk4::gdk::ModifierType::CONTROL_MASK) {
        result.push_str("<Control>");
    }
    if modifier.contains(gtk4::gdk::ModifierType::ALT_MASK) {
        result.push_str("<Alt>");
    }
    if modifier.contains(gtk4::gdk::ModifierType::SHIFT_MASK) {
        result.push_str("<Shift>");
    }
    if modifier.contains(gtk4::gdk::ModifierType::SUPER_MASK) {
        result.push_str("<Super>");
    }

    result.push_str(key_name);
    result
}

fn draw_waveform_bars(
    cr: &gtk4::cairo::Context,
    waveform_data: &[f32],
    width: i32,
    height: i32,
    is_recording: bool,
) {
    // Nord0 background
    cr.set_source_rgb(0.18, 0.20, 0.25);
    let _ = cr.rectangle(0.0, 0.0, width as f64, height as f64);
    let _ = cr.fill();

    let num_bars = 60;
    let bar_width = 3.0;
    let spacing = (width as f64 - (num_bars as f64 * bar_width)) / (num_bars as f64 + 1.0);

    for i in 0..num_bars {
        let x = spacing + (i as f64 * (bar_width + spacing));
        let level = waveform_data.get(i).copied().unwrap_or(0.0) as f64;

        let bar_height = (level * height as f64 * 0.85).max(2.0).min(height as f64 * 0.9);
        let y = (height as f64 - bar_height) / 2.0;

        if is_recording && level > 0.05 {
            // Nord14 green
            cr.set_source_rgb(0.64, 0.75, 0.55);
        } else {
            // Nord3 gray
            cr.set_source_rgb(0.30, 0.34, 0.42);
        }

        let _ = cr.rectangle(x, y, bar_width, bar_height);
        let _ = cr.fill();
    }
}

fn show_settings_dialog(parent: &ApplicationWindow, state: &Rc<RefCell<RecorderState>>) {
    let state_borrow = state.borrow();

    let dialog = Dialog::builder()
        .title("Settings")
        .transient_for(parent)
        .modal(true)
        .default_width(400)
        .default_height(420)
        .build();

    let content = dialog.content_area();
    let vbox = gtk4::Box::new(Orientation::Vertical, 12);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(16);
    vbox.set_margin_end(16);

    // === Whisper Section ===
    let whisper_label = Label::new(Some("WHISPER"));
    whisper_label.set_halign(gtk4::Align::Start);
    whisper_label.add_css_class("section-title");
    vbox.append(&whisper_label);

    // Language
    let lang_box = gtk4::Box::new(Orientation::Horizontal, 8);
    let lang_label = Label::new(Some("Language:"));
    lang_label.set_width_chars(12);
    lang_label.set_xalign(0.0);
    lang_box.append(&lang_label);

    let lang_combo = ComboBoxText::new();
    lang_combo.append(Some("auto"), "Auto-detect");
    lang_combo.append(Some("en"), "English");
    lang_combo.append(Some("fr"), "Français");
    lang_combo.append(Some("de"), "Deutsch");
    lang_combo.append(Some("es"), "Español");
    lang_combo.append(Some("it"), "Italiano");
    lang_combo.append(Some("pt"), "Português");
    lang_combo.append(Some("nl"), "Nederlands");
    lang_combo.append(Some("pl"), "Polski");
    lang_combo.append(Some("ru"), "Русский");
    lang_combo.append(Some("zh"), "中文");
    lang_combo.append(Some("ja"), "日本語");
    lang_combo.append(Some("ko"), "한국어");

    let current_lang = state_borrow
        .config
        .whisper
        .language
        .as_deref()
        .unwrap_or("auto");
    lang_combo.set_active_id(Some(current_lang));
    lang_combo.set_hexpand(true);
    lang_box.append(&lang_combo);
    vbox.append(&lang_box);

    // Model
    let model_box = gtk4::Box::new(Orientation::Horizontal, 8);
    let model_label = Label::new(Some("Model:"));
    model_label.set_width_chars(12);
    model_label.set_xalign(0.0);
    model_box.append(&model_label);

    let model_combo = ComboBoxText::new();
    model_combo.append(Some("tiny"), "Tiny (75MB - fastest)");
    model_combo.append(Some("base"), "Base (142MB - balanced)");
    model_combo.append(Some("small"), "Small (466MB - better)");
    model_combo.append(Some("medium"), "Medium (1.5GB - good)");
    model_combo.append(Some("large"), "Large (3GB - best)");
    model_combo.set_active_id(Some(&state_borrow.config.whisper.model));
    model_combo.set_hexpand(true);
    model_box.append(&model_combo);
    vbox.append(&model_box);

    // === LLM Section ===
    let llm_label = Label::new(Some("LLM ENHANCEMENT"));
    llm_label.set_halign(gtk4::Align::Start);
    llm_label.add_css_class("section-title");
    llm_label.set_margin_top(12);
    vbox.append(&llm_label);

    // Provider selector
    let provider_box = gtk4::Box::new(Orientation::Horizontal, 8);
    let provider_label = Label::new(Some("Provider:"));
    provider_label.set_width_chars(12);
    provider_label.set_xalign(0.0);
    provider_box.append(&provider_label);

    let provider_combo = ComboBoxText::new();
    provider_combo.add_css_class("mode-combo");
    for provider in config::LlmProvider::all() {
        let id = format!("{:?}", provider).to_lowercase();
        provider_combo.append(Some(&id), provider.display_name());
    }
    let current_provider = format!("{:?}", state_borrow.config.llm.provider).to_lowercase();
    provider_combo.set_active_id(Some(&current_provider));
    provider_combo.set_hexpand(true);
    provider_box.append(&provider_combo);
    vbox.append(&provider_box);

    // URL
    let url_box = gtk4::Box::new(Orientation::Horizontal, 8);
    let url_label = Label::new(Some("URL:"));
    url_label.set_width_chars(12);
    url_label.set_xalign(0.0);
    url_box.append(&url_label);

    let url_entry = Entry::new();
    url_entry.set_text(&state_borrow.config.llm.url);
    url_entry.set_hexpand(true);
    url_entry.set_placeholder_text(Some(state_borrow.config.llm.provider.default_url()));
    url_box.append(&url_entry);
    vbox.append(&url_box);

    // API Key (only visible for cloud providers)
    let api_key_box = gtk4::Box::new(Orientation::Horizontal, 8);
    let api_key_label = Label::new(Some("API Key:"));
    api_key_label.set_width_chars(12);
    api_key_label.set_xalign(0.0);
    api_key_box.append(&api_key_label);

    let api_key_entry = Entry::new();
    api_key_entry.set_visibility(false); // Hide API key
    api_key_entry.set_input_purpose(gtk4::InputPurpose::Password);
    if let Some(ref key) = state_borrow.config.llm.api_key {
        api_key_entry.set_text(key);
    }
    api_key_entry.set_hexpand(true);
    api_key_entry.set_placeholder_text(Some("sk-... or your API key"));
    api_key_box.append(&api_key_entry);
    vbox.append(&api_key_box);

    // Show/hide API key based on provider
    let requires_key = state_borrow.config.llm.provider.requires_api_key();
    api_key_box.set_visible(requires_key);

    // Model with dropdown and refresh button
    let llm_model_box = gtk4::Box::new(Orientation::Horizontal, 8);
    let llm_model_label = Label::new(Some("Model:"));
    llm_model_label.set_width_chars(12);
    llm_model_label.set_xalign(0.0);
    llm_model_box.append(&llm_model_label);

    let llm_model_combo = ComboBoxText::new();
    llm_model_combo.set_hexpand(true);
    llm_model_combo.add_css_class("mode-combo");
    // Add current model as fallback option
    let current_llm_model = state_borrow.config.llm.model.clone();
    llm_model_combo.append(Some(&current_llm_model), &current_llm_model);
    llm_model_combo.set_active_id(Some(&current_llm_model));
    llm_model_box.append(&llm_model_combo);

    let refresh_btn = Button::with_label("⟳");
    refresh_btn.add_css_class("settings-button");
    refresh_btn.set_tooltip_text(Some("Refresh available models"));
    llm_model_box.append(&refresh_btn);
    vbox.append(&llm_model_box);

    // Status label for model fetching
    let llm_status = Label::new(Some(""));
    llm_status.set_halign(gtk4::Align::Start);
    llm_status.add_css_class("status-label");
    vbox.append(&llm_status);

    // Provider change handler - update URL placeholder and API key visibility
    let url_entry_for_provider = url_entry.clone();
    let api_key_box_for_provider = api_key_box.clone();
    let llm_model_combo_for_provider = llm_model_combo.clone();
    provider_combo.connect_changed(move |combo| {
        if let Some(id) = combo.active_id() {
            let provider = match id.as_str() {
                "ollama" => config::LlmProvider::Ollama,
                "openai" => config::LlmProvider::OpenAI,
                "anthropic" => config::LlmProvider::Anthropic,
                "openrouter" => config::LlmProvider::OpenRouter,
                _ => config::LlmProvider::Ollama,
            };
            
            // Update URL placeholder
            url_entry_for_provider.set_placeholder_text(Some(provider.default_url()));
            
            // Show/hide API key field
            api_key_box_for_provider.set_visible(provider.requires_api_key());
            
            // Update model combo with default model for this provider
            llm_model_combo_for_provider.remove_all();
            let default_model = provider.default_model();
            llm_model_combo_for_provider.append(Some(default_model), default_model);
            llm_model_combo_for_provider.set_active(Some(0));
        }
    });

    // Wire up refresh button
    let provider_combo_for_refresh = provider_combo.clone();
    let url_entry_for_refresh = url_entry.clone();
    let api_key_entry_for_refresh = api_key_entry.clone();
    let llm_model_combo_for_refresh = llm_model_combo.clone();
    let llm_status_for_refresh = llm_status.clone();
    let current_model_for_refresh = current_llm_model.clone();
    let refresh_btn_clone = refresh_btn.clone();
    refresh_btn.connect_clicked(move |btn| {
        btn.set_sensitive(false);
        llm_status_for_refresh.set_text("Fetching models...");
        
        let provider = match provider_combo_for_refresh.active_id().as_deref() {
            Some("ollama") => config::LlmProvider::Ollama,
            Some("openai") => config::LlmProvider::OpenAI,
            Some("anthropic") => config::LlmProvider::Anthropic,
            Some("openrouter") => config::LlmProvider::OpenRouter,
            _ => config::LlmProvider::Ollama,
        };
        let url = url_entry_for_refresh.text().to_string();
        let url = if url.is_empty() { provider.default_url().to_string() } else { url };
        let api_key = {
            let text = api_key_entry_for_refresh.text().to_string();
            if text.is_empty() { None } else { Some(text) }
        };
        let current_model = current_model_for_refresh.clone();
        
        // Channel for results
        let (tx, rx) = std::sync::mpsc::channel::<Result<Vec<String>, String>>();
        
        // Fetch models in background thread
        std::thread::spawn(move || {
            let result = enhance::list_models_blocking(&provider, &url, api_key.as_deref())
                .map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        
        // Poll for results on main thread
        let combo = llm_model_combo_for_refresh.clone();
        let status = llm_status_for_refresh.clone();
        let button = refresh_btn_clone.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            match rx.try_recv() {
                Ok(Ok(models)) => {
                    button.set_sensitive(true);
                    combo.remove_all();
                    
                    if models.is_empty() {
                        status.set_text("No models found");
                        combo.append(Some(&current_model), &current_model);
                        combo.set_active_id(Some(&current_model));
                    } else {
                        status.set_text(&format!("Found {} models", models.len()));
                        
                        for model in &models {
                            combo.append(Some(model), model);
                        }
                        
                        if models.contains(&current_model) {
                            combo.set_active_id(Some(&current_model));
                        } else {
                            combo.set_active(Some(0));
                        }
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    button.set_sensitive(true);
                    status.set_text(&format!("Error: {}", e));
                    eprintln!("Failed to fetch models: {}", e);
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    glib::ControlFlow::Continue
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    button.set_sensitive(true);
                    status.set_text("Error: Connection lost");
                    glib::ControlFlow::Break
                }
            }
        });
    });

    // Auto-fetch models on dialog open
    refresh_btn.emit_clicked();

    // === Advanced Settings Section ===
    let advanced_label = Label::new(Some("ADVANCED"));
    advanced_label.set_halign(gtk4::Align::Start);
    advanced_label.add_css_class("section-title");
    advanced_label.set_margin_top(12);
    vbox.append(&advanced_label);

    let advanced_box = gtk4::Box::new(Orientation::Horizontal, 8);

    let prompts_btn = Button::with_label("Edit Prompts");
    prompts_btn.add_css_class("dialog-button");
    prompts_btn.set_tooltip_text(Some("Edit prompt templates for each mode"));
    advanced_box.append(&prompts_btn);

    let shortcuts_btn = Button::with_label("Shortcuts");
    shortcuts_btn.add_css_class("dialog-button");
    shortcuts_btn.set_tooltip_text(Some("Configure keyboard shortcuts"));
    advanced_box.append(&shortcuts_btn);

    vbox.append(&advanced_box);

    // Wire up advanced buttons (need parent window reference)
    let state_for_prompts = Rc::clone(state);
    let parent_for_prompts = parent.clone();
    let dialog_for_prompts = dialog.clone();
    prompts_btn.connect_clicked(move |_| {
        dialog_for_prompts.hide();
        show_prompts_dialog(&parent_for_prompts, &state_for_prompts);
    });

    let state_for_shortcuts = Rc::clone(state);
    let parent_for_shortcuts = parent.clone();
    let dialog_for_shortcuts = dialog.clone();
    shortcuts_btn.connect_clicked(move |_| {
        dialog_for_shortcuts.hide();
        show_shortcuts_dialog(&parent_for_shortcuts, &state_for_shortcuts);
    });

    // Buttons
    let button_box = gtk4::Box::new(Orientation::Horizontal, 8);
    button_box.set_halign(gtk4::Align::End);
    button_box.set_margin_top(16);

    let cancel_btn = Button::with_label("Cancel");
    cancel_btn.add_css_class("dialog-button");
    let dialog_cancel = dialog.clone();
    cancel_btn.connect_clicked(move |_| {
        dialog_cancel.response(ResponseType::Cancel);
    });
    button_box.append(&cancel_btn);

    let save_btn = Button::with_label("Save");
    save_btn.add_css_class("dialog-button");
    save_btn.add_css_class("suggested-action");
    let dialog_save = dialog.clone();
    save_btn.connect_clicked(move |_| {
        dialog_save.response(ResponseType::Accept);
    });
    button_box.append(&save_btn);

    vbox.append(&button_box);
    content.append(&vbox);

    drop(state_borrow);

    let state_clone = Rc::clone(state);
    dialog.connect_response(move |dialog, response| {
        if response == ResponseType::Accept {
            let mut state = state_clone.borrow_mut();

            let lang = lang_combo.active_id();
            state.config.whisper.language = if lang.as_deref() == Some("auto") {
                None
            } else {
                lang.map(|s| s.to_string())
            };

            if let Some(model) = model_combo.active_id() {
                state.config.whisper.model = model.to_string();
            }

            // Save LLM settings
            if let Some(provider_id) = provider_combo.active_id() {
                state.config.llm.provider = match provider_id.as_str() {
                    "ollama" => config::LlmProvider::Ollama,
                    "openai" => config::LlmProvider::OpenAI,
                    "anthropic" => config::LlmProvider::Anthropic,
                    "openrouter" => config::LlmProvider::OpenRouter,
                    _ => config::LlmProvider::Ollama,
                };
            }

            let url = url_entry.text().to_string();
            if !url.is_empty() {
                state.config.llm.url = url;
            }

            let api_key = api_key_entry.text().to_string();
            state.config.llm.api_key = if api_key.is_empty() { None } else { Some(api_key) };

            if let Some(model) = llm_model_combo.active_id() {
                state.config.llm.model = model.to_string();
            }

            if let Err(e) = config::save_config(&state.config) {
                eprintln!("Failed to save config: {}", e);
                show_notification("Error", "Failed to save configuration");
            } else {
                println!("Config saved");
                show_notification("Settings saved", "Configuration updated");
            }
        }
        dialog.close();
    });

    dialog.present();
}

fn show_prompts_dialog(parent: &ApplicationWindow, state: &Rc<RefCell<RecorderState>>) {
    let state_borrow = state.borrow();

    let dialog = Dialog::builder()
        .title("Edit Prompts")
        .transient_for(parent)
        .modal(true)
        .default_width(500)
        .default_height(450)
        .build();

    let content = dialog.content_area();
    let vbox = gtk4::Box::new(Orientation::Vertical, 8);
    vbox.set_margin_top(12);
    vbox.set_margin_bottom(12);
    vbox.set_margin_start(12);
    vbox.set_margin_end(12);

    // Instructions
    let info_label = Label::new(Some("Edit prompt templates for each enhancement mode:"));
    info_label.set_halign(gtk4::Align::Start);
    info_label.add_css_class("section-label");
    vbox.append(&info_label);

    // Mode selector
    let mode_box = gtk4::Box::new(Orientation::Horizontal, 8);
    let mode_label = Label::new(Some("Mode:"));
    mode_label.set_width_chars(8);
    mode_box.append(&mode_label);

    let mode_combo = ComboBoxText::new();
    mode_combo.add_css_class("mode-combo");
    
    // Get modes from config
    let mut modes: Vec<(String, String)> = state_borrow
        .config
        .prompts
        .templates
        .iter()
        .map(|(k, v)| (k.clone(), v.name.clone()))
        .collect();
    modes.sort_by(|a, b| a.0.cmp(&b.0));
    
    for (mode_id, mode_name) in &modes {
        mode_combo.append(Some(mode_id), mode_name);
    }
    mode_combo.set_active(Some(0));
    mode_combo.set_hexpand(true);
    mode_box.append(&mode_combo);
    vbox.append(&mode_box);

    // Prompt text area
    let prompt_label = Label::new(Some("Prompt Template:"));
    prompt_label.set_halign(gtk4::Align::Start);
    prompt_label.add_css_class("section-label");
    prompt_label.set_margin_top(8);
    vbox.append(&prompt_label);

    let scrolled = ScrolledWindow::new();
    scrolled.set_min_content_height(200);
    scrolled.set_vexpand(true);
    scrolled.add_css_class("text-pane");

    let prompt_textview = TextView::new();
    prompt_textview.set_wrap_mode(gtk4::WrapMode::Word);
    prompt_textview.add_css_class("result-text");
    prompt_textview.set_left_margin(8);
    prompt_textview.set_right_margin(8);
    prompt_textview.set_top_margin(6);
    prompt_textview.set_bottom_margin(6);
    scrolled.set_child(Some(&prompt_textview));
    vbox.append(&scrolled);

    // Load initial prompt
    let prompt_buffer = prompt_textview.buffer();
    if let Some(first_mode) = modes.first() {
        if let Some(template) = state_borrow.config.prompts.templates.get(&first_mode.0) {
            prompt_buffer.set_text(&template.prompt);
        }
    }

    // Store current prompts in a RefCell for editing
    let prompts_copy = Rc::new(RefCell::new(state_borrow.config.prompts.templates.clone()));
    let current_mode = Rc::new(RefCell::new(modes.first().map(|m| m.0.clone()).unwrap_or_default()));
    // Flag to prevent recursive updates when programmatically setting text
    let updating_programmatically = Rc::new(RefCell::new(false));

    // Update prompt when mode changes
    let prompts_for_combo = Rc::clone(&prompts_copy);
    let current_mode_for_combo = Rc::clone(&current_mode);
    let prompt_buffer_for_combo = prompt_buffer.clone();
    let updating_for_combo = Rc::clone(&updating_programmatically);
    mode_combo.connect_changed(move |combo| {
        if let Some(mode_id) = combo.active_id() {
            let mode_str = mode_id.to_string();
            *current_mode_for_combo.borrow_mut() = mode_str.clone();
            
            if let Some(template) = prompts_for_combo.borrow().get(&mode_str) {
                *updating_for_combo.borrow_mut() = true;
                prompt_buffer_for_combo.set_text(&template.prompt);
                *updating_for_combo.borrow_mut() = false;
            }
        }
    });

    // Save prompt text when buffer changes
    let prompts_for_buffer = Rc::clone(&prompts_copy);
    let current_mode_for_buffer = Rc::clone(&current_mode);
    let updating_for_buffer = Rc::clone(&updating_programmatically);
    prompt_buffer.connect_changed(move |buffer| {
        // Skip if we're programmatically updating
        if *updating_for_buffer.borrow() {
            return;
        }
        
        let start = buffer.start_iter();
        let end = buffer.end_iter();
        let text = buffer.text(&start, &end, false).to_string();
        
        let mode = current_mode_for_buffer.borrow().clone();
        if let Some(template) = prompts_for_buffer.borrow_mut().get_mut(&mode) {
            template.prompt = text;
        }
    });

    // Buttons
    let button_box = gtk4::Box::new(Orientation::Horizontal, 8);
    button_box.set_halign(gtk4::Align::End);
    button_box.set_margin_top(12);

    let reset_btn = Button::with_label("Reset to Defaults");
    reset_btn.add_css_class("dialog-button");
    let prompts_for_reset = Rc::clone(&prompts_copy);
    let prompt_buffer_for_reset = prompt_buffer.clone();
    let mode_combo_for_reset = mode_combo.clone();
    let updating_for_reset = Rc::clone(&updating_programmatically);
    reset_btn.connect_clicked(move |_| {
        *prompts_for_reset.borrow_mut() = config::PromptsConfig::default().templates;
        // Reload current prompt
        if let Some(mode_id) = mode_combo_for_reset.active_id() {
            if let Some(template) = prompts_for_reset.borrow().get(mode_id.as_str()) {
                *updating_for_reset.borrow_mut() = true;
                prompt_buffer_for_reset.set_text(&template.prompt);
                *updating_for_reset.borrow_mut() = false;
            }
        }
    });
    button_box.append(&reset_btn);

    let cancel_btn = Button::with_label("Cancel");
    cancel_btn.add_css_class("dialog-button");
    let dialog_cancel = dialog.clone();
    cancel_btn.connect_clicked(move |_| {
        dialog_cancel.response(ResponseType::Cancel);
    });
    button_box.append(&cancel_btn);

    let save_btn = Button::with_label("Save");
    save_btn.add_css_class("dialog-button");
    save_btn.add_css_class("suggested-action");
    let dialog_save = dialog.clone();
    save_btn.connect_clicked(move |_| {
        dialog_save.response(ResponseType::Accept);
    });
    button_box.append(&save_btn);

    vbox.append(&button_box);
    content.append(&vbox);

    drop(state_borrow);

    let state_clone = Rc::clone(state);
    let prompts_for_save = Rc::clone(&prompts_copy);
    dialog.connect_response(move |dialog, response| {
        if response == ResponseType::Accept {
            let mut state = state_clone.borrow_mut();
            state.config.prompts.templates = prompts_for_save.borrow().clone();

            if let Err(e) = config::save_config(&state.config) {
                eprintln!("Failed to save prompts: {}", e);
                show_notification("Error", "Failed to save prompts");
            } else {
                println!("Prompts saved");
                show_notification("Prompts saved", "Prompt templates updated");
            }
        }
        dialog.close();
    });

    dialog.present();
}

fn show_shortcuts_dialog(parent: &ApplicationWindow, state: &Rc<RefCell<RecorderState>>) {
    let state_borrow = state.borrow();

    let dialog = Dialog::builder()
        .title("Keyboard Shortcuts")
        .transient_for(parent)
        .modal(true)
        .default_width(450)
        .default_height(400)
        .build();

    let content = dialog.content_area();
    let vbox = gtk4::Box::new(Orientation::Vertical, 8);
    vbox.set_margin_top(12);
    vbox.set_margin_bottom(12);
    vbox.set_margin_start(12);
    vbox.set_margin_end(12);

    // Instructions
    let info_label = Label::new(Some("Configure keyboard shortcuts (GTK format):"));
    info_label.set_halign(gtk4::Align::Start);
    info_label.add_css_class("section-label");
    vbox.append(&info_label);

    let hint_label = Label::new(Some("Examples: <Control>r, <Control><Shift>c, <Alt>s, Escape"));
    hint_label.set_halign(gtk4::Align::Start);
    hint_label.add_css_class("status-label");
    vbox.append(&hint_label);

    // Scrolled list of shortcuts
    let scrolled = ScrolledWindow::new();
    scrolled.set_min_content_height(200);
    scrolled.set_vexpand(true);

    let list_box = gtk4::Box::new(Orientation::Vertical, 4);
    list_box.set_margin_top(8);

    // Action display names
    let action_names: std::collections::HashMap<&str, &str> = [
        ("toggle_recording", "Toggle Recording"),
        ("cancel_recording", "Cancel Recording"),
        ("copy_transcript", "Copy Transcript"),
        ("copy_enhanced", "Copy Enhanced"),
        ("show_hide_window", "Show/Hide Window"),
        ("open_settings", "Open Settings"),
    ]
    .into_iter()
    .collect();

    // Store entries for later retrieval
    let entries: Rc<RefCell<Vec<(String, Entry)>>> = Rc::new(RefCell::new(Vec::new()));

    // Sort shortcuts by action name
    let mut shortcuts: Vec<_> = state_borrow.config.shortcuts.bindings.iter().collect();
    shortcuts.sort_by(|a, b| a.0.cmp(b.0));

    for (action, keybinding) in shortcuts {
        let row = gtk4::Box::new(Orientation::Horizontal, 8);
        
        let display_name = action_names
            .get(action.as_str())
            .copied()
            .unwrap_or(action.as_str());
        let label = Label::new(Some(display_name));
        label.set_width_chars(18);
        label.set_xalign(0.0);
        row.append(&label);

        let entry = Entry::new();
        entry.set_text(keybinding);
        entry.set_hexpand(true);
        entry.set_placeholder_text(Some("<Control>key"));
        row.append(&entry);

        entries.borrow_mut().push((action.clone(), entry));
        list_box.append(&row);
    }

    scrolled.set_child(Some(&list_box));
    vbox.append(&scrolled);

    // Buttons
    let button_box = gtk4::Box::new(Orientation::Horizontal, 8);
    button_box.set_halign(gtk4::Align::End);
    button_box.set_margin_top(12);

    let reset_btn = Button::with_label("Reset to Defaults");
    reset_btn.add_css_class("dialog-button");
    let entries_for_reset = Rc::clone(&entries);
    reset_btn.connect_clicked(move |_| {
        let defaults = config::ShortcutsConfig::default().bindings;
        for (action, entry) in entries_for_reset.borrow().iter() {
            if let Some(keybinding) = defaults.get(action) {
                entry.set_text(keybinding);
            }
        }
    });
    button_box.append(&reset_btn);

    let cancel_btn = Button::with_label("Cancel");
    cancel_btn.add_css_class("dialog-button");
    let dialog_cancel = dialog.clone();
    cancel_btn.connect_clicked(move |_| {
        dialog_cancel.response(ResponseType::Cancel);
    });
    button_box.append(&cancel_btn);

    let save_btn = Button::with_label("Save");
    save_btn.add_css_class("dialog-button");
    save_btn.add_css_class("suggested-action");
    let dialog_save = dialog.clone();
    save_btn.connect_clicked(move |_| {
        dialog_save.response(ResponseType::Accept);
    });
    button_box.append(&save_btn);

    vbox.append(&button_box);
    content.append(&vbox);

    drop(state_borrow);

    let state_clone = Rc::clone(state);
    let entries_for_save = Rc::clone(&entries);
    dialog.connect_response(move |dialog, response| {
        if response == ResponseType::Accept {
            let mut state = state_clone.borrow_mut();
            
            for (action, entry) in entries_for_save.borrow().iter() {
                let keybinding = entry.text().to_string();
                if !keybinding.is_empty() {
                    state.config.shortcuts.bindings.insert(action.clone(), keybinding);
                }
            }

            if let Err(e) = config::save_config(&state.config) {
                eprintln!("Failed to save shortcuts: {}", e);
                show_notification("Error", "Failed to save shortcuts");
            } else {
                println!("Shortcuts saved");
                show_notification("Shortcuts saved", "Keyboard shortcuts updated.\nRestart app to apply.");
            }
        }
        dialog.close();
    });

    dialog.present();
}

fn start_tray_icon() {
    use ksni::TrayService;

    struct WhisperTray {
        recording: bool,
        processing: bool,
    }

    // Simple 22x22 microphone icon in ARGB format (fallback if theme icon fails)
    fn create_microphone_icon() -> ksni::Icon {
        let size = 22;
        let mut data = Vec::with_capacity(size * size * 4);
        
        // Nord10 blue: #5e81ac
        let color: [u8; 4] = [255, 94, 129, 172]; // ARGB
        let transparent: [u8; 4] = [0, 0, 0, 0];
        
        for y in 0..size {
            for x in 0..size {
                let cx = (x as i32) - 11; // Center x
                let cy = (y as i32) - 8;  // Shift center up for microphone shape
                
                // Microphone head (rounded rectangle / capsule shape)
                let in_head = cy >= -6 && cy <= 2 && cx.abs() <= 4 
                    && ((cy > -5 && cy < 1) || (cx * cx + (cy + 5) * (cy + 5)) <= 16 || (cx * cx + (cy - 1) * (cy - 1)) <= 16);
                
                // Microphone body/stand - U shape around the head
                let in_stand = cy >= 0 && cy <= 5 && cx.abs() >= 5 && cx.abs() <= 6
                    || (cy == 5 && cx.abs() <= 6);
                
                // Stem
                let in_stem = cy >= 5 && cy <= 10 && cx.abs() <= 1;
                
                // Base
                let in_base = cy >= 9 && cy <= 10 && cx.abs() <= 4;
                
                if in_head || in_stand || in_stem || in_base {
                    data.extend_from_slice(&color);
                } else {
                    data.extend_from_slice(&transparent);
                }
            }
        }
        
        ksni::Icon {
            width: size as i32,
            height: size as i32,
            data,
        }
    }

    fn create_record_icon() -> ksni::Icon {
        let size = 22;
        let mut data = Vec::with_capacity(size * size * 4);
        
        for y in 0..size {
            for x in 0..size {
                let cx = (x as i32) - 11;
                let cy = (y as i32) - 11;
                
                // Red circle for recording
                if cx * cx + cy * cy < 81 {
                    // Nord11 red: #bf616a
                    data.extend_from_slice(&[255, 191, 97, 106]); // ARGB
                } else {
                    data.extend_from_slice(&[0, 0, 0, 0]);
                }
            }
        }
        
        ksni::Icon {
            width: size as i32,
            height: size as i32,
            data,
        }
    }

    fn create_processing_icon() -> ksni::Icon {
        let size = 22;
        let mut data = Vec::with_capacity(size * size * 4);
        
        for y in 0..size {
            for x in 0..size {
                let cx = (x as i32) - 11;
                let cy = (y as i32) - 11;
                
                // Gear-like shape for processing
                let dist = ((cx * cx + cy * cy) as f32).sqrt();
                let in_ring = dist > 5.0 && dist < 9.0;
                
                if in_ring {
                    // Nord13 yellow: #ebcb8b
                    data.extend_from_slice(&[255, 235, 203, 139]); // ARGB
                } else {
                    data.extend_from_slice(&[0, 0, 0, 0]);
                }
            }
        }
        
        ksni::Icon {
            width: size as i32,
            height: size as i32,
            data,
        }
    }

    impl ksni::Tray for WhisperTray {
        fn id(&self) -> String {
            "whisper-tool".to_string()
        }

        fn title(&self) -> String {
            if self.processing {
                "Whisper Tool (Processing...)".to_string()
            } else if self.recording {
                "Whisper Tool (Recording...)".to_string()
            } else {
                "Whisper Tool".to_string()
            }
        }

        fn icon_name(&self) -> String {
            // Always use pixmap icons for consistency across all themes
            String::new()
        }

        // Provide pixmap as fallback when theme icons are not found
        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            let icon = if self.recording {
                create_record_icon()
            } else if self.processing {
                create_processing_icon()
            } else {
                create_microphone_icon()
            };
            vec![icon]
        }

        fn activate(&mut self, _x: i32, _y: i32) {
            toggle_window_visibility();
        }

        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            use ksni::menu::*;

            let mut items = vec![StandardItem {
                label: "Show/Hide".to_string(),
                activate: Box::new(|_| {
                    toggle_window_visibility();
                }),
                ..Default::default()
            }
            .into()];

            items.push(ksni::MenuItem::Separator);

            if !self.recording && !self.processing {
                items.push(
                    StandardItem {
                        label: "⏺ Start Recording".to_string(),
                        activate: Box::new(|_| {
                            request_start_recording();
                        }),
                        ..Default::default()
                    }
                    .into(),
                );
                items.push(
                    StandardItem {
                        label: "🎯 Quick Dictate".to_string(),
                        activate: Box::new(|_| {
                            request_floating_dictation();
                        }),
                        ..Default::default()
                    }
                    .into(),
                );
            } else if self.recording {
                items.push(
                    StandardItem {
                        label: "⏹ Stop Recording".to_string(),
                        activate: Box::new(|_| {
                            request_stop_recording();
                        }),
                        ..Default::default()
                    }
                    .into(),
                );
            }

            items.push(ksni::MenuItem::Separator);

            items.push(
                StandardItem {
                    label: "Quit".to_string(),
                    activate: Box::new(|_| {
                        std::process::exit(0);
                    }),
                    ..Default::default()
                }
                .into(),
            );

            items
        }
    }

    fn toggle_window_visibility() {
        if let Ok(guard) = WINDOW_VISIBLE.lock() {
            if let Some(visible_ref) = guard.as_ref() {
                if let Ok(mut visible) = visible_ref.lock() {
                    *visible = !*visible;
                }
            }
        }
    }

    fn request_start_recording() {
        if let Ok(guard) = RECORDING_STATE.lock() {
            if let Some(state_ref) = guard.as_ref() {
                if let Ok(mut state) = state_ref.lock() {
                    state.start_requested = true;
                }
            }
        }
    }

    fn request_stop_recording() {
        if let Ok(guard) = RECORDING_STATE.lock() {
            if let Some(state_ref) = guard.as_ref() {
                if let Ok(mut state) = state_ref.lock() {
                    state.stop_requested = true;
                }
            }
        }
    }

    fn request_floating_dictation() {
        if let Ok(guard) = RECORDING_STATE.lock() {
            if let Some(state_ref) = guard.as_ref() {
                if let Ok(mut state) = state_ref.lock() {
                    state.floating_requested = true;
                }
            }
        }
    }

    let service = TrayService::new(WhisperTray {
        recording: false,
        processing: false,
    });
    let handle = service.handle();
    service.spawn();

    println!("Tray icon started");

    // Force an initial update to ensure the icon is visible
    // Some tray hosts need an update signal to properly display the icon
    std::thread::sleep(Duration::from_millis(100));
    handle.update(|tray: &mut WhisperTray| {
        // Trigger a refresh without changing state
        tray.recording = false;
        tray.processing = false;
    });

    let mut last_recording = false;
    let mut last_processing = false;

    loop {
        std::thread::sleep(Duration::from_millis(200));

        let (recording, processing) = {
            if let Ok(guard) = RECORDING_STATE.lock() {
                if let Some(state_ref) = guard.as_ref() {
                    if let Ok(state) = state_ref.lock() {
                        (state.recording, state.processing)
                    } else {
                        (false, false)
                    }
                } else {
                    (false, false)
                }
            } else {
                (false, false)
            }
        };

        if recording != last_recording || processing != last_processing {
            handle.update(|tray: &mut WhisperTray| {
                tray.recording = recording;
                tray.processing = processing;
            });
            last_recording = recording;
            last_processing = processing;
        }
    }
}

fn create_text_pane(
    label_text: &str,
    copy_tooltip: &str,
) -> (gtk4::Box, gtk4::TextView, Button) {
    let container = gtk4::Box::new(Orientation::Vertical, 4);

    // Header with label and copy button
    let header = gtk4::Box::new(Orientation::Horizontal, 8);

    let label = Label::new(Some(label_text));
    label.set_halign(gtk4::Align::Start);
    label.add_css_class("section-label");
    label.set_hexpand(true);
    header.append(&label);

    let copy_btn = Button::with_label("📋");
    copy_btn.add_css_class("copy-button");
    copy_btn.set_tooltip_text(Some(copy_tooltip));
    header.append(&copy_btn);

    container.append(&header);

    // Scrolled text view
    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_min_content_height(80);
    scrolled.set_max_content_height(100);
    scrolled.set_policy(gtk4::PolicyType::Automatic, gtk4::PolicyType::Automatic);
    scrolled.add_css_class("text-pane");

    let textview = gtk4::TextView::new();
    textview.set_editable(false);
    textview.set_wrap_mode(gtk4::WrapMode::Word);
    textview.set_cursor_visible(false);
    textview.add_css_class("result-text");
    textview.set_left_margin(8);
    textview.set_right_margin(8);
    textview.set_top_margin(6);
    textview.set_bottom_margin(6);

    scrolled.set_child(Some(&textview));
    container.append(&scrolled);

    (container, textview, copy_btn)
}

fn build_ui(app: &Application, tray_state: Arc<Mutex<TrayState>>, visible: Arc<Mutex<bool>>) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Whisper Tool")
        .default_width(420)
        .default_height(520)
        .resizable(false)
        .decorated(false)
        .build();

    // CSS - Nord theme
    let css_provider = gtk4::CssProvider::new();
    css_provider.load_from_data(&format!(
        r#"
        window {{
            background-color: {nord0};
            border-radius: 12px;
        }}
        button {{
            min-width: 44px;
            min-height: 44px;
            border-radius: 22px;
            font-size: 18px;
            background-color: {nord1};
            color: {nord4};
            border: 1px solid {nord3};
        }}
        button:hover {{
            background-color: {nord2};
            border-color: {nord8};
        }}
        .close-button {{
            min-width: 32px;
            min-height: 32px;
            border-radius: 16px;
            font-size: 24px;
            font-weight: bold;
            padding: 0;
            background: transparent;
            border: none;
            color: {nord3};
        }}
        .close-button:hover {{
            background: {nord1};
            color: {nord11};
        }}
        .settings-button {{
            min-width: 32px;
            min-height: 32px;
            border-radius: 16px;
            font-size: 16px;
            padding: 0;
            background: transparent;
            border: none;
            color: {nord3};
        }}
        .settings-button:hover {{
            background: {nord1};
            color: {nord8};
        }}
        .copy-button {{
            min-width: 28px;
            min-height: 28px;
            border-radius: 14px;
            font-size: 14px;
            padding: 0;
            background: {nord1};
            border: 1px solid {nord3};
        }}
        .copy-button:hover {{
            background: {nord2};
            border-color: {nord8};
        }}
        .title-label {{
            color: {nord6};
            font-weight: bold;
            font-size: 14px;
        }}
        .timer-label {{
            color: {nord6};
            font-weight: bold;
            font-size: 18px;
        }}
        .status-label {{
            color: {nord4};
            font-size: 12px;
        }}
        .section-label {{
            color: {nord8};
            font-size: 11px;
            font-weight: bold;
        }}
        .section-title {{
            color: {nord9};
            font-size: 12px;
            font-weight: bold;
        }}
        .result-text {{
            font-size: 13px;
            color: {nord4};
            background-color: {nord1};
        }}
        .text-pane {{
            background-color: {nord1};
            border-radius: 6px;
            border: 1px solid {nord3};
        }}
        .mode-combo {{
            min-height: 32px;
            font-size: 12px;
            background-color: {nord1};
            color: {nord4};
            border: 1px solid {nord3};
            border-radius: 6px;
        }}
        .gain-scale {{
            min-height: 24px;
        }}
        .gain-scale trough {{
            background-color: {nord1};
            min-height: 6px;
            border-radius: 3px;
        }}
        .gain-scale highlight {{
            background-color: {nord8};
            border-radius: 3px;
        }}
        .gain-scale slider {{
            background-color: {nord4};
            min-width: 16px;
            min-height: 16px;
            border-radius: 8px;
        }}
        .gain-label {{
            color: {nord13};
            font-size: 11px;
            font-weight: bold;
            min-width: 60px;
        }}
        .dialog-button {{
            min-width: 80px;
            min-height: 32px;
            border-radius: 6px;
            font-size: 13px;
        }}
        .suggested-action {{
            background-color: {nord10};
            color: {nord6};
        }}
        .suggested-action:hover {{
            background-color: {nord9};
        }}
        dialog {{
            background-color: {nord0};
        }}
        label {{
            color: {nord4};
        }}
        checkbutton {{
            color: {nord4};
            font-size: 12px;
        }}
        checkbutton check {{
            background-color: {nord1};
            border: 1px solid {nord3};
            border-radius: 4px;
            min-width: 18px;
            min-height: 18px;
        }}
        checkbutton:checked check {{
            background-color: {nord10};
            border-color: {nord8};
        }}
        entry {{
            background-color: {nord1};
            color: {nord4};
            border: 1px solid {nord3};
            border-radius: 6px;
            padding: 6px 10px;
        }}
        entry:focus {{
            border-color: {nord8};
        }}
        scrolledwindow {{
            background-color: transparent;
        }}
        textview {{
            background-color: {nord1};
            color: {nord4};
        }}
        textview text {{
            background-color: {nord1};
            color: {nord4};
        }}
        .enhanced-label {{
            color: {nord14};
        }}
        "#,
        nord0 = nord::NORD0,
        nord1 = nord::NORD1,
        nord2 = nord::NORD2,
        nord3 = nord::NORD3,
        nord4 = nord::NORD4,
        nord6 = nord::NORD6,
        nord8 = nord::NORD8,
        nord9 = nord::NORD9,
        nord10 = nord::NORD10,
        nord11 = nord::NORD11,
        nord13 = nord::NORD13,
        nord14 = nord::NORD14,
    ));

    gtk4::style_context_add_provider_for_display(
        &gtk4::prelude::WidgetExt::display(&window),
        &css_provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    // Drag gesture for window
    let gesture = gtk4::GestureDrag::new();
    let window_clone = window.clone();
    gesture.connect_drag_begin(move |gesture, x, y| {
        if let Some(device) = gesture.device() {
            if let Some(surface) = window_clone.surface() {
                use gtk4::gdk;
                if let Ok(toplevel) = surface.downcast::<gdk::Toplevel>() {
                    toplevel.begin_move(&device, 0, x, y, gdk::CURRENT_TIME);
                }
            }
        }
    });

    let state = Rc::new(RefCell::new(RecorderState::new()));

    // tray_state and visible are now passed as parameters from main()
    // (initialized before the tray thread starts)

    // Main container
    let vbox = gtk4::Box::new(Orientation::Vertical, 0);

    // Title bar
    let titlebar = gtk4::Box::new(Orientation::Horizontal, 0);
    titlebar.set_margin_top(8);
    titlebar.set_margin_bottom(4);
    titlebar.set_margin_start(12);
    titlebar.set_margin_end(8);

    let drag_area = gtk4::Box::new(Orientation::Horizontal, 0);
    drag_area.set_hexpand(true);
    drag_area.add_controller(gesture);

    let title_label = Label::new(Some("🎤 Whisper Tool"));
    title_label.set_halign(gtk4::Align::Start);
    title_label.add_css_class("title-label");
    drag_area.append(&title_label);

    titlebar.append(&drag_area);

    // Settings button
    let settings_button = Button::with_label("⚙");
    settings_button.add_css_class("settings-button");
    settings_button.set_tooltip_text(Some("Settings"));
    let state_for_settings = Rc::clone(&state);
    let window_for_settings = window.clone();
    settings_button.connect_clicked(move |_| {
        show_settings_dialog(&window_for_settings, &state_for_settings);
    });
    titlebar.append(&settings_button);

    // Close button
    let close_button = Button::with_label("×");
    close_button.add_css_class("close-button");
    close_button.set_tooltip_text(Some("Hide window"));
    let visible_for_close = Arc::clone(&visible);
    close_button.connect_clicked(move |_| {
        if let Ok(mut v) = visible_for_close.try_lock() {
            *v = false;
        }
    });
    titlebar.append(&close_button);

    vbox.append(&titlebar);

    // Content
    let content = gtk4::Box::new(Orientation::Vertical, 6);
    content.set_margin_top(4);
    content.set_margin_bottom(12);
    content.set_margin_start(16);
    content.set_margin_end(16);

    // Mode selector row
    let mode_box = gtk4::Box::new(Orientation::Horizontal, 8);
    let mode_label = Label::new(Some("Mode:"));
    mode_label.add_css_class("section-label");
    mode_box.append(&mode_label);

    let mode_combo = ComboBoxText::new();
    mode_combo.add_css_class("mode-combo");
    mode_combo.append(Some("raw"), "Raw (no enhancement)");
    mode_combo.append(Some("clean"), "Clean (fix grammar)");
    mode_combo.append(Some("code"), "Code (for coding)");
    mode_combo.append(Some("email"), "Email (professional)");
    mode_combo.append(Some("notes"), "Notes (bullet points)");
    // Load persisted mode from config
    mode_combo.set_active_id(Some(&state.borrow().selected_mode));
    mode_combo.set_hexpand(true);

    let state_for_mode = Rc::clone(&state);
    mode_combo.connect_changed(move |combo| {
        if let Some(id) = combo.active_id() {
            let mut state = state_for_mode.borrow_mut();
            state.selected_mode = id.to_string();
            // Persist to config
            state.config.ui.mode = id.to_string();
            if let Err(e) = config::save_config(&state.config) {
                eprintln!("Failed to save mode setting: {}", e);
            }
        }
    });

    mode_box.append(&mode_combo);
    content.append(&mode_box);

    // Gain slider row
    let gain_box = gtk4::Box::new(Orientation::Horizontal, 8);
    let gain_label = Label::new(Some("Gain:"));
    gain_label.add_css_class("section-label");
    gain_box.append(&gain_label);

    let gain_scale = Scale::with_range(Orientation::Horizontal, 0.5, 10.0, 0.5);
    // Load persisted gain from config
    let initial_gain = state.borrow().config.ui.mic_gain as f64;
    gain_scale.set_value(initial_gain);
    gain_scale.set_hexpand(true);
    gain_scale.add_css_class("gain-scale");

    let initial_db = 20.0 * initial_gain.log10();
    let gain_value_label = Label::new(Some(&format!("{:+.0} dB", initial_db)));
    gain_value_label.add_css_class("gain-label");

    let state_for_gain = Rc::clone(&state);
    let gain_value_label_clone = gain_value_label.clone();
    gain_scale.connect_value_changed(move |scale| {
        let gain = scale.value() as f32;
        let db = 20.0 * (gain as f64).log10();
        gain_value_label_clone.set_text(&format!("{:+.0} dB", db));
        
        let mut state = state_for_gain.borrow_mut();
        *state.mic_gain.lock().unwrap() = gain;
        // Persist to config
        state.config.ui.mic_gain = gain;
        if let Err(e) = config::save_config(&state.config) {
            eprintln!("Failed to save gain setting: {}", e);
        }
    });

    gain_box.append(&gain_scale);
    gain_box.append(&gain_value_label);
    content.append(&gain_box);

    // Waveform
    let drawing_area = DrawingArea::new();
    drawing_area.set_content_width(388);
    drawing_area.set_content_height(40);

    let state_for_draw = Rc::clone(&state);
    drawing_area.set_draw_func(move |_area, cr, width, height| {
        let state = state_for_draw.borrow();
        let waveform_data = state.waveform_history.lock().unwrap();
        let is_recording = state.app_state == AppState::Recording;
        draw_waveform_bars(cr, &waveform_data, width, height, is_recording);
    });

    content.append(&drawing_area);

    // Controls row
    let controls = gtk4::Box::new(Orientation::Horizontal, 16);
    controls.set_halign(gtk4::Align::Center);
    controls.set_margin_top(8);

    let timer_label = Label::new(Some("00:00"));
    timer_label.add_css_class("timer-label");
    controls.append(&timer_label);

    let record_button = Button::with_label("⏺");
    record_button.set_tooltip_text(Some("Start/Stop recording"));
    controls.append(&record_button);

    content.append(&controls);

    // Status
    let status_label = Label::new(Some("Ready - Press record to start"));
    status_label.add_css_class("status-label");
    status_label.set_margin_top(4);
    content.append(&status_label);

    // === TRANSCRIPT PANE ===
    let (transcript_pane, transcript_textview, transcript_copy_btn) =
        create_text_pane("TRANSCRIPT (live)", "Copy transcript");
    transcript_pane.set_margin_top(8);
    content.append(&transcript_pane);

    // Wire up transcript copy button
    let transcript_buffer = transcript_textview.buffer();
    let transcript_buffer_for_copy = transcript_buffer.clone();
    transcript_copy_btn.connect_clicked(move |_| {
        let text = transcript_buffer_for_copy
            .text(&transcript_buffer_for_copy.start_iter(), &transcript_buffer_for_copy.end_iter(), false)
            .to_string();
        if !text.is_empty() {
            if let Err(e) = output::copy_to_clipboard(&text) {
                eprintln!("Clipboard error: {}", e);
            } else {
                show_notification("Copied", "Transcript copied to clipboard");
            }
        }
    });

    // === ENHANCED PANE ===
    let (enhanced_pane, enhanced_textview, enhanced_copy_btn) =
        create_text_pane("ENHANCED", "Copy enhanced text");
    enhanced_pane.set_margin_top(4);
    // Add green label style
    if let Some(first_child) = enhanced_pane.first_child() {
        if let Some(header_box) = first_child.downcast_ref::<gtk4::Box>() {
            if let Some(label) = header_box.first_child() {
                if let Some(label_widget) = label.downcast_ref::<Label>() {
                    label_widget.add_css_class("enhanced-label");
                }
            }
        }
    }
    content.append(&enhanced_pane);

    // Wire up enhanced copy button
    let enhanced_buffer = enhanced_textview.buffer();
    let enhanced_buffer_for_copy = enhanced_buffer.clone();
    enhanced_copy_btn.connect_clicked(move |_| {
        let text = enhanced_buffer_for_copy
            .text(&enhanced_buffer_for_copy.start_iter(), &enhanced_buffer_for_copy.end_iter(), false)
            .to_string();
        if !text.is_empty() {
            if let Err(e) = output::copy_to_clipboard(&text) {
                eprintln!("Clipboard error: {}", e);
            } else {
                show_notification("Copied", "Enhanced text copied to clipboard");
            }
        }
    });

    // === OPTIONS ROW ===
    let options_box = gtk4::Box::new(Orientation::Horizontal, 16);
    options_box.set_margin_top(8);
    options_box.set_halign(gtk4::Align::Center);

    let auto_enhance_check = CheckButton::with_label("Auto-enhance");
    // Load persisted value from config
    auto_enhance_check.set_active(state.borrow().auto_enhance);
    let state_for_enhance = Rc::clone(&state);
    auto_enhance_check.connect_toggled(move |btn| {
        let mut state = state_for_enhance.borrow_mut();
        state.auto_enhance = btn.is_active();
        // Persist to config
        state.config.ui.auto_enhance = btn.is_active();
        if let Err(e) = config::save_config(&state.config) {
            eprintln!("Failed to save auto_enhance setting: {}", e);
        }
    });
    options_box.append(&auto_enhance_check);

    let auto_copy_check = CheckButton::with_label("Auto-copy");
    // Load persisted value from config
    auto_copy_check.set_active(state.borrow().auto_copy);
    let state_for_copy_toggle = Rc::clone(&state);
    auto_copy_check.connect_toggled(move |btn| {
        let mut state = state_for_copy_toggle.borrow_mut();
        state.auto_copy = btn.is_active();
        // Persist to config
        state.config.ui.auto_copy = btn.is_active();
        if let Err(e) = config::save_config(&state.config) {
            eprintln!("Failed to save auto_copy setting: {}", e);
        }
    });
    options_box.append(&auto_copy_check);

    content.append(&options_box);

    vbox.append(&content);
    window.set_child(Some(&vbox));

    // === RECORD BUTTON HANDLER ===
    let state_for_record = Rc::clone(&state);
    let drawing_area_clone = drawing_area.clone();
    let timer_label_clone = timer_label.clone();
    let tray_state_for_btn = Arc::clone(&tray_state);
    let status_label_for_btn = status_label.clone();
    let transcript_buffer_for_record = transcript_buffer.clone();
    let enhanced_buffer_for_record = enhanced_buffer.clone();

    record_button.connect_clicked(move |button| {
        let mut state = state_for_record.borrow_mut();

        match state.app_state {
            AppState::Idle => {
                // Clear previous results
                transcript_buffer_for_record.set_text("");
                enhanced_buffer_for_record.set_text("");

                state.start_recording();
                button.set_label("⏹");
                status_label_for_btn.set_text("🔴 Recording...");
                if let Ok(mut tray) = tray_state_for_btn.try_lock() {
                    tray.recording = true;
                }
            }
            AppState::Recording => {
                let samples = state.stop_recording();
                button.set_label("⏺");
                button.set_sensitive(false);
                timer_label_clone.set_text("Processing...");
                status_label_for_btn.set_text("⏳ Transcribing...");

                if let Ok(mut tray) = tray_state_for_btn.try_lock() {
                    tray.recording = false;
                    tray.processing = true;
                }

                let config = state.config.clone();
                let mode = state.selected_mode.clone();
                let sample_rate = config.audio.sample_rate;
                let auto_enhance = state.auto_enhance && mode != "raw";
                let auto_copy = state.auto_copy;
                let tray_state_bg = Arc::clone(&tray_state_for_btn);
                let whisper_engine = Arc::clone(&state.whisper_engine);

                // Channel for results: (transcript, enhanced_option)
                let (tx, rx) = std::sync::mpsc::channel::<(String, Option<String>)>();

                std::thread::spawn(move || {
                    let audio_data = audio::AudioData {
                        samples,
                        sample_rate,
                    };

                    // Single complete transcription using cached engine
                    let transcript = match whisper_engine.transcribe(&audio_data, &config) {
                        Ok(t) => {
                            println!("Transcription: {}", t);
                            t
                        }
                        Err(e) => {
                            eprintln!("Transcription error: {}", e);
                            format!("Error: {}", e)
                        }
                    };

                    // Enhancement if enabled
                    let enhanced = if auto_enhance && !transcript.starts_with("Error:") {
                        let rt = tokio::runtime::Runtime::new().unwrap();
                        match rt.block_on(enhance::enhance(&transcript, &mode, &config)) {
                            Ok(e) => {
                                println!("Enhanced: {}", e);
                                Some(e)
                            }
                            Err(e) => {
                                eprintln!("Enhancement error: {}", e);
                                None
                            }
                        }
                    } else {
                        None
                    };

                    if let Ok(mut tray) = tray_state_bg.try_lock() {
                        tray.processing = false;
                    }

                    let _ = tx.send((transcript, enhanced));
                });

                // Poll for results
                let state_for_result = Rc::clone(&state_for_record);
                let button_clone = button.clone();
                let timer_clone = timer_label_clone.clone();
                let status_clone = status_label_for_btn.clone();
                let transcript_buf = transcript_buffer_for_record.clone();
                let enhanced_buf = enhanced_buffer_for_record.clone();

                glib::timeout_add_local(Duration::from_millis(100), move || {
                    match rx.try_recv() {
                        Ok((transcript, enhanced)) => {
                            let mut state = state_for_result.borrow_mut();
                            state.app_state = AppState::Idle;

                            // Update transcript pane
                            transcript_buf.set_text(&transcript);

                            // Update enhanced pane if available
                            if let Some(ref enh) = enhanced {
                                enhanced_buf.set_text(enh);
                                state.enhanced_result = enh.clone();
                            }

                            // Auto-copy: prefer enhanced, fallback to transcript
                            if auto_copy {
                                let copy_text = enhanced.as_ref().unwrap_or(&transcript);
                                if let Err(e) = output::copy_to_clipboard(copy_text) {
                                    eprintln!("Clipboard error: {}", e);
                                } else {
                                    show_notification("Copied to clipboard", copy_text);
                                }
                            }

                            button_clone.set_sensitive(true);
                            timer_clone.set_text("00:00");
                            status_clone.set_text("Ready - Press record to start");

                            glib::ControlFlow::Break
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            button_clone.set_sensitive(true);
                            timer_clone.set_text("00:00");
                            status_clone.set_text("Error occurred");
                            glib::ControlFlow::Break
                        }
                    }
                });
            }
        }

        drawing_area_clone.queue_draw();
    });

    // === WINDOW VISIBILITY MONITORING ===
    let window_clone = window.clone();
    let visible_clone = Arc::clone(&visible);
    glib::timeout_add_local(Duration::from_millis(50), move || {
        if let Ok(should_be_visible) = visible_clone.try_lock() {
            let is_visible = window_clone.is_visible();
            if *should_be_visible && !is_visible {
                window_clone.present();
            } else if !*should_be_visible && is_visible {
                window_clone.hide();
            }
        }
        glib::ControlFlow::Continue
    });

    // === PERIODIC UI UPDATE (timer, waveform) ===
    let state_for_timer = Rc::clone(&state);
    let timer_label_update = timer_label.clone();
    let drawing_area_update = drawing_area.clone();
    let status_label_update = status_label.clone();
    let record_button_update = record_button.clone();
    let tray_state_update = Arc::clone(&tray_state);
    let window_for_floating = window.clone();

    glib::timeout_add_local(Duration::from_millis(50), move || {
        let state = state_for_timer.borrow();

        // Check tray commands - use lock() instead of try_lock() to ensure we process requests
        if let Ok(mut tray) = tray_state_update.lock() {
            if tray.start_requested && state.app_state == AppState::Idle {
                tray.start_requested = false;
                drop(tray);
                drop(state);
                record_button_update.emit_clicked();
                return glib::ControlFlow::Continue;
            }
            if tray.stop_requested && state.app_state == AppState::Recording {
                tray.stop_requested = false;
                drop(tray);
                drop(state);
                record_button_update.emit_clicked();
                return glib::ControlFlow::Continue;
            }
            if tray.floating_requested && state.app_state == AppState::Idle {
                tray.floating_requested = false;
                let config = state.config.clone();
                // Use quick_dictate.mode if configured, otherwise use current UI mode
                let mode = config.quick_dictate.mode.clone()
                    .unwrap_or_else(|| state.selected_mode.clone());
                let whisper_engine = Arc::clone(&state.whisper_engine);
                drop(tray);
                drop(state);
                floating::launch_floating_window(
                    &window_for_floating,
                    config,
                    mode,
                    whisper_engine,
                    &FLOATING_ACTIVE,
                    &FLOATING_STOP_REQUESTED,
                );
                return glib::ControlFlow::Continue;
            }
        }

        // Update timer during recording (no streaming transcription - just waveform)
        if state.app_state == AppState::Recording {
            if let Some(start) = state.start_time {
                let elapsed = start.elapsed();
                let secs = elapsed.as_secs();
                let mins = secs / 60;
                let secs_display = secs % 60;
                timer_label_update.set_text(&format!("{:02}:{:02}", mins, secs_display));
            }
            status_label_update.set_text("🔴 Recording...");
        }

        drop(state);
        drawing_area_update.queue_draw();
        glib::ControlFlow::Continue
    });

    // === KEYBOARD SHORTCUT HANDLING ===
    let key_controller = gtk4::EventControllerKey::new();
    let state_for_keys = Rc::clone(&state);
    let record_button_for_keys = record_button.clone();
    let transcript_buffer_for_keys = transcript_buffer.clone();
    let enhanced_buffer_for_keys = enhanced_buffer.clone();
    let visible_for_keys = Arc::clone(&visible);
    let settings_button_for_keys = settings_button.clone();
    let window_for_keys = window.clone();

    key_controller.connect_key_pressed(move |_, keyval, _keycode, modifier| {
        // Clone shortcuts to avoid borrow issues
        let shortcuts = state_for_keys.borrow().config.shortcuts.bindings.clone();

        // Build the current keypress string in GTK format
        let key_name = keyval.name().map(|s| s.to_string()).unwrap_or_default();
        let current_keypress = format_keypress(&key_name, modifier);

        // Check each configured shortcut
        for (action, binding) in shortcuts.iter() {
            if binding.eq_ignore_ascii_case(&current_keypress) {
                match action.as_str() {
                    "toggle_recording" => {
                        record_button_for_keys.emit_clicked();
                        return gtk4::glib::Propagation::Stop;
                    }
                    "cancel_recording" => {
                        let state = state_for_keys.borrow();
                        if state.app_state == AppState::Recording {
                            drop(state);
                            // Stop recording without processing (just click the button)
                            record_button_for_keys.emit_clicked();
                        }
                        return gtk4::glib::Propagation::Stop;
                    }
                    "copy_transcript" => {
                        let text = transcript_buffer_for_keys
                            .text(
                                &transcript_buffer_for_keys.start_iter(),
                                &transcript_buffer_for_keys.end_iter(),
                                false,
                            )
                            .to_string();
                        if !text.is_empty() {
                            if let Err(e) = output::copy_to_clipboard(&text) {
                                eprintln!("Clipboard error: {}", e);
                            } else {
                                show_notification("Copied", "Transcript copied to clipboard");
                            }
                        }
                        return gtk4::glib::Propagation::Stop;
                    }
                    "copy_enhanced" => {
                        let text = enhanced_buffer_for_keys
                            .text(
                                &enhanced_buffer_for_keys.start_iter(),
                                &enhanced_buffer_for_keys.end_iter(),
                                false,
                            )
                            .to_string();
                        if !text.is_empty() {
                            if let Err(e) = output::copy_to_clipboard(&text) {
                                eprintln!("Clipboard error: {}", e);
                            } else {
                                show_notification("Copied", "Enhanced text copied to clipboard");
                            }
                        }
                        return gtk4::glib::Propagation::Stop;
                    }
                    "show_hide_window" => {
                        if let Ok(mut v) = visible_for_keys.try_lock() {
                            *v = !*v;
                        }
                        return gtk4::glib::Propagation::Stop;
                    }
                    "open_settings" => {
                        settings_button_for_keys.emit_clicked();
                        return gtk4::glib::Propagation::Stop;
                    }
                    "quick_dictate" => {
                        let state = state_for_keys.borrow();
                        if state.app_state == AppState::Idle {
                            let config = state.config.clone();
                            let mode = config.quick_dictate.mode.clone()
                                .unwrap_or_else(|| state.selected_mode.clone());
                            let whisper_engine = Arc::clone(&state.whisper_engine);
                            drop(state);
                            floating::launch_floating_window(
                                &window_for_keys,
                                config,
                                mode,
                                whisper_engine,
                                &FLOATING_ACTIVE,
                                &FLOATING_STOP_REQUESTED,
                            );
                        }
                        return gtk4::glib::Propagation::Stop;
                    }
                    _ => {}
                }
                break;
            }
        }

        gtk4::glib::Propagation::Proceed
    });

    window.add_controller(key_controller);

    window.present();
}

fn main() -> glib::ExitCode {
    *NOTIFIER.lock().unwrap() = Some(Notifier::new());

    // Initialize global state BEFORE starting tray thread
    // This ensures the tray icon can communicate with the app immediately
    let tray_state = Arc::new(Mutex::new(TrayState::default()));
    *RECORDING_STATE.lock().unwrap() = Some(Arc::clone(&tray_state));

    let visible = Arc::new(Mutex::new(true));
    *WINDOW_VISIBLE.lock().unwrap() = Some(Arc::clone(&visible));

    let app = Application::builder().application_id(APP_ID).build();

    // Pass the pre-initialized state to build_ui
    let tray_state_for_ui = Arc::clone(&tray_state);
    let visible_for_ui = Arc::clone(&visible);
    app.connect_activate(move |app| {
        build_ui(app, Arc::clone(&tray_state_for_ui), Arc::clone(&visible_for_ui));
    });

    // Start tray icon thread AFTER globals are initialized
    std::thread::spawn(|| {
        start_tray_icon();
    });

    // Setup SIGUSR1 handler for global quick dictate shortcut (toggle)
    // Usage: pkill -SIGUSR1 -f whisper-tool-gui (or bind in Hyprland config)
    glib::unix_signal_add_local(libc::SIGUSR1, move || {
        if FLOATING_ACTIVE.load(Ordering::SeqCst) {
            // Floating window is active -> request stop
            FLOATING_STOP_REQUESTED.store(true, Ordering::SeqCst);
        } else {
            // No floating window -> request start
            if let Ok(mut state) = tray_state.lock() {
                state.floating_requested = true;
            }
        }
        glib::ControlFlow::Continue
    });

    app.run()
}
