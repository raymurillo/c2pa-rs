# Spec 01 — Manifest Data Layer

**Phase:** 1 (concurrent with spec-02, spec-03, spec-04, spec-05)  
**Depends on:** spec-00 foundation committed and compiling  
**Produces:** `manifest/loader.rs`, `manifest/tree.rs`, `manifest/filter.rs` fully implemented

---

## Goal

Implement the three manifest data modules: file/directory loading, conversion of a
`c2pa::ManifestStore` into a `DisplayNode` tree, and field filtering with glob patterns.
No TUI code. No HTTP code. Pure data transformation.

Follow **TDD order**: write the unit tests first (they will fail), then implement
until they pass. Use `mockall::automock` (already on the `ManifestSource` trait in
spec-00) rather than hand-rolled fakes where a mock is needed.

---

## Files to modify

- `src/manifest/loader.rs` — implement `FileSource::load` and `DirSource`
- `src/manifest/tree.rs` — implement `store_to_nodes`
- `src/manifest/filter.rs` — implement `FieldFilter`

Do **not** touch `RemoteSource::load` — that is spec-02's responsibility.

---

## `manifest/loader.rs`

### Supported file extensions

C2PA supports the following MIME types. Map these extensions:

```
.jpg / .jpeg   image/jpeg
.png           image/png
.gif           image/gif
.webp          image/webp
.tiff / .tif   image/tiff
.avif          image/avif
.heic / .heif  image/heic
.mp4 / .m4v    video/mp4
.mov           video/quicktime
.avi           video/x-msvideo
.pdf           application/pdf
.c2pa          application/x-c2pa-manifest-store
```

Any other extension → `AppError::UnsupportedFormat(ext)`.

### `FileSource::load`

Annotate with `#[tracing::instrument(skip(self, _client), fields(path = %self.path.display()))]`
so every load call emits a tracing span. Use `tracing::debug!` on success and
`tracing::warn!` when a file has no manifest.

Do **not** use `.unwrap()` anywhere in the implementation. Map all fallible calls
through `?` or `.map_err(AppError::...)`.

```rust
#[tracing::instrument(skip(self, _client), fields(path = %self.path.display()))]
async fn load(&self, _client: &RemoteClient) -> Result<Vec<DisplayNode>> {
    let ext = self.path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let mime = ext_to_mime(&ext)
        .ok_or_else(|| AppError::UnsupportedFormat(ext.clone()))?;
    let store = c2pa::ManifestStore::from_file(&self.path)
        .map_err(|e| match e {
            // Treat "no manifest found" as an informational node, not fatal
            c2pa::Error::JumbfNotFound | c2pa::Error::ProvenanceMissing => {
                // return a special "no manifest" tree rather than Err
                // see store_to_nodes for how NoManifest is handled
                todo!()  // handle inline — see below
            }
            other => AppError::C2pa(other),
        })?;
    Ok(store_to_nodes(&store))
}
```

For `c2pa::Error::JumbfNotFound` / `c2pa::Error::ProvenanceMissing` (or whatever the
SDK's "no manifest" variant is — check the SDK source), do NOT return `Err`. Instead
return `Ok(vec![DisplayNode { key: "status".into(), value: NodeValue::Str("No C2PA manifest found".into()), children: vec![] }])`.

> **Action required:** Look at `sdk/src/error.rs` in the c2pa-rs repo to find the
> exact error variant that means "no manifest embedded". The variant name may differ.

### `DirSource::entries`

Use `walkdir::WalkDir` to recursively enumerate files. Yield one `FileSource` per
file whose extension is in the supported set (ignore others silently). Sort entries
by path for deterministic ordering.

```rust
pub fn entries(&self) -> Result<Vec<FileSource>> {
    let mut sources = Vec::new();
    for entry in WalkDir::new(&self.path).sort_by_file_name() {
        let entry = entry?;
        if entry.file_type().is_file() {
            let ext = entry.path().extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if ext_to_mime(&ext).is_some() {
                sources.push(FileSource::new(entry.path().to_path_buf()));
            }
        }
    }
    Ok(sources)
}
```

`DirSource::load` should call `entries()` and load all entries sequentially (not
concurrently — avoid overwhelming the c2pa parser), collecting results. If any file
fails to parse (e.g. corrupted), include a single error node for that file rather
than aborting the whole directory.

> **Design note — Directory expansion model:** `DirSource::load()` returns a
> combined `Vec<DisplayNode>` where each file's nodes are wrapped under a
> `DisplayNode { key: filename, value: NodeValue::Missing, children: file_nodes }`.
> **However**, the primary usage pattern (from `main.rs` in spec-09) is to call
> `DirSource::entries()` directly and add each `FileSource` individually as its
> own source in `App.sources`. This gives the user one navigable file-list row per
> file. `DirSource::load()` is a convenience for any caller that wants the
> aggregate view; `main.rs` must not use it.

---

## `manifest/tree.rs` — `store_to_nodes`

Convert a `c2pa::ManifestStore` into the following tree structure. Each top-level
section is a `DisplayNode` whose children are the fields within it.

### Tree structure to produce

```
▾ Manifest: <label>            // one per manifest in the store
  ▾ Claim
      title:        <string>
      format:       <string>
      instance_id:  <string>
      claim_generator: <string>
  ▾ Claim Signature
      issuer:       <string or "unknown">
      time:         <RFC3339 string or "unknown">
      alg:          <string or "unknown">
  ▾ Assertions (<n>)
    ▾ <assertion_label>         // e.g. "c2pa.actions"
        <field>: <value>        // serde_json expand of assertion data
        ...
  ▾ Ingredients (<n>)
    ▾ <ingredient_title or index>
        format:       <string>
        instance_id:  <string>
        relationship: <string>
        ▾ Manifest (if has_embedded_manifest)
            ... recursive
  ▾ Validation
      status:  <"valid" | "invalid" | "unknown">
      ▾ Errors (<n>)           // only if validation_status has errors
          <code>: <explanation>
```

### Implementation notes

- Use `store.manifests()` to iterate manifests. The active manifest is
  `store.get_active_manifest()` — put it first, label it with `(active)`.
- Assertion data: call `.labeled_assertion::<serde_json::Value>(label)` or use the
  raw assertion bytes. Convert to a JSON value and recurse into its fields via a
  `json_to_nodes(key, value)` helper.
- `json_to_nodes` should:
  - For a JSON object: produce a parent node with children for each field
  - For a JSON array: produce a parent node with children `[0]`, `[1]`, …
  - For scalars: produce a leaf `NodeValue::Str`
  - For binary/base64 fields whose decoded length > 64 bytes: use `NodeValue::Bytes(n)`
- Validation: call `store.validation_status()`. Map each status item to a child
  under the Validation node. Use `NodeValue::Str` with the status code + explanation.

> **Action required:** Explore `c2pa::ManifestStore` and `c2pa::Manifest` API in
> `sdk/src/` to find the exact method names. The API may differ from what is listed
> here. Write the actual method calls against the real SDK, not assumptions.

---

## `manifest/filter.rs` — `FieldFilter`

### `FieldFilter::from_query`

Parse a semicolon-separated query string into include/exclude patterns:

```
"assertions.*"                → include assertions.*
"!claim_signature"            → exclude claim_signature
"assertions.*;!assertions.c2pa.hash.*"  → include assertions.* minus hash subtrees
```

Rules:
- Tokens prefixed with `!` are exclude patterns; all others are include patterns
- If no include patterns are specified, default include is `**` (everything)
- Patterns use `glob::Pattern` syntax

**Input limits (enforce before calling `glob::Pattern::new`):**
- Reject the entire query if it exceeds **256 characters** → `AppError::Glob`
- Reject any single token that exceeds **128 characters** → `AppError::Glob`
- Reject any token containing more than **4 `{` characters** (limits alternation
  depth) → `AppError::Glob`

These caps prevent a user from crafting a pathologically complex pattern that causes
the glob crate to spend unbounded time during matching.

### `FieldFilter::apply`

Walk the `DisplayNode` tree recursively, pruning nodes whose dot-joined paths do not
match any include pattern, or that do match an exclude pattern. A node is **kept**
if:

1. Its path matches at least one include pattern (or no include patterns were given)
2. Its path does NOT match any exclude pattern

When a parent is kept but a child is pruned, the parent's `children` field is
filtered. When a parent is pruned, its entire subtree is removed.

---

## Unit tests

Write `#[cfg(test)]` blocks in each file **before** writing the implementation
(TDD). The tests should fail with `todo!()` bodies and pass after implementation.

### `loader.rs` tests

```rust
#[test]
fn unsupported_ext_returns_error() {
    // create a FileSource with a .txt path
    // verify load() returns AppError::UnsupportedFormat
}

#[test]
fn dir_entries_returns_only_supported_files() {
    // create a temp dir with .jpg, .png, .txt files
    // verify DirSource::entries() returns only .jpg and .png
}
```

### `tree.rs` tests

Use the fixture files from `tests/fixtures/` (load them with `c2pa::ManifestStore::from_file`).

```rust
#[test]
fn signed_jpeg_has_claim_node() {
    let store = c2pa::ManifestStore::from_file("tests/fixtures/signed.jpg").unwrap();
    let nodes = store_to_nodes(&store);
    assert!(nodes.iter().any(|n| n.key.starts_with("Manifest")));
    let manifest_node = &nodes[0];
    assert!(manifest_node.children.iter().any(|n| n.key == "Claim"));
}

#[test]
fn unsigned_file_returns_no_manifest_node() {
    // load a file with no C2PA manifest
    // verify the result is a single informational node
}

#[test]
fn json_array_assertion_expands_to_indexed_children() {
    // synthetic: build a DisplayNode for a JSON array value
    // verify children are [0], [1], etc.
}

#[test]
fn flatten_produces_dot_joined_paths() {
    let nodes = vec![
        DisplayNode {
            key: "Claim".into(),
            value: NodeValue::Missing,
            children: vec![
                DisplayNode { key: "title".into(), value: NodeValue::Str("x".into()), children: vec![] }
            ],
        }
    ];
    let flat = flatten(&nodes);
    assert_eq!(flat[0].path, "Claim");
    assert_eq!(flat[1].path, "Claim.title");
}
```

### `filter.rs` tests

```rust
#[test]
fn query_exceeding_max_length_returns_error() {
    let long = "a".repeat(257);
    assert!(FieldFilter::from_query(&long).is_err());
}

#[test]
fn token_exceeding_max_length_returns_error() {
    let long_token = "a".repeat(129);
    assert!(FieldFilter::from_query(&long_token).is_err());
}

#[test]
fn token_with_excess_alternation_depth_returns_error() {
    // 5 opening braces — should be rejected
    assert!(FieldFilter::from_query("a.{b.{c.{d.{e.{f}}}}}").is_err());
}

#[test]
fn include_only_assertions() {
    let f = FieldFilter::from_query("assertions.*").unwrap();
    let nodes = make_sample_tree(); // helper producing Claim + Assertions nodes
    let filtered = f.apply(nodes);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].key, "Assertions");
}

#[test]
fn exclude_pattern_removes_node() {
    let f = FieldFilter::from_query("**;!Claim Signature").unwrap();
    let nodes = make_sample_tree();
    let filtered = f.apply(nodes);
    assert!(!filtered.iter().any(|n| n.key == "Claim Signature"));
}

#[test]
fn empty_filter_returns_all_nodes() {
    let f = FieldFilter::default();
    let nodes = make_sample_tree();
    let count = nodes.len();
    assert_eq!(f.apply(nodes).len(), count);
}
```

---

## Property-based tests (`proptest`)

Add a `proptest!` block in `filter.rs` covering arbitrary include/exclude patterns:

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn filter_never_adds_nodes(include in "[a-z.*]+", nodes in prop::collection::vec(arb_node(), 0..10)) {
        let f = FieldFilter::from_query(&include).unwrap_or_default();
        let filtered = f.apply(nodes.clone());
        prop_assert!(filtered.len() <= nodes.len());
    }

    #[test]
    fn filter_with_wildcard_includes_all(nodes in prop::collection::vec(arb_node(), 1..10)) {
        let f = FieldFilter::from_query("**").unwrap();
        prop_assert_eq!(f.apply(nodes.clone()).len(), nodes.len());
    }
}
```

Write an `arb_node()` `Strategy` that produces a `DisplayNode` with a random
`key` (lowercase alpha, 1–12 chars) and `NodeValue::Str` with a random value.

---

## Done criteria

```
cargo test --lib manifest        # all unit tests and proptest cases pass
cargo build                      # no regressions in other stubs
cargo fmt -- --check
cargo clippy -- -D warnings
```
