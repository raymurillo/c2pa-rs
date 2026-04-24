# Spec 23 — Release CI Pipeline

**Phase:** 9 (sequential — requires spec-21 and spec-22 merged)
**Depends on:** spec-21, spec-22
**Produces:** release workflow; `cargo deny` gate; `cargo auditable` + SBOM; SHA-256 checksums; GitHub Release automation

---

## Goal

Once the binary is ready for release (spec-21) and a diagnostic bundle exists
(spec-22), the CI pipeline must turn a version tag into a signed, verifiable
release with supply-chain metadata — without a human running local commands.

This spec consolidates the size-gate workflow from spec-21 P3 into a full
release pipeline that produces:

- Stripped binaries for 5 targets (best-effort Windows per user confirmation)
- Debug-info sidecars per target
- SBOM (CycloneDX format, via `syft`)
- `cargo auditable` metadata embedded in every binary
- `cargo deny` report blocking release on advisory hits
- SHA-256 checksums file signed via GitHub artifact attestation
- A GitHub Release with all artifacts attached and auto-generated notes

---

## Files to modify

- `.github/workflows/release.yml` — replace the skeleton from spec-21 with the
  full pipeline
- `.github/workflows/security-audit.yml` — new; weekly `cargo deny` + `cargo audit`
- `deny.toml` — extend the existing file with `c2pa-tui`-specific rules
- `c2pa-tui/docs/RELEASE.md` — extend with release process, checksum verification
- `c2pa-tui/Cargo.toml` — add `[package.metadata.release]` (release-plz hints)

---

## C1 — Release workflow

```yaml
# .github/workflows/release.yml

name: c2pa-tui release

on:
  push:
    tags: ['c2pa-tui-v*']
  workflow_dispatch:
    inputs:
      dry_run:
        description: 'Build and attach artifacts but do not create Release'
        type: boolean
        default: false

permissions:
  contents: write         # create Release
  id-token: write         # OIDC for attestations
  attestations: write     # artifact attestation

jobs:
  deny:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: EmbarkStudios/cargo-deny-action@v2
        with: { manifest-path: c2pa-tui/Cargo.toml }

  build:
    needs: deny
    strategy:
      fail-fast: false
      matrix:
        include:
          - { os: ubuntu-latest,  target: x86_64-unknown-linux-gnu,   size_limit: 15728640, cross: false }
          - { os: ubuntu-latest,  target: x86_64-unknown-linux-musl,  size_limit: 16777216, cross: true  }
          - { os: macos-14,       target: aarch64-apple-darwin,       size_limit: 17825792, cross: false }
          - { os: macos-13,       target: x86_64-apple-darwin,        size_limit: 17825792, cross: false }
          - { os: windows-latest, target: x86_64-pc-windows-msvc,     size_limit: 19922944, cross: false, best_effort: true }
    runs-on: ${{ matrix.os }}
    continue-on-error: ${{ matrix.best_effort == true }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with: { targets: ${{ matrix.target }} }
      - name: Install cargo-auditable
        run: cargo install cargo-auditable --locked
      - name: Install cross (musl only)
        if: matrix.cross
        run: cargo install cross --git https://github.com/cross-rs/cross --locked
      - name: Build stripped binary
        run: |
          if [ "${{ matrix.cross }}" = "true" ]; then
            cross auditable build -p c2pa-tui --release --target ${{ matrix.target }}
          else
            cargo auditable build -p c2pa-tui --release --target ${{ matrix.target }}
          fi
        shell: bash
      - name: Build debug-info sidecar
        run: cargo auditable build -p c2pa-tui --profile release-debug --target ${{ matrix.target }}
      - name: Enforce size budget
        shell: bash
        run: |
          bin="target/${{ matrix.target }}/release/c2pa-tui${{ matrix.os == 'windows-latest' && '.exe' || '' }}"
          size=$(wc -c < "$bin")
          echo "size=$size limit=${{ matrix.size_limit }}"
          [ "$size" -le ${{ matrix.size_limit }} ] || { echo "::error::binary too large: $size"; exit 1; }
      - name: Generate SBOM
        uses: anchore/sbom-action@v0
        with:
          path: target/${{ matrix.target }}/release/
          format: cyclonedx-json
          output-file: sbom-${{ matrix.target }}.cdx.json
      - name: Compute SHA-256
        shell: bash
        run: |
          cd target/${{ matrix.target }}/release
          for f in c2pa-tui c2pa-tui.exe; do
            [ -f "$f" ] && sha256sum "$f" > "$f.sha256" || true
          done
      - name: Attest artifact provenance
        uses: actions/attest-build-provenance@v1
        with:
          subject-path: target/${{ matrix.target }}/release/c2pa-tui*
      - uses: actions/upload-artifact@v4
        with:
          name: c2pa-tui-${{ matrix.target }}
          path: |
            target/${{ matrix.target }}/release/c2pa-tui*
            target/${{ matrix.target }}/release-debug/c2pa-tui*
            sbom-${{ matrix.target }}.cdx.json

  release:
    needs: build
    if: startsWith(github.ref, 'refs/tags/c2pa-tui-v') && !inputs.dry_run
    runs-on: ubuntu-latest
    steps:
      - uses: actions/download-artifact@v4
        with: { pattern: 'c2pa-tui-*', path: dist/ }
      - name: Create GitHub Release
        uses: softprops/action-gh-release@v2
        with:
          files: dist/**/*
          generate_release_notes: true
          draft: true   # human reviews before publishing
```

### Requirements

- The `deny` job blocks everything — no artifact is uploaded if advisories
  trip.
- Every binary embeds its dep graph via `cargo auditable` so downstream
  scanners (`cargo audit bin`) work against the shipped file.
- `continue-on-error: true` on the Windows row means a Windows build
  failure does not fail the workflow (best-effort per user confirmation).
- The Release is created as a **draft** — a maintainer reviews the
  checksums and notes before publishing. This is a deliberate choice to
  prevent a tag-push from immediately going public.

---

## C2 — `deny.toml` rules for c2pa-tui

The existing workspace `deny.toml` may have broad rules. Append a
c2pa-tui-specific section:

```toml
# deny.toml (existing file — append)

[advisories]
# Fail on any unmaintained advisory in c2pa-tui's transitive closure.
unmaintained = "deny"
yanked       = "deny"

[bans]
multiple-versions = "warn"  # c2pa-tui-specific; workspace may be stricter
# New dependency from spec-22 — explicitly allow.
allow = [
    { name = "tar",   version = "0.4" },
    { name = "flate2", version = "1" },
]

[licenses]
# Matches the MIT OR Apache-2.0 stance of c2pa-tui itself.
allow = ["MIT", "Apache-2.0", "Apache-2.0 WITH LLVM-exception", "BSD-3-Clause", "ISC", "Unicode-DFS-2016", "Zlib"]
confidence-threshold = 0.9

[sources]
unknown-registry = "deny"
unknown-git      = "deny"
```

### Requirements

- `cargo deny check advisories` passes at the moment of the release tag.
- A *new* unmaintained advisory on a transitive dep fails the workflow
  within one week (the weekly security-audit job catches it between
  releases).

---

## C3 — Weekly security audit

```yaml
# .github/workflows/security-audit.yml

name: security audit

on:
  schedule: [ { cron: '0 6 * * 1' } ]   # Monday 06:00 UTC
  workflow_dispatch:

jobs:
  audit:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: rustsec/audit-check@v1.4.1
        with:
          token: ${{ secrets.GITHUB_TOKEN }}
      - uses: EmbarkStudios/cargo-deny-action@v2
        with: { manifest-path: c2pa-tui/Cargo.toml }
```

### Requirements

- Failure opens a GitHub issue with the `security` label (default behaviour
  of `rustsec/audit-check`).
- Runs independently of the release workflow so advisories are caught in
  the dead time between releases.

---

## C4 — Release docs update

Extend `c2pa-tui/docs/RELEASE.md` (created in spec-21 P4) with:

- **Tagging conventions**: `c2pa-tui-v0.1.0`, annotated tags only.
- **Checksum verification**: one-liner for users on each platform to verify
  their downloaded binary matches the `.sha256` sidecar.
- **Provenance verification**: `gh attestation verify c2pa-tui --owner contentauth`.
- **SBOM lookup**: pointer to the Release asset and a 3-line example of
  running `grype` against it.
- **Yanking a release**: procedure if an issue is found post-publish
  (delete the Release, keep the tag for reproducibility, file a GHSA).

---

## C5 — `release-plz` hints (optional)

c2pa-tui's workspace already has `release-plz.toml`. Add a package-level
hint so version bumps generate a tag matching the workflow trigger:

```toml
# c2pa-tui/Cargo.toml

[package.metadata.release]
tag-name = "c2pa-tui-v{{version}}"
```

Optional — the workflow works without it as long as the manual tag follows
the convention. Included for maintainer convenience.

---

## Edge cases

- **Tag pushed during a dependency advisory**: the `deny` job blocks before
  any build runs. The tag remains but no artifacts are produced. Maintainer
  investigates the advisory, lands a fix, retags (or force-moves the tag
  after coordination — documented).
- **GitHub Actions outage during a release**: no artifacts produced, no
  Release created, tag stays pointing at the commit. Re-run the workflow
  from the UI using `workflow_dispatch` once Actions is back.
- **`cargo auditable` unavailable for a target**: it supports all Tier-1
  and most Tier-2 targets. If a new target is added that doesn't support
  it, the build step fails clearly; we drop the target or wait for upstream
  support rather than shipping without the metadata.
- **Draft Release accidentally published early**: rare; draft status is
  enforced in the workflow. A bot labelled review step could be added later
  if this becomes a problem.

---

## Dependencies

- No new runtime dependencies.
- CI dependencies: `cargo-auditable`, `cargo-deny`, `syft` (via
  `anchore/sbom-action`), `cross` (musl only).
- All pinned to major versions via the workflow; dependabot PRs maintain
  minor/patch bumps.

---

## Done criteria

```bash
# Local validation of workflow syntax
act -l                             # or gh workflow view release.yml
act -j deny                        # dry-run the deny gate locally

cargo deny check --manifest-path c2pa-tui/Cargo.toml
```

- `.github/workflows/release.yml` syntactically valid and covers all five
  targets with correct size limits.
- `.github/workflows/security-audit.yml` runs on schedule and on dispatch.
- `deny.toml` additions do not regress existing workspace rules
  (`cargo deny check` still passes on main).
- End-to-end: pushing `c2pa-tui-v0.1.0-rc1` to a test branch produces a
  draft Release with 5 target archives, 5 SBOMs, 5 SHA-256 files, and 1
  attestation — verified by downloading and running `gh attestation verify`.
- `docs/RELEASE.md` includes user-facing verification commands for all
  three host platforms.
