//! Reasoning levels.

use std::fmt;
use std::str::FromStr;

/// How hard a reasoning model thinks. `None` where a level is optional means the model default.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ReasoningLevel {
    Off,
    Low,
    Medium,
    High,
}

impl ReasoningLevel {
    /// Every level, in order.
    pub const ALL: [Self; 4] = [Self::Off, Self::Low, Self::Medium, Self::High];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

impl fmt::Display for ReasoningLevel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ReasoningLevel {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "off" => Ok(Self::Off),
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            other => Err(other.to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_level() {
        for level in ReasoningLevel::ALL {
            assert_eq!(level.as_str().parse::<ReasoningLevel>(), Ok(level));
        }
        assert!("max".parse::<ReasoningLevel>().is_err());
    }
}
