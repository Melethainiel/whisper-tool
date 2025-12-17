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
        ARCH_SUFFIX="x86_64"
        ;;
    aarch64)
        ARCH_SUFFIX="aarch64"
        ;;
    *)
        echo "Error: Unsupported architecture: $ARCH"
        echo "Supported: x86_64, aarch64"
        exit 1
        ;;
esac

# Detect GPU and select appropriate binary
detect_gpu() {
    # Check for NVIDIA GPU with CUDA support
    if command -v nvidia-smi &> /dev/null; then
        if nvidia-smi &> /dev/null; then
            echo "cuda"
            return
        fi
    fi
    
    # Check for Vulkan support (works on NVIDIA, AMD, Intel)
    if command -v vulkaninfo &> /dev/null; then
        if vulkaninfo &> /dev/null 2>&1; then
            echo "vulkan"
            return
        fi
    fi
    
    # Check for Vulkan libraries even without vulkaninfo
    if ldconfig -p 2>/dev/null | grep -q libvulkan; then
        echo "vulkan"
        return
    fi
    
    # Fall back to CPU
    echo "cpu"
}

# Allow manual override via environment variable
GPU_BACKEND="${GPU_BACKEND:-auto}"

if [ "$GPU_BACKEND" = "auto" ]; then
    GPU_BACKEND=$(detect_gpu)
    echo "Auto-detected GPU backend: $GPU_BACKEND"
else
    echo "Using specified GPU backend: $GPU_BACKEND"
fi

# Set binary name based on GPU backend
case "$GPU_BACKEND" in
    cuda)
        BINARY_NAME="whisper-tool-gui-linux-${ARCH_SUFFIX}-cuda"
        GPU_DESC="CUDA (NVIDIA)"
        ;;
    vulkan)
        BINARY_NAME="whisper-tool-gui-linux-${ARCH_SUFFIX}-vulkan"
        GPU_DESC="Vulkan (cross-vendor GPU)"
        ;;
    cpu|*)
        BINARY_NAME="whisper-tool-gui-linux-${ARCH_SUFFIX}"
        GPU_DESC="CPU only"
        ;;
esac

echo "Selected binary: $BINARY_NAME ($GPU_DESC)"

# Check for required dependencies
echo "Checking dependencies..."
MISSING_DEPS=""

if ! command -v wtype &> /dev/null; then
    MISSING_DEPS="$MISSING_DEPS wtype"
fi

# GPU-specific dependency checks
if [ "$GPU_BACKEND" = "vulkan" ]; then
    if ! ldconfig -p 2>/dev/null | grep -q libvulkan; then
        echo "Warning: Vulkan libraries not found. GPU acceleration may not work."
        echo "Install Vulkan drivers for your GPU:"
        echo "  NVIDIA: nvidia-utils (includes Vulkan)"
        echo "  AMD: vulkan-radeon or amdvlk"
        echo "  Intel: vulkan-intel"
    fi
fi

if [ "$GPU_BACKEND" = "cuda" ]; then
    if ! command -v nvidia-smi &> /dev/null; then
        echo "Warning: NVIDIA drivers not found. CUDA acceleration may not work."
        echo "Install NVIDIA drivers and CUDA toolkit."
    fi
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
echo "Downloading Whisper Tool ($GPU_DESC)..."
DOWNLOAD_URL="https://github.com/$REPO/releases/latest/download/$BINARY_NAME"

if command -v curl &> /dev/null; then
    if ! curl -fL "$DOWNLOAD_URL" -o whisper-tool-gui 2>/dev/null; then
        echo "GPU-specific binary not found, falling back to CPU version..."
        BINARY_NAME="whisper-tool-gui-linux-${ARCH_SUFFIX}"
        curl -fL "https://github.com/$REPO/releases/latest/download/$BINARY_NAME" -o whisper-tool-gui
    fi
elif command -v wget &> /dev/null; then
    if ! wget -q "$DOWNLOAD_URL" -O whisper-tool-gui 2>/dev/null; then
        echo "GPU-specific binary not found, falling back to CPU version..."
        BINARY_NAME="whisper-tool-gui-linux-${ARCH_SUFFIX}"
        wget -q "https://github.com/$REPO/releases/latest/download/$BINARY_NAME" -O whisper-tool-gui
    fi
else
    echo "Error: Neither curl nor wget found. Please install one of them."
    exit 1
fi

# Download icon
echo "Downloading icon..."
if command -v curl &> /dev/null; then
    curl -fL "https://github.com/$REPO/releases/latest/download/whisper-tool.png" -o whisper-tool.png
else
    wget -q "https://github.com/$REPO/releases/latest/download/whisper-tool.png" -O whisper-tool.png
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
# GPU backend: auto, cpu, vulkan, cuda
# - auto: automatically use best available (cuda > vulkan > cpu)
# - cpu: force CPU-only inference
# - vulkan: use Vulkan GPU acceleration (works on NVIDIA, AMD, Intel)
# - cuda: use CUDA acceleration (NVIDIA only, best performance)
gpu = "auto"

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
echo "  GPU Backend: $GPU_DESC"
echo ""
echo "You can now:"
echo "  - Launch it from your application menu"
echo "  - Run 'whisper-tool-gui' in terminal"
echo "  - Use the system tray icon"
echo ""
echo "To reinstall with a different GPU backend:"
echo "  GPU_BACKEND=cuda curl -sSL ... | bash   # For NVIDIA CUDA"
echo "  GPU_BACKEND=vulkan curl -sSL ... | bash # For Vulkan (any GPU)"
echo "  GPU_BACKEND=cpu curl -sSL ... | bash    # For CPU only"
echo ""
echo "For global shortcut (Hyprland), add to ~/.config/hypr/hyprland.conf:"
echo "  bind = CTRL SHIFT, D, exec, pkill -SIGUSR1 -f whisper-tool-gui"
echo "  exec-once = whisper-tool-gui"
echo ""
echo "Config file: ~/.config/whisper-tool/config.toml"
