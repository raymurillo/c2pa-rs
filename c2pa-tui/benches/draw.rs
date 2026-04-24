use std::hint::black_box;
use std::path::PathBuf;
use std::sync::Arc;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;

use c2pa_tui::app::{App, AppState, LoadState};
use c2pa_tui::config::Config;
use c2pa_tui::manifest::filter::FieldFilter;
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

    // Detail pane with hide_empty active — measures filter_empty_nodes + tree render.
    // This filter runs every frame when enabled, so its cost must be tracked.
    c.bench_function("draw/detail_hide_empty", |b| {
        b.iter_batched(
            || {
                let mut app = make_populated_app(1, 1);
                app.hide_empty = true;
                (make_terminal(), app)
            },
            |(mut terminal, mut app)| {
                terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
            },
            BatchSize::SmallInput,
        )
    });
}

// ---------------------------------------------------------------------------
// Spec-07 hot-path benchmarks
// ---------------------------------------------------------------------------

fn bench_search(c: &mut Criterion) {
    // --- draw/search_overlay_active ---
    // Measures a full draw() call when the search overlay is open:
    // flatten (detail path) + HashSet lookup + search_bar render.
    c.bench_function("draw/search_overlay_active", |b| {
        b.iter_batched(
            || {
                let mut app = make_populated_app(1, 1);
                app.state = AppState::Searching {
                    query: "jpeg".into(),
                };
                app.reindex_for_selected();
                app.reindex_and_search();
                (make_terminal(), app)
            },
            |(mut terminal, mut app)| {
                terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
            },
            BatchSize::SmallInput,
        )
    });

    // --- draw/filter_overlay_active ---
    // Measures a full draw() call when the filter bar is open:
    // from_query (once) + apply_ref + filter_bar render.
    c.bench_function("draw/filter_overlay_active", |b| {
        b.iter_batched(
            || {
                let mut app = make_populated_app(1, 1);
                app.state = AppState::Filtering {
                    query: "assertions.*".into(),
                };
                (make_terminal(), app)
            },
            |(mut terminal, mut app)| {
                terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
            },
            BatchSize::SmallInput,
        )
    });

    // --- search/reindex_for_selected ---
    // Cost of the flatten + nucleo re-index triggered by a manifest load or
    // file-list navigation.  This is the formerly-per-keystroke cost.
    c.bench_function("search/reindex_for_selected", |b| {
        b.iter_batched(
            || make_populated_app(1, 1),
            |mut app| {
                app.reindex_for_selected();
            },
            BatchSize::SmallInput,
        )
    });

    // --- search/keystroke ---
    // Cost of a single character keystroke while the search overlay is open:
    // matcher.query() + cursor update + HashSet rebuild.
    // The index is already warm; this should be significantly cheaper than
    // the old reindex_and_search which called index() every time.
    c.bench_function("search/keystroke", |b| {
        b.iter_batched(
            || {
                let mut app = make_populated_app(1, 1);
                app.reindex_for_selected();
                app.state = AppState::Searching {
                    query: "jpe".into(),
                };
                app.reindex_and_search();
                app
            },
            |mut app| {
                if let AppState::Searching { query } = &mut app.state {
                    query.push('g');
                }
                app.reindex_and_search();
            },
            BatchSize::SmallInput,
        )
    });

    // --- filter/apply_ref vs apply_clone ---
    // Directly compare apply_ref (borrow) against apply(nodes.clone()) to
    // quantify the allocation savings in the filter bar's preview path.
    let mut group = c.benchmark_group("filter");
    let nodes = make_manifest_nodes();
    let filter = FieldFilter::from_query("assertions.*").unwrap();

    group.bench_function("apply_clone", |b| {
        b.iter(|| black_box(filter.apply(black_box(nodes.clone()))))
    });
    group.bench_function("apply_ref", |b| {
        b.iter(|| black_box(filter.apply_ref(black_box(&nodes))))
    });
    group.finish();
}

// ---------------------------------------------------------------------------
// Spec-09 compare-view benchmarks
// ---------------------------------------------------------------------------
//
// Three scenarios isolate different parts of the compare rendering path:
//
//   compare_warm  — cache is hot; measures pure Table widget render (~60 fps steady state)
//   compare_cold  — cache busted every iteration; measures flatten+diff+render
//   compare_show_all — like warm, but equal rows are visible (larger Table)

fn make_compare_nodes(n: usize, mutate_even: bool) -> Vec<DisplayNode> {
    let children: Vec<DisplayNode> = (0..n)
        .map(|i| DisplayNode {
            key: format!("field_{i}"),
            value: NodeValue::Str(if mutate_even && i % 2 == 0 {
                format!("changed_{i}")
            } else {
                format!("value_{i}")
            }),
            children: vec![],
        })
        .collect();
    vec![DisplayNode {
        key: "Claim".into(),
        value: NodeValue::Missing,
        children,
    }]
}

fn make_compare_app_n(n: usize) -> App {
    let mut app = make_populated_app(2, 0);
    app.loaded
        .insert(0, LoadState::Loaded(make_compare_nodes(n, false)));
    app.loaded
        .insert(1, LoadState::Loaded(make_compare_nodes(n, true)));
    app.compare_selection = Some(1);
    app.state = AppState::Comparing;
    app
}

fn bench_draw_compare(c: &mut Criterion) {
    let sizes = [10usize, 50, 200];

    // Warm cache — steady-state per-frame cost (pure rendering).
    let mut group = c.benchmark_group("draw/compare_warm");
    for n in sizes {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_batched(
                || {
                    let mut terminal = make_terminal();
                    let mut app = make_compare_app_n(n);
                    // Prime the cache.
                    terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
                    (terminal, app)
                },
                |(mut terminal, mut app)| {
                    terminal
                        .draw(|f| c2pa_tui::ui::draw(f, black_box(&mut app)))
                        .unwrap();
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();

    // Cold cache — cost of entering compare mode (flatten + diff allocation + render).
    let mut group = c.benchmark_group("draw/compare_cold");
    for n in sizes {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_batched(
                || {
                    let mut app = make_compare_app_n(n);
                    // Pre-warm and then bust so the first iteration measures cold.
                    let mut terminal = make_terminal();
                    terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
                    app.compare_diff_cache = None;
                    (terminal, app)
                },
                |(mut terminal, mut app)| {
                    // Reset the cache each iteration to always hit the cold path.
                    app.compare_diff_cache = None;
                    terminal
                        .draw(|f| c2pa_tui::ui::draw(f, black_box(&mut app)))
                        .unwrap();
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();

    // Warm cache, show_all=true — all rows (equal + diff) rendered; larger Table.
    let mut group = c.benchmark_group("draw/compare_show_all_warm");
    for n in sizes {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_batched(
                || {
                    let mut terminal = make_terminal();
                    let mut app = make_compare_app_n(n);
                    app.show_all_diffs = true;
                    terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
                    (terminal, app)
                },
                |(mut terminal, mut app)| {
                    terminal
                        .draw(|f| c2pa_tui::ui::draw(f, black_box(&mut app)))
                        .unwrap();
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// Criterion entry-points
// ---------------------------------------------------------------------------

criterion_group!(
    benches,
    bench_layout,
    bench_draw,
    bench_search,
    bench_draw_compare
);
criterion_main!(benches);
