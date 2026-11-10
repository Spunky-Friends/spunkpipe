use crate::{Error, PublicKey, Result};
use std::fmt;

/// `spunkpipe://host[:port][/path][?game=id]#publickey` (HTTPS) or `spunkpipe+http://...` (plain HTTP).
///
/// The invite is the trust root: it carries the publisher's public key, so it must be passed
/// to friends over a channel the server's web host does not control.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invite {
    pub secure: bool,
    pub host_path: String,
    /// Which game profile this invite is for. Clients default to 7 Days to Die when absent.
    pub game: Option<String>,
    pub public_key: PublicKey,
}

impl Invite {
    pub fn new(secure: bool, host_path: &str, public_key: PublicKey) -> Self {
        Invite { secure, host_path: host_path.trim_end_matches('/').to_string(), game: None, public_key }
    }

    pub fn with_game(mut self, game: &str) -> Self {
        self.game = Some(game.to_string());
        self
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
        let (host_path, query) = match host_path.split_once('?') {
            Some((h, q)) => (h, Some(q)),
            None => (host_path, None),
        };
        let host_path = host_path.trim_end_matches('/');
        if host_path.is_empty() || host_path.chars().any(|c| c.is_whitespace() || c == '?' || c == '@') {
            return Err(bad("bad host"));
        }
        let game = match query {
            None => None,
            Some(q) => {
                let g = q.strip_prefix("game=").ok_or_else(|| bad("unknown option after ?"))?;
                if g.is_empty() || g.len() > 32 || !g.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                    return Err(bad("bad game id"));
                }
                Some(g.to_string())
            }
        };
        Ok(Invite { secure, host_path: host_path.to_string(), game, public_key: PublicKey::from_base64(key)? })
    }

    /// Base URL with a trailing slash.
    pub fn base_url(&self) -> String {
        format!("{}://{}/", if self.secure { "https" } else { "http" }, self.host_path)
    }
}

impl fmt::Display for Invite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scheme = if self.secure { "spunkpipe" } else { "spunkpipe+http" };
        let game = self.game.as_ref().map(|g| format!("?game={g}")).unwrap_or_default();
        write!(f, "{scheme}://{}{game}#{}", self.host_path, self.public_key.to_base64())
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

        let g = Invite::new(true, "h", pk.clone()).with_game("7dtd");
        assert_eq!(g.to_string(), format!("spunkpipe://h?game=7dtd#{}", pk.to_base64()));
        assert_eq!(Invite::parse(&g.to_string()).unwrap().game.as_deref(), Some("7dtd"));
        assert_eq!(Invite::parse(&g.to_string()).unwrap().base_url(), "https://h/");
    }

    #[test]
    fn rejects_garbage() {
        for s in ["https://x#abc", "spunkpipe://host", "spunkpipe://#abc", "spunkpipe://h#short", "spunkpipe://u@h#x", "spunkpipe://h?foo=1#abc", "spunkpipe://h?game=a/b#abc"] {
            assert!(Invite::parse(s).is_err(), "{s}");
        }
    }
}
