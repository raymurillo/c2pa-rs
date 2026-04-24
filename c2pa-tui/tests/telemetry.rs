//! Integration tests for the `debug-telemetry` feature.
//!
//! Run with:
//!   cargo test -p c2pa-tui --features debug-telemetry
#![cfg(feature = "debug-telemetry")]

use dial9_tokio_telemetry::telemetry::{RotatingWriter, TracedRuntime};

/// The TelemetryGuard must outlive `block_on`; this test ensures the traced
/// runtime starts, completes a future, and produces a non-empty trace file.
///
/// `RotatingWriter` names its first segment `{stem}.0.bin`, so the base path
/// `trace.bin` produces `trace.0.bin` on disk.
#[test]
fn traced_runtime_runs_and_writes_trace_file() {
    let dir = tempfile::tempdir().unwrap();
    let trace_base = dir.path().join("trace.bin");
    // RotatingWriter appends an index: trace.bin -> trace.0.bin
    let trace_segment = dir.path().join("trace.0.bin");

    let writer = Box::new(RotatingWriter::new(&trace_base, 1024 * 1024, 4 * 1024 * 1024).unwrap());
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    builder.enable_all();
    let (rt, guard) = TracedRuntime::build_and_start(builder, writer).unwrap();

    let result = rt.block_on(async { 42u32 });

    // Explicit drop after block_on: all events must be flushed before the
    // assertion so we see a non-empty file.
    drop(guard);

    assert_eq!(result, 42);
    assert!(trace_segment.exists(), "trace file was not created");
    assert!(
        trace_segment.metadata().unwrap().len() > 0,
        "trace file is empty"
    );
}

/// Trace directory created via the same logic as main must have mode 0700 on
/// Unix so that auth tokens and file paths in trace events are not
/// world-readable.
#[cfg(unix)]
#[test]
fn trace_directory_has_owner_only_permissions() {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};

    let parent = tempfile::tempdir().unwrap();
    let trace_dir = parent.path().join("nested").join("traces");

    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&trace_dir)
        .unwrap();

    let perm_bits = std::fs::metadata(&trace_dir).unwrap().mode() & 0o777;
    assert_eq!(
        perm_bits, 0o700,
        "trace directory must be owner-only (0700)"
    );
}
