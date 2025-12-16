#!/bin/bash
set -e

echo "Uninstalling Whisper Tool..."

sudo rm -f /usr/local/bin/whisper-tool-gui
sudo rm -f /usr/share/pixmaps/whisper-tool.png
sudo rm -f /usr/share/applications/whisper-tool.desktop

echo "✓ Whisper Tool uninstalled successfully!"
echo ""
echo "Note: Config files in ~/.config/whisper-tool/ were preserved."
echo "To remove them: rm -rf ~/.config/whisper-tool"
