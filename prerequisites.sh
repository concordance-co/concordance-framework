#!/bin/bash

# Function to check if a command exists
command_exists() {
    command -v "$1" &> /dev/null
}

# Function to check if a Python package is installed
python_package_installed() {
    python3 -c "import importlib.util; print(importlib.util.find_spec('$1') is not None)" | grep -q True
}

echo "Checking and installing prerequisites..."

# Check and install Protobufs for vector database
echo "Checking for protobuf installation..."
if command_exists protoc; then
    echo "Protobuf is already installed."
else
    echo "Installing protobuf..."
    if [[ "$OSTYPE" == "darwin"* ]]; then
        # macOS
        brew install protobuf
    elif command_exists apt-get; then
        # Debian/Ubuntu
        sudo apt install -y protobuf-compiler libssl-dev
    else
        echo "Could not install protobuf. Please install it manually for your OS."
    fi
fi

# Check and install marker-pdf
echo "Checking for marker-pdf installation..."
if python_package_installed marker_pdf; then
    echo "marker-pdf is already installed."
else
    echo "Installing marker-pdf..."
    python3 -m pip install "marker-pdf[full]"
fi

# Check and install Rust
echo "Checking for Rust installation..."
if command_exists rustc; then
    echo "Rust is already installed."
else
    echo "Installing Rust..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
    # Source the cargo environment
    source "$HOME/.cargo/env"
fi

# Check and add wasm32-wasip2 target
echo "Checking for wasm32-wasip2 target..."
if rustup target list | grep "wasm32-wasip2"; then
    echo "wasm32-wasip2 target is already installed."
else
    echo "Adding wasm32-wasip2 target..."
    rustup target add wasm32-wasip2
fi

echo "All prerequisites have been checked and installed if needed."
