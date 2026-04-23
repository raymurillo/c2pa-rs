use crate::error::Result;

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
    /// Stub: not yet implemented. Implemented in spec-02.
    pub fn from_spec(s: &str) -> Result<Self> {
        let _ = s;
        todo!("spec-02: implement Auth::from_spec")
    }

    /// Apply this auth method to a `reqwest::RequestBuilder`.
    ///
    /// Stub: not yet implemented. Implemented in spec-02.
    pub fn apply(&self, _builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        todo!("spec-02: implement Auth::apply")
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
}
