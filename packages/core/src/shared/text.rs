//! Validation helpers for user-entered text.

use crate::shared::errors::{DomainError, DomainResult};

/// A required single-line name: trimmed, not empty, at most `max` characters.
///
/// `label` names the value in messages ("Project name"); `field` is the request field.
pub fn name(value: &str, label: &str, field: &str, max: usize) -> DomainResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(DomainError::validation(format!("{label} is required.")).with_field(field));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(
            DomainError::validation(format!("{label} must be a single line.")).with_field(field),
        );
    }
    if trimmed.chars().count() > max {
        return Err(
            DomainError::validation(format!("{label} must be at most {max} characters."))
                .with_field(field)
                .with_context("max", max),
        );
    }
    Ok(trimmed.to_owned())
}

/// Free text such as instructions: trimmed at both ends, may be empty.
pub fn trimmed(value: &str) -> String {
    value.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(error: &DomainError) -> Option<&str> {
        match error {
            DomainError::Validation { field, .. } => field.as_deref(),
            _ => None,
        }
    }

    #[test]
    fn trims_a_valid_name() {
        assert_eq!(
            name("  Work  ", "Project name", "name", 10).unwrap(),
            "Work"
        );
    }

    #[test]
    fn rejects_an_empty_name() {
        let error = name("   ", "Project name", "name", 10).unwrap_err();
        assert_eq!(error.to_string(), "Project name is required.");
        assert_eq!(field(&error), Some("name"));
    }

    #[test]
    fn rejects_a_long_name() {
        let error = name("abcdefghijk", "Project name", "name", 10).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Project name must be at most 10 characters."
        );
    }

    #[test]
    fn counts_characters_not_bytes() {
        assert!(name("éééééééééé", "Name", "name", 10).is_ok());
    }

    #[test]
    fn rejects_line_breaks() {
        let error = name("a\nb", "Name", "name", 10).unwrap_err();
        assert_eq!(error.to_string(), "Name must be a single line.");
    }

    #[test]
    fn trims_free_text() {
        assert_eq!(trimmed("\n  Be brief.\n"), "Be brief.");
    }
}
