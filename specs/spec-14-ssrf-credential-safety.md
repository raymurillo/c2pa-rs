# Spec 14 — SSRF Mitigation & Credential Safety

**Phase:** 5 (parallel — requires spec-13 merged and `cargo build` clean)  
**Depends on:** spec-13  
**Produces:** redirect policy blocking SSRF; sanitized auth error messages; tests

---

## Goal

Two security findings from the architecture review:

- **Finding 2** — `Auth::from_spec` includes the full auth spec string in its
  error message. If a user accidentally supplies a password-bearing string, it
  surfaces verbatim in logs and terminal output.
- **Finding 3** — `RemoteClient` uses the default `reqwest` redirect policy,
  which follows redirects to any scheme and host. A crafted URL can redirect to
  an internal service (`http://169.254.169.254/`, `http://localhost:8080/`) or
  to plain HTTP, bypassing the per-request scheme and credential checks.

> Note: credential-debug redaction (`Auth` Debug impl) is already covered by
> spec-10 A1 and must not be duplicated here.

---

## Files to modify

- `src/remote/client.rs` — custom redirect policy
- `src/remote/auth.rs` — sanitize error message in `from_spec`

---

## S1 — Sanitize `Auth::from_spec` error message

### Problem

```rust
// current — auth spec string (possibly containing a password) appears verbatim
Err(AppError::Auth(format!("invalid auth spec: {s:?}")))
```

### Fix

Show only the scheme token (the part before the first `:`), never the
credential fields:

```rust
let scheme_hint = s.splitn(2, ':').next().unwrap_or("<empty>");
Err(AppError::Auth(format!(
    "invalid auth spec; unrecognised scheme {scheme_hint:?} \
     (expected: none | bearer:<token> | basic:<user>:<pass> | digest:<user>:<pass>)"
)))
```

### Requirements

- The error message must not include any substring after the first `:` in the
  original input.
- The message must include the list of recognised schemes so the user can
  self-correct.
- `Auth::from_spec("")` and `Auth::from_spec("none")` continue to return
  `Ok(Auth::None)` unchanged.

---

## S2 — Block SSRF via redirect policy

### Problem

```rust
// current — default policy follows all redirects unconditionally
reqwest::Client::builder()
    .timeout(...)
    .connect_timeout(...)
    .user_agent(...)
    .build()
```

A server at `https://attacker.com/asset.jpg` can respond with
`302 Location: http://169.254.169.254/latest/meta-data/iam/security-credentials/`
and the client will follow it, bypassing the `scheme != "https"` check that
happens before the first request.

### Fix

Install a custom `redirect::Policy` on the `reqwest::Client` that:

1. Rejects any redirect to a non-`https` URL (same rule as the pre-request
   check in `fetch`).
2. Limits the redirect chain to 5 hops (reqwest default is 10; 5 is more than
   enough for real CDN chains).
3. Returns a descriptive error that `fetch` surfaces as `AppError::Auth` so the
   UI shows a clear message.

```rust
use reqwest::redirect;

let policy = redirect::Policy::custom(|attempt| {
    if attempt.previous().len() >= 5 {
        attempt.error("too many redirects (limit: 5)")
    } else if attempt.url().scheme() != "https" {
        attempt.error(format!(
            "redirect to non-HTTPS URL refused: {}",
            attempt.url()
        ))
    } else {
        attempt.follow()
    }
});

reqwest::Client::builder()
    .redirect(policy)
    .timeout(std::time::Duration::from_secs(30))
    .connect_timeout(std::time::Duration::from_secs(10))
    .user_agent(concat!("c2pa-tui/", env!("CARGO_PKG_VERSION")))
    .build()
    .map_err(AppError::Http)
```

### Requirements

- Redirects from `https://` → `https://` are followed (up to 5 hops).
- Redirects from `https://` → `http://` are refused.
- Redirects from `https://` → `ftp://` or any other scheme are refused.
- The error from a refused redirect propagates as `AppError::Http` (reqwest
  wraps the custom error as a `reqwest::Error`).
- `RemoteClient::default()` (used in tests) should also apply the same policy
  so test helpers are not accidentally more permissive than production code.

---

## API / Interface Design

No public API changes. Both fixes are internal to `new()` and `from_spec()`.

---

## Testing Strategy

### `src/remote/auth.rs`

```rust
#[test]
fn from_spec_error_does_not_include_credential() {
    let err = Auth::from_spec("oauth:mysecrettoken").unwrap_err().to_string();
    assert!(!err.contains("mysecrettoken"),
        "error message must not echo credentials: {err}");
    assert!(err.contains("oauth"), "scheme hint should appear: {err}");
}

#[test]
fn from_spec_error_lists_valid_schemes() {
    let err = Auth::from_spec("badscheme:x:y").unwrap_err().to_string();
    assert!(err.contains("bearer"), "valid schemes must be listed: {err}");
}
```

### `src/remote/client.rs`

All redirect tests require an HTTP mock server.  Use `wiremock` (already a dev
dependency) to stand up local servers.

```rust
#[tokio::test]
async fn redirect_https_to_http_is_refused() {
    // mock server 1: https (simulated by http on different port, testing policy logic)
    // In unit test context: build client and verify the redirect policy closure
    // directly by inspecting attempt.url() logic, or use wiremock redirect chains.
    let client = RemoteClient::new().unwrap();
    let url = url::Url::parse("http://localhost:1/redirect-to-internal").unwrap();
    // We cannot fully test redirect in unit tests without a real TLS server;
    // verify that the policy is installed by asserting the client's redirect
    // max is 5 via a mock that returns 301 chains.
}

#[tokio::test]
async fn more_than_5_redirects_returns_error() {
    // wiremock: /r0 → 301 /r1 → 301 /r2 → ... → 301 /r5 → 200
    // expect AppError::Http on the 6th hop
}
```

**Minimum tests to add:**

| Test | Location | What it checks |
|------|----------|----------------|
| `from_spec_error_does_not_include_credential` | `auth.rs` | error message sanitization |
| `from_spec_error_lists_valid_schemes` | `auth.rs` | helpfulness of error text |
| `redirect_to_http_refused` | `client.rs` | https→http redirect blocked |
| `redirect_chain_limit_enforced` | `client.rs` | >5 hops → error |
| `valid_https_redirect_followed` | `client.rs` | https→https still works |

---

## Edge Cases

- URL with no redirect: unchanged behaviour.
- `reqwest` custom error type: the closure passed to `Policy::custom` can return
  any `Into<Box<dyn Error>>`.  Map through `AppError::Http` in `fetch` when
  `e.is_redirect()` is true on the `reqwest::Error`.
- `RemoteClient::default()` is used in tests; apply the same policy there to
  avoid a two-tier security posture.

---

## Dependencies

No new crate dependencies.  `wiremock` is already in `[dev-dependencies]`.

---

## Done criteria

```bash
cargo test -p c2pa-tui -- remote::auth::tests remote::client::tests
cargo clippy -p c2pa-tui -- -D warnings
cargo fmt -p c2pa-tui -- --check
```

All new tests pass.  No existing tests regress.
