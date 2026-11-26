//! The cron rule of a schedule: a 5-field pattern in an IANA time zone.

use std::fmt;

use chrono::{DateTime, Utc};
use croner::Cron;
use croner::parser::{CronParser, Seconds, Year};
use jiff::Timestamp;
use jiff::tz::TimeZone;

use crate::shared::constants::SCHEDULE_MIN_INTERVAL_MINUTES;
use crate::shared::errors::{DomainError, DomainResult};

/// Fires checked for the shortest gap when a rule is saved.
const GAP_SAMPLE: usize = 50;
/// Day-of-week names, Sunday first (cron numbers 0 to 6; 7 is Sunday too).
const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const WEEKDAY_NAMES: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

/// A parsed 5-field `croner` pattern (minute, hour, day of month, month, day of week) plus a
/// `jiff` time zone. The system clock fires it; daylight saving time follows croner's rules
/// (a fixed-time job runs once in an overlap and at the first valid time after a gap).
#[derive(Clone)]
pub struct CronRule {
    expression: String,
    timezone: String,
    cron: Cron,
    zone: TimeZone,
}

impl CronRule {
    /// Validates a rule a user or the model wrote: exactly 5 fields, a valid pattern that runs,
    /// an IANA time zone, and no two of the next 50 fires after `now` closer than
    /// `SCHEDULE_MIN_INTERVAL_MINUTES`.
    pub fn parse(expression: &str, timezone: &str, now: DateTime<Utc>) -> DomainResult<Self> {
        let rule = Self::read(expression, timezone)?;
        let fires = rule.next_n(now, GAP_SAMPLE);
        if fires.is_empty() {
            return Err(DomainError::validation("This cron rule never runs.").with_field("cron"));
        }
        let minimum = chrono::Duration::minutes(i64::from(SCHEDULE_MIN_INTERVAL_MINUTES));
        if fires.windows(2).any(|pair| pair[1] - pair[0] < minimum) {
            return Err(DomainError::validation(format!(
                "Schedules must run at least {SCHEDULE_MIN_INTERVAL_MINUTES} minutes apart."
            ))
            .with_field("cron")
            .with_context("minIntervalMinutes", SCHEDULE_MIN_INTERVAL_MINUTES));
        }
        Ok(rule)
    }

    /// A stored rule. It was validated when it was saved; only the syntax and the time zone are
    /// read again.
    pub fn restore(expression: &str, timezone: &str) -> DomainResult<Self> {
        Self::read(expression, timezone).map_err(|error| DomainError::Internal {
            key: "core.cron-rule.restore.invalid".to_owned(),
            source: Some(error.into()),
        })
    }

    fn read(expression: &str, timezone: &str) -> DomainResult<Self> {
        let fields: Vec<&str> = expression.split_whitespace().collect();
        if fields.is_empty() {
            return Err(DomainError::validation("Enter a cron rule.").with_field("cron"));
        }
        if fields.len() != 5 {
            return Err(DomainError::validation(
                "A cron rule has 5 fields: minute, hour, day of month, month, and day of week.",
            )
            .with_field("cron"));
        }
        let expression = fields.join(" ");
        let cron = CronParser::builder()
            .seconds(Seconds::Disallowed)
            .year(Year::Disallowed)
            .build()
            .parse(&expression)
            .map_err(|_| {
                DomainError::validation("This cron rule is not valid.").with_field("cron")
            })?;
        let name = timezone.trim();
        let zone = (!name.is_empty())
            .then(|| TimeZone::get(name).ok())
            .flatten()
            .ok_or_else(|| {
                DomainError::validation("Choose a time zone, for example Europe/Berlin.")
                    .with_field("timezone")
            })?;
        let timezone = zone.iana_name().unwrap_or(name).to_owned();
        Ok(Self {
            expression,
            timezone,
            cron,
            zone,
        })
    }

    /// The first fire strictly after `instant`; `None` when the rule never fires again.
    pub fn next_after(&self, instant: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let start = to_timestamp(instant)?.to_zoned_datetime(self.zone.clone());
        let next = self.cron.find_next_occurrence(&start, false).ok()?;
        to_chrono(next.timestamp())
    }

    /// The next `count` fires after `instant`, oldest first.
    pub fn next_n(&self, instant: DateTime<Utc>, count: usize) -> Vec<DateTime<Utc>> {
        let mut fires = Vec::with_capacity(count);
        let mut cursor = instant;
        while fires.len() < count {
            let Some(next) = self.next_after(cursor) else {
                break;
            };
            fires.push(next);
            cursor = next;
        }
        fires
    }

    /// A sentence for preset-shaped rules ("Weekdays at 09:30"), the expression otherwise. The
    /// time zone is not part of it.
    pub fn describe(&self) -> String {
        describe(&self.expression).unwrap_or_else(|| self.expression.clone())
    }

    /// `instant` as a calendar date in the rule's time zone (`2026-01-31`), for chat titles.
    pub fn local_date(&self, instant: DateTime<Utc>) -> String {
        to_timestamp(instant)
            .map(|timestamp| {
                timestamp
                    .to_zoned_datetime(self.zone.clone())
                    .date()
                    .to_string()
            })
            .unwrap_or_else(|| instant.date_naive().to_string())
    }

    /// The pattern, with fields separated by single spaces.
    pub fn expression(&self) -> &str {
        &self.expression
    }

    /// The IANA time zone name.
    pub fn timezone(&self) -> &str {
        &self.timezone
    }
}

impl PartialEq for CronRule {
    fn eq(&self, other: &Self) -> bool {
        self.expression == other.expression && self.timezone == other.timezone
    }
}

impl Eq for CronRule {}

impl fmt::Debug for CronRule {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CronRule")
            .field("expression", &self.expression)
            .field("timezone", &self.timezone)
            .finish()
    }
}

fn to_timestamp(instant: DateTime<Utc>) -> Option<Timestamp> {
    let nanos = i32::try_from(instant.timestamp_subsec_nanos()).ok()?;
    Timestamp::new(instant.timestamp(), nanos).ok()
}

fn to_chrono(timestamp: Timestamp) -> Option<DateTime<Utc>> {
    let nanos = u32::try_from(timestamp.subsec_nanosecond()).ok()?;
    DateTime::from_timestamp(timestamp.as_second(), nanos)
}

/// The sentence for the presets of the schedule form: every hour, every day at, weekdays at,
/// weekly on, and monthly on day.
fn describe(expression: &str) -> Option<String> {
    let fields: Vec<String> = expression.split(' ').map(str::to_ascii_uppercase).collect();
    let [minute, hour, day, month, weekday] = fields.as_slice() else {
        return None;
    };
    let minute = number(minute, 59)?;
    if month != "*" {
        return None;
    }
    if hour == "*" {
        return (day == "*" && weekday == "*").then(|| format!("Every hour at :{minute:02}"));
    }
    let time = format!("{:02}:{minute:02}", number(hour, 23)?);
    match (day.as_str(), weekday.as_str()) {
        ("*", "*") => Some(format!("Every day at {time}")),
        ("*", "1-5" | "MON-FRI") => Some(format!("Weekdays at {time}")),
        ("*", weekday) => {
            let index = match WEEKDAY_NAMES.iter().position(|name| *name == weekday) {
                Some(index) => index,
                None => number(weekday, 7)? % 7,
            };
            Some(format!("Every {} at {time}", WEEKDAYS[index]))
        }
        (day, "*") => Some(format!("Monthly on day {} at {time}", number(day, 31)?)),
        _ => None,
    }
}

/// A plain decimal number up to `max`.
fn number(field: &str, max: usize) -> Option<usize> {
    if field.is_empty() || !field.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    field.parse().ok().filter(|value| *value <= max)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;

    fn utc(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
            .unwrap()
    }

    fn now() -> DateTime<Utc> {
        utc(2026, 1, 1, 0, 0)
    }

    fn rule(expression: &str, timezone: &str) -> CronRule {
        CronRule::parse(expression, timezone, now()).unwrap()
    }

    fn message(expression: &str, timezone: &str) -> String {
        CronRule::parse(expression, timezone, now())
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn accepts_five_fields_only() {
        assert_eq!(rule("  30  9 * * 1-5 ", "UTC").expression(), "30 9 * * 1-5");
        let five = "A cron rule has 5 fields: minute, hour, day of month, month, and day of week.";
        assert_eq!(message("0 30 9 * * 1-5", "UTC"), five, "no seconds field");
        assert_eq!(message("30 9 * * 1-5 2027", "UTC"), five, "no year field");
        assert_eq!(message("@daily", "UTC"), five, "no nicknames");
        assert_eq!(message("30 9 * *", "UTC"), five);
        assert_eq!(message("   ", "UTC"), "Enter a cron rule.");
        assert_eq!(message("61 9 * * *", "UTC"), "This cron rule is not valid.");
        assert_eq!(
            message("0/10 9 * * *", "UTC"),
            "This cron rule is not valid."
        );
    }

    #[test]
    fn requires_an_iana_time_zone() {
        assert_eq!(
            rule("0 9 * * *", "Europe/Berlin").timezone(),
            "Europe/Berlin"
        );
        for zone in ["", "Mars/Olympus", "+02:00"] {
            let error = CronRule::parse("0 9 * * *", zone, now()).unwrap_err();
            assert_eq!(
                error.to_string(),
                "Choose a time zone, for example Europe/Berlin."
            );
            assert!(
                matches!(error, DomainError::Validation { field: Some(ref field), .. } if field == "timezone")
            );
        }
    }

    #[test]
    fn rejects_rules_that_never_run() {
        assert_eq!(message("0 9 30 2 *", "UTC"), "This cron rule never runs.");
    }

    #[test]
    fn enforces_the_minimum_interval_over_the_next_fifty_fires() {
        let too_often = "Schedules must run at least 15 minutes apart.";
        assert_eq!(message("*/10 * * * *", "UTC"), too_often);
        assert_eq!(message("0,10 9 * * *", "UTC"), too_often);
        assert_eq!(
            message("* 9 1 1 *", "UTC"),
            too_often,
            "a burst once a year"
        );
        assert!(CronRule::parse("*/15 * * * *", "UTC", now()).is_ok());
        assert!(CronRule::parse("0,15,30,45 * * * *", "UTC", now()).is_ok());
        let error = CronRule::parse("*/5 * * * *", "UTC", now()).unwrap_err();
        assert_eq!(error.context().get("minIntervalMinutes"), Some(&15.into()));
    }

    #[test]
    fn computes_fires_in_the_time_zone() {
        let rule = rule("30 9 * * 1-5", "America/New_York");
        // 2026-01-01 is a Thursday; 09:30 EST is 14:30 UTC.
        let fires = rule.next_n(now(), 3);
        assert_eq!(
            fires,
            [
                utc(2026, 1, 1, 14, 30),
                utc(2026, 1, 2, 14, 30),
                utc(2026, 1, 5, 14, 30)
            ]
        );
        assert_eq!(
            rule.next_after(utc(2026, 1, 1, 14, 30)),
            Some(utc(2026, 1, 2, 14, 30))
        );
        assert_eq!(
            rule.next_after(utc(2026, 1, 1, 14, 29)),
            Some(utc(2026, 1, 1, 14, 30))
        );
    }

    #[test]
    fn follows_daylight_saving_time() {
        let rule = rule("30 2 * * *", "America/New_York");
        let fires = rule.next_n(utc(2026, 3, 7, 12, 0), 3);
        assert_eq!(
            fires,
            [
                utc(2026, 3, 8, 7, 0),   // 02:30 does not exist: 03:00 EDT
                utc(2026, 3, 9, 6, 30),  // 02:30 EDT
                utc(2026, 3, 10, 6, 30), // 02:30 EDT
            ]
        );
        let once = CronRule::parse("30 1 * * *", "America/New_York", now()).unwrap();
        let fires = once.next_n(utc(2026, 10, 31, 12, 0), 2);
        assert_eq!(
            fires,
            [utc(2026, 11, 1, 5, 30), utc(2026, 11, 2, 6, 30)],
            "a fixed-time job runs once in the repeated hour"
        );
    }

    #[test]
    fn describes_preset_shaped_rules() {
        let cases = [
            ("5 * * * *", "Every hour at :05"),
            ("30 9 * * *", "Every day at 09:30"),
            ("0 18 * * 1-5", "Weekdays at 18:00"),
            ("0 18 * * mon-fri", "Weekdays at 18:00"),
            ("15 7 * * 1", "Every Monday at 07:15"),
            ("15 7 * * SUN", "Every Sunday at 07:15"),
            ("15 7 * * 7", "Every Sunday at 07:15"),
            ("0 8 15 * *", "Monthly on day 15 at 08:00"),
            ("*/30 9-17 * * 1-5", "*/30 9-17 * * 1-5"),
            ("0 9 1 1 *", "0 9 1 1 *"),
            ("0 9 1 * 1", "0 9 1 * 1"),
            ("0 9 L * *", "0 9 L * *"),
        ];
        for (expression, sentence) in cases {
            assert_eq!(rule(expression, "UTC").describe(), sentence, "{expression}");
        }
    }

    #[test]
    fn formats_the_local_date() {
        let rule = rule("0 9 * * *", "Pacific/Auckland");
        assert_eq!(rule.local_date(utc(2026, 1, 31, 12, 0)), "2026-02-01");
    }

    #[test]
    fn restores_without_the_interval_check() {
        assert!(CronRule::restore("*/5 * * * *", "UTC").is_ok());
        let error = CronRule::restore("bad", "UTC").unwrap_err();
        assert_eq!(error.code(), "INTERNAL_ERROR");
        assert_eq!(
            rule("0 9 * * *", "UTC"),
            CronRule::restore("0  9 * * *", "UTC").unwrap()
        );
    }
}
