//! AES-256-GCM sealing of stored secrets (provider keys, tool server headers).
//!
//! Sealed format (`docs/design/copilot.md`, Secrets): version byte ([`FORMAT_VERSION`]) || key id
//! (1 byte) || 12-byte nonce || AES-256-GCM ciphertext with its 16-byte tag. The associated data
//! is [`SealContext::aad`] (`<table>.<column>:<row id>:<user_id>`), so a value copied into another
//! row, column, or account does not open.
//!
//! The key id is the first byte of `SHA-256("copilot.secret-key.id" || key)`. It tells a later
//! keyring (`SECRET_KEY_PREVIOUS` plus a reseal binary) which key sealed a value without a
//! migration, and lets [`AesCipher::open`] report a changed `SECRET_KEY` before trying to decrypt.

use std::fmt;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use copilot_core::{DomainError, DomainResult, SealContext, Secret, SecretCipher, unreadable};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

/// The sealed format this build writes and reads.
pub const FORMAT_VERSION: u8 = 1;
/// Bytes of `SECRET_KEY` once decoded.
pub const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
/// Version byte, key id byte, nonce.
const HEADER_LEN: usize = 2 + NONCE_LEN;
/// Domain separation for the key id hash.
const KEY_ID_LABEL: &[u8] = b"copilot.secret-key.id";

/// Why `SECRET_KEY` was rejected. The detail never includes the value.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum KeyError {
    #[error("security.secret-key.encoding")]
    Encoding,
    #[error("security.secret-key.length {0}")]
    Length(usize),
}

/// The decoded `SECRET_KEY`: the AES-256 key schedule (zeroized on drop) and its key id.
#[derive(Clone)]
pub struct SecretKey {
    cipher: Aes256Gcm,
    id: u8,
}

impl SecretKey {
    /// Parses `SECRET_KEY`: standard base64 (`openssl rand -base64 32`) of exactly 32 bytes.
    /// Surrounding whitespace is ignored.
    pub fn parse(value: &str) -> Result<Self, KeyError> {
        let bytes = Zeroizing::new(
            STANDARD
                .decode(value.trim())
                .map_err(|_| KeyError::Encoding)?,
        );
        let key: &[u8; KEY_LEN] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| KeyError::Length(bytes.len()))?;
        Ok(Self::new(key))
    }

    pub fn new(key: &[u8; KEY_LEN]) -> Self {
        let cipher = Aes256Gcm::new_from_slice(key).expect("a 32-byte key is an AES-256 key");
        let digest = Sha256::new()
            .chain_update(KEY_ID_LABEL)
            .chain_update(key)
            .finalize();
        Self {
            cipher,
            id: digest[0],
        }
    }

    /// The byte that marks values sealed with this key.
    pub fn id(&self) -> u8 {
        self.id
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretKey")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// Implements [`SecretCipher`] with AES-256-GCM under `SECRET_KEY`.
#[derive(Clone, Debug)]
pub struct AesCipher {
    key: SecretKey,
}

impl AesCipher {
    pub fn new(key: SecretKey) -> Self {
        Self { key }
    }
}

impl SecretCipher for AesCipher {
    fn seal(&self, plain: &[u8], context: &SealContext) -> DomainResult<Vec<u8>> {
        let mut nonce = [0_u8; NONCE_LEN];
        getrandom::fill(&mut nonce)
            .map_err(|error| DomainError::internal("security.seal.random.failed", error))?;
        let aad = context.aad();
        let body = self
            .key
            .cipher
            .encrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: plain,
                    aad: &aad,
                },
            )
            .map_err(|error| DomainError::internal("security.seal.failed", error))?;
        let mut sealed = Vec::with_capacity(HEADER_LEN + body.len());
        sealed.push(FORMAT_VERSION);
        sealed.push(self.key.id);
        sealed.extend_from_slice(&nonce);
        sealed.extend_from_slice(&body);
        Ok(sealed)
    }

    fn open(&self, sealed: &[u8], context: &SealContext) -> DomainResult<Secret> {
        let refuse = |reason: &'static str| {
            tracing::warn!(
                reason,
                table = context.table,
                column = context.column,
                "security.seal.open.refused"
            );
            unreadable()
        };
        if sealed.len() < HEADER_LEN + TAG_LEN {
            return Err(refuse("length"));
        }
        if sealed[0] != FORMAT_VERSION {
            return Err(refuse("version"));
        }
        if sealed[1] != self.key.id {
            return Err(refuse("key-id"));
        }
        let nonce: [u8; NONCE_LEN] = sealed[2..HEADER_LEN]
            .try_into()
            .expect("the header holds a 12-byte nonce");
        let aad = context.aad();
        let plain = self
            .key
            .cipher
            .decrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: &sealed[HEADER_LEN..],
                    aad: &aad,
                },
            )
            .map_err(|_| refuse("authentication"))?;
        Ok(Secret::new(plain))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use copilot_core::{SECRET_UNREADABLE, UserId};
    use uuid::Uuid;

    const KEY: [u8; KEY_LEN] = [7; KEY_LEN];
    const OTHER_KEY: [u8; KEY_LEN] = [9; KEY_LEN];

    fn cipher(key: &[u8; KEY_LEN]) -> AesCipher {
        AesCipher::new(SecretKey::new(key))
    }

    fn context() -> SealContext {
        SealContext {
            table: "providers",
            column: "api_key_sealed",
            row_id: Uuid::from_u128(1),
            user_id: UserId::new(Uuid::from_u128(2)),
        }
    }

    fn refused(result: DomainResult<Secret>) {
        let error = result.expect_err("the value must not open");
        assert_eq!(error.code(), SECRET_UNREADABLE);
        assert_eq!(error.to_string(), "Key must be re-entered.");
    }

    #[test]
    fn seals_and_opens() {
        let cipher = cipher(&KEY);
        for plain in [&b"sk-live-123"[..], b"", &[0, 255, 1, 254]] {
            let sealed = cipher.seal(plain, &context()).unwrap();
            assert_eq!(cipher.open(&sealed, &context()).unwrap().expose(), plain);
        }
    }

    #[test]
    fn writes_the_versioned_format() {
        let key = SecretKey::new(&KEY);
        let key_id = key.id();
        let cipher = AesCipher::new(key);
        let plain = b"sk-live-123";
        let sealed = cipher.seal(plain, &context()).unwrap();
        assert_eq!(sealed.len(), 1 + 1 + 12 + plain.len() + 16);
        assert_eq!(sealed[0], FORMAT_VERSION);
        assert_eq!(sealed[1], key_id);
        assert!(
            !sealed.windows(plain.len()).any(|window| window == plain),
            "the plaintext never appears in the sealed value"
        );
    }

    #[test]
    fn uses_a_fresh_nonce_for_every_seal() {
        let cipher = cipher(&KEY);
        let first = cipher.seal(b"same", &context()).unwrap();
        let second = cipher.seal(b"same", &context()).unwrap();
        assert_ne!(first[2..HEADER_LEN], second[2..HEADER_LEN]);
        assert_ne!(first, second);
    }

    #[test]
    fn opens_values_sealed_by_earlier_builds() {
        // Sealed outside this crate (Python `cryptography`, AESGCM) with KEY, context(), and the
        // nonce 01..0c. A change to the format, the key id, or the AAD would make every stored
        // secret unreadable, so this value must keep opening.
        let sealed = hex(STORED);
        assert_eq!(
            cipher(&KEY).open(&sealed, &context()).unwrap().expose(),
            b"sk-live-123"
        );
    }

    const STORED: &str =
        "01350102030405060708090a0b0cc817c5f2039c8701d40c316ffdd5193e61c4cbe4992aee1dd6e15c";

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&text[index..index + 2], 16).expect("hex"))
            .collect()
    }

    #[test]
    fn refuses_another_key() {
        let sealed = cipher(&KEY).seal(b"sk-1", &context()).unwrap();
        let other = cipher(&OTHER_KEY);
        refused(other.open(&sealed, &context()));

        // Same key id byte (a 1-in-256 collision): the authentication tag still fails.
        let mut relabeled = sealed.clone();
        relabeled[1] = SecretKey::new(&OTHER_KEY).id();
        refused(other.open(&relabeled, &context()));
    }

    #[test]
    fn refuses_tampered_values() {
        let cipher = cipher(&KEY);
        let sealed = cipher.seal(b"sk-live-123", &context()).unwrap();
        for index in 0..sealed.len() {
            let mut tampered = sealed.clone();
            tampered[index] ^= 0x01;
            refused(cipher.open(&tampered, &context()));
        }
        for length in 0..sealed.len() {
            refused(cipher.open(&sealed[..length], &context()));
        }
        let mut longer = sealed.clone();
        longer.push(0);
        refused(cipher.open(&longer, &context()));
    }

    #[test]
    fn refuses_another_row_column_table_or_user() {
        let cipher = cipher(&KEY);
        let sealed = cipher.seal(b"sk-1", &context()).unwrap();
        let others = [
            SealContext {
                row_id: Uuid::from_u128(3),
                ..context()
            },
            SealContext {
                user_id: UserId::new(Uuid::from_u128(3)),
                ..context()
            },
            SealContext {
                column: "headers_sealed",
                ..context()
            },
            SealContext {
                table: "tool_servers",
                ..context()
            },
        ];
        for other in others {
            refused(cipher.open(&sealed, &other));
        }
    }

    #[test]
    fn parses_the_secret_key() {
        let encoded = STANDARD.encode(KEY);
        let key = SecretKey::parse(&format!(" {encoded}\n")).unwrap();
        assert_eq!(key.id(), SecretKey::new(&KEY).id());

        for length in [0, 16, 31, 33, 64] {
            let encoded = STANDARD.encode(vec![1_u8; length]);
            assert_eq!(
                SecretKey::parse(&encoded).unwrap_err(),
                KeyError::Length(length)
            );
        }
        for invalid in ["not base64!", "-_-_", "QUJD=X"] {
            assert_eq!(SecretKey::parse(invalid).unwrap_err(), KeyError::Encoding);
        }
        assert_eq!(
            KeyError::Length(16).to_string(),
            "security.secret-key.length 16"
        );
    }

    #[test]
    fn derives_a_stable_key_id_per_key() {
        assert_eq!(SecretKey::new(&KEY).id(), SecretKey::new(&KEY).id());
        assert_ne!(SecretKey::new(&KEY).id(), SecretKey::new(&OTHER_KEY).id());
    }

    #[test]
    fn never_prints_the_key() {
        let key = SecretKey::new(&KEY);
        let printed = format!("{:?} {:?}", key, AesCipher::new(key.clone()));
        assert!(printed.starts_with("SecretKey { id: "), "{printed}");
        assert!(!printed.contains("7, 7"), "{printed}");
    }
}
