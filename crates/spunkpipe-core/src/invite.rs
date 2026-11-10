use crate::{Error, PublicKey, Result};
use std::fmt;

/// `spunkpipe://host[:port][/path]#publickey` (HTTPS) or `spunkpipe+http://...` (plain HTTP).
///
/// The invite is the trust root: it carries the publisher's public key, so it must be passed
/// to friends over a channel the server's web host does not control.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invite {
    pub secure: bool,
    pub host_path: String,
    pub public_key: PublicKey,
}

impl Invite {
    pub fn new(secure: bool, host_path: &str, public_key: PublicKey) -> Self {
        Invite { secure, host_path: host_path.trim_end_matches('/').to_string(), public_key }
    }

    pub fn parse(s: &str) -> Result<Self> {
        let bad = |m: &str| Error::BadInvite(m.to_string());
        let s = s.trim();
        let (secure, rest) = if let Some(r) = s.strip_prefix("spunkpipe://") {
            (true, r)
        } else if let Some(r) = s.strip_prefix("spunkpipe+http://") {
            (false, r)
        } else {
            return Err(bad("must start with spunkpipe://"));
        };
        let (host_path, key) = rest.split_once('#').ok_or_else(|| bad("missing #publickey"))?;
        let host_path = host_path.trim_end_matches('/');
        if host_path.is_empty() || host_path.chars().any(|c| c.is_whitespace() || c == '?' || c == '@') {
            return Err(bad("bad host"));
        }
        Ok(Invite { secure, host_path: host_path.to_string(), public_key: PublicKey::from_base64(key)? })
    }

    /// Base URL with a trailing slash.
    pub fn base_url(&self) -> String {
        format!("{}://{}/", if self.secure { "https" } else { "http" }, self.host_path)
    }
}

impl fmt::Display for Invite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scheme = if self.secure { "spunkpipe" } else { "spunkpipe+http" };
        write!(f, "{scheme}://{}#{}", self.host_path, self.public_key.to_base64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SecretKey;

    #[test]
    fn parse_and_print() {
        let pk = SecretKey::generate().public();
        let inv = Invite::new(true, "mods.example.com/", pk.clone());
        let s = inv.to_string();
        assert!(s.starts_with("spunkpipe://mods.example.com#"));
        let back = Invite::parse(&s).unwrap();
        assert_eq!(back, inv);
        assert_eq!(back.base_url(), "https://mods.example.com/");
        let http = Invite::parse(&format!("spunkpipe+http://h:8080/p#{}", pk.to_base64())).unwrap();
        assert_eq!(http.base_url(), "http://h:8080/p/");
    }

    #[test]
    fn rejects_garbage() {
        for s in ["https://x#abc", "spunkpipe://host", "spunkpipe://#abc", "spunkpipe://h#short", "spunkpipe://u@h#x"] {
            assert!(Invite::parse(s).is_err(), "{s}");
        }
    }
}
