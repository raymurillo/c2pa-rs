# c2pa-tui — Spec Index

Each spec is a self-contained brief for one Claude session. Sessions in the same
phase can run concurrently; start a phase only after all specs in the prior phase
are merged and `cargo build` is clean.

## Dependency phases

```
Phase 0 ──► Phase 1 ──────────────────────► Phase 2 ──────────────────► Phase 3 ──► Phase 4 (strictly sequential)
             (all concurrent)                (all concurrent)

spec-00      spec-01  Manifest data layer    spec-06  Panes & status      spec-09      spec-10  Security hardening
Foundation   spec-02  Remote HTTP layer      spec-07  Search & filter     Integration  spec-11  Architecture corrections
             spec-03  Search engine          spec-08  Compare view        CLI polish,       │   (absorbs A4; adds
             spec-04  Compare engine                                       full tests        │    SourceId, add_dir,
             spec-05  TUI skeleton                                                           │    with_loaded_for_tests)
                                                                                        spec-12  Test coverage gaps
                                                                                        spec-13  Polish & idioms
                                                                                             │
                                                                                             ▼
                                                                              Phase 5 (parallel after spec-13)
                                                                              spec-14  SSRF & credential safety
                                                                              spec-16  Hot-path alloc reductions
                                                                              spec-18  LoadState invariants
                                                                                             │
                                                                                             ▼
                                                                              Phase 6 (parallel after Phase 5)
                                                                              spec-15  Async-safe Matcher
                                                                              spec-17  Data model cleanup
                                                                                             │
                                                                                             ▼
                                                                              Phase 7 (sequential after Phase 6)
                                                                              spec-19  Observability core
                                                                              spec-20  Error reporter & log overlay
                                                                                             │
                                                                                             ▼
                                                                              Phase 8 (parallel after spec-20)
                                                                              spec-21  Release profile & panic safety
                                                                              spec-22  Diagnostic bundle
                                                                                             │
                                                                                             ▼
                                                                              Phase 9 (sequential after Phase 8)
                                                                              spec-23  Release CI pipeline
```

**Phase 4 must run strictly in order: 10 → 11 → 12 → 13.** The specs have
these cross-spec dependencies:

| Spec | Requires from earlier spec |
|------|---------------------------|
| spec-11 | spec-10's `Auth::apply` signature (must update `fetch` atomically) |
| spec-12 | spec-11's `entries_async()`, `App::with_loaded_for_tests`, and `SourceId` types |
| spec-13 | spec-12's migration of snapshot tests to `with_loaded_for_tests` (D2 would otherwise break external tests) |

**Phase 5 can run concurrently** (no file overlap between spec-14, spec-16, spec-18).  
**Phase 6:** spec-15 requires spec-18 (both modify `app.rs`); spec-17 requires spec-16 (both modify `manifest/filter.rs`).

**Phase 7–9 (release readiness):**
- **Phase 7** is sequential: spec-19 installs the `obs/` subscriber; spec-20 builds the `error::Reporter` and F2 log overlay on top of it. Depends on spec-17 (decoupled `AppState::Error` shape) and spec-18 (`LoadState::Failed`).
- **Phase 8** is parallel: spec-21 (release profile, panic safety, size gate) and spec-22 (`--diag-bundle` CLI + auto-offer) touch disjoint files.
- **Phase 9** is spec-23 alone — assembles the full release workflow, `cargo deny`, `cargo auditable`, SBOM, and signed checksums.

## Spec summary

| Spec | Phase | What it implements | Files changed |
|------|-------|--------------------|---------------|
| [spec-00](spec-00-foundation.md) | 0 | Workspace Cargo.toml, error types, all type/trait stubs | All `src/` files (stubs) |
| [spec-01](spec-01-manifest-layer.md) | 1 | FileSource, DirSource, ManifestStore→DisplayNode, FieldFilter | `manifest/loader.rs`, `manifest/tree.rs`, `manifest/filter.rs` |
| [spec-02](spec-02-remote-layer.md) | 1 | Auth, RemoteClient, RemoteSource::load, wiremock tests | `remote/auth.rs`, `remote/client.rs`, `manifest/loader.rs` (RemoteSource) |
| [spec-03](spec-03-search-engine.md) | 1 | Fuzzy/substring search via nucleo, MatchResult with highlight ranges | `search/matcher.rs` |
| [spec-04](spec-04-compare-engine.md) | 1 | Field-level diff between two DisplayNode trees | `compare/diff.rs` |
| [spec-05](spec-05-tui-skeleton.md) | 1 | Ratatui event loop, App state machine, crossterm lifecycle, layout helpers | `app.rs`, `ui/layout.rs`, `ui/mod.rs` |
| [spec-06](spec-06-panes.md) | 2 | File list pane, detail tree pane, status bar, mouse handling | `ui/file_list.rs`, `ui/detail.rs`, `ui/status_bar.rs`, `app.rs` |
| [spec-07](spec-07-overlays.md) | 2 | Search bar overlay + filter bar overlay + highlight rendering | `ui/search_bar.rs`, `ui/filter_bar.rs`, `app.rs` |
| [spec-08](spec-08-compare-ui.md) | 2 | Side-by-side compare table, diff cache, colour coding | `ui/compare.rs`, `app.rs` |
| [spec-09](spec-09-integration.md) | 3 | Full clap CLI, `--theme` colour switching, help overlay, all integration + snapshot tests | `main.rs`, `config.rs`, `tests/` |
| [spec-10](spec-10-security-hardening.md) | 4 | Redacted `Auth` Debug; secure `Default`; env/file credential indirection; `is_timeout()` retry | `remote/auth.rs`, `remote/client.rs` |
| [spec-11](spec-11-architecture-corrections.md) | 4 | `SourceId` stable key; `App::add_dir`; `entries_async`; `Auth::apply → Result` (A4); `with_loaded_for_tests` | `app.rs`, `manifest/loader.rs`, `main.rs`, `remote/auth.rs`, `remote/client.rs` |
| [spec-12](spec-12-test-coverage.md) | 4 | State-machine tests; async DirSource tests; HTTP status mapping tests; snapshot suite | `app.rs`, `remote/client.rs`, `manifest/loader.rs`, `tests/snapshot_ui.rs` |
| [spec-13](spec-13-polish.md) | 4 | `FromStr` for `Auth`; `pub(crate)` field visibility with accessor audit; `Default` rustdoc | `remote/auth.rs`, `main.rs`, `app.rs`, `remote/client.rs`, `tests/snapshot_ui.rs` |
| [spec-14](spec-14-ssrf-credential-safety.md) | 5 | Redirect policy blocking SSRF; sanitized auth error messages | `remote/client.rs`, `remote/auth.rs` |
| [spec-15](spec-15-async-safe-matcher.md) | 6 | Non-blocking nucleo via `spawn_blocking`; `Arc<str>` display sharing; remove query clone | `search/matcher.rs`, `app.rs` |
| [spec-16](spec-16-hot-path-alloc.md) | 5 | `Cow<'_, str>` for `NodeValue::as_str`; path-buffer in filter; depth guard on `flatten_inner` | `manifest/tree.rs`, `manifest/filter.rs` |
| [spec-17](spec-17-data-model-cleanup.md) | 6 | Unify `apply`/`apply_ref`; `FieldDiff` struct refactor; `AppError::InvalidInput`; decouple error UI text | `manifest/filter.rs`, `compare/diff.rs`, `error.rs`, `app.rs` |
| [spec-18](spec-18-load-state-invariants.md) | 5 | `LoadState::Failed`; `loading_count()` computed; eager `ext_to_mime` check in `RemoteSource` | `app.rs`, `manifest/loader.rs`, `ui/file_list.rs` |
| [spec-19](spec-19-observability-core.md) | 7 | Single `obs::init`; layered subscriber (file · ring · stderr); credential redaction layer | `obs/` (new), `main.rs`, `app.rs`, `Cargo.toml` |
| [spec-20](spec-20-error-reporter.md) | 7 | `error::Reporter` with correlation IDs; F2 log overlay; `AppState::Error` carries `CorrelationId` | `error.rs`, `app.rs`, `ui/mod.rs`, `ui/log_overlay.rs` (new) |
| [spec-21](spec-21-release-profile.md) | 8 | Crate-local `[profile.release]`; `release-debug` sidecar; audited panic hook; size gate | `c2pa-tui/Cargo.toml`, `app.rs`, `.github/workflows/release.yml` |
| [spec-22](spec-22-diagnostic-bundle.md) | 8 | `--diag-bundle <path>` CLI; auto-offer on `Internal` errors; redacted tar.gz writer | `diag/` (new), `main.rs`, `app.rs`, `ui/diag_bundle_prompt.rs` (new) |
| [spec-23](spec-23-release-ci.md) | 9 | 5-target release workflow; `cargo deny`; `cargo auditable`; SBOM; signed checksums | `.github/workflows/release.yml`, `deny.toml`, `docs/RELEASE.md` |

## Quality requirements (apply to every spec)

From `CLAUDE.md` — every session must meet these before marking done:

- **No `unwrap()` in production code** — use `?`, `.context(...)`, or `map_err`.
- **`cargo fmt -- --check`** passes.
- **`cargo clippy -- -D warnings`** passes (zero warnings).
- **`///` rustdoc on every `pub` item** — at least a one-line description.
- **`#[tracing::instrument]`** on every `async fn` in the public API surface.
- **Iterators** over manual `for` loops that build `Vec`s.
- **Tests written before implementation** (TDD).
- **`mockall::MockManifestSource`** used instead of hand-rolled fakes.
- **`proptest!`** blocks in all data-transformation modules (01, 02, 03, 04).

## How to hand a spec to a Claude session

Open a new Claude Code session in the `c2pa-tui/` worktree and paste:

> Read specs/spec-XX-name.md and implement everything it describes.
> The foundation (spec-00) is already in place and `cargo build` passes.
> Do not modify files outside the scope listed in the spec.
> Follow all quality requirements in specs/README.md.

Each spec ends with **Done criteria** — a set of `cargo` commands that must pass
before the session is considered complete.
