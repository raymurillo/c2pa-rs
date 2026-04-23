fn main() {
    let config = c2pa_tui::config::Config::default();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let app = c2pa_tui::app::App::new(config).expect("app init");
    if let Err(e) = rt.block_on(app.run()) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
