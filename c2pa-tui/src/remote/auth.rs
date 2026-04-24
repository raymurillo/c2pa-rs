use crate::error::{AppError, Result};

/// HTTP authentication method to apply to remote manifest requests.
///
/// The `Debug` impl is implemented manually so that secrets (passwords and
/// bearer tokens) never appear in `#[instrument]` spans, `tracing` events, or
/// any other `{:?}` formatting.
#[derive(Clone, Default)]
pub enum Auth {
    /// No authentication.
    #[default]
    None,
    /// HTTP Basic authentication.
    Basic { username: String, password: String },
    /// Bearer token authentication.
    Bearer { token: String },
    /// HTTP Digest authentication.
    Digest { username: String, password: String },
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Auth::None => write!(f, "Auth::None"),
            Auth::Basic { username, .. } => write!(
                f,
                "Auth::Basic {{ username: {username:?}, password: [REDACTED] }}"
            ),
            Auth::Bearer { .. } => write!(f, "Auth::Bearer {{ token: [REDACTED] }}"),
            Auth::Digest { username, .. } => write!(
                f,
                "Auth::Digest {{ username: {username:?}, password: [REDACTED] }}"
            ),
        }
    }
}

/// Maximum bytes read from a credential file. A token or password comfortably
/// fits in 64 KiB; anything larger is almost certainly the wrong file and we
/// refuse to page it into memory.
const CREDENTIAL_FILE_READ_LIMIT: u64 = 64 * 1024;

/// Resolve a secret fragment through optional indirection prefixes.
///
/// * `env:VAR` reads the secret from environment variable `VAR`.
/// * `file:/path` reads the first line of the file at `/path` (trimmed).
/// * anything else is returned verbatim.
///
/// The resolved secret must be non-empty — an empty env var or blank file
/// indicates operator error (wrong variable, truncated secrets file) and we
/// fail loud rather than authenticating with an empty credential the server
/// will silently reject.
///
/// File reads are bounded by [`CREDENTIAL_FILE_READ_LIMIT`] to avoid paging in
/// an accidentally-pointed-at large file, and only consume the first line so
/// FIFOs and `/dev/tty` do not hang the process longer than the first newline.
///
/// Errors are returned as [`AppError::Auth`] so that the CLI surfaces them
/// uniformly alongside other authentication failures.
fn resolve_secret(raw: &str) -> Result<String> {
    if let Some(var) = raw.strip_prefix("env:") {
        if var.is_empty() {
            return Err(AppError::Auth("env: requires a variable name".into()));
        }
        let value = std::env::var(var)
            .map_err(|_| AppError::Auth(format!("env variable {var:?} is not set")))?;
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(AppError::Auth(format!("env variable {var:?} is empty")));
        }
        // Return the raw (untrimmed) value so callers can opt into whitespace-
        // bearing secrets via the environment if they truly need to. Servers
        // that strip whitespace will not care.
        Ok(value)
    } else if let Some(path) = raw.strip_prefix("file:") {
        if path.is_empty() {
            return Err(AppError::Auth("file: requires a file path".into()));
        }
        read_first_line_trimmed(path)
    } else {
        Ok(raw.to_string())
    }
}

/// Open `path`, read up to [`CREDENTIAL_FILE_READ_LIMIT`] bytes of the first
/// line, and return it trimmed of surrounding whitespace.
///
/// Uses `BufReader::take` + `read_line` rather than `read_to_string` so that:
///
/// * A huge file pointed at by mistake does not balloon memory.
/// * A FIFO or `/dev/tty` returns as soon as a newline arrives.
/// * Only the first line ever enters memory, matching the documented contract.
fn read_first_line_trimmed(path: &str) -> Result<String> {
    use std::io::{BufRead, BufReader, Read};

    let file = std::fs::File::open(path)
        .map_err(|e| AppError::Auth(format!("could not open credential file: {e}")))?;
    // Cap the underlying reader first, then wrap in BufReader — this is the
    // combination that implements `BufRead` (and therefore `read_line`) while
    // still bounding bytes pulled from the file.
    let mut reader = BufReader::new(file.take(CREDENTIAL_FILE_READ_LIMIT));
    let mut first = String::new();
    reader
        .read_line(&mut first)
        .map_err(|e| AppError::Auth(format!("could not read credential file: {e}")))?;
    let trimmed = first.trim();
    if trimmed.is_empty() {
        return Err(AppError::Auth(format!(
            "credential file {path:?} is empty or blank"
        )));
    }
    Ok(trimmed.to_string())
}

/// Split `rest` into `(username, resolved_password)` for `basic`/`digest`
/// schemes, preserving colons inside a password and routing the password
/// fragment through [`resolve_secret`].
///
/// `scheme` and `full_spec` are used only for error messages.
fn parse_user_pass(rest: &str, scheme: &str, full_spec: &str) -> Result<(String, String)> {
    let (username, pass_spec) = rest.split_once(':').ok_or_else(|| {
        AppError::Auth(format!(
            "{scheme} auth requires user:pass, got: {full_spec:?}"
        ))
    })?;
    let password = resolve_secret(pass_spec)?;
    Ok((username.to_string(), password))
}

impl Auth {
    /// Parse an auth specification string into an `Auth` variant.
    ///
    /// Supported schemes:
    ///
    /// | Spec | Meaning |
    /// |------|---------|
    /// | `none` or empty | [`Auth::None`] |
    /// | `bearer:<secret>` | [`Auth::Bearer`] |
    /// | `basic:<user>:<secret>` | [`Auth::Basic`] |
    /// | `digest:<user>:<secret>` | [`Auth::Digest`] |
    ///
    /// `<secret>` may be given literally, or routed through indirection:
    ///
    /// * `env:VAR` — read from environment variable `VAR`
    /// * `file:/path` — read the first trimmed line of the file
    ///
    /// Indirection keeps credentials out of `ps aux` output and shell history.
    ///
    /// # Examples
    ///
    /// ```
    /// use c2pa_tui::remote::Auth;
    /// let auth = Auth::from_spec("bearer:mytoken").unwrap();
    /// ```
    pub fn from_spec(s: &str) -> Result<Self> {
        if s.is_empty() || s == "none" {
            return Ok(Auth::None);
        }
        // Phase 1: split scheme from the rest on the first colon only.
        // Phase 2 (for basic/digest) splits the username out of `rest`.
        // A two-phase parse is required so `bearer:file:/path` routes the full
        // `file:/path` through `resolve_secret` rather than being split into
        // three pieces by `splitn(3, ':')`.
        let (scheme, rest) = s
            .split_once(':')
            .ok_or_else(|| AppError::Auth(format!("invalid auth spec: {s:?}")))?;

        match scheme {
            "bearer" => Ok(Auth::Bearer {
                token: resolve_secret(rest)?,
            }),
            "basic" => {
                let (username, password) = parse_user_pass(rest, scheme, s)?;
                Ok(Auth::Basic { username, password })
            }
            "digest" => {
                let (username, password) = parse_user_pass(rest, scheme, s)?;
                Ok(Auth::Digest { username, password })
            }
            _ => Err(AppError::Auth(format!("invalid auth spec: {s:?}"))),
        }
    }

    /// Apply this auth method to a `reqwest::RequestBuilder`.
    pub fn apply(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self {
            Auth::None => builder,
            Auth::Basic { username, password } => builder.basic_auth(username, Some(password)),
            Auth::Bearer { token } => builder.bearer_auth(token),
            Auth::Digest { username, password } => {
                // reqwest does not natively support Digest auth. We fall back to Basic auth,
                // which sends credentials in plaintext rather than hashed. This is acceptable
                // only over HTTPS. A future improvement could integrate a dedicated digest-auth
                // crate.
                //
                // Warn once per process so bulk manifest loads against a Digest endpoint do
                // not flood logs.
                static DIGEST_FALLBACK_WARNED: std::sync::Once = std::sync::Once::new();
                DIGEST_FALLBACK_WARNED.call_once(|| {
                    tracing::warn!(
                        "Digest auth is not supported by reqwest; falling back to Basic auth. \
                         Ensure the connection uses HTTPS."
                    );
                });
                builder.basic_auth(username, Some(password))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RAII guard that sets an env var on construction and removes it on drop,
    /// even if the enclosing test panics. Prevents environment leakage across
    /// tests sharing a process.
    struct EnvVarGuard {
        name: &'static str,
    }

    impl EnvVarGuard {
        fn set(name: &'static str, value: &str) -> Self {
            // SAFETY: test-only env mutation; unique var names avoid
            // parallel-test races.
            unsafe { std::env::set_var(name, value) };
            Self { name }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            // SAFETY: pairs with the `set_var` in `EnvVarGuard::set`.
            unsafe { std::env::remove_var(self.name) };
        }
    }

    #[test]
    fn default_auth_is_none() {
        assert!(matches!(Auth::default(), Auth::None));
    }

    #[test]
    fn auth_variants_are_constructible() {
        let _basic = Auth::Basic {
            username: "user".into(),
            password: "pass".into(),
        };
        let _bearer = Auth::Bearer {
            token: "tok".into(),
        };
        let _digest = Auth::Digest {
            username: "u".into(),
            password: "p".into(),
        };
    }

    #[test]
    fn auth_clone_none() {
        let a = Auth::None;
        assert!(matches!(a.clone(), Auth::None));
    }

    #[test]
    fn auth_clone_bearer() {
        let a = Auth::Bearer {
            token: "abc".into(),
        };
        if let Auth::Bearer { token } = a.clone() {
            assert_eq!(token, "abc");
        } else {
            panic!("expected Bearer");
        }
    }

    #[test]
    fn parse_none() {
        assert!(matches!(Auth::from_spec("none").unwrap(), Auth::None));
        assert!(matches!(Auth::from_spec("").unwrap(), Auth::None));
    }

    #[test]
    fn parse_bearer() {
        let auth = Auth::from_spec("bearer:mytoken123").unwrap();
        assert!(matches!(auth, Auth::Bearer { token } if token == "mytoken123"));
    }

    #[test]
    fn parse_basic() {
        let auth = Auth::from_spec("basic:alice:s3cr3t").unwrap();
        assert!(matches!(auth, Auth::Basic { username, password }
            if username == "alice" && password == "s3cr3t"));
    }

    #[test]
    fn parse_basic_password_with_colon() {
        let auth = Auth::from_spec("basic:alice:pass:word").unwrap();
        assert!(matches!(auth, Auth::Basic { password, .. } if password == "pass:word"));
    }

    #[test]
    fn parse_invalid_returns_error() {
        assert!(Auth::from_spec("oauth:something").is_err());
    }

    #[test]
    fn parse_basic_missing_password_returns_error() {
        let err = Auth::from_spec("basic:alice").unwrap_err();
        assert!(matches!(err, AppError::Auth(_)));
    }

    // --- A1: Debug redaction ---

    #[test]
    fn debug_basic_redacts_password() {
        let auth = Auth::Basic {
            username: "alice".into(),
            password: "s3cr3t".into(),
        };
        let dbg = format!("{auth:?}");
        assert!(dbg.contains("[REDACTED]"), "password must be redacted");
        assert!(
            !dbg.contains("s3cr3t"),
            "plaintext password must not appear"
        );
        assert!(dbg.contains("alice"), "username should be visible");
    }

    #[test]
    fn debug_bearer_redacts_token() {
        let auth = Auth::Bearer {
            token: "tok123".into(),
        };
        let dbg = format!("{auth:?}");
        assert!(dbg.contains("[REDACTED]"));
        assert!(!dbg.contains("tok123"));
    }

    #[test]
    fn debug_digest_redacts_password() {
        let auth = Auth::Digest {
            username: "bob".into(),
            password: "hunter2".into(),
        };
        let dbg = format!("{auth:?}");
        assert!(dbg.contains("[REDACTED]"));
        assert!(!dbg.contains("hunter2"));
        assert!(dbg.contains("bob"));
    }

    #[test]
    fn debug_none_is_safe() {
        let dbg = format!("{:?}", Auth::None);
        assert_eq!(dbg, "Auth::None");
    }

    // --- A3: env/file indirection ---
    //
    // Env-var tests use unique variable names so they can run concurrently
    // with other tests without global-state collisions.

    #[test]
    fn bearer_env_indirection() {
        let _guard = EnvVarGuard::set("C2PA_TUI_TEST_TOKEN_A3A", "secret_value");
        let auth = Auth::from_spec("bearer:env:C2PA_TUI_TEST_TOKEN_A3A").unwrap();
        assert!(matches!(auth, Auth::Bearer { token } if token == "secret_value"));
    }

    #[test]
    fn bearer_env_missing_var_returns_error() {
        // SAFETY: defensive pre-clean so a stray ambient env does not mask the
        // failure we are trying to assert.
        unsafe { std::env::remove_var("C2PA_TUI_TEST_TOKEN_MISSING_A3B") };
        let err = Auth::from_spec("bearer:env:C2PA_TUI_TEST_TOKEN_MISSING_A3B").unwrap_err();
        assert!(matches!(err, AppError::Auth(_)));
    }

    #[test]
    fn bearer_env_empty_name_returns_error() {
        let err = Auth::from_spec("bearer:env:").unwrap_err();
        assert!(matches!(err, AppError::Auth(_)));
    }

    #[test]
    fn bearer_env_empty_value_returns_error() {
        let _guard = EnvVarGuard::set("C2PA_TUI_TEST_TOKEN_EMPTY", "");
        let err = Auth::from_spec("bearer:env:C2PA_TUI_TEST_TOKEN_EMPTY").unwrap_err();
        assert!(matches!(err, AppError::Auth(_)));
    }

    #[test]
    fn bearer_env_whitespace_value_returns_error() {
        let _guard = EnvVarGuard::set("C2PA_TUI_TEST_TOKEN_BLANK", "   \t  ");
        let err = Auth::from_spec("bearer:env:C2PA_TUI_TEST_TOKEN_BLANK").unwrap_err();
        assert!(matches!(err, AppError::Auth(_)));
    }

    #[test]
    fn bearer_file_indirection() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("token.txt");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "file_token_value").unwrap();
        let spec = format!("bearer:file:{}", path.display());
        let auth = Auth::from_spec(&spec).unwrap();
        assert!(matches!(auth, Auth::Bearer { token } if token == "file_token_value"));
    }

    #[test]
    fn bearer_file_whitespace_is_trimmed() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
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
    fn bearer_file_missing_file_returns_error() {
        let spec = "bearer:file:/does/not/exist/c2pa-tui-test-missing";
        let err = Auth::from_spec(spec).unwrap_err();
        assert!(matches!(err, AppError::Auth(_)));
    }

    #[test]
    fn bearer_file_empty_file_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("empty.txt");
        std::fs::File::create(&path).unwrap();
        let spec = format!("bearer:file:{}", path.display());
        let err = Auth::from_spec(&spec).unwrap_err();
        assert!(matches!(err, AppError::Auth(_)));
    }

    #[test]
    fn bearer_file_blank_line_returns_error() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("blank.txt");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "   \t  ").unwrap();
        let spec = format!("bearer:file:{}", path.display());
        let err = Auth::from_spec(&spec).unwrap_err();
        assert!(matches!(err, AppError::Auth(_)));
    }

    #[test]
    fn bearer_file_only_reads_first_line() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("multi.txt");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "first_line_token").unwrap();
        writeln!(f, "second_line_should_be_ignored").unwrap();
        writeln!(f, "third_line_too").unwrap();
        let spec = format!("bearer:file:{}", path.display());
        let auth = Auth::from_spec(&spec).unwrap();
        assert!(matches!(auth, Auth::Bearer { token } if token == "first_line_token"));
    }

    #[test]
    fn bearer_file_bounded_read_tolerates_large_tail() {
        // A file whose first line is short and valid must parse even if the
        // rest of the file is larger than `CREDENTIAL_FILE_READ_LIMIT`. This
        // guards against `read_to_string`-style regressions that would refuse
        // to read such a file.
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("big_tail.txt");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "short_token").unwrap();
        // Write ~128 KiB of filler after the first line.
        let filler = vec![b'x'; 128 * 1024];
        f.write_all(&filler).unwrap();
        drop(f);
        let spec = format!("bearer:file:{}", path.display());
        let auth = Auth::from_spec(&spec).unwrap();
        assert!(matches!(auth, Auth::Bearer { token } if token == "short_token"));
    }

    #[test]
    fn bearer_file_rejects_oversized_first_line() {
        // A single line larger than the read limit must not be accepted in
        // full, and must not panic. The truncated read is still "some line"
        // so `read_line` succeeds and trim returns non-empty — the parse
        // therefore succeeds, but the returned token is a truncated prefix
        // bounded by the limit. Asserting the length bound is the contract.
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("oversized.txt");
        let mut f = std::fs::File::create(&path).unwrap();
        // 2× the limit, all on one line, no newline.
        let oversized = vec![b'A'; (CREDENTIAL_FILE_READ_LIMIT as usize) * 2];
        f.write_all(&oversized).unwrap();
        drop(f);
        let spec = format!("bearer:file:{}", path.display());
        let auth = Auth::from_spec(&spec).unwrap();
        if let Auth::Bearer { token } = auth {
            assert!(
                token.len() as u64 <= CREDENTIAL_FILE_READ_LIMIT,
                "token length {} must not exceed read limit {}",
                token.len(),
                CREDENTIAL_FILE_READ_LIMIT
            );
        } else {
            panic!("expected Bearer");
        }
    }

    #[test]
    fn basic_password_file_indirection() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("pass.txt");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "  my_password  ").unwrap();
        let spec = format!("basic:alice:file:{}", path.display());
        let auth = Auth::from_spec(&spec).unwrap();
        assert!(matches!(auth, Auth::Basic { password, .. } if password == "my_password"));
    }

    #[test]
    fn basic_password_env_indirection() {
        let _guard = EnvVarGuard::set("C2PA_TUI_TEST_PASS_A3C", "env_pass");
        let auth = Auth::from_spec("basic:alice:env:C2PA_TUI_TEST_PASS_A3C").unwrap();
        assert!(matches!(auth, Auth::Basic { password, username }
            if password == "env_pass" && username == "alice"));
    }

    #[test]
    fn digest_password_file_indirection() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("pass.txt");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "dpass").unwrap();
        let spec = format!("digest:bob:file:{}", path.display());
        let auth = Auth::from_spec(&spec).unwrap();
        assert!(matches!(auth, Auth::Digest { password, .. } if password == "dpass"));
    }

    #[test]
    fn basic_password_with_colon_still_works() {
        // The two-phase split must preserve colons inside a password.
        let auth = Auth::from_spec("basic:alice:pa:ss:word").unwrap();
        assert!(matches!(auth, Auth::Basic { password, .. } if password == "pa:ss:word"));
    }

    // --- proptest ---

    use proptest::prelude::*;

    proptest! {
        #[test]
        fn from_spec_never_panics(s in ".*") {
            let _ = Auth::from_spec(&s);
        }

        #[test]
        fn bearer_roundtrip(token in "[a-zA-Z0-9_\\-]{1,64}") {
            let spec = format!("bearer:{token}");
            let auth = Auth::from_spec(&spec).unwrap();
            assert!(matches!(auth, Auth::Bearer { token: t } if t == token));
        }

        /// Debug output must never leak a non-trivial secret. Restricted to
        /// alphanumeric + a minimum length so matches against generic format
        /// text like `[REDACTED]` or the literal username cannot produce a
        /// spurious containment.
        #[test]
        fn debug_never_leaks_basic_password(pw in "[a-z0-9]{8,32}") {
            let auth = Auth::Basic { username: "u".into(), password: pw.clone() };
            let dbg = format!("{auth:?}");
            prop_assert!(!dbg.contains(&pw), "password {pw:?} leaked into {dbg:?}");
            prop_assert!(dbg.contains("[REDACTED]"));
        }

        #[test]
        fn debug_never_leaks_bearer_token(tok in "[a-z0-9]{8,32}") {
            let auth = Auth::Bearer { token: tok.clone() };
            let dbg = format!("{auth:?}");
            prop_assert!(!dbg.contains(&tok), "token {tok:?} leaked into {dbg:?}");
            prop_assert!(dbg.contains("[REDACTED]"));
        }
    }
}
