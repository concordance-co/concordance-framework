## Concordance Plugin System Overview

The Concordance project implements a WebAssembly (WASM) plugin architecture that allows for extending functionality through modular components. This architecture provides several key benefits:

1. **Isolation**: Plugins run in a sandboxed WASM environment
2. **Language-agnostic**: Though currently using Rust, the architecture supports any language that compiles to WASM
3. **Dynamic loading**: Plugins can be uploaded and executed at runtime
4. **Host-provided capabilities**: A set of common functions are provided by the host environment

### Plugin Architecture

The architecture consists of:

1. **Plugin Interface**: Defined using WebAssembly Interface Types (WIT) which specify the contract between plugins and the host
2. **Plugin Host**: A server that loads and executes plugins
3. **Individual Plugins**: Implementations of specific functionality

### Main Plugin Categories

The codebase organizes plugins into several functional categories:

1. **Data Processing**
   - `md-converter`: Converts various document formats to Markdown
   - `md-chunker`: Chunks Markdown documents into smaller pieces
   - `json-manipulator`: Performs transformations on JSON data
   - `sandbox-fs`: Provides file system access within the sandbox

2. **Knowledge Graph**
   - `entity-extraction`: Extracts entities from text using LLMs
   - `neo4j-kg`: Interacts with Neo4j knowledge graph databases
   - `kg-ctx-retrieval`: Retrieves context from knowledge graphs

3. **Vector Database**
   - `doc-vector-store`: Manages document vector storage
   - `vec-ctx-retrieval`: Retrieves context based on vector similarity

4. **External Connections**
   - `github`: Integrates with GitHub API

5. **Applications**
   - `ea-test`: Implements a chat interface using OpenAI

### Plugin Development Process

The process for creating a new plugin includes:

1. Create a new directory in the appropriate category
2. Use `wit_bindgen` to generate Rust bindings from the WIT interface
3. Implement the `Guest` trait for metadata and `GuestJsonToJson` for functionality
4. Build using the WASM target and upload to the plugin host

Here's a simplified example of a plugin implementation:

```rust
// Generate bindings from WIT definitions
wit_bindgen::generate!({
    world: "injector",
    path: "../../../../wit",
});

// Implement the Guest trait for metadata
impl Guest for MyPlugin {
    type JsonToJson = MyImplementation;

    fn get_metadata() -> Metadata {
        Metadata {
            name: "My Plugin".to_string(),
            version: "0.1.0".to_string(),
            author: "Me".to_string(),
            description: "Does something useful".to_string(),
            kind: PluginKind::Tool,
            input_schema: serde_json::to_string(&inlined_schema_for!(MyRequest)).unwrap(),
            default_input: serde_json::to_string(&MyRequest::default()).unwrap(),
            output_schema: serde_json::to_string(&inlined_schema_for!(MyResponse)).unwrap(),
        }
    }
}

// Implement the actual functionality
struct MyImplementation;

impl GuestJsonToJson for MyImplementation {
    fn work(&self, input: String) -> Result<String, PluginError> {
        // Parse input JSON
        let request = serde_json::from_str::<MyRequest>(&input)?;

        // Do something with it
        let result = process_request(request)?;

        // Return JSON response
        serde_json::to_string(&result).map_err(|e| PluginError::Json(e.to_string()))
    }

    fn new() -> Self {
        Self {}
    }
}

// Export the plugin
export!(MyPlugin);
```

### Host-Provided Capabilities

The host environment provides several capabilities to plugins:

1. **HTTP Client**: Make web requests through `get`, `post`, etc.
2. **Logging**: Log messages at different severity levels
3. **Database Connections**: Connect to vector databases
4. **LLM Clients**: Create clients for embedding and LLM services
5. **Plugin Invocation**: Call other plugins with `call_plugin`

These capabilities are imported in plugins through the WIT interface:

```rust
use plugin::injector::host::{log, new_client, call_plugin};
use plugin::injector::logger::Level;
```

### Building and Deploying Plugins

The project includes a `update_server.sh` script that:

1. Builds all plugins for WASM target
2. Uploads the resulting WASM files to the plugin host

This enables a smooth development workflow with quick iteration.

## Conclusion

The Concordance plugin system provides a flexible, extensible architecture for building modular functionality. The use of WASM enables secure isolation while maintaining performance, and the WIT interface provides a clear contract between plugins and the host environment.
