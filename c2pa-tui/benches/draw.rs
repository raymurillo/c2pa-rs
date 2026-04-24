use std::hint::black_box;
use std::path::PathBuf;
use std::sync::Arc;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;

use c2pa_tui::app::{App, AppState, LoadState};
use c2pa_tui::config::Config;
use c2pa_tui::manifest::loader::FileSource;
use c2pa_tui::manifest::tree::{DisplayNode, NodeValue};
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

fn leaf(key: &str, value: &str) -> DisplayNode {
    DisplayNode {
        key: key.to_owned(),
        value: NodeValue::Str(value.to_owned()),
        children: vec![],
    }
}

fn branch(key: &str, children: Vec<DisplayNode>) -> DisplayNode {
    DisplayNode {
        key: key.to_owned(),
        value: NodeValue::Missing,
        children,
    }
}

/// A realistic C2PA manifest tree similar to what `store_to_nodes` produces.
fn make_manifest_nodes() -> Vec<DisplayNode> {
    vec![branch(
        "Manifest: urn:uuid:12345678-0000-0000-0000-000000000000 (active)",
        vec![
            branch(
                "Claim",
                vec![
                    leaf("title", "sample.jpg"),
                    leaf("format", "image/jpeg"),
                    leaf(
                        "instance_id",
                        "xmp:iid:12345678-0000-0000-0000-000000000000",
                    ),
                    leaf("claim_generator", "TestApp/1.0 c2pa-rs/0.80.0"),
                ],
            ),
            branch(
                "Claim Signature",
                vec![
                    leaf("issuer", "Test Certificate Authority"),
                    leaf("time", "2024-06-01T12:00:00Z"),
                    leaf("alg", "Ps256"),
                ],
            ),
            branch(
                "Assertions (3)",
                vec![
                    branch(
                        "c2pa.actions",
                        vec![branch(
                            "[0]",
                            vec![
                                leaf("action", "c2pa.created"),
                                leaf("softwareAgent", "TestApp 1.0"),
                                leaf("when", "2024-06-01T11:59:00Z"),
                            ],
                        )],
                    ),
                    branch(
                        "c2pa.hash.data",
                        vec![
                            leaf("alg", "sha256"),
                            leaf("hash", "YWJjMTIz"),
                            leaf("name", "jumbf manifest"),
                        ],
                    ),
                    leaf(
                        "stds.schema-org.CreativeWork",
                        r#"{"@type":"CreativeWork","author":[{"@type":"Person","name":"Alice"}]}"#,
                    ),
                ],
            ),
            branch(
                "Ingredients (1)",
                vec![branch(
                    "source.jpg",
                    vec![
                        leaf("format", "image/jpeg"),
                        leaf("instance_id", "xmp:iid:source-0000-0000-0000-000000000000"),
                        leaf("relationship", "parentOf"),
                    ],
                )],
            ),
            branch("Validation", vec![leaf("status", "valid")]),
        ],
    )]
}

fn make_populated_app(source_count: usize, loaded_count: usize) -> App {
    let mut app = make_app();
    for i in 0..source_count {
        app.add_source(Arc::new(FileSource::new(PathBuf::from(format!(
            "file_{i}.jpg"
        )))));
        if i < loaded_count {
            app.loaded
                .insert(i, LoadState::Loaded(make_manifest_nodes()));
        }
    }
    // Point detail pane at a loaded entry so the tree widget actually renders.
    app.selected_left = 0;
    app
}

// ---------------------------------------------------------------------------
// Layout benchmarks
// ---------------------------------------------------------------------------

fn bench_layout(c: &mut Criterion) {
    let area = Rect::new(0, 0, 120, 40);

    c.bench_function("layout/split_status", |b| {
        b.iter(|| black_box(split_status(black_box(area))))
    });

    c.bench_function("layout/split_horizontal_25pct", |b| {
        b.iter(|| black_box(split_horizontal(black_box(area), 25)))
    });

    c.bench_function("layout/centered_popup_60x20", |b| {
        b.iter(|| black_box(centered_popup(black_box(area), 60, 20)))
    });

    c.bench_function("layout/CachedLayout_compute", |b| {
        b.iter(|| black_box(CachedLayout::compute(black_box(area), 25)))
    });
}

// ---------------------------------------------------------------------------
// draw() benchmarks
//
// State (terminal + app) is created in iter_batched setup so that no mutable
// local variables are captured across criterion's FnMut boundary.
// ---------------------------------------------------------------------------

fn bench_draw(c: &mut Criterion) {
    // --- Empty app ---

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

    // --- Populated app: these are the hot paths introduced in spec-06 ---

    // File list with 10 sources, 5 loaded — measures list widget build + render.
    c.bench_function("draw/file_list_10_sources_5_loaded", |b| {
        b.iter_batched(
            || (make_terminal(), make_populated_app(10, 5)),
            |(mut terminal, mut app)| {
                terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
            },
            BatchSize::SmallInput,
        )
    });

    // Detail pane with a loaded manifest — measures node_to_tree_item + Tree render.
    c.bench_function("draw/detail_pane_loaded_manifest", |b| {
        b.iter_batched(
            || (make_terminal(), make_populated_app(1, 1)),
            |(mut terminal, mut app)| {
                terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
            },
            BatchSize::SmallInput,
        )
    });

    // Detail pane, warm cache — isolates the tree widget render from layout.
    c.bench_function("draw/detail_pane_warm_cache", |b| {
        b.iter_batched(
            || {
                let mut terminal = make_terminal();
                let mut app = make_populated_app(1, 1);
                terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
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
