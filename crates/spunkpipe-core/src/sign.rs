use crate::{Error, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use std::path::Path;

/// The publisher's private key. Never printed or logged.
pub struct SecretKey(SigningKey);

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PublicKey(VerifyingKey);

impl SecretKey {
    pub fn generate() -> Self {
        SecretKey(SigningKey::generate(&mut rand::rngs::OsRng))
    }

    pub fn to_base64(&self) -> String {
        B64.encode(self.0.to_bytes())
    }

    pub fn from_base64(s: &str) -> Result<Self> {
        let bytes = B64
            .decode(s.trim())
            .map_err(|e| Error::BadKey(e.to_string()))?;
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| Error::BadKey("secret key must be 32 bytes".into()))?;
        Ok(SecretKey(SigningKey::from_bytes(&arr)))
    }

    pub fn load(path: &Path) -> Result<Self> {
        Self::from_base64(&std::fs::read_to_string(path)?)
    }

    /// Writes the key readable by the owner only. Fails if the file already exists.
    pub fn save_new(&self, path: &Path) -> Result<()> {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(path)?;
        f.write_all(self.to_base64().as_bytes())?;
        f.write_all(b"\n")?;
        Ok(())
    }

    pub fn public(&self) -> PublicKey {
        PublicKey(self.0.verifying_key())
    }

    /// Signs `msg`, returning the signature as URL-safe base64.
    pub fn sign(&self, msg: &[u8]) -> String {
        B64.encode(self.0.sign(msg).to_bytes())
    }
}

impl PublicKey {
    pub fn to_base64(&self) -> String {
        B64.encode(self.0.to_bytes())
    }

    pub fn from_base64(s: &str) -> Result<Self> {
        let bytes = B64
            .decode(s.trim())
            .map_err(|e| Error::BadKey(e.to_string()))?;
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| Error::BadKey("public key must be 32 bytes".into()))?;
        VerifyingKey::from_bytes(&arr)
            .map(PublicKey)
            .map_err(|e| Error::BadKey(e.to_string()))
    }

    /// A short, stable summary of the key for people to compare out of band, like
    /// `a1b2-c3d4-e5f6-0718`. Show it wherever a key is about to be trusted.
    pub fn fingerprint(&self) -> String {
        let h = blake3::hash(&self.0.to_bytes());
        let hex = hex::encode(&h.as_bytes()[..8]);
        hex.as_bytes().chunks(4).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join("-")
    }

    pub fn verify(&self, msg: &[u8], sig_b64: &str) -> Result<()> {
        let bytes = B64.decode(sig_b64.trim()).map_err(|_| Error::BadSignature)?;
        let arr: [u8; 64] = bytes.try_into().map_err(|_| Error::BadSignature)?;
        self.0
            .verify(msg, &Signature::from_bytes(&arr))
            .map_err(|_| Error::BadSignature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify() {
        let sk = SecretKey::generate();
        let sig = sk.sign(b"manifest");
        sk.public().verify(b"manifest", &sig).unwrap();
        assert!(sk.public().verify(b"tampered", &sig).is_err());
        assert!(SecretKey::generate().public().verify(b"manifest", &sig).is_err());
        assert!(sk.public().verify(b"manifest", "not base64!").is_err());
    }

    #[test]
    fn fingerprint_is_stable_and_distinguishes_keys() {
        let a = SecretKey::generate().public();
        let b = SecretKey::generate().public();
        assert_eq!(a.fingerprint(), PublicKey::from_base64(&a.to_base64()).unwrap().fingerprint());
        assert_ne!(a.fingerprint(), b.fingerprint());
        let f = a.fingerprint();
        assert_eq!(f.len(), 19);
        assert!(f.split('-').all(|g| g.len() == 4 && g.chars().all(|c| c.is_ascii_hexdigit())));
    }

    #[test]
    fn keys_round_trip() {
        let sk = SecretKey::generate();
        let sk2 = SecretKey::from_base64(&sk.to_base64()).unwrap();
        assert_eq!(sk.public(), sk2.public());
        assert_eq!(PublicKey::from_base64(&sk.public().to_base64()).unwrap(), sk.public());
        assert!(PublicKey::from_base64("abc").is_err());
    }

    #[test]
    fn key_file_is_private_and_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("k.key");
        let sk = SecretKey::generate();
        sk.save_new(&p).unwrap();
        assert!(sk.save_new(&p).is_err());
        assert_eq!(SecretKey::load(&p).unwrap().public(), sk.public());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }
}
