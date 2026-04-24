# Spec 10 — Security Hardening

**Phase:** 4 (sequential — requires all Phase 3 specs merged and `cargo build` clean)  
**Depends on:** spec-09  
**Produces:** redacted `Auth` debug output; secure `RemoteClient::default()`; credential
indirection via env/file; timeout retry on both connect and timeout errors.

---

## Goal

Harden the authentication layer so that credentials are never accidentally logged,
never exposed in the process table via plain CLI arguments, and never silently lost
when `RemoteClient::default()` is called.  Also extend the retry logic to cover
timeout errors, not just connection errors.

A4 (surfacing the Digest-fallback as an error) is in **spec-11**, because that
change requires updating `RemoteClient::fetch` in the same commit as
`Auth::apply`'s new signature to avoid an intermediate compile failure.

---

## Files to modify

- `src/remote/auth.rs` — manual `Debug` impl; revised `from_spec` with `env:`/`file:` indirection
- `src/remote/client.rs` — fix `Default` impl; extend retry to `is_timeout()`

---

## A1 — Redact credentials from `Auth`'s `Debug` output

`Auth` currently derives `Debug`, which causes `Basic { password }` and
`Bearer { token }` to appear verbatim in any `#[instrument]`-annotated span that
does not `skip` the value.

Remove `Debug` from the `#[derive(...)]` list and add a manual impl:

```rust
impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Auth::None => write!(f, "Auth::None"),
            Auth::Basic { username, .. } => write!(
                f,
                "Auth::Basic {{ username: {:?}, password: [REDACTED] }}",
                username
            ),
            Auth::Bearer { .. } => write!(f, "Auth::Bearer {{ token: [REDACTED] }}"),
            Auth::Digest { username, .. } => write!(
                f,
                "Auth::Digest {{ username: {:?}, password: [REDACTED] }}",
                username
            ),
        }
    }
}
```

Add unit tests asserting that `format!("{:?}", auth)` contains `"[REDACTED]"` for
every credential-bearing variant and does **not** contain the actual secret:

```rust
#[test]
fn debug_basic_redacts_password() {
    let auth = Auth::Basic { username: "alice".into(), password: "s3cr3t".into() };
    let dbg = format!("{auth:?}");
    assert!(dbg.contains("[REDACTED]"), "password must be redacted");
    assert!(!dbg.contains("s3cr3t"), "plaintext password must not appear");
    assert!(dbg.contains("alice"), "username should be visible");
}

#[test]
fn debug_bearer_redacts_token() {
    let auth = Auth::Bearer { token: "tok123".into() };
    let dbg = format!("{auth:?}");
    assert!(dbg.contains("[REDACTED]"));
    assert!(!dbg.contains("tok123"));
}

#[test]
fn debug_digest_redacts_password() {
    let auth = Auth::Digest { username: "bob".into(), password: "hunter2".into() };
    let dbg = format!("{auth:?}");
    assert!(dbg.contains("[REDACTED]"));
    assert!(!dbg.contains("hunter2"));
}

#[test]
fn debug_none_is_safe() {
    let dbg = format!("{:?}", Auth::None);
    assert_eq!(dbg, "Auth::None");
}
```

---

## A2 — Fix `RemoteClient::Default` bypassing security configuration

The current `Default` impl calls `reqwest::Client::new()` with no timeout, no
`connect_timeout`, and no `user_agent`, silently losing the hardening applied in
`RemoteClient::new()`.

Replace the `Default` impl so it delegates to `new()`:

```rust
impl Default for RemoteClient {
    /// Construct a `RemoteClient` using the same timeouts and user-agent as
    /// [`RemoteClient::new`].  Provided for test convenience; production code
    /// should prefer `new()` for explicit error handling.
    fn default() -> Self {
        // new() is infallible with these builder settings (no TLS customisation
        // that can fail at runtime), so expect() is appropriate here.
        Self::new().expect("RemoteClient::new is infallible with default settings")
    }
}
```

Add a smoke test:

```rust
#[test]
fn default_does_not_panic() {
    let client = RemoteClient::default();
    let _ = client.client(); // confirms the inner client was initialised
}
```

---

## A3 — Credential indirection via `env:` and `file:` prefixes

Bearer tokens and passwords passed as `--auth bearer:<token>` are visible in
`ps aux` output and are recorded by most shells in history files.

Extend `Auth::from_spec()` to support two indirection prefixes for the secret
portion of `bearer`, `basic`, and `digest` specs:

| Prefix | Example | Behaviour |
|--------|---------|-----------|
| `env:VAR` | `bearer:env:MY_TOKEN` | Read secret from `$MY_TOKEN`; error if unset |
| `file:/path` | `basic:user:file:/run/secrets/pass` | Read first line (trimmed) from the file; error on I/O |

### Parsing architecture

The current `from_spec` uses `s.splitn(3, ':')`.  This cannot be extended to
support `file:` indirection for bearer tokens because `bearer:file:/home/u/pass`
would split into three parts `["bearer", "file", "/home/u/pass"]` instead of
correctly routing `"file:/home/u/pass"` through `resolve_secret`.

**Replace `from_spec` with a two-phase parse:**

```rust
fn resolve_secret(raw: &str) -> Result<String> {
    if let Some(var) = raw.strip_prefix("env:") {
        if var.is_empty() {
            return Err(AppError::Auth("env: requires a variable name".into()));
        }
        std::env::var(var)
            .map_err(|_| AppError::Auth(format!("env variable {var:?} is not set")))
    } else if let Some(path) = raw.strip_prefix("file:") {
        if path.is_empty() {
            return Err(AppError::Auth("file: requires a file path".into()));
        }
        let content = std::fs::read_to_string(path)
            .map_err(|e| AppError::Auth(format!("could not read credential file: {e}")))?;
        Ok(content.lines().next().unwrap_or("").trim().to_string())
    } else {
        Ok(raw.to_string())
    }
}

pub fn from_spec(s: &str) -> Result<Self> {
    if s.is_empty() || s == "none" {
        return Ok(Auth::None);
    }
    // Phase 1: split scheme from the rest on the first colon only.
    let (scheme, rest) = s
        .split_once(':')
        .ok_or_else(|| AppError::Auth(format!("invalid auth spec: {s:?}")))?;

    match scheme {
        "bearer" => {
            // rest is the full token spec, e.g. "mytoken", "env:MY_VAR", or
            // "file:/run/secrets/token".  resolve_secret handles all three.
            let token = resolve_secret(rest)?;
            Ok(Auth::Bearer { token })
        }
        "basic" | "digest" => {
            // Phase 2: split username from the password spec on the first colon.
            // This correctly preserves colons inside a password ("pa:ss") and
            // allows "user:file:/some/path" to route the full "file:/some/path"
            // string through resolve_secret.
            let (username, pass_spec) = rest.split_once(':').ok_or_else(|| {
                AppError::Auth(format!(
                    "{scheme} auth requires user:pass, got: {s:?}"
                ))
            })?;
            let password = resolve_secret(pass_spec)?;
            let username = username.to_string();
            if scheme == "basic" {
                Ok(Auth::Basic { username, password })
            } else {
                Ok(Auth::Digest { username, password })
            }
        }
        _ => Err(AppError::Auth(format!("invalid auth spec: {s:?}"))),
    }
}
```

This approach correctly handles:
- `bearer:mytoken` → token = `"mytoken"`
- `bearer:env:MY_TOKEN` → token = value of `$MY_TOKEN`
- `bearer:file:/run/secrets/token` → token = first line of that file
- `basic:alice:pa:ss` → username = `"alice"`, password = `"pa:ss"`
- `basic:alice:file:/run/secrets/pass` → username = `"alice"`, password = first line of file

### Update `main.rs` `--auth` help text

Add a `long_help` annotation to the `--auth` argument in the `Cli` struct to
document the indirection options and warn about the process-table risk:

```rust
#[arg(
    long,
    default_value = "none",
    long_help = "Authentication spec. Supported schemes:\n\
        \  none                    No authentication (default)\n\
        \  basic:user:pass         HTTP Basic (HTTPS only)\n\
        \  bearer:token            Bearer token\n\
        \  digest:user:pass        Digest (HTTPS only; falls back to Basic)\n\
        \n\
        Inline secrets are visible in `ps aux` and shell history.\n\
        Use indirection to avoid exposure:\n\
        \  bearer:env:MY_TOKEN     Read token from $MY_TOKEN\n\
        \  bearer:file:/path/tok   Read first line of file\n\
        \  basic:user:env:MY_PASS  Same for passwords"
)]
auth: String,
```

### Tests

Env-var tests mutate global state; mark them `#[serial_test::serial]` if
`serial_test` is available, or document that they must not run concurrently.
For simplicity the tests below use unique env-var names unlikely to collide.

```rust
#[test]
fn bearer_env_indirection() {
    // SAFETY: test-only env mutation; unique var name avoids parallel-test races.
    unsafe { std::env::set_var("C2PA_TUI_TEST_TOKEN_A3A", "secret_value") };
    let auth = Auth::from_spec("bearer:env:C2PA_TUI_TEST_TOKEN_A3A").unwrap();
    assert!(matches!(auth, Auth::Bearer { token } if token == "secret_value"));
    unsafe { std::env::remove_var("C2PA_TUI_TEST_TOKEN_A3A") };
}

#[test]
fn bearer_env_missing_var_returns_error() {
    unsafe { std::env::remove_var("C2PA_TUI_TEST_TOKEN_MISSING") };
    let err = Auth::from_spec("bearer:env:C2PA_TUI_TEST_TOKEN_MISSING").unwrap_err();
    assert!(matches!(err, AppError::Auth(_)));
}

#[test]
fn bearer_file_indirection() {
    let tmp = tempfile::tempdir().unwrap();
    use std::io::Write;
    let path = tmp.path().join("token.txt");
    let mut f = std::fs::File::create(&path).unwrap();
    writeln!(f, "file_token_value").unwrap();
    let spec = format!("bearer:file:{}", path.display());
    let auth = Auth::from_spec(&spec).unwrap();
    assert!(matches!(auth, Auth::Bearer { token } if token == "file_token_value"));
}

#[test]
fn bearer_file_whitespace_is_trimmed() {
    let tmp = tempfile::tempdir().unwrap();
    use std::io::Write;
    let path = tmp.path().join("token.txt");
    let mut f = std::fs::File::create(&path).unwrap();
    writeln!(f, "  trimmed_value  ").unwrap();
    let spec = format!("bearer:file:{}", path.display());
    let auth = Auth::from_spec(&spec).unwrap();
    assert!(matches!(auth, Auth::Bearer { token } if token == "trimmed_value"));
}

#[test]
fn bearer_file_empty_path_returns_error() {
    let err = Auth::from_spec("bearer:file:").unwrap_err();
    assert!(matches!(err, AppError::Auth(_)));
}

#[test]
fn basic_password_file_indirection() {
    let tmp = tempfile::tempdir().unwrap();
    use std::io::Write;
    let path = tmp.path().join("pass.txt");
    let mut f = std::fs::File::create(&path).unwrap();
    writeln!(f, "  my_password  ").unwrap();
    let spec = format!("basic:alice:file:{}", path.display());
    let auth = Auth::from_spec(&spec).unwrap();
    assert!(matches!(auth, Auth::Basic { password, .. } if password == "my_password"));
}

#[test]
fn basic_password_with_colon_still_works() {
    // Verifies the two-phase split preserves passwords containing colons.
    let auth = Auth::from_spec("basic:alice:pa:ss:word").unwrap();
    assert!(matches!(auth, Auth::Basic { password, .. } if password == "pa:ss:word"));
}
```

---

## A4 (production code) — Extend retry to `is_timeout()` errors

The retry guard in `RemoteClient::fetch` currently only retries on `is_connect()`.
Transient timeout errors (e.g. a briefly overloaded server) should also be retried.

In `src/remote/client.rs`, change:

```rust
// Before
Err(e) if attempts < 2 && e.is_connect() => {

// After
Err(e) if attempts < 2 && (e.is_connect() || e.is_timeout()) => {
```

This is a one-line production code change; the corresponding test is in spec-12 C3.

---

## Done criteria

```
cargo build
cargo test
cargo fmt -- --check
cargo clippy -- -D warnings
```

Verify manually:
```sh
# Credentials are redacted in debug output
RUST_LOG=debug cargo run -- --auth bearer:mytoken https://example.invalid/ 2>&1 | grep "mytoken" | wc -l
# Expected: 0

# env: indirection works
export C2PA_TUI_TOKEN=test123
cargo run -- --auth bearer:env:C2PA_TUI_TOKEN --help
unset C2PA_TUI_TOKEN
```
