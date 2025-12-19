#!/bin/bash

# Install Wraith Language Support for VSCode
# This script creates a proper extension in the VSCode extensions directory

set -e

echo "Installing Wraith Language Support for VSCode..."

# Detect VSCode extensions directory
if [[ "$OSTYPE" == "darwin"* ]]; then
    # macOS
    VSCODE_EXT_DIR="$HOME/.vscode/extensions"
elif [[ "$OSTYPE" == "linux-gnu"* ]]; then
    # Linux
    VSCODE_EXT_DIR="$HOME/.vscode/extensions"
elif [[ "$OSTYPE" == "msys" || "$OSTYPE" == "win32" ]]; then
    # Windows
    VSCODE_EXT_DIR="$USERPROFILE/.vscode/extensions"
else
    echo "Unsupported OS: $OSTYPE"
    exit 1
fi

# Create extension directory
EXT_NAME="wraith-language-0.1.0"
EXT_PATH="$VSCODE_EXT_DIR/$EXT_NAME"

echo "Creating extension directory: $EXT_PATH"
mkdir -p "$EXT_PATH"

# Copy grammar files
echo "Copying grammar files..."
cp wraith.tmLanguage.json "$EXT_PATH/"
cp language-configuration.json "$EXT_PATH/"

# Create package.json
echo "Creating package.json..."
cat > "$EXT_PATH/package.json" << 'EOF'
{
  "name": "wraith-language",
  "displayName": "Wraith Language Support",
  "description": "Syntax highlighting for Wraith programming language",
  "version": "0.1.0",
  "publisher": "wraith-dev",
  "engines": {
    "vscode": "^1.60.0"
  },
  "categories": [
    "Programming Languages"
  ],
  "contributes": {
    "languages": [
      {
        "id": "wraith",
        "aliases": [
          "Wraith",
          "wraith"
        ],
        "extensions": [
          ".wr"
        ],
        "configuration": "./language-configuration.json"
      }
    ],
    "grammars": [
      {
        "language": "wraith",
        "scopeName": "source.wraith",
        "path": "./wraith.tmLanguage.json"
      }
    ]
  }
}
EOF

echo ""
echo "✅ Installation complete!"
echo ""
echo "Next steps:"
echo "1. Restart VSCode (or run 'Developer: Reload Window')"
echo "2. Open a .wr file"
echo "3. Check the language mode (bottom right) - it should say 'Wraith'"
echo ""
echo "If it doesn't work:"
echo "- Press Ctrl+Shift+P (Cmd+Shift+P on Mac)"
echo "- Type 'Change Language Mode'"
echo "- Select 'Wraith'"
