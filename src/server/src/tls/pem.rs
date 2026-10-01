//! Operator-supplied certificate: PEM loading, validation and change detection.
//!
//! Validation: a non-empty chain, a key that loads under the ring provider and matches the leaf,
//! and a leaf currently inside its validity window. A failure is reported to the caller, which
//! degrades to the self-signed certificate (a bad PEM never aborts boot). Reload is mtime
//! polling (portable; no filesystem-notify dependency and no signal) driven by
//! `TlsRuntime::maintenance_tick`.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

use super::resolver::LoadedCert;
use super::TlsError;

/// The configured certificate and key file locations.
///
/// # Examples
///
/// ```
/// use shadowcat::tls::pem::PemPaths;
///
/// let p = PemPaths { cert: "chain.pem".into(), key: "key.pem".into() };
/// assert_eq!(p.cert.to_str(), Some("chain.pem"));
/// ```
#[derive(Debug, Clone)]
pub struct PemPaths {
    /// Certificate chain, leaf first.
    pub cert: PathBuf,
    /// Private key.
    pub key: PathBuf,
}

/// The modification stamps of a [`PemPaths`] pair, for change detection.
///
/// # Examples
///
/// ```
/// use shadowcat::tls::pem::PemStamp;
///
/// assert_eq!(PemStamp::default(), PemStamp::default());
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PemStamp {
    /// Certificate file mtime, `None` when unreadable.
    cert: Option<SystemTime>,
    /// Key file mtime, `None` when unreadable.
    key: Option<SystemTime>,
}

impl PemPaths {
    /// The current modification stamps.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::tls::pem::PemPaths;
    ///
    /// let p = PemPaths { cert: "no-such-cert.pem".into(), key: "no-such-key.pem".into() };
    /// assert_eq!(p.stamp(), Default::default());
    /// ```
    pub fn stamp(&self) -> PemStamp {
        let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        PemStamp {
            cert: mtime(&self.cert),
            key: mtime(&self.key),
        }
    }

    /// Loads and validates the pair as of `now_unix` (seconds). Expired and not-yet-valid
    /// certificates are rejected: an expired operator certificate is worse than the self-signed
    /// fallback.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::tls::pem::PemPaths;
    /// use shadowcat::tls::selfsigned::{generate, SanSet};
    ///
    /// let dir = tempfile::tempdir().unwrap();
    /// let now = time::OffsetDateTime::now_utc();
    /// let g = generate(&SanSet { dns: vec!["game.example".into()], ips: vec![] }, now).unwrap();
    /// std::fs::write(dir.path().join("c.pem"), &g.cert_pem).unwrap();
    /// std::fs::write(dir.path().join("k.pem"), &g.key_pem).unwrap();
    /// let paths = PemPaths { cert: dir.path().join("c.pem"), key: dir.path().join("k.pem") };
    /// let cert = paths.load(now.unix_timestamp()).unwrap();
    /// assert!(cert.covers_name("game.example"));
    /// ```
    pub fn load(&self, now_unix: i64) -> Result<LoadedCert, TlsError> {
        let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(&self.cert)
            .map_err(|e| TlsError::Pem(format!("{}: {e}", self.cert.display())))?
            .collect::<Result<_, _>>()
            .map_err(|e| TlsError::Pem(format!("{}: {e}", self.cert.display())))?;
        if chain.is_empty() {
            return Err(TlsError::Pem(format!(
                "{}: no certificate found",
                self.cert.display()
            )));
        }
        let key = PrivateKeyDer::from_pem_file(&self.key)
            .map_err(|e| TlsError::Pem(format!("{}: {e}", self.key.display())))?;
        let loaded = LoadedCert::from_der(chain, key)?;
        if !loaded.is_valid_at(now_unix) {
            return Err(TlsError::Expired(format!(
                "{} is outside its validity window",
                self.cert.display()
            )));
        }
        Ok(loaded)
    }
}
