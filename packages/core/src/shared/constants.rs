/// Default pagination. A request may override limit and offset; these apply when it does not.
pub const DEFAULT_PAGE_SIZE: u32 = 25;
pub const DEFAULT_PAGE_OFFSET: u32 = 0;

/// Common durations.
pub mod time {
    use std::time::Duration;

    pub const SECOND: Duration = Duration::from_secs(1);
    pub const MINUTE: Duration = Duration::from_secs(60);
    pub const HOUR: Duration = Duration::from_secs(60 * 60);
    pub const DAY: Duration = Duration::from_secs(24 * 60 * 60);
    pub const WEEK: Duration = Duration::from_secs(7 * 24 * 60 * 60);
}
