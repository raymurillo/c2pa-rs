use c2pa_tui::{
    app::App,
    config::{Config, Theme},
    manifest::loader::{DirSource, FileSource, RemoteSource},
    remote::Auth,
};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "c2pa-tui", version, about = "Terminal UI for C2PA manifests")]
struct Cli {
    /// Files, directories, or HTTP URLs to load on startup.
    #[arg(name = "PATHS_OR_URLS")]
    inputs: Vec<String>,

    /// Authentication spec: none | basic:user:pass | bearer:token | digest:user:pass
    #[arg(long, default_value = "none")]
    auth: String,

    /// Initial field filter glob (e.g. "assertions.*")
    #[arg(long)]
    filter: Option<String>,

    /// Disable mouse support
    #[arg(long)]
    no_mouse: bool,

    /// Color theme: dark | light | mono
    #[arg(long, default_value = "dark")]
    theme: String,

    /// Directory for dial9 telemetry trace files.
    /// Defaults to $HOME/.local/share/c2pa-tui/traces.
    /// Warning: trace files may contain file paths, remote URLs, and auth tokens;
    /// keep this directory private and do not share trace files untrusted parties.
    #[cfg(feature = "debug-telemetry")]
    #[arg(long)]
    trace_dir: Option<std::path::PathBuf>,
}

fn main() {
    // Initialise tracing before everything else so that startup messages
    // (including the telemetry announcement below) honour RUST_LOG filtering.
    // app::run() calls try_init() again as a fallback for library consumers.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .try_init();

    let cli = Cli::parse();

    let auth = Auth::from_spec(&cli.auth).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });

    let theme = match cli.theme.as_str() {
        "light" => Theme::Light,
        "mono" => Theme::Mono,
        _ => Theme::Dark,
    };

    let config = Config {
        theme,
        mouse_enabled: !cli.no_mouse,
        auth: auth.clone(),
        initial_filter: cli.filter.clone(),
        ..Config::default()
    };

    let mut app = App::new(config).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });

    // Apply initial filter if provided.
    if let Some(f) = &cli.filter {
        match c2pa_tui::manifest::filter::FieldFilter::from_query(f) {
            Ok(filter) => app.filter = filter,
            Err(e) => {
                eprintln!("error: invalid filter: {e}");
                std::process::exit(1);
            }
        }
    }

    // Populate sources from CLI inputs.
    // Directories are expanded into individual FileSource entries so each file
    // gets its own row in the file list.
    for input in &cli.inputs {
        if input.starts_with("http://") || input.starts_with("https://") {
            match url::Url::parse(input) {
                Ok(url) => {
                    app.add_source(std::sync::Arc::new(RemoteSource::new(url, auth.clone())))
                }
                Err(e) => eprintln!("warning: invalid URL {input:?}: {e}"),
            }
        } else {
            let path = std::path::PathBuf::from(input);
            if path.is_dir() {
                match DirSource::new(path.clone()).entries() {
                    Ok(entries) => {
                        for file_src in entries {
                            app.add_source(std::sync::Arc::new(file_src));
                        }
                    }
                    Err(e) => eprintln!("warning: could not read directory {path:?}: {e}"),
                }
            } else {
                app.add_source(std::sync::Arc::new(FileSource::new(path)));
            }
        }
    }

    // Build and run the Tokio runtime.  The telemetry path wraps the runtime in
    // dial9's TracedRuntime and keeps the TelemetryGuard alive for the entire
    // duration of block_on so that all events are flushed on exit.
    #[cfg(feature = "debug-telemetry")]
    let (result, _telemetry_guard) = {
        use dial9_tokio_telemetry::telemetry::{RotatingWriter, TracedRuntime};

        let trace_dir = cli.trace_dir.unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(std::env::temp_dir)
                .join(".local/share/c2pa-tui/traces")
        });

        // Create the trace directory.  Restrict permissions on Unix so trace
        // files (which may contain auth tokens and file paths) are not
        // world-readable.
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&trace_dir)
                .unwrap_or_else(|e| {
                    eprintln!("error: cannot create trace directory {trace_dir:?}: {e}");
                    std::process::exit(1);
                });
        }
        #[cfg(not(unix))]
        std::fs::create_dir_all(&trace_dir).unwrap_or_else(|e| {
            eprintln!("error: cannot create trace directory {trace_dir:?}: {e}");
            std::process::exit(1);
        });

        let trace_path = trace_dir.join("trace.bin");
        let writer = Box::new(
            RotatingWriter::new(&trace_path, 20 * 1024 * 1024, 100 * 1024 * 1024)
                .unwrap_or_else(|e| {
                    eprintln!("error: cannot open trace file {trace_path:?}: {e}");
                    std::process::exit(1);
                }),
        );

        let mut builder = tokio::runtime::Builder::new_multi_thread();
        builder.enable_all();
        let (rt, guard) = TracedRuntime::build_and_start(builder, writer).unwrap_or_else(|e| {
            eprintln!("error: cannot start traced runtime: {e}");
            std::process::exit(1);
        });
        tracing::info!("dial9 telemetry enabled, traces at: {}", trace_dir.display());
        let result = rt.block_on(app.run());
        // Return guard alongside result so it remains alive until after block_on.
        (result, guard)
    };

    #[cfg(not(feature = "debug-telemetry"))]
    let result = {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap_or_else(|e| {
                eprintln!("error: cannot start runtime: {e}");
                std::process::exit(1);
            });
        rt.block_on(app.run())
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
