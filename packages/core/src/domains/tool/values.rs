//! Tool value objects: server slugs, tool aliases, and approval modes.

use std::fmt;
use std::str::FromStr;

use sha2::{Digest, Sha256};

use crate::shared::errors::{DomainError, DomainResult};

/// Longest slug.
pub const SLUG_MAX_CHARS: usize = 12;
/// Longest alias any provider accepts (OpenAI function names).
pub const ALIAS_MAX_CHARS: usize = 64;
/// Hex characters of the hash suffix of a cut or colliding alias.
const ALIAS_HASH_CHARS: usize = 8;
/// Slug used when a server name has no ASCII letters or digits.
const FALLBACK_SLUG: &str = "server";

/// A tool server's short name in aliases: `[a-z0-9]{1,12}`, unique per user, immutable.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Slug(String);

impl Slug {
    /// Validates a slug the user typed.
    pub fn parse(value: &str) -> DomainResult<Self> {
        let value = value.trim();
        let valid = !value.is_empty()
            && value.len() <= SLUG_MAX_CHARS
            && value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit());
        if valid {
            Ok(Self(value.to_owned()))
        } else {
            Err(DomainError::validation(format!(
                "The slug must be 1 to {SLUG_MAX_CHARS} lowercase letters or digits."
            ))
            .with_field("slug"))
        }
    }

    /// A stored slug; validated when it was saved.
    pub fn restore(value: String) -> Self {
        Self(value)
    }

    /// A slug for a server called `name` that none of `taken` uses: the name's ASCII letters and
    /// digits, lowercased and cut to 12 characters (`server` when there are none), then with a
    /// number suffix (`2`, `3`, ...) while it is taken.
    pub fn derive(name: &str, taken: &[Self]) -> Self {
        let mut base: String = name
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|character| character.to_ascii_lowercase())
            .take(SLUG_MAX_CHARS)
            .collect();
        if base.is_empty() {
            base = FALLBACK_SLUG.to_owned();
        }
        let is_free = |candidate: &str| taken.iter().all(|slug| slug.0 != candidate);
        if is_free(&base) {
            return Self(base);
        }
        (2_u32..)
            .map(|number| {
                let suffix = number.to_string();
                let keep = SLUG_MAX_CHARS - suffix.len();
                let prefix: String = base.chars().take(keep).collect();
                format!("{prefix}{suffix}")
            })
            .find(|candidate| is_free(candidate))
            .map(Self)
            .expect("a free numbered slug exists")
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Slug {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// The name a tool is sent to providers under: `{slug}__{tool}` normalized to
/// `^[a-zA-Z0-9_-]{1,64}$`. Other characters become `_`; a longer name is cut and gets an
/// 8-character hash suffix. Built-in tools use reserved names without `__`, so the two never
/// collide.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ToolAlias(String);

impl ToolAlias {
    pub fn new(slug: &Slug, tool: &str) -> Self {
        let raw = format!("{slug}__{tool}");
        let normalized = normalize(&raw);
        if normalized.len() <= ALIAS_MAX_CHARS {
            Self(normalized)
        } else {
            Self::cut(&raw, &normalized)
        }
    }

    /// The hashed form, used when [`ToolAlias::new`] gives two tools of one server the same
    /// alias (for example `a.b` and `a_b`): the normalized name cut to leave room for `_` and
    /// the first 8 hex characters of the SHA-256 of `{slug}__{tool}`.
    pub fn hashed(slug: &Slug, tool: &str) -> Self {
        let raw = format!("{slug}__{tool}");
        Self::cut(&raw, &normalize(&raw))
    }

    /// A built-in tool's reserved name (no `__`).
    pub fn builtin(name: &'static str) -> Self {
        debug_assert!(!name.contains("__"), "built-in names never contain __");
        Self(name.to_owned())
    }

    /// A stored alias (a snapshot or `chats.approved_tools`).
    pub fn restore(value: String) -> Self {
        Self(value)
    }

    fn cut(raw: &str, normalized: &str) -> Self {
        let keep = ALIAS_MAX_CHARS - ALIAS_HASH_CHARS - 1;
        let prefix: String = normalized.chars().take(keep).collect();
        let digest = Sha256::digest(raw.as_bytes());
        let hash: String = digest
            .iter()
            .flat_map(|byte| [byte >> 4, byte & 0x0f])
            .take(ALIAS_HASH_CHARS)
            .map(|nibble| char::from_digit(u32::from(nibble), 16).expect("a hex digit"))
            .collect();
        Self(format!("{prefix}_{hash}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this names a built-in tool rather than a server tool.
    pub fn is_builtin(&self) -> bool {
        !self.0.contains("__")
    }
}

impl fmt::Display for ToolAlias {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn normalize(raw: &str) -> String {
    raw.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

/// A tool server's default approval mode. New servers default to `ask`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum ApprovalMode {
    #[default]
    Ask,
    Auto,
}

impl ApprovalMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Auto => "auto",
        }
    }
}

impl fmt::Display for ApprovalMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ApprovalMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "ask" => Ok(Self::Ask),
            "auto" => Ok(Self::Auto),
            other => Err(other.to_owned()),
        }
    }
}

/// A single tool's mode. `off` tools are never sent to the model.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ToolMode {
    Off,
    Ask,
    Auto,
}

impl ToolMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Ask => "ask",
            Self::Auto => "auto",
        }
    }
}

impl From<ApprovalMode> for ToolMode {
    fn from(mode: ApprovalMode) -> Self {
        match mode {
            ApprovalMode::Ask => Self::Ask,
            ApprovalMode::Auto => Self::Auto,
        }
    }
}

impl fmt::Display for ToolMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ToolMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "off" => Ok(Self::Off),
            "ask" => Ok(Self::Ask),
            "auto" => Ok(Self::Auto),
            other => Err(other.to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slug(value: &str) -> Slug {
        Slug::parse(value).unwrap()
    }

    fn is_valid_alias(alias: &ToolAlias) -> bool {
        let text = alias.as_str();
        !text.is_empty()
            && text.len() <= ALIAS_MAX_CHARS
            && text
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "_-".contains(character))
    }

    #[test]
    fn parses_slugs() {
        assert_eq!(slug(" github ").as_str(), "github");
        assert_eq!(slug("a1b2c3d4e5f6").as_str(), "a1b2c3d4e5f6");
        for bad in ["", "GitHub", "git-hub", "git_hub", "abcdefghijklm", "é"] {
            let error = Slug::parse(bad).unwrap_err();
            assert_eq!(
                error.to_string(),
                "The slug must be 1 to 12 lowercase letters or digits.",
                "{bad}"
            );
        }
    }

    #[test]
    fn derives_unique_slugs_from_names() {
        assert_eq!(Slug::derive("GitHub Tools!", &[]).as_str(), "githubtools");
        assert_eq!(Slug::derive("Événements", &[]).as_str(), "vnements");
        assert_eq!(Slug::derive("🔧", &[]).as_str(), "server");
        assert_eq!(
            Slug::derive("A very long server name", &[]).as_str(),
            "averylongser"
        );
        let taken = [slug("github"), slug("github2")];
        assert_eq!(Slug::derive("GitHub", &taken).as_str(), "github3");
        let taken = [slug("averylongser")];
        assert_eq!(
            Slug::derive("A very long server name", &taken).as_str(),
            "averylongse2",
            "the suffix replaces the last characters"
        );
    }

    #[test]
    fn builds_aliases_from_slug_and_tool() {
        let alias = ToolAlias::new(&slug("github"), "create_issue");
        assert_eq!(alias.as_str(), "github__create_issue");
        assert!(!alias.is_builtin());
        let alias = ToolAlias::new(&slug("fs"), "read.file/v2 é");
        assert_eq!(alias.as_str(), "fs__read_file_v2__");
        assert!(is_valid_alias(&alias));
    }

    #[test]
    fn cuts_long_aliases_with_a_hash_suffix() {
        let long = "x".repeat(80);
        let alias = ToolAlias::new(&slug("github"), &long);
        assert_eq!(alias.as_str().len(), ALIAS_MAX_CHARS);
        assert!(alias.as_str().starts_with("github__xxx"));
        assert!(is_valid_alias(&alias));
        let other = ToolAlias::new(&slug("github"), &format!("{long}y"));
        assert_ne!(alias, other, "different long names keep different aliases");
        assert_eq!(alias, ToolAlias::new(&slug("github"), &long), "stable");
    }

    #[test]
    fn hashes_colliding_aliases_apart() {
        let server = slug("fs");
        assert_eq!(
            ToolAlias::new(&server, "a.b"),
            ToolAlias::new(&server, "a_b")
        );
        let hashed = ToolAlias::hashed(&server, "a.b");
        assert_ne!(hashed, ToolAlias::new(&server, "a_b"));
        assert_ne!(hashed, ToolAlias::hashed(&server, "a_b"));
        assert!(hashed.as_str().starts_with("fs__a_b_"));
        assert!(is_valid_alias(&hashed));
    }

    #[test]
    fn built_in_names_have_no_double_underscore() {
        assert!(ToolAlias::builtin("schedule_prompt").is_builtin());
    }

    #[test]
    fn parses_modes() {
        assert_eq!("ask".parse::<ApprovalMode>(), Ok(ApprovalMode::Ask));
        assert_eq!("auto".parse::<ApprovalMode>(), Ok(ApprovalMode::Auto));
        assert!("off".parse::<ApprovalMode>().is_err());
        assert_eq!(ApprovalMode::default(), ApprovalMode::Ask);
        for mode in [ToolMode::Off, ToolMode::Ask, ToolMode::Auto] {
            assert_eq!(mode.as_str().parse::<ToolMode>(), Ok(mode));
        }
        assert_eq!(ToolMode::from(ApprovalMode::Auto), ToolMode::Auto);
    }
}
