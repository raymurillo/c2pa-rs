use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;

use c2pa_tui::app::{App, AppState};
use c2pa_tui::config::Config;
use c2pa_tui::ui::layout::{centered_popup, split_horizontal, split_status, CachedLayout};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_app() -> App {
    App::new(Config::default()).expect("App::new")
}

fn make_terminal() -> ratatui::Terminal<TestBackend> {
    ratatui::Terminal::new(TestBackend::new(120, 40)).expect("terminal")
}

// ---------------------------------------------------------------------------
// Layout benchmarks
// ---------------------------------------------------------------------------

fn bench_layout(c: &mut Criterion) {
    let area = Rect::new(0, 0, 120, 40);

    c.bench_function("layout/split_status", |b| b.iter(|| split_status(area)));

    c.bench_function("layout/split_horizontal_25pct", |b| {
        b.iter(|| split_horizontal(area, 25))
    });

    c.bench_function("layout/centered_popup_60x20", |b| {
        b.iter(|| centered_popup(area, 60, 20))
    });

    c.bench_function("layout/CachedLayout_compute", |b| {
        b.iter(|| CachedLayout::compute(area, 25))
    });
}

// ---------------------------------------------------------------------------
// draw() benchmarks
//
// State (terminal + app) is created in iter_batched setup so that no mutable
// local variables are captured across criterion's FnMut boundary.
// ---------------------------------------------------------------------------

fn bench_draw(c: &mut Criterion) {
    // Browse state, cold layout cache (first draw after startup).
    c.bench_function("draw/browse_state_cold_cache", |b| {
        b.iter_batched(
            || (make_terminal(), make_app()),
            |(mut terminal, mut app)| {
                terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
            },
            BatchSize::SmallInput,
        )
    });

    // Browse state, warm layout cache (steady-state — cache primed in setup).
    c.bench_function("draw/browse_state_warm_cache", |b| {
        b.iter_batched(
            || {
                let mut terminal = make_terminal();
                let mut app = make_app();
                // Prime the layout cache.
                terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
                (terminal, app)
            },
            |(mut terminal, mut app)| {
                terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
            },
            BatchSize::SmallInput,
        )
    });

    // Error overlay: popup rendering path.
    c.bench_function("draw/error_overlay", |b| {
        b.iter_batched(
            || {
                let terminal = make_terminal();
                let mut app = make_app();
                app.state = AppState::Error {
                    message: "Error: something went wrong\n\nPress any key to dismiss.".into(),
                };
                (terminal, app)
            },
            |(mut terminal, mut app)| {
                terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
            },
            BatchSize::SmallInput,
        )
    });
}

// ---------------------------------------------------------------------------
// Criterion entry-points
// ---------------------------------------------------------------------------

criterion_group!(benches, bench_layout, bench_draw);
criterion_main!(benches);
