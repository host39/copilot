//! A reversible stand-in for the AES-256-GCM cipher.

use std::sync::atomic::{AtomicU8, Ordering};

use crate::ports::secret_cipher::{SealContext, SecretCipher, unreadable};
use crate::shared::errors::DomainResult;
use crate::shared::secret::Secret;

/// Seals as `key id || aad length (u32 BE) || aad || plain`. It is not encryption; it keeps the
/// real cipher's observable rules: a value opens only with the same key id and the same
/// [`SealContext`]. [`FakeCipher::rotate`] simulates a `SECRET_KEY` change.
#[derive(Debug, Default)]
pub struct FakeCipher {
    key_id: AtomicU8,
}

impl FakeCipher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Changes the key, so every value sealed before no longer opens.
    pub fn rotate(&self) {
        self.key_id.fetch_add(1, Ordering::SeqCst);
    }
}

impl SecretCipher for FakeCipher {
    fn seal(&self, plain: &[u8], aad: &SealContext) -> DomainResult<Vec<u8>> {
        let aad = aad.aad();
        let length = u32::try_from(aad.len()).expect("aad fits in u32");
        let mut sealed = vec![self.key_id.load(Ordering::SeqCst)];
        sealed.extend_from_slice(&length.to_be_bytes());
        sealed.extend_from_slice(&aad);
        sealed.extend_from_slice(plain);
        Ok(sealed)
    }

    fn open(&self, sealed: &[u8], aad: &SealContext) -> DomainResult<Secret> {
        let (&key_id, rest) = sealed.split_first().ok_or_else(unreadable)?;
        if key_id != self.key_id.load(Ordering::SeqCst) || rest.len() < 4 {
            return Err(unreadable());
        }
        let (length, rest) = rest.split_at(4);
        let length = u32::from_be_bytes(length.try_into().expect("four bytes")) as usize;
        if rest.len() < length || rest[..length] != aad.aad()[..] {
            return Err(unreadable());
        }
        Ok(Secret::new(&rest[length..]))
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::ports::secret_cipher::SECRET_UNREADABLE;
    use crate::shared::ids::UserId;

    fn context(row: Uuid) -> SealContext {
        SealContext {
            table: "providers",
            column: "api_key_sealed",
            row_id: row,
            user_id: UserId::new(Uuid::nil()),
        }
    }

    #[test]
    fn opens_only_with_the_same_context_and_key() {
        let cipher = FakeCipher::new();
        let row = Uuid::now_v7();
        let sealed = cipher.seal(b"sk-1", &context(row)).unwrap();
        assert_eq!(
            cipher.open(&sealed, &context(row)).unwrap().expose(),
            b"sk-1"
        );

        let moved = cipher.open(&sealed, &context(Uuid::now_v7())).unwrap_err();
        assert_eq!(moved.code(), SECRET_UNREADABLE);

        cipher.rotate();
        let rotated = cipher.open(&sealed, &context(row)).unwrap_err();
        assert_eq!(rotated.code(), SECRET_UNREADABLE);
        assert!(cipher.open(&[], &context(row)).is_err());
    }
}
