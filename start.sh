#!/bin/bash
# Model Speed - 启动脚本

cd "$(dirname "$0")"

echo "Checking dependencies..."

# Check Rust
if ! command -v cargo &> /dev/null; then
    echo "Installing Rust..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
fi

# Check Node (for dev mode)
if ! command -v node &> /dev/null; then
    echo "Node.js not found"
    exit 1
fi

# Run Tauri
echo "Starting Model Speed..."
cargo tauri dev
