//! Secret sealing port.
//!
//! `copilot-security` implements [`SecretCipher`] with AES-256-GCM (sealed format: version byte,
//! key id byte, 12-byte nonce, ciphertext). Core decides what is sealed and binds every sealed
//! value to its row through [`SealContext`], so a value copied into another row does not open.

use uuid::Uuid;

use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::UserId;
use crate::shared::secret::Secret;

/// Error code when a sealed value no longer opens (the key changed, or the value was moved).
pub const SECRET_UNREADABLE: &str = "SECRET_UNREADABLE";

/// Seals and opens secrets at rest.
pub trait SecretCipher: Send + Sync {
    fn seal(&self, plain: &[u8], aad: &SealContext) -> DomainResult<Vec<u8>>;

    /// Fails with [`unreadable`] (`SECRET_UNREADABLE`) when the value does not open, for example
    /// after a `SECRET_KEY` change. Callers show "Key must be re-entered" instead of a 500.
    fn open(&self, sealed: &[u8], aad: &SealContext) -> DomainResult<Secret>;
}

/// Where a sealed value lives. It is the associated data of the seal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SealContext {
    pub table: &'static str,
    pub column: &'static str,
    pub row_id: Uuid,
    pub user_id: UserId,
}

impl SealContext {
    /// The associated data: `<table>.<column>:<row id>:<user_id>`.
    pub fn aad(&self) -> Vec<u8> {
        format!(
            "{}.{}:{}:{}",
            self.table, self.column, self.row_id, self.user_id
        )
        .into_bytes()
    }
}

/// The error for a sealed value that does not open.
pub fn unreadable() -> DomainError {
    DomainError::business_rule("Key must be re-entered.", SECRET_UNREADABLE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds_the_table_column_row_and_user() {
        let row_id = Uuid::now_v7();
        let user_id = UserId::new(Uuid::now_v7());
        let context = SealContext {
            table: "providers",
            column: "api_key_sealed",
            row_id,
            user_id,
        };
        assert_eq!(
            String::from_utf8(context.aad()).expect("ascii"),
            format!("providers.api_key_sealed:{row_id}:{user_id}")
        );
    }

    #[test]
    fn reports_unreadable_secrets_as_a_business_rule() {
        let error = unreadable();
        assert_eq!(error.code(), SECRET_UNREADABLE);
        assert_eq!(error.to_string(), "Key must be re-entered.");
    }
}
