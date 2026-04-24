use c2pa_tui::{
    app::App,
    config::{Config, Theme},
    manifest::loader::{FileSource, RemoteSource},
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
    #[arg(
        long,
        default_value = "none",
        long_help = "Authentication spec. Supported schemes:\n\
          none                    No authentication (default)\n\
          basic:user:pass         HTTP Basic (HTTPS only)\n\
          bearer:token            Bearer token\n\
          digest:user:pass        Digest (HTTPS only; falls back to Basic)\n\
        \n\
        Inline secrets are visible in `ps aux` and shell history.\n\
        Use indirection to avoid exposure:\n\
          bearer:env:MY_TOKEN     Read token from $MY_TOKEN\n\
          bearer:file:/path/tok   Read first line of file\n\
          basic:user:env:MY_PASS  Same for passwords"
    )]
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

    // URL sources are populated synchronously — directories are deferred to
    // `App::add_dir` inside `block_on` below so walkdir runs on the tokio
    // blocking pool rather than stalling the async runtime.
    //
    // Partition the CLI inputs once so the tokio block body stays small.
    let mut url_inputs: Vec<url::Url> = Vec::new();
    let mut dir_inputs: Vec<std::path::PathBuf> = Vec::new();
    let mut file_inputs: Vec<std::path::PathBuf> = Vec::new();
    for input in &cli.inputs {
        if input.starts_with("http://") || input.starts_with("https://") {
            match url::Url::parse(input) {
                Ok(url) => url_inputs.push(url),
                Err(e) => eprintln!("warning: invalid URL {input:?}: {e}"),
            }
        } else {
            let path = std::path::PathBuf::from(input);
            if path.is_dir() {
                dir_inputs.push(path);
            } else {
                file_inputs.push(path);
            }
        }
    }

    for url in url_inputs {
        app.add_source(std::sync::Arc::new(RemoteSource::new(url, auth.clone())));
    }
    for path in file_inputs {
        app.add_source(std::sync::Arc::new(FileSource::new(path)));
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
            RotatingWriter::new(&trace_path, 20 * 1024 * 1024, 100 * 1024 * 1024).unwrap_or_else(
                |e| {
                    eprintln!("error: cannot open trace file {trace_path:?}: {e}");
                    std::process::exit(1);
                },
            ),
        );

        let mut builder = tokio::runtime::Builder::new_multi_thread();
        builder.enable_all();
        let (rt, guard) = TracedRuntime::build_and_start(builder, writer).unwrap_or_else(|e| {
            eprintln!("error: cannot start traced runtime: {e}");
            std::process::exit(1);
        });
        tracing::info!(
            "dial9 telemetry enabled, traces at: {}",
            trace_dir.display()
        );
        let result = rt.block_on(async move {
            for path in dir_inputs {
                if let Err(e) = app.add_dir(path.clone()).await {
                    eprintln!("warning: could not read directory {path:?}: {e}");
                }
            }
            app.run().await
        });
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
        rt.block_on(async move {
            for path in dir_inputs {
                if let Err(e) = app.add_dir(path.clone()).await {
                    eprintln!("warning: could not read directory {path:?}: {e}");
                }
            }
            app.run().await
        })
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
