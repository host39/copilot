//! The project file capacity bar.

use std::cmp::Reverse;

use crate::domains::file::entities::StoredFile;
use crate::shared::constants::CONTEXT_WARN_RATIO;

/// Included project files against the default model's context window. Uploads past
/// `CONTEXT_WARN_RATIO` (80%) warn; past 100%, sending is blocked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Capacity {
    /// Summed token estimates of the included files.
    pub used: u64,
    /// The default model's context window; `None` when there is no default model or its window
    /// is unknown.
    pub window: Option<u32>,
}

/// Where the bar stands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapacityLevel {
    /// No context window to compare with.
    Unknown,
    Fits,
    /// Past 80%.
    Warn,
    /// Past 100%: sending is blocked.
    Over,
}

impl Capacity {
    pub fn measure(files: &[StoredFile], window: Option<u32>) -> Self {
        let used = files
            .iter()
            .filter(|file| file.included())
            .map(|file| u64::from(file.token_estimate()))
            .sum();
        Self { used, window }
    }

    /// `used / window`, when the window is known and not zero.
    pub fn ratio(&self) -> Option<f64> {
        self.window
            .filter(|window| *window > 0)
            .map(|window| self.used as f64 / f64::from(window))
    }

    pub fn level(&self) -> CapacityLevel {
        match self.ratio() {
            None => CapacityLevel::Unknown,
            Some(ratio) if ratio > 1.0 => CapacityLevel::Over,
            Some(ratio) if ratio > CONTEXT_WARN_RATIO => CapacityLevel::Warn,
            Some(_) => CapacityLevel::Fits,
        }
    }
}

/// The `limit` largest included files by token estimate, largest first (for the message that
/// blocks sending).
pub fn largest(files: &[StoredFile], limit: usize) -> Vec<&StoredFile> {
    let mut included: Vec<&StoredFile> = files.iter().filter(|file| file.included()).collect();
    included.sort_by_key(|file| Reverse(file.token_estimate()));
    included.truncate(limit);
    included
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use uuid::Uuid;

    use super::*;
    use crate::domains::file::entities::{FileOwner, Measured};
    use crate::domains::file::media::MediaType;
    use crate::shared::ids::{ProjectId, UserId};

    fn file(name: &str, tokens: u32, included: bool) -> StoredFile {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let measured = Measured {
            media_type: MediaType::Text,
            size_bytes: u64::from(tokens) * 4,
            sha256: [0; 32],
            token_estimate: tokens,
            page_count: None,
        };
        StoredFile::create(
            UserId::new(Uuid::now_v7()),
            FileOwner::Project(ProjectId::generate()),
            name,
            measured,
            now,
        )
        .unwrap()
        .with_included(included)
        .unwrap()
    }

    #[test]
    fn sums_included_files_only() {
        let files = [file("a.txt", 100, true), file("b.txt", 900, false)];
        assert_eq!(Capacity::measure(&files, Some(1_000)).used, 100);
    }

    #[test]
    fn warns_past_80_percent_and_blocks_past_100() {
        let level =
            |used: u32| Capacity::measure(&[file("a.txt", used, true)], Some(1_000)).level();
        assert_eq!(level(800), CapacityLevel::Fits);
        assert_eq!(level(801), CapacityLevel::Warn);
        assert_eq!(level(1_000), CapacityLevel::Warn);
        assert_eq!(level(1_001), CapacityLevel::Over);
        assert_eq!(Capacity::measure(&[], None).level(), CapacityLevel::Unknown);
        assert_eq!(
            Capacity::measure(&[], Some(0)).level(),
            CapacityLevel::Unknown
        );
    }

    #[test]
    fn names_the_largest_included_files() {
        let files = [
            file("small.txt", 10, true),
            file("huge.txt", 9_000, false),
            file("big.txt", 5_000, true),
            file("mid.txt", 500, true),
        ];
        let names: Vec<_> = largest(&files, 2)
            .into_iter()
            .map(StoredFile::name)
            .collect();
        assert_eq!(names, ["big.txt", "mid.txt"]);
    }
}
