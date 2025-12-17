#!/bin/bash
set -e

# GPU backend to build with (vulkan, cuda, or cpu)
GPU_BACKEND="${GPU_BACKEND:-vulkan}"

echo "Building Whisper Tool release..."
echo "GPU Backend: $GPU_BACKEND"

# Build release binary with appropriate features
case "$GPU_BACKEND" in
    vulkan)
        echo "Building with Vulkan GPU support (cross-vendor: NVIDIA, AMD, Intel)"
        cargo build --release --features vulkan
        SUFFIX="-vulkan"
        ;;
    cuda)
        echo "Building with CUDA GPU support (NVIDIA only)"
        cargo build --release --features cuda
        SUFFIX="-cuda"
        ;;
    cpu|none|"")
        echo "Building CPU-only version"
        cargo build --release
        SUFFIX=""
        ;;
    *)
        echo "Unknown GPU backend: $GPU_BACKEND"
        echo "Valid options: vulkan, cuda, cpu"
        exit 1
        ;;
esac

# Get architecture
ARCH=$(uname -m)

# Create release directory
mkdir -p release

# Copy binary with architecture and GPU suffix
BINARY_NAME="whisper-tool-gui-linux-$ARCH$SUFFIX"
cp target/release/whisper-tool-gui "release/$BINARY_NAME"

# Create a simple icon (placeholder - replace with actual icon)
if [ -f icon.png ]; then
    cp icon.png release/whisper-tool.png
else
    echo "Warning: icon.png not found, skipping icon"
fi

echo ""
echo "✓ Release built successfully!"
echo ""
echo "Files created in release/:"
ls -lh release/
echo ""
echo "Build variants:"
echo "  GPU_BACKEND=vulkan ./build-release.sh  # Vulkan (cross-vendor GPU)"
echo "  GPU_BACKEND=cuda ./build-release.sh    # CUDA (NVIDIA only)"
echo "  GPU_BACKEND=cpu ./build-release.sh     # CPU only"
echo ""
echo "To create a GitHub release:"
echo "1. Create a new tag: git tag v0.1.0"
echo "2. Push the tag: git push origin v0.1.0"
echo "3. The GitHub Action will automatically build and create the release"
