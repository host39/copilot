use crate::shared::constants::{DEFAULT_PAGE_OFFSET, DEFAULT_PAGE_SIZE};

/// A slice of items plus the total count. Use in services that return pages.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaginatedResult<T> {
    pub items: Vec<T>,
    pub total: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaginationOptions {
    pub limit: u32,
    pub offset: u32,
}

impl Default for PaginationOptions {
    fn default() -> Self {
        Self {
            limit: DEFAULT_PAGE_SIZE,
            offset: DEFAULT_PAGE_OFFSET,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SortDirection {
    #[default]
    Asc,
    Desc,
}

/// Sort by `field`, typically a domain-specific column enum.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SortOptions<Field> {
    pub field: Field,
    pub direction: SortDirection,
}
