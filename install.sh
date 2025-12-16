#!/bin/bash
set -e

REPO="Melethainiel/whisper-tool"
VERSION="latest"

echo "Installing Whisper Tool..."

# Check if running as root
if [ "$EUID" -eq 0 ]; then
    echo "Error: Do not run this script as root (without sudo)"
    echo "The script will ask for sudo when needed"
    exit 1
fi

# Detect architecture
ARCH=$(uname -m)
case $ARCH in
    x86_64)
        BINARY_NAME="whisper-tool-gui-linux-x86_64"
        ;;
    aarch64)
        BINARY_NAME="whisper-tool-gui-linux-aarch64"
        ;;
    *)
        echo "Error: Unsupported architecture: $ARCH"
        echo "Supported: x86_64, aarch64"
        exit 1
        ;;
esac

# Check for required dependencies
echo "Checking dependencies..."
MISSING_DEPS=""

if ! command -v wtype &> /dev/null; then
    MISSING_DEPS="$MISSING_DEPS wtype"
fi

if [ -n "$MISSING_DEPS" ]; then
    echo ""
    echo "Warning: Missing optional dependencies:$MISSING_DEPS"
    echo "Install them for full functionality:"
    echo "  Arch: sudo pacman -S$MISSING_DEPS"
    echo "  Ubuntu: sudo apt install$MISSING_DEPS"
    echo ""
fi

# Create temporary directory
TMP_DIR=$(mktemp -d)
trap "rm -rf $TMP_DIR" EXIT

cd "$TMP_DIR"

# Download binary from GitHub releases
echo "Downloading Whisper Tool..."
if command -v curl &> /dev/null; then
    curl -L "https://github.com/$REPO/releases/latest/download/$BINARY_NAME" -o whisper-tool-gui
elif command -v wget &> /dev/null; then
    wget "https://github.com/$REPO/releases/latest/download/$BINARY_NAME" -O whisper-tool-gui
else
    echo "Error: Neither curl nor wget found. Please install one of them."
    exit 1
fi

# Download icon
echo "Downloading icon..."
if command -v curl &> /dev/null; then
    curl -L "https://github.com/$REPO/releases/latest/download/whisper-tool.png" -o whisper-tool.png
else
    wget "https://github.com/$REPO/releases/latest/download/whisper-tool.png" -O whisper-tool.png
fi

# Make binary executable
chmod +x whisper-tool-gui

# Install binary
echo "Installing to /usr/local/bin..."
sudo install -Dm755 whisper-tool-gui /usr/local/bin/whisper-tool-gui

# Install icon
sudo install -Dm644 whisper-tool.png /usr/share/pixmaps/whisper-tool.png

# Create desktop entry
echo "Creating desktop entry..."
sudo tee /usr/share/applications/whisper-tool.desktop > /dev/null << EOF
[Desktop Entry]
Name=Whisper Tool
Comment=Voice dictation with Whisper STT and LLM enhancement
Exec=/usr/local/bin/whisper-tool-gui
Icon=whisper-tool
Terminal=false
Type=Application
Categories=AudioVideo;Audio;Utility;
Keywords=whisper;voice;dictation;transcription;stt;
StartupWMClass=whisper-tool
EOF

# Create default config directory
mkdir -p ~/.config/whisper-tool

# Create example config if none exists
if [ ! -f ~/.config/whisper-tool/config.toml ]; then
    echo "Creating example config..."
    cat > ~/.config/whisper-tool/config.toml << 'EOF'
# Whisper Tool Configuration

[whisper]
# Model size: tiny, base, small, medium, large
model = "base"

[llm]
# Provider: ollama, openai, anthropic, openrouter
provider = "ollama"
url = "http://localhost:11434"
model = "llama3.2"
# api_key = "your-api-key"  # Required for non-ollama providers

[ui]
# Default mode: raw, dictation, clean, email, notes, code
mode = "dictation"
mic_gain = 2.0
auto_enhance = true
auto_copy = true

[quick_dictate]
# Mode for quick dictate (floating window). If not set, uses current UI mode.
# mode = "dictation"
EOF
fi

echo ""
echo "✓ Whisper Tool installed successfully!"
echo ""
echo "You can now:"
echo "  - Launch it from your application menu"
echo "  - Run 'whisper-tool-gui' in terminal"
echo "  - Use the system tray icon"
echo ""
echo "For global shortcut (Hyprland), add to ~/.config/hypr/hyprland.conf:"
echo "  bind = CTRL SHIFT, D, exec, pkill -SIGUSR1 -f whisper-tool-gui"
echo "  exec-once = whisper-tool-gui"
echo ""
echo "Config file: ~/.config/whisper-tool/config.toml"
