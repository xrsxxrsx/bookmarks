//! Publication dates that may be partial or estimated.
//!
//! Real libraries contain dates like "somewhere in 2019" for a work found on another
//! platform, alongside exact AO3 timestamps. Storing both as a string would break
//! sorting, and storing the guessed one as `2019-01-01` would silently assert a
//! precision that does not exist.
//!
//! So a date is stored as a normalised ISO string with the missing components filled
//! to the *start* of their range, plus a separate precision and an estimated flag:
//!   - sorting always has a definite, comparable value
//!   - display shows only the components that are actually known
//!   - `date_is_approx` distinguishes "the source only stated the year" from
//!     "the user guessed the year", which matters when deciding what to trust.

use serde::{Deserialize, Serialize};

/// How much of the date is actually known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DatePrecision {
    Year,
    Month,
    Day,
}

impl DatePrecision {
    pub fn as_str(self) -> &'static str {
        match self {
            DatePrecision::Year => "year",
            DatePrecision::Month => "month",
            DatePrecision::Day => "day",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "year" => Some(DatePrecision::Year),
            "month" => Some(DatePrecision::Month),
            "day" => Some(DatePrecision::Day),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DateValue {
    /// ISO-8601 with missing components filled to the start of their range:
    /// `2019`, `2019-06` and `2019-06-09` all become a `YYYY-MM-DD` string.
    pub iso: String,
    pub precision: DatePrecision,
    /// The value is a guess rather than a statement from the source.
    pub is_approx: bool,
}

impl DateValue {
    /// `year`/`month`/`day` are 1-based; `month` and `day` default to 1 so the
    /// resulting value sits at the start of the known range.
    pub fn new(year: i32, month: Option<u32>, day: Option<u32>, is_approx: bool) -> Option<Self> {
        let (m, d, precision) = match (month, day) {
            (Some(m), Some(d)) => (m, d, DatePrecision::Day),
            (Some(m), None) => (m, 1, DatePrecision::Month),
            (None, _) => (1, 1, DatePrecision::Year),
        };
        if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
            return None;
        }
        Some(DateValue {
            iso: format!("{year:04}-{m:02}-{d:02}"),
            precision,
            is_approx,
        })
    }

    pub fn from_year(year: i32, is_approx: bool) -> Self {
        Self::new(year, None, None, is_approx).expect("year alone is always valid")
    }

    /// Parses the loose date strings that sources actually produce:
    /// `2018-06-09`, `2018-06`, `2018`, and the same with `/` or `.` separators.
    ///
    /// Unparseable input returns `None` rather than a fabricated date, so a malformed
    /// field shows up as missing instead of silently becoming 1970-01-01.
    pub fn parse_loose(input: &str, is_approx: bool) -> Option<Self> {
        let cleaned: String = input
            .trim()
            .chars()
            .map(|c| if c == '/' || c == '.' { '-' } else { c })
            .collect();

        let mut parts = cleaned.split('-').map(str::trim);
        let year = parts.next()?.parse::<i32>().ok()?;
        let month = parts.next().and_then(|p| p.parse::<u32>().ok());
        let day = parts.next().and_then(|p| p.parse::<u32>().ok());
        if parts.next().is_some() {
            return None;
        }
        if !(0..=9999).contains(&year) {
            return None;
        }
        Self::new(year, month, day, is_approx)
    }

    /// The value to sort by. Always a full `YYYY-MM-DD`, so string comparison is
    /// chronological comparison.
    pub fn sort_key(&self) -> &str {
        &self.iso
    }

    /// Renders only the known components. An estimated date is prefixed with `~`,
    /// matching how estimated word counts are marked elsewhere in the UI.
    pub fn display(&self) -> String {
        let shown = match self.precision {
            DatePrecision::Day => &self.iso[..10],
            DatePrecision::Month => &self.iso[..7],
            DatePrecision::Year => &self.iso[..4],
        };
        if self.is_approx {
            format!("~{shown}")
        } else {
            shown.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_date() {
        let d = DateValue::parse_loose("2018-06-09", false).unwrap();
        assert_eq!(d.iso, "2018-06-09");
        assert_eq!(d.precision, DatePrecision::Day);
        assert_eq!(d.display(), "2018-06-09");
    }

    #[test]
    fn parses_year_month_and_year_only() {
        let m = DateValue::parse_loose("2020-06", false).unwrap();
        assert_eq!(m.iso, "2020-06-01", "missing day fills to start of range");
        assert_eq!(m.precision, DatePrecision::Month);
        assert_eq!(m.display(), "2020-06", "display must not invent a day");

        let y = DateValue::parse_loose("2020", false).unwrap();
        assert_eq!(y.iso, "2020-01-01");
        assert_eq!(y.precision, DatePrecision::Year);
        assert_eq!(y.display(), "2020");
    }

    #[test]
    fn estimated_dates_are_marked_in_display() {
        let d = DateValue::parse_loose("2019", true).unwrap();
        assert!(d.is_approx);
        assert_eq!(
            d.display(),
            "~2019",
            "estimate marker matches the word-count convention"
        );
        // The sort key stays a clean date so ordering is unaffected by the marker.
        assert_eq!(d.sort_key(), "2019-01-01");
    }

    #[test]
    fn accuracy_and_precision_are_independent() {
        // "The source only gave a year" is not the same as "the user guessed".
        let stated = DateValue::parse_loose("2019", false).unwrap();
        let guessed = DateValue::parse_loose("2019", true).unwrap();
        assert_eq!(stated.precision, guessed.precision);
        assert_ne!(stated.is_approx, guessed.is_approx);
        assert_eq!(stated.display(), "2019");
        assert_eq!(guessed.display(), "~2019");
    }

    #[test]
    fn accepts_alternative_separators() {
        assert_eq!(
            DateValue::parse_loose("2018/06/09", false).unwrap().iso,
            "2018-06-09"
        );
        assert_eq!(
            DateValue::parse_loose("2018.06.09", false).unwrap().iso,
            "2018-06-09"
        );
    }

    #[test]
    fn rejects_unparseable_input_instead_of_fabricating() {
        assert!(DateValue::parse_loose("", false).is_none());
        assert!(DateValue::parse_loose("not a date", false).is_none());
        assert!(DateValue::parse_loose("2018-06-09-01", false).is_none());
        assert!(
            DateValue::parse_loose("2018-13", false).is_none(),
            "month 13 is invalid"
        );
        assert!(DateValue::parse_loose("2018-00", false).is_none());
    }

    #[test]
    fn partial_dates_sort_by_their_known_position() {
        let exact = DateValue::parse_loose("2019-03-01", false).unwrap();
        let year_only = DateValue::parse_loose("2019", false).unwrap();
        let earlier = DateValue::parse_loose("2018-12-31", false).unwrap();
        assert!(earlier.sort_key() < year_only.sort_key());
        assert!(year_only.sort_key() < exact.sort_key());
    }

    #[test]
    fn precision_ordering_is_year_then_month_then_day() {
        assert!(DatePrecision::Year < DatePrecision::Month);
        assert!(DatePrecision::Month < DatePrecision::Day);
    }
}
