#!/bin/bash

# builds the plugins and uploads them to the server

cargo build --release --target wasm32-wasip2

# Find all .wasm files in the target/release directory (without recursion)
wasm_files=$(find ./target/wasm32-wasip2/release -maxdepth 1 -name "*.wasm" -type f)

# Check if any files were found
if [ -z "$wasm_files" ]; then
    echo "No .wasm files found in ./target/wasm32-wasip2/release"
    exit 1
fi

# Get API key from environment variable or use empty string if not set
CONC_API_KEY=${CONC_API_KEY:-""}

# Upload each file to the server
for file in $wasm_files; do
    echo "\nUploading $file..."
    curl -H "X-API-Key: $CONC_API_KEY" --data-binary @"$file" http://127.0.0.1:8080/plugins/upload

    # Check if curl command was successful
    if [ $? -eq 0 ]; then
        echo "\nSuccessfully uploaded $file"
    else
        echo "\nFailed to upload $file"
        exit 1
    fi
done

echo "\nAll .wasm files uploaded successfully"
