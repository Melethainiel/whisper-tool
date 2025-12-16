//! Floating dictation window module
//! 
//! Provides a minimal popup window near the cursor for quick voice-to-text
//! with automatic paste functionality.

use std::process::Command;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{ApplicationWindow, Label, Orientation, Window};

use crate::audio::AudioData;
use crate::config::Config;
use crate::enhance;
use crate::transcribe::WhisperEngine;

/// Offset from cursor position (pixels)
const CURSOR_OFFSET_X: i32 = 20;
const CURSOR_OFFSET_Y: i32 = 20;

/// Window dimensions  
const WINDOW_WIDTH: i32 = 200;
const WINDOW_HEIGHT: i32 = 60;

// Nord colors for the floating window
mod colors {
    pub const NORD0: &str = "#2e3440";
    pub const NORD3: &str = "#4c566a";
    pub const NORD4: &str = "#d8dee9";
    pub const NORD11: &str = "#bf616a";
    pub const NORD13: &str = "#ebcb8b";
    pub const NORD14: &str = "#a3be8c";
}

/// Get current cursor position using hyprctl (Hyprland)
/// Returns (x, y) coordinates or None if failed
pub fn get_cursor_position() -> Option<(i32, i32)> {
    let output = Command::new("hyprctl")
        .arg("cursorpos")
        .output()
        .ok()?;
    
    if !output.status.success() {
        return None;
    }
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parts: Vec<&str> = stdout.trim().split(", ").collect();
    
    if parts.len() != 2 {
        return None;
    }
    
    let x = parts[0].parse::<i32>().ok()?;
    let y = parts[1].parse::<i32>().ok()?;
    
    Some((x, y))
}

/// Copy text to clipboard using wl-copy
fn copy_to_clipboard_wl(text: &str) -> Result<(), String> {
    use std::io::Write;
    
    let mut child = Command::new("wl-copy")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn wl-copy: {}", e))?;
    
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(text.as_bytes())
            .map_err(|e| format!("Failed to write to wl-copy: {}", e))?;
    }
    
    child.wait()
        .map_err(|e| format!("wl-copy failed: {}", e))?;
    
    Ok(())
}

/// Simulate Ctrl+V paste using wtype
fn simulate_paste() -> Result<(), String> {
    // Small delay to ensure clipboard is ready
    std::thread::sleep(Duration::from_millis(100));
    
    Command::new("wtype")
        .args(["-M", "ctrl", "-k", "v", "-m", "ctrl"])
        .status()
        .map_err(|e| format!("Failed to run wtype: {}", e))?;
    
    Ok(())
}

/// Type text directly using wtype (alternative to paste)
fn type_text_directly(text: &str) -> Result<(), String> {
    std::thread::sleep(Duration::from_millis(100));
    
    Command::new("wtype")
        .arg(text)
        .status()
        .map_err(|e| format!("Failed to run wtype: {}", e))?;
    
    Ok(())
}

/// Copy text to clipboard and paste it
pub fn paste_text(text: &str) -> Result<(), String> {
    copy_to_clipboard_wl(text)?;
    simulate_paste()
}

#[derive(Clone, Copy, PartialEq)]
pub enum FloatingState {
    Recording,
    Processing,
    Done,
    Error,
}

impl FloatingState {
    pub fn icon(&self) -> &'static str {
        match self {
            FloatingState::Recording => "🎤",
            FloatingState::Processing => "⏳",
            FloatingState::Done => "✓",
            FloatingState::Error => "✗",
        }
    }
    
    pub fn label(&self) -> &'static str {
        match self {
            FloatingState::Recording => "Recording...",
            FloatingState::Processing => "Processing...",
            FloatingState::Done => "Done!",
            FloatingState::Error => "Error",
        }
    }
}

/// Launch the floating dictation window as a child of the main app window
pub fn launch_floating_window(
    parent: &ApplicationWindow,
    config: Config,
    mode: String,
    whisper_engine: Arc<WhisperEngine>,
    floating_active: &'static AtomicBool,
    stop_requested: &'static AtomicBool,
) {
    // Mark floating window as active
    floating_active.store(true, Ordering::SeqCst);
    stop_requested.store(false, Ordering::SeqCst);
    
    // Get cursor position before creating window
    let (cursor_x, cursor_y) = get_cursor_position().unwrap_or((100, 100));
    
    let window = Window::builder()
        .transient_for(parent)
        .title("Dictation")
        .default_width(WINDOW_WIDTH)
        .default_height(WINDOW_HEIGHT)
        .resizable(false)
        .decorated(false)
        .modal(false)
        .build();
    
    // Apply CSS
    let css_provider = gtk4::CssProvider::new();
    css_provider.load_from_data(&format!(
        r#"
        .floating-window {{
            background-color: {};
            border-radius: 12px;
            border: 2px solid {};
        }}
        .floating-icon {{
            font-size: 24px;
        }}
        .floating-label {{
            color: {};
            font-size: 14px;
            font-weight: bold;
        }}
        "#,
        colors::NORD0,
        colors::NORD3,
        colors::NORD4,
    ));
    
    gtk4::style_context_add_provider_for_display(
        &gtk4::prelude::WidgetExt::display(&window),
        &css_provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    
    window.add_css_class("floating-window");
    
    // Layout
    let hbox = gtk4::Box::new(Orientation::Horizontal, 12);
    hbox.set_margin_top(12);
    hbox.set_margin_bottom(12);
    hbox.set_margin_start(16);
    hbox.set_margin_end(16);
    hbox.set_halign(gtk4::Align::Center);
    hbox.set_valign(gtk4::Align::Center);
    
    let icon_label = Label::new(Some("🎤"));
    icon_label.add_css_class("floating-icon");
    hbox.append(&icon_label);
    
    let status_label = Label::new(Some("Recording..."));
    status_label.add_css_class("floating-label");
    hbox.append(&status_label);
    
    window.set_child(Some(&hbox));
    
    // Recording state
    let samples = Arc::new(Mutex::new(Vec::<f32>::new()));
    let running = Arc::new(AtomicBool::new(true));
    let recording_stopped = Arc::new(AtomicBool::new(false));
    let mic_gain = config.ui.mic_gain;
    let sample_rate = config.audio.sample_rate;
    
    // Start recording immediately
    let samples_for_stream = Arc::clone(&samples);
    let running_for_stream = Arc::clone(&running);
    
    let host = cpal::default_host();
    let device = host.default_input_device().expect("No input device");
    let stream_config = cpal::StreamConfig {
        channels: 1,
        sample_rate: cpal::SampleRate(sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };
    
    let stream = device
        .build_input_stream(
            &stream_config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if !running_for_stream.load(Ordering::SeqCst) {
                    return;
                }
                let gained: Vec<f32> = data
                    .iter()
                    .map(|&s| (s * mic_gain).clamp(-1.0, 1.0))
                    .collect();
                samples_for_stream.lock().unwrap().extend_from_slice(&gained);
            },
            |err| eprintln!("Stream error: {}", err),
            None,
        )
        .expect("Failed to build input stream");
    
    stream.play().expect("Failed to start stream");
    
    // Store stream in Arc to keep it alive and allow stopping
    let stream = Arc::new(Mutex::new(Some(stream)));
    
    // Stop recording on click anywhere in window
    let click_gesture = gtk4::GestureClick::new();
    let running_click = Arc::clone(&running);
    let recording_stopped_click = Arc::clone(&recording_stopped);
    let stream_click = Arc::clone(&stream);
    
    click_gesture.connect_released(move |_, _, _, _| {
        // Only process first click
        if recording_stopped_click.load(Ordering::SeqCst) {
            return;
        }
        recording_stopped_click.store(true, Ordering::SeqCst);
        
        // Stop recording
        running_click.store(false, Ordering::SeqCst);
        if let Ok(mut guard) = stream_click.lock() {
            if let Some(s) = guard.take() {
                drop(s);
            }
        }
    });
    
    window.add_controller(click_gesture);
    
    // Also stop on Escape key (and close window)
    let key_controller = gtk4::EventControllerKey::new();
    let window_for_key = window.clone();
    let running_key = Arc::clone(&running);
    let stream_key = Arc::clone(&stream);
    
    key_controller.connect_key_pressed(move |_, keyval, _, _| {
        if keyval == gtk4::gdk::Key::Escape {
            // Stop recording and close
            running_key.store(false, Ordering::SeqCst);
            if let Ok(mut guard) = stream_key.lock() {
                guard.take();
            }
            window_for_key.close();
            return gtk4::glib::Propagation::Stop;
        }
        gtk4::glib::Propagation::Proceed
    });
    window.add_controller(key_controller);
    
    // Position window using Hyprland
    let pos_x = cursor_x + CURSOR_OFFSET_X;
    let pos_y = cursor_y + CURSOR_OFFSET_Y;
    
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        let _ = Command::new("hyprctl")
            .args([
                "dispatch",
                "movewindowpixel",
                &format!("exact {} {},title:Dictation", pos_x, pos_y),
            ])
            .output();
    });
    
    window.present();
    
    // Monitor for recording stop and process
    let config = Arc::new(config);
    let mode = Arc::new(mode);
    let icon_label_poll = icon_label.clone();
    let status_label_poll = status_label.clone();
    let window_poll = window.clone();
    let samples_poll = Arc::clone(&samples);
    let recording_stopped_poll = Arc::clone(&recording_stopped);
    let running_poll = Arc::clone(&running);
    let stream_poll = Arc::clone(&stream);
    
    glib::timeout_add_local(Duration::from_millis(100), move || {
        // Check if global stop was requested (SIGUSR1 toggle)
        if stop_requested.load(Ordering::SeqCst) && !recording_stopped_poll.load(Ordering::SeqCst) {
            stop_requested.store(false, Ordering::SeqCst);
            recording_stopped_poll.store(true, Ordering::SeqCst);
            running_poll.store(false, Ordering::SeqCst);
            if let Ok(mut guard) = stream_poll.lock() {
                guard.take();
            }
        }
        
        // Wait for recording to stop
        if !recording_stopped_poll.load(Ordering::SeqCst) {
            return glib::ControlFlow::Continue;
        }
        
        // Mark floating as inactive now that recording stopped
        floating_active.store(false, Ordering::SeqCst);
        
        // Update UI to processing
        icon_label_poll.set_label(FloatingState::Processing.icon());
        status_label_poll.set_label(FloatingState::Processing.label());
        
        // Get samples
        let recorded_samples = samples_poll.lock().unwrap().clone();
        
        if recorded_samples.is_empty() {
            icon_label_poll.set_label(FloatingState::Error.icon());
            status_label_poll.set_label("No audio");
            let window_close = window_poll.clone();
            glib::timeout_add_local_once(Duration::from_millis(1000), move || {
                window_close.close();
            });
            return glib::ControlFlow::Break;
        }
        
        // Process in background thread
        let config_bg = Arc::clone(&config);
        let mode_bg = Arc::clone(&mode);
        let whisper_bg = Arc::clone(&whisper_engine);
        let sample_rate_bg = config.audio.sample_rate;
        
        let (tx, rx) = std::sync::mpsc::channel::<Result<String, String>>();
        
        std::thread::spawn(move || {
            let audio_data = AudioData {
                samples: recorded_samples,
                sample_rate: sample_rate_bg,
            };
            
            // Transcribe
            let transcript = match whisper_bg.transcribe(&audio_data, &config_bg) {
                Ok(t) => {
                    println!("[Floating] Transcription: {}", t);
                    t
                }
                Err(e) => {
                    let _ = tx.send(Err(format!("Transcription error: {}", e)));
                    return;
                }
            };
            
            // Enhance if not raw mode
            let result = if mode_bg.as_str() != "raw" && !transcript.is_empty() {
                let rt = tokio::runtime::Runtime::new().unwrap();
                match rt.block_on(enhance::enhance(&transcript, &mode_bg, &config_bg)) {
                    Ok(enhanced) => {
                        println!("[Floating] Enhanced: {}", enhanced);
                        enhanced
                    }
                    Err(e) => {
                        eprintln!("[Floating] Enhancement error: {}", e);
                        transcript // Fallback to transcript
                    }
                }
            } else {
                transcript
            };
            
            let _ = tx.send(Ok(result));
        });
        
        // Poll for processing result
        let icon_final = icon_label_poll.clone();
        let status_final = status_label_poll.clone();
        let window_final = window_poll.clone();
        
        glib::timeout_add_local(Duration::from_millis(100), move || {
            match rx.try_recv() {
                Ok(Ok(text)) => {
                    if text.trim().is_empty() {
                        icon_final.set_label(FloatingState::Error.icon());
                        status_final.set_label("Empty result");
                        
                        let window_close = window_final.clone();
                        glib::timeout_add_local_once(Duration::from_millis(800), move || {
                            window_close.close();
                        });
                    } else {
                        icon_final.set_label(FloatingState::Done.icon());
                        status_final.set_label("Done!");
                        
                        // Close window FIRST, then type text after focus returns to previous app
                        let text_clone = text.clone();
                        let window_close = window_final.clone();
                        
                        glib::timeout_add_local_once(Duration::from_millis(100), move || {
                            window_close.close();
                            
                            // Type text after window closes and focus returns
                            std::thread::spawn(move || {
                                std::thread::sleep(Duration::from_millis(200));
                                if let Err(e) = type_text_directly(&text_clone) {
                                    eprintln!("[Floating] Type error: {}", e);
                                }
                            });
                        });
                    }
                    
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("[Floating] Error: {}", e);
                    icon_final.set_label(FloatingState::Error.icon());
                    status_final.set_label("Error");
                    
                    let window_close = window_final.clone();
                    glib::timeout_add_local_once(Duration::from_millis(1500), move || {
                        window_close.close();
                    });
                    
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    glib::ControlFlow::Continue
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    icon_final.set_label(FloatingState::Error.icon());
                    status_final.set_label("Error");
                    
                    let window_close = window_final.clone();
                    glib::timeout_add_local_once(Duration::from_millis(1000), move || {
                        window_close.close();
                    });
                    
                    glib::ControlFlow::Break
                }
            }
        });
        
        glib::ControlFlow::Break
    });
}
