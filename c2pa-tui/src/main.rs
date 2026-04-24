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
}

fn main() {
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

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    if let Err(e) = rt.block_on(app.run()) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
