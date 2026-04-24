# c2pa-tui Architecture

Terminal UI for browsing and comparing C2PA manifests. Built on [ratatui](https://ratatui.rs/) + crossterm, driven by a tokio async event loop.

---

## Directory layout

```
c2pa-tui/
├── src/
│   ├── main.rs          # CLI entry point (clap), source registration, runtime bootstrap
│   ├── lib.rs           # Public module declarations
│   ├── app.rs           # App struct, state machine, event loop, key handlers
│   ├── config.rs        # Config struct, Theme enum, style helpers
│   ├── error.rs         # AppError enum (thiserror), Result alias
│   ├── manifest/
│   │   ├── loader.rs    # ManifestSource trait + FileSource / DirSource / RemoteSource
│   │   ├── tree.rs      # DisplayNode, NodeValue, FlatNode, store_to_nodes, flatten
│   │   └── filter.rs    # FieldFilter — glob-based include/exclude over node paths
│   ├── search/
│   │   └── matcher.rs   # Matcher — nucleo-backed fuzzy search, MatchResult
│   ├── compare/
│   │   └── diff.rs      # ManifestDiff, FieldDiff, diff()
│   ├── remote/
│   │   ├── auth.rs      # Auth enum (None / Basic / Bearer / Digest), from_spec()
│   │   └── client.rs    # RemoteClient — reqwest wrapper, retry, scheme guard
│   └── ui/
│       ├── mod.rs       # draw() top-level dispatcher, help/error overlays
│       ├── layout.rs    # CachedLayout, split helpers, centered_popup()
│       ├── file_list.rs # Left-pane file list widget
│       ├── detail.rs    # Right-pane manifest tree widget (tui-tree-widget)
│       ├── status_bar.rs# Bottom status line
│       ├── search_bar.rs# Search overlay widget
│       ├── filter_bar.rs# Filter overlay widget
│       └── compare.rs   # Side-by-side diff overlay widget
├── tests/
│   ├── integration_loader.rs   # FileSource / DirSource / RemoteSource against fixtures
│   ├── integration_remote.rs   # RemoteClient against wiremock server
│   └── snapshot_ui.rs          # insta snapshot tests for every widget
└── benches/
    └── draw.rs          # Criterion benchmark for one full draw() call
```

---

## Module map

```mermaid
graph TD
    main["main.rs\nCLI · source registration · runtime"]
    app["app.rs\nApp · AppState · event loop"]
    config["config.rs\nConfig · Theme"]
    error["error.rs\nAppError · Result"]

    subgraph manifest
        loader["loader.rs\nManifestSource\nFileSource · DirSource · RemoteSource"]
        tree["tree.rs\nDisplayNode · NodeValue\nFlatNode · store_to_nodes · flatten"]
        filter["filter.rs\nFieldFilter"]
    end

    subgraph search
        matcher["matcher.rs\nMatcher · MatchResult\n(nucleo)"]
    end

    subgraph compare
        diff["diff.rs\nManifestDiff · FieldDiff · diff()"]
    end

    subgraph remote
        auth["auth.rs\nAuth"]
        client["client.rs\nRemoteClient\n(reqwest)"]
    end

    subgraph ui
        ui_mod["mod.rs\ndraw() dispatcher"]
        layout["layout.rs\nCachedLayout"]
        file_list["file_list.rs"]
        detail["detail.rs"]
        status_bar["status_bar.rs"]
        search_bar["search_bar.rs"]
        filter_bar["filter_bar.rs"]
        compare_ui["compare.rs"]
    end

    main --> app
    main --> config
    main --> loader
    main --> auth
    app --> loader
    app --> tree
    app --> filter
    app --> matcher
    app --> diff
    app --> client
    app --> config
    app --> ui_mod
    loader --> tree
    loader --> client
    loader --> auth
    ui_mod --> layout
    ui_mod --> file_list
    ui_mod --> detail
    ui_mod --> status_bar
    ui_mod --> search_bar
    ui_mod --> filter_bar
    ui_mod --> compare_ui
```

---

## Data flow

```mermaid
flowchart TD
    CLI["CLI args\n(paths / URLs / flags)"]
    Config["Config"]
    Sources["App::sources\nVec&lt;Arc&lt;dyn ManifestSource&gt;&gt;"]
    Spawn["tokio::spawn\n(background task)"]
    SDK["c2pa::Reader::with_file()"]
    Nodes["store_to_nodes()\nVec&lt;DisplayNode&gt;"]
    Channel["mpsc channel\n(usize, Result&lt;Vec&lt;DisplayNode&gt;&gt;)"]
    Loaded["App::loaded\nHashMap&lt;usize, LoadState&gt;"]
    Filter["FieldFilter::apply_ref()\nprune by glob path"]
    Render["ui::detail::draw()\ntui-tree-widget"]

    CLI --> Config
    CLI --> Sources
    Sources -->|"Enter / r"| Spawn
    Spawn --> SDK
    SDK --> Nodes
    Nodes --> Channel
    Channel --> Loaded
    Loaded -->|"each frame"| Filter
    Filter --> Render
```

---

## App state machine

`AppState` is the central discriminant that controls which key handlers and which UI overlays are active.

```mermaid
stateDiagram-v2
    [*] --> Browse

    Browse --> Searching   : /
    Browse --> Filtering   : f
    Browse --> Comparing   : c (second press)
    Browse --> Browse      : c (first press, sets compare_selection)
    Browse --> Browse      : Esc (clears compare_selection)

    Searching --> Browse   : Esc
    Searching --> Searching: typing / Backspace\n(reindex_and_search)
    Searching --> Searching: ↑ / ↓ / Tab\n(navigate results)

    Filtering --> Browse   : Esc
    Filtering --> Browse   : Enter (valid glob)
    Filtering --> Error    : Enter (invalid glob)

    Comparing --> Browse   : Esc
    Comparing --> Comparing: a (toggle show_all_diffs)

    Error --> Browse       : any key
```

`StateKind` is a cheap copy of the discriminant used for dispatch — avoids cloning the heap-allocated `String` inside `Searching { query }` or `Filtering { query }` on every key event.

---

## Key types

### `App` ([app.rs](src/app.rs))

The single owner of all runtime state. Passed by `&mut` reference to every draw call and event handler.

| Field | Purpose |
|---|---|
| `sources` | `Vec<Arc<dyn ManifestSource>>` — all registered inputs |
| `loaded` | `HashMap<usize, LoadState>` — per-source load results |
| `selected_left` | Index of the file list's selected row |
| `compare_selection` | Index of the left-pinned source for comparison |
| `filter` | Active `FieldFilter` applied before every render |
| `matcher` | Nucleo-backed fuzzy search index for the selected manifest |
| `state` | `AppState` — drives overlay visibility and key routing |
| `compare_diff_cache` | `Option<ManifestDiff>` — cached diff, invalidated on reload |
| `layout_cache` | `Option<(Rect, CachedLayout)>` — invalidated on resize |
| `detail_tree_state` | Expand/collapse/scroll state for `tui-tree-widget` |
| `search_result_indices` | `HashSet<usize>` pre-computed from `search_results` to avoid per-frame allocation in the detail pane |

### `DisplayNode` ([manifest/tree.rs](src/manifest/tree.rs))

The universal representation of a parsed manifest. A recursive tree:

```rust
pub struct DisplayNode {
    pub key: String,        // field name or section label
    pub value: NodeValue,   // Str / Json / Bytes / Missing
    pub children: Vec<DisplayNode>,
}
```

`store_to_nodes` converts a `c2pa::Reader` into this tree. Each manifest becomes one root node with five fixed children: **Claim**, **Claim Signature**, **Assertions**, **Ingredients**, **Validation**.

```mermaid
graph TD
    M["Manifest: urn:uuid:… (active)"]
    M --> Claim
    M --> Sig["Claim Signature"]
    M --> Assertions["Assertions (N)"]
    M --> Ingredients["Ingredients (N)"]
    M --> Validation

    Claim --> title["title: …"]
    Claim --> format["format: …"]
    Claim --> iid["instance_id: …"]
    Claim --> cg["claim_generator: …"]

    Sig --> issuer["issuer: …"]
    Sig --> time["time: …"]
    Sig --> alg["alg: …"]

    Assertions --> A1["c2pa.actions"]
    A1 --> a1a["[0].action: …"]
    Assertions --> A2["stds.schema-org.ClaimReview\n(Bytes)"]

    Ingredients --> I1["photo.jpg"]
    I1 --> i1f["format: …"]
    I1 --> i1r["relationship: …"]

    Validation --> vstatus["status: valid"]
```

`flatten` DFS-walks the tree into `Vec<FlatNode>` (dot-joined paths) for the search engine.

### `ManifestSource` ([manifest/loader.rs](src/manifest/loader.rs))

```rust
#[async_trait]
pub trait ManifestSource: Send + Sync {
    fn label(&self) -> &str;
    async fn load(&self, client: &RemoteClient) -> Result<Vec<DisplayNode>>;
    fn is_remote(&self) -> bool { false }
}
```

Three concrete implementations:

| Type | Input | Notes |
|---|---|---|
| `FileSource` | Single local path | Checks extension against MIME allowlist |
| `DirSource` | Directory | `walkdir` expand; each file gets its own row |
| `RemoteSource` | `https://` URL | Downloads to tempfile, delegates to `FileSource::load` |

### `FieldFilter` ([manifest/filter.rs](src/manifest/filter.rs))

Parses a semicolon-separated glob query (`assertions.*; !Claim Signature`) into include/exclude pattern lists. Applied on every render via `apply_ref` (borrow-based, only clones surviving nodes).

### `Matcher` ([search/matcher.rs](src/search/matcher.rs))

Wraps the `nucleo` fuzzy-matching engine. `index(nodes)` feeds a `FlatNode` list; `query(pattern)` returns ranked `MatchResult`s with byte-range highlights. Re-indexed whenever the selected manifest changes.

### `ManifestDiff` ([compare/diff.rs](src/compare/diff.rs))

`diff(left_label, left_nodes, right_label, right_nodes)` flattens both trees to `IndexMap<path, display>` and emits one `FieldDiff` per union key:

- `Equal { path, value }` — same value in both
- `Changed { path, left, right }` — value differs
- `OnlyLeft { path, value }` — missing from right
- `OnlyRight { path, value }` — missing from left

Left DFS order is preserved; right-only fields are appended. The result is cached in `App::compare_diff_cache` and only recomputed when either source reloads or the pair changes.

---

## Rendering pipeline

Every iteration of the event loop calls `ui::draw(frame, app)`:

```mermaid
flowchart TD
    Draw["ui::draw(frame, app)"]
    Layout["CachedLayout::compute(area)\nskipped if area unchanged"]
    FileList["file_list::draw()\nleft pane"]
    Detail["detail::draw()\nright pane\n(filtered tree · search highlights)"]
    StatusBar["status_bar::draw()\nbottom line"]
    OverlayCheck{AppState?}
    SearchBar["search_bar::draw()"]
    FilterBar["filter_bar::draw()"]
    Compare["compare::draw()"]
    ErrorOverlay["draw_error_overlay()"]
    HelpCheck{show_help?}
    HelpOverlay["draw_help_overlay()"]

    Draw --> Layout
    Layout --> FileList
    Layout --> Detail
    Layout --> StatusBar
    StatusBar --> OverlayCheck
    OverlayCheck -->|Searching| SearchBar
    OverlayCheck -->|Filtering| FilterBar
    OverlayCheck -->|Comparing| Compare
    OverlayCheck -->|Error| ErrorOverlay
    OverlayCheck -->|Browse| HelpCheck
    SearchBar --> HelpCheck
    FilterBar --> HelpCheck
    Compare --> HelpCheck
    ErrorOverlay --> HelpCheck
    HelpCheck -->|true| HelpOverlay
    HelpCheck -->|false| Done["frame complete"]
    HelpOverlay --> Done
```

`CachedLayout` stores four `Rect`s computed once per terminal size. It is also used for mouse hit-testing so click coordinates are always consistent with what was rendered.

---

## Async loading

```mermaid
sequenceDiagram
    participant User
    participant App
    participant Tokio as tokio::spawn
    participant Src as ManifestSource
    participant SDK as c2pa::Reader
    participant Chan as mpsc channel

    User->>App: Enter (or r)
    App->>App: loaded[idx] = Loading\nloading_count += 1
    App->>Tokio: spawn(async move)
    Tokio->>Src: load(&client).await
    Src->>SDK: Reader::with_file()
    SDK-->>Src: Reader
    Src->>Src: store_to_nodes(reader)
    Src-->>Tokio: Ok(Vec<DisplayNode>)
    Tokio->>Chan: tx.send((idx, Ok(nodes)))

    loop event_loop: tokio::select!
        Chan-->>App: load_rx.recv() → (idx, result)
        App->>App: loading_count -= 1\nloaded[idx] = Loaded(nodes)
        App->>App: reset detail_tree_state\nreindex matcher
        App->>App: invalidate compare_diff_cache\n(if idx is either compared source)
    end
```

The channel is unbounded so `tokio::spawn` never blocks. `loading_count` tracks in-flight tasks for the status bar spinner.

---

## Configuration

`Config` is constructed in `main.rs` from CLI flags and passed into `App::new`. Nothing reads environment variables at runtime — all configuration is resolved before the event loop starts.

| Flag | Field | Default |
|---|---|---|
| `--theme dark\|light\|mono` | `Theme` | `Dark` |
| `--no-mouse` | `mouse_enabled` | `true` |
| `--filter <glob>` | `initial_filter` | `None` |
| `--auth <spec>` | `Auth` | `None` |
| _(internal)_ | `left_pane_pct` | `25` |

`Theme` owns all styling via methods (`border_focused`, `highlight`, `match_highlight`, `diff_changed`, `diff_only_left`, `diff_only_right`) so widget code never hard-codes colors.

---

## Error handling

`AppError` (thiserror) unifies all failure modes. Errors from background loads are converted to `AppState::Error { message }` pre-formatted for display — one allocation at load time, zero on every subsequent frame redraw. Any key press dismisses the error and returns to `Browse`.

---

## Testing

| Layer | Approach |
|---|---|
| Unit | `#[cfg(test)]` modules inline in every source file; proptest for `diff`, `filter`, `matcher`, `auth` |
| Integration | `tests/integration_loader.rs` — real fixture files; `tests/integration_remote.rs` — wiremock HTTP server |
| Snapshot | `tests/snapshot_ui.rs` — insta snapshots for every widget variant |
| Benchmark | `benches/draw.rs` — criterion benchmark for a full `draw()` call with loaded manifest |
