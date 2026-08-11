# Concordance: WASM-Based Plugin System for LLM Applications

## Executive Summary

Concordance is a flexible, extensible platform that enables seamless collaboration between Large Language Models (LLMs), AI agents, and human users through a robust WebAssembly (WASM) plugin system. The architecture allows for dynamic expansion of capabilities, integration with external services, and contextual augmentation of AI interactions.

The system's core strength lies in its modular design, allowing developers to create and deploy custom plugins that extend functionality without modifying the core platform. This document outlines the system architecture, key capabilities, and the significant problems it solves for both developers and end users.

## Key Capabilities & Strengths

1. **Plugin-Based Architecture**
   - Dynamically load and execute WASM components with clear interfaces
   - Support for pipeline workflows where plugins can be chained together
   - Just-in-time (JIT) execution for on-the-fly plugin deployment

2. **AI Integration Capabilities**
   - Native support for OpenAI and compatible LLM APIs
   - Embedding generation and vector similarity search
   - Structured chat completions with response schema enforcement

3. **Developer Experience**
   - Simple HTTP API for plugin registration and execution
   - Asynchronous job management for long-running operations
   - Comprehensive error handling and status reporting

4. **Data Processing Features**
   - Document conversion from various formats (PDF, PPT, DOCX, XLSX, HTML) to Markdown
   - Vector database integration for semantic search
   - JSON path-based data transformation between plugins in a pipeline

5. **Security & Isolation**
   - Secure execution environment through WASM sandboxing
   - Path normalization to prevent directory traversal attacks
   - Resource control and cleanup for stability

## System Architecture

### Core Components

1. **Plugin Host**
   - Manages WASM module lifecycle and execution environment
   - Provides host functions for plugins to interact with system resources
   - Handles resource allocation and cleanup

2. **HTTP Server**
   - RESTful API for plugin and pipeline management
   - Endpoints for synchronous and asynchronous execution
   - Status reporting and results retrieval

3. **Pipeline Execution Engine**
   - Orchestrates execution of multiple plugins in sequence
   - Manages data flow between pipeline stages
   - Supports branching execution paths

4. **Resource Providers**
   - HTTP client for external API integration
   - Document converter for file processing
   - AI service clients (OpenAI compatible)
   - Vector database interface for semantic search

### Integration Points

1. **Plugin Interface**
   - WebAssembly Component Model for standardized interaction
   - WIT (WebAssembly Interface Type) definitions in the `injector` world
   - JSON-based input/output for flexibility

2. **AI Service Integration**
   - Configurable OpenAI-compatible clients
   - Embedding generation and similarity search
   - Structured completion requests with schema validation

3. **Storage Layer**
   - File-based plugin storage
   - User sandbox directories for isolation
   - Vector database integration for semantic data

## Problems Solved

### For Developers

1. **Simplified AI Integration**
   - No need to manage complex AI client libraries and authentication
   - Standardized interfaces for different LLM providers
   - Built-in support for embedding and semantic search

2. **Reduced Development Overhead**
   - Plugin development in any language that compiles to WASM
   - Clear contract-based interfaces through WIT definitions
   - No need to build complex orchestration systems

3. **Enhanced Composability**
   - Create complex workflows by combining simple plugins
   - Reuse components across different projects
   - Share plugins with the community

4. **Secure Deployment**
   - Run untrusted code safely through WASM sandboxing
   - Control resource access through the host interface
   - Manage authentication and API keys centrally

### For End Users

1. **Expanded AI Capabilities**
   - Access to specialized tools through plugins
   - Connect LLMs to external data sources and APIs
   - Process various document formats seamlessly

2. **Customizable Workflows**
   - Create tailored pipelines for specific use cases
   - Combine multiple AI services in a single workflow
   - Save and reuse successful patterns

3. **Better Context Management**
   - Import external data through plugins
   - Maintain persistent vector databases for knowledge
   - Transform and filter information between processing steps

4. **Reliability and Monitoring**
   - Track long-running operations asynchronously
   - Receive detailed error reports
   - Restart failed operations without losing context

## Implementation Highlights

The system leverages several key technologies:

1. **WebAssembly Component Model** - Enables language-agnostic plugin development with standardized interfaces

2. **Tokio** - Provides asynchronous runtime for handling concurrent requests and operations

3. **Axum** - Powers the HTTP server with modern routing and middleware capabilities

4. **LanceDB** (optional) - Offers vector database capabilities for semantic search

5. **JSON Schema** - Enforces structured inputs and outputs for plugins and LLM interactions

## Deployment Considerations

1. **Resource Requirements**
   - Memory: Scales based on number of concurrent plugins and vector database size
   - CPU: Affected by document conversion and embedding generation workloads
   - Storage: Needed for plugin storage and document caching

2. **Security**
   - All file operations are sandboxed to the user's `.concordance` directory
   - External API access is controlled through the host interface
   - WASM execution is isolated from the host system

3. **Performance Optimization**
   - Document conversion results are cached by content hash
   - Vector databases can be persisted for repeated queries
   - Asynchronous execution for long-running operations

## Future Directions

1. **Extended Plugin Ecosystem**
   - Community plugin marketplace
   - Versioning and dependency management
   - Plugin composition templates

2. **Enhanced AI Capabilities**
   - Support for more LLM providers and models
   - Streaming responses for real-time interaction
   - Fine-tuning integration for specialized use cases

3. **Advanced Orchestration**
   - Conditional execution paths based on results
   - Parallel processing of compatible pipeline stages
   - Auto-retry mechanisms for transient failures

4. **Developer Tools**
   - Plugin development templates and CLI tools
   - Testing frameworks for plugin validation
   - Performance profiling for optimizing complex pipelines

## Conclusion

Concordance provides a powerful, extensible platform for building AI-enabled applications through its WASM-based plugin system. By solving key integration challenges for developers and expanding capabilities for end users, it enables the creation of sophisticated AI workflows that combine the strengths of multiple tools and services.

The architecture's focus on modularity, security, and standardization makes it suitable for a wide range of applications, from simple chatbots to complex AI agent systems requiring rich context and external integrations.

# Quickstart

### MacOS

Certain plugins may require a newer version of clang. If you get an error like:
```bash
cargo:warning=error: unable to create target: 'No available targets are compatible with triple "wasm32-unknown-wasip2"'
cargo:warning=1 error generated.
```

this is due to Apple using an old, wasi incompatible version of clang. To fix this, install a newer version of clang:

Note: change .zshrc to whatever shell profile you are using
```bash
brew install llvm
echo 'export PATH="/opt/homebrew/opt/llvm/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc
```


### 1. Install prerequisites:

If you want to have a vector database embedded you need to install protobufs:
#### Install Protobufs (optional)
macOS:
```bash
brew install protobuf
```

Debian/Ubuntu:
```bash
sudo apt install -y protobuf-compiler libssl-dev
```


#### Install document converter `marker-pdf`
```bash
python3 -m pip install marker-pdf[full]
```

#### Install rust & `wasm32-wasip2` target
Install rust:
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Add wasm32-wasip2 target:
```bash
rustup target add wasm32-wasip2
```

### 2. Build the plugins:

```bash
cd plugins && cargo build --release --target wasm32-wasip2 && cd ..
```

### 3. Build the project:


With embedded vector database:
```bash
cargo build --release --features "vectordb"
```

Without embedded vector database:
```bash
cargo build --release
```

## TODO

Multi-tenancy fs support
User Auth
User Permission prompt
Easier default configurations
