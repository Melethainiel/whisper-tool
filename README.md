# whisper-tool

Voice dictation tool for Linux with Speech-to-Text (Whisper) and LLM enhancement (Ollama).

## Features

- **Voice Recording**: Capture audio from microphone
- **Speech-to-Text**: Local transcription using Whisper
- **LLM Enhancement**: Improve text with Ollama (optional)
- **Clipboard Integration**: Auto-copy results
- **GUI Interface**: GTK4 interface with system tray icon
- **CLI Interface**: Command-line interface for scripting

## Enhancement Modes

| Mode | Description |
|------|-------------|
| `raw` | No enhancement, raw transcription |
| `clean` | Fix spelling and grammar |
| `code` | Optimize for code generation prompts |
| `email` | Professional email style |
| `notes` | Bullet point format |

## Installation

### Prerequisites

```bash
# Ubuntu/Debian
sudo apt install libasound2-dev libclang-dev cmake libgtk-4-dev libdbus-1-dev

# Arch
sudo pacman -S alsa-lib clang cmake gtk4 dbus

# Ollama (optional, for LLM enhancement)
curl -fsSL https://ollama.com/install.sh | sh
ollama pull llama3.2
```

### Build

```bash
cargo build --release
```

Binaries:
- `target/release/whisper-tool` - CLI interface
- `target/release/whisper-tool-gui` - GTK4 GUI with tray icon

## Usage

### GUI Mode

```bash
whisper-tool-gui
```

Features:
- System tray icon for quick access
- Mode selector (raw, clean, code, email, notes)
- Real-time waveform visualization
- Auto-copy to clipboard
- Desktop notifications

### CLI Mode

#### Download Whisper model (first time)

```bash
whisper-tool download --model base
```

Available models: `tiny`, `base`, `small`, `medium`, `large`

#### Record and transcribe

```bash
# Basic recording (Ctrl+C to stop)
whisper-tool record

# With duration limit (5 seconds)
whisper-tool record --duration 5

# With LLM enhancement for code
whisper-tool record --mode code

# Clean up grammar
whisper-tool record --mode clean
```

#### List audio devices

```bash
whisper-tool devices
```

## Configuration

Create `~/.config/whisper-tool/config.toml`:

```toml
[audio]
sample_rate = 16000
# device = "pulse"

[whisper]
model = "base"
# language = "fr"

[ollama]
url = "http://localhost:11434"
model = "llama3.2"
```

## Screenshots

```
┌─────────────────────────────────────┐
│ 🎤 Whisper Tool                   × │
├─────────────────────────────────────┤
│ Mode: [Code (for coding)      ▼]    │
│                                     │
│ Audio Input                         │
│ ▁▂▃▅▇▅▃▂▁▂▃▅▇▅▃▂▁▂▃▅▇▅▃▂▁         │
│                                     │
│        00:05    ⏹    📋            │
│                                     │
│ Ready - Press record to start       │
│ ┌─────────────────────────────────┐ │
│ │ Last Result                     │ │
│ │ Create a function that...       │ │
│ └─────────────────────────────────┘ │
└─────────────────────────────────────┘
```

## License

MIT
