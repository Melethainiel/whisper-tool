#!/bin/bash
set -e

echo "Building Whisper Tool release..."

# Build release binary
cargo build --release

# Get architecture
ARCH=$(uname -m)

# Create release directory
mkdir -p release

# Copy binary with architecture suffix
cp target/release/whisper-tool-gui "release/whisper-tool-gui-linux-$ARCH"

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
echo "To create a GitHub release:"
echo "1. Create a new tag: git tag v0.1.0"
echo "2. Push the tag: git push origin v0.1.0"
echo "3. The GitHub Action will automatically build and create the release"
