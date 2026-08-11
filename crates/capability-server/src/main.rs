use anyhow::Result;
use clap::Parser;
use plugin_host::server::start_server;

// Initialize the plugin system
#[derive(Parser)]
#[command(name = "wasm-plugin-server")]
#[command(about = "WASM Plugin Server that loads and runs WASM plugins")]
struct CliArgs {
    /// Plugin files or directories to load (.wasm files)
    #[arg(value_name = "PLUGINS")]
    plugins: Vec<String>,

    /// Enable authentication for the plugin server
    #[arg(long, default_value_t = false)]
    enable_auth: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let args = CliArgs::parse();
    let mut init_plugins = Vec::new();
    let enable_auth = args.enable_auth;

    // Process each plugin path
    for path in &args.plugins {
        // Check if the path exists
        if !std::path::Path::new(path).exists() && !path.contains('*') {
            return Err(anyhow::anyhow!("Path does not exist: {}", path));
        }

        if path.contains('*') || std::path::Path::new(path).is_dir() {
            // This is a glob pattern or directory, find all .wasm files
            let glob_pattern = if std::path::Path::new(path).is_dir() {
                format!("{}/*.wasm", path)
            } else {
                path.to_string()
            };

            match glob::glob(&glob_pattern) {
                Ok(paths) => {
                    paths.into_iter().for_each(|entry| {
                        if let Ok(path) = entry {
                            init_plugins.push(path.to_string_lossy().to_string());
                        }
                    });
                }
                Err(e) => println!("Failed to read glob pattern {}: {}", glob_pattern, e),
            }
        } else {
            // This is a direct file path
            init_plugins.push(path.to_string());
        }
    }

    if init_plugins.is_empty() {
        println!("No plugins found. Try specifying paths to .wasm files or directories.");
    } else {
        println!("Found {} plugins", init_plugins.len());
        for plugin in &init_plugins {
            println!("  {}", plugin);
        }
    }

    let jwt_secret = if enable_auth {
        println!("Authentication is enabled");
        Some(
            std::env::var("JWT_SECRET")
                .expect("JWT_SECRET environment variable must be set when auth is enabled"),
        )
    } else {
        None
    };

    // Start the server
    println!("Starting WASM Plugin Server on 127.0.0.1:8080");
    start_server(([127, 0, 0, 1], 8080), init_plugins, jwt_secret).await?;

    Ok(())
}
