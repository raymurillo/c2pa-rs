# Architecture Improvements Plan

**Source:** Principal architect review of c2pa-tui data and state layers  
**Review date:** 2026-04-24  
**Specs produced:** spec-14 through spec-18  
**Depends on:** spec-13 merged and `cargo build` clean

---

## Overview

A principal architect review of `c2pa-tui`'s data layer (`manifest/`, `search/`,
`compare/`) and state layer (`app.rs`) identified 19 findings across four
categories: security, performance, idiomatic Rust, and code quality.

Three findings (credential-debug redaction, `App` field visibility,
`entries_async`) overlap with existing specs-10/11/13 and are already addressed.
The remaining 16 findings are divided into five independent specs that can run
in two parallel phases after spec-13 lands.

---

## Findings cross-reference

| # | Finding | Severity | Spec |
|---|---------|----------|------|
| 1 | `Auth` Debug leaks credentials | Critical | ✅ spec-10 A1 |
| 2 | `from_spec` error echoes full auth spec string | Critical | spec-14 |
| 3 | SSRF via unchecked redirect in `RemoteClient` | Critical | spec-14 |
| 4 | Blocking nucleo spin-loops stall async event loop | High | spec-15 |
| 5 | `NodeValue::as_str()` allocates on every render frame | High | spec-16 |
| 6 | Per-node path `String` alloc in filter traversal | High | spec-16 |
| 7 | `Matcher` double-stores display strings | High | spec-15 |
| 8 | `DirSource::load` is sequential | High | ✅ spec-11 entries_async |
| 9 | `apply` / `apply_ref` are near-identical | Medium | spec-17 |
| 10 | `LoadState` missing `Failed` variant | Medium | spec-18 |
| 11 | `loading_count` can drift from actual state | Medium | spec-18 |
| 12 | `FieldDiff` repeats `path: String` in every variant | Medium | spec-17 |
| 13 | `AppError::Glob` misused for input validation | Medium | spec-17 |
| 14 | `reindex_and_search` clones query on every keystroke | Medium | spec-15 |
| 15 | `flatten_inner` has no recursion depth guard | Medium | spec-16 |
| 16 | All `App` fields are public | Medium | ✅ spec-13 D2 |
| 17 | `ext_to_mime` not checked in `RemoteSource` extension validation | Low | spec-18 |
| 18 | `AppState::Error` embeds UI prompt in data | Low | spec-17 |
| 19 | `comparison_value` re-parses on every diff comparison | Low | spec-17 |

---

## Dependency graph

```
spec-13 (complete)
     │
     ├──► spec-14  SSRF & credential safety    (auth.rs, client.rs)        ─┐
     │                                                                        │
     ├──► spec-16  Hot-path allocation         (tree.rs, filter.rs)         ─┤── Phase 5 (parallel)
     │                                                                        │
     └──► spec-18  LoadState invariants        (app.rs, loader.rs)          ─┘
              │                     │
              ▼                     ▼
         spec-15               spec-17
    Async-safe Matcher       Data model cleanup
    (matcher.rs, app.rs)   (filter.rs, diff.rs,     Phase 6 (parallel after Phase 5)
                            error.rs, app.rs)
```

**Phase 5** (run concurrently after spec-13):
- `spec-14` — touches `remote/auth.rs` + `remote/client.rs` only
- `spec-16` — touches `manifest/tree.rs` + `manifest/filter.rs` only
- `spec-18` — touches `app.rs` + `manifest/loader.rs` + `ui/file_list.rs`

**Phase 6** (run after their Phase-5 predecessors land):
- `spec-15` — after spec-18 (both modify `app.rs`)
- `spec-17` — after spec-16 (both modify `manifest/filter.rs`)

---

## Spec summaries

| Spec | Phase | Summary | Files |
|------|-------|---------|-------|
| [spec-14](spec-14-ssrf-credential-safety.md) | 5 | Redirect policy to block SSRF; sanitize auth error messages | `remote/client.rs`, `remote/auth.rs` |
| [spec-15](spec-15-async-safe-matcher.md) | 6 | Move nucleo ticking off the event loop; `Arc<str>` sharing; remove query clone | `search/matcher.rs`, `app.rs` |
| [spec-16](spec-16-hot-path-alloc.md) | 5 | `Cow<'_, str>` for `NodeValue::as_str`; path-buffer for filter; depth guard on `flatten_inner` | `manifest/tree.rs`, `manifest/filter.rs` |
| [spec-17](spec-17-data-model-cleanup.md) | 6 | Unify `apply`/`apply_ref`; `FieldDiff` refactor; `AppError::InvalidInput`; decouple error UI text | `manifest/filter.rs`, `compare/diff.rs`, `error.rs`, `app.rs` |
| [spec-18](spec-18-load-state-invariants.md) | 5 | `LoadState::Failed`; `loading_count()` computed; `ext_to_mime` eager check in `RemoteSource` | `app.rs`, `manifest/loader.rs`, `ui/file_list.rs` |

---

## Quality bar (applies to all specs)

All existing quality requirements from `specs/README.md` apply:

- `cargo fmt -- --check` passes
- `cargo clippy -- -D warnings` passes (zero warnings)
- `///` rustdoc on every new or modified `pub` item
- No `unwrap()` in production code
- `#[tracing::instrument]` on every `async fn` in the public API
- `proptest!` blocks in all data-transformation changes
- Tests written before implementation (TDD)
