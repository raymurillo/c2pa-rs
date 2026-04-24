# Getting Started with c2pa-tui

A terminal interface for browsing and comparing C2PA manifests — inspect content provenance data directly from your shell.

## Installation

Build and install from the repository root:

```bash
cargo install --path c2pa-tui
```

Or build and run directly during development:

```bash
cargo run -p c2pa-tui -- [PATHS_OR_URLS]
```

## Quick Start

Open one or more files:

```bash
c2pa-tui image.jpg
c2pa-tui photo.jpg video.mp4
c2pa-tui ./media-folder/
```

Fetch a manifest from a remote URL:

```bash
c2pa-tui https://example.com/signed-image.jpg
```

The interface opens immediately. Use `↑`/`↓` to move through the file list on the left, and `Tab` to switch focus to the manifest detail tree on the right.

Press `?` at any time to show all keyboard shortcuts. Press `q` or `Ctrl+C` to quit.

---

## The Interface

```
┌─ Files ──────────┬─ Manifest ──────────────────────────────────────┐
│ [✓] image.jpg    │ ▼ active_manifest                               │
│ [ ] video.mp4    │   ▼ assertions                                  │
│ [~] remote.jpg   │     ▶ c2pa.actions                              │
│                  │       created_with: Adobe Photoshop             │
│                  │     ▼ c2pa.hash.data                            │
│                  │       alg: sha256                               │
│                  │       hash: a1b2c3...                           │
├──────────────────┴─────────────────────────────────────────────────┤
│ ↑↓ navigate  Tab focus  / search  f filter  c compare  ? help     │
└────────────────────────────────────────────────────────────────────┘
```

**Left pane — File list**

- `[ ]` not yet loaded
- `[~]` loading in background
- `[✓]` loaded successfully

**Right pane — Manifest tree**

Each entry in the file list has its own manifest tree. Focus the detail pane with `Tab`, then use `Space` to expand or collapse nodes, and `↑`/`↓` to scroll.

---

## Loading Files

### Local files and directories

Pass one or more paths at startup. Directories are expanded automatically:

```bash
# Single file
c2pa-tui photo.jpg

# Multiple files
c2pa-tui photo.jpg video.mp4 document.pdf

# Entire directory
c2pa-tui ./my-media/

# Mix of files and directories
c2pa-tui banner.png ./archive/
```

Supported formats include JPEG, PNG, GIF, WebP, TIFF, AVIF, HEIC, MP4, MOV, AVI, and PDF.

### Remote URLs

HTTP and HTTPS URLs are fetched asynchronously in the background — the UI stays responsive while loading:

```bash
c2pa-tui https://example.com/image.jpg
```

The file list shows `[~]` while loading and `[✓]` when complete.

**With authentication:**

```bash
# Bearer token
c2pa-tui --auth bearer:mytoken123 https://api.example.com/asset.jpg

# HTTP Basic auth
c2pa-tui --auth basic:username:password https://example.com/image.jpg
```

### Reloading

Press `r` on a selected file to force a fresh reload (remote files are re-fetched, bypassing the cache).

---

## Browsing Manifests

### Navigate the file list

| Key | Action |
|-----|--------|
| `↑` / `k` | Previous file |
| `↓` / `j` | Next file |
| `Enter` | Load selected file |
| `r` | Reload / re-fetch selected file |

### Explore the detail tree

Switch to the detail tree with `Tab`, then:

| Key | Action |
|-----|--------|
| `↑` / `k` | Scroll up |
| `↓` / `j` | Scroll down |
| `Space` | Expand or collapse the selected node |
| `Tab` | Switch focus back to the file list |

---

## Search

Press `/` to open the fuzzy search overlay. Type any part of a field name or value — results update in real time as you type.

```
┌─ Search ── 3 matches ────────────────────┐
│ > sha256                                 │
├──────────────────────────────────────────┤
│   c2pa.hash.data › alg: sha256           │
│ ▸ c2pa.hash.data › hash: a1b2c3...       │
│   c2pa.ingredient › hash › alg: sha256  │
└──────────────────────────────────────────┘
```

| Key | Action |
|-----|--------|
| `↓` / `Tab` | Next match |
| `↑` | Previous match |
| `Esc` | Close search |

The detail tree scrolls to each match as you cycle through results.

---

## Filtering Fields

Press `f` to open the filter bar. Enter a glob pattern to show only matching fields:

```
┌─ Filter ─────────────────────────────────┐
│ > assertions.*                           │
├──────────────────────────────────────────┤
│ Matching fields:                         │
│   assertions                             │
│     c2pa.actions                         │
│     c2pa.hash.data                       │
└──────────────────────────────────────────┘
```

Press `Enter` to apply, `Esc` to cancel.

**Filter syntax:**

| Pattern | Matches |
|---------|---------|
| `assertions.*` | All fields under `assertions` |
| `*.hash` | Any field named `hash` at any depth |
| `!*.metadata` | Exclude fields matching `*.metadata` |
| `{creator,created}` | Fields named `creator` or `created` |

Multiple patterns can be separated with semicolons: `assertions.*;!*.metadata`

**Apply a filter at startup** with `--filter`:

```bash
c2pa-tui --filter "assertions.*" image.jpg
```

---

## Comparing Two Manifests

To compare manifests side by side:

1. Navigate to the **first file** in the file list and press `c` to mark it.
2. Navigate to the **second file** and press `c` again.

The compare view opens:

```
┌─ Compare — 5 differences ──────────────────────────────────────────────────┐
│ Field                       Left: image-a.jpg    Right: image-b.jpg        │
├─────────────────────────────────────────────────────────────────────────────┤
│ created_with                Adobe Photoshop      Adobe Lightroom           │ ← changed
│ claim_generator_version     23.0                 24.1                      │ ← changed
│ assertions.c2pa.actions     [present]            [missing]                 │ ← left only
│ assertions.c2pa.hash.data   a1b2c3...            a1b2c3...                 │
└─────────────────────────────────────────────────────────────────────────────┘
```

**Diff color coding:**

- **Yellow** — Field value differs between the two manifests
- **Red** — Field exists only in the left manifest
- **Green** — Field exists only in the right manifest
- Dim — Field is identical in both

Press `a` to toggle showing identical fields (hidden by default to reduce noise).

Press `Esc` to exit compare mode.

---

## Themes

Choose a color theme with `--theme`:

```bash
c2pa-tui --theme dark   # default: yellow highlights
c2pa-tui --theme light  # blue highlights on light background
c2pa-tui --theme mono   # accessible: bold/underline only, no color
```

---

## Mouse Support

Mouse is enabled by default:

- **Scroll wheel** — Scroll the focused pane
- **Click** — Click a file in the list to select it; click a pane to focus it

Disable mouse support (keyboard-only mode):

```bash
c2pa-tui --no-mouse image.jpg
```

---

## All Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `↑` / `k` | Move up |
| `↓` / `j` | Move down |
| `Tab` | Switch focus between file list and detail tree |
| `Enter` | Load selected file |
| `r` | Reload / re-fetch selected file |
| `Space` | Expand or collapse tree node (detail pane) |
| `/` | Open fuzzy search |
| `f` | Open field filter |
| `c` | Mark for compare (press twice on different files) |
| `a` | Toggle equal rows in compare view |
| `Esc` | Close overlay, exit compare, clear selection |
| `?` | Toggle help overlay |
| `q` / `Ctrl+C` | Quit |

---

## Examples

**Inspect a single image:**

```bash
c2pa-tui photo.jpg
```

**Compare two versions of a document:**

```bash
c2pa-tui v1.pdf v2.pdf
# Press c on v1.pdf, then c on v2.pdf
```

**Audit all assets in a directory, filtered to assertions only:**

```bash
c2pa-tui --filter "assertions.*" ./campaign-assets/
```

**Fetch and inspect a remote asset with a Bearer token:**

```bash
c2pa-tui --auth bearer:$API_TOKEN https://cdn.example.com/asset.jpg
```

**Run in monochrome mode on a minimal terminal:**

```bash
c2pa-tui --theme mono --no-mouse image.jpg
```
