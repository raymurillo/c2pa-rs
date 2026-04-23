use crate::error::{AppError, Result};

/// HTTP authentication method to apply to remote manifest requests.
#[derive(Debug, Clone, Default)]
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

impl Auth {
    /// Parse an auth specification string into an `Auth` variant.
    ///
    /// Format: `scheme:arg1:arg2`. Supported schemes: `none`, `bearer`, `basic`, `digest`.
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
        let parts: Vec<&str> = s.splitn(3, ':').collect();
        match parts.as_slice() {
            ["bearer", token] => Ok(Auth::Bearer {
                token: token.to_string(),
            }),
            ["basic", user, pass] => Ok(Auth::Basic {
                username: user.to_string(),
                password: pass.to_string(),
            }),
            ["digest", user, pass] => Ok(Auth::Digest {
                username: user.to_string(),
                password: pass.to_string(),
            }),
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
                tracing::warn!(
                    "Digest auth is not supported by reqwest; falling back to Basic auth. \
                     Ensure the connection uses HTTPS."
                );
                builder.basic_auth(username, Some(password))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }
}
