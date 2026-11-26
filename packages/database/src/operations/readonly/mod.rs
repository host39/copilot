mod constants;
mod grant;

pub use crate::operations::readonly::constants::{READONLY_ROLE, READONLY_STATEMENT_TIMEOUT};
pub use crate::operations::readonly::grant::{GrantError, grant, select_grant, statements};
