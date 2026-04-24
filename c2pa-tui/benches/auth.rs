//! Benchmarks for the `Auth` parser and its security-hardened `Debug` impl.
//!
//! `from_spec` is called on every process start (once per `--auth`) and also
//! invoked in tests. It is not a hot path, but the parser's branch structure
//! changed in spec-10 (two-phase split + `resolve_secret`) so these benches
//! give us a baseline to detect future regressions.
//!
//! The manual `Debug` impl runs on every `tracing` event that names an `Auth`
//! value; ensuring it stays allocation-light is worthwhile.

use std::hint::black_box;
use std::io::Write;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use c2pa_tui::remote::Auth;

fn bench_from_spec(c: &mut Criterion) {
    let mut group = c.benchmark_group("Auth::from_spec");

    // Exercise every scheme plus the degenerate no-op case so regressions in
    // any single branch show up individually.
    let cases: &[(&str, &str)] = &[
        ("none", "none"),
        ("bearer", "bearer:abcdef0123456789abcdef0123456789"),
        ("basic", "basic:alice:s3cr3t_p4ssw0rd_with_some_length"),
        ("digest", "digest:bob:another_password_value_1234567"),
        ("basic_colon", "basic:alice:pa:ss:word:with:many:colons"),
    ];

    for (label, spec) in cases {
        group.throughput(Throughput::Bytes(spec.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(label), spec, |b, s| {
            b.iter(|| {
                let auth = Auth::from_spec(black_box(s)).expect("valid spec");
                black_box(auth);
            });
        });
    }
    group.finish();
}

/// RAII guard that removes an env var on drop, ensuring the benchmark's
/// process env is clean even if a group panics.
struct BenchEnvGuard(&'static str);
impl Drop for BenchEnvGuard {
    fn drop(&mut self) {
        // SAFETY: pairs with the `set_var` below; single-threaded context.
        unsafe { std::env::remove_var(self.0) };
    }
}

fn bench_from_spec_indirection(c: &mut Criterion) {
    // Env indirection: measure in isolation so the env-var lookup cost is
    // separable from the plain-token path.
    // SAFETY: benchmark-only env mutation with a unique name.
    unsafe { std::env::set_var("C2PA_TUI_BENCH_TOKEN", "bench_token_value_abcdef") };
    let _env_guard = BenchEnvGuard("C2PA_TUI_BENCH_TOKEN");

    let mut group = c.benchmark_group("Auth::from_spec/indirection");

    group.bench_function("bearer_env", |b| {
        b.iter(|| {
            let auth = Auth::from_spec(black_box("bearer:env:C2PA_TUI_BENCH_TOKEN"))
                .expect("valid env spec");
            black_box(auth);
        });
    });

    // File indirection: prepare a temp file once and reuse the path across
    // iterations so only the read+trim cost is amortised. The `tempdir` RAII
    // handle cleans up automatically on drop.
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("bench_token.txt");
    {
        let mut f = std::fs::File::create(&path).expect("create file");
        writeln!(f, "bench_file_token_value_abcdef").expect("write file");
    }
    let spec = format!("bearer:file:{}", path.display());

    group.bench_function("bearer_file", |b| {
        b.iter(|| {
            let auth = Auth::from_spec(black_box(&spec)).expect("valid file spec");
            black_box(auth);
        });
    });

    group.finish();
}

fn bench_debug_format(c: &mut Criterion) {
    // Validate that the manual `Debug` impl (which must never leak secrets)
    // stays competitive. A single `write!` per variant with no intermediate
    // allocation is the goal.
    let mut group = c.benchmark_group("Auth::Debug");

    let none = Auth::None;
    let basic = Auth::Basic {
        username: "alice".into(),
        password: "s3cr3t_password_value".into(),
    };
    let bearer = Auth::Bearer {
        token: "bearer_token_abcdef0123456789".into(),
    };
    let digest = Auth::Digest {
        username: "bob".into(),
        password: "digest_password_value".into(),
    };

    group.bench_function("none", |b| {
        b.iter(|| {
            let s = format!("{:?}", black_box(&none));
            black_box(s);
        });
    });
    group.bench_function("basic", |b| {
        b.iter(|| {
            let s = format!("{:?}", black_box(&basic));
            black_box(s);
        });
    });
    group.bench_function("bearer", |b| {
        b.iter(|| {
            let s = format!("{:?}", black_box(&bearer));
            black_box(s);
        });
    });
    group.bench_function("digest", |b| {
        b.iter(|| {
            let s = format!("{:?}", black_box(&digest));
            black_box(s);
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_from_spec,
    bench_from_spec_indirection,
    bench_debug_format
);
criterion_main!(benches);
