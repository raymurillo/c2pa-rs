# c2pa-tui — Spec Index

Each spec is a self-contained brief for one Claude session. Sessions in the same
phase can run concurrently; start a phase only after all specs in the prior phase
are merged and `cargo build` is clean.

## Dependency phases

```
Phase 0 ──► Phase 1 ──────────────────────► Phase 2 ──────────────────► Phase 3
             (all concurrent)                (all concurrent)

spec-00      spec-01  Manifest data layer    spec-06  Panes & status      spec-09  Integration,
Foundation   spec-02  Remote HTTP layer      spec-07  Search & filter              CLI polish,
             spec-03  Search engine          spec-08  Compare view                 full tests
             spec-04  Compare engine
             spec-05  TUI skeleton
```

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
