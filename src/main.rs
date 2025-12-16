mod audio;
mod config;
mod enhance;
mod output;
mod transcribe;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::info;

#[derive(Parser)]
#[command(name = "whisper-tool")]
#[command(about = "Voice dictation with STT and LLM enhancement")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Config file path
    #[arg(short, long, global = true)]
    config: Option<String>,

    /// Verbose output
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Record and transcribe voice
    Record {
        /// Enhancement mode: raw, clean, code, email, notes
        #[arg(short, long, default_value = "raw")]
        mode: String,

        /// Recording duration in seconds (0 = manual stop with Ctrl+C)
        #[arg(short, long, default_value = "0")]
        duration: u64,

        /// Copy result to clipboard
        #[arg(long, default_value = "true")]
        clipboard: bool,
    },
    /// List available audio devices
    Devices,
    /// Download Whisper model
    Download {
        /// Model size: tiny, base, small, medium, large
        #[arg(short, long, default_value = "base")]
        model: String,
    },
}

fn setup_logging(verbose: bool) {
    let filter = if verbose { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    setup_logging(cli.verbose);

    let config = config::load_config(cli.config.as_deref())?;

    match cli.command {
        Commands::Record {
            mode,
            duration,
            clipboard,
        } => {
            info!("Recording... (Ctrl+C to stop)");

            // Record audio
            let audio_data = audio::record(duration, &config)?;
            info!("Recording complete, transcribing...");

            // Transcribe
            let transcript = transcribe::transcribe(&audio_data, &config)?;
            info!("Transcription: {}", transcript);

            // Enhance if needed
            let result = if mode != "raw" {
                info!("Enhancing with mode: {}", mode);
                enhance::enhance(&transcript, &mode, &config).await?
            } else {
                transcript
            };

            // Output
            println!("\n{}", result);

            if clipboard {
                output::copy_to_clipboard(&result)?;
                info!("Copied to clipboard");
            }
        }
        Commands::Devices => {
            audio::list_devices()?;
        }
        Commands::Download { model } => {
            transcribe::download_model(&model, &config)?;
        }
    }

    Ok(())
}
