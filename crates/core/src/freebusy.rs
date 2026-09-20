//! Bounded, provider-free free/busy projection helpers.
//!
//! UTC, floating, and TZID-associated values are compared by normalized
//! wall-clock value. Timezone conversion and recurrence expansion belong to a
//! higher layer and are intentionally outside this helper.

use std::cmp::{max, min};
use std::fmt;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct FreeBusyInterval {
    pub start: String,
    pub end: String,
}

impl FreeBusyInterval {
    pub fn new(start: impl AsRef<str>, end: impl AsRef<str>) -> Result<Self, FreeBusyError> {
        let start = normalize_time(start.as_ref())?;
        let end = normalize_time(end.as_ref())?;
        if start >= end {
            return Err(FreeBusyError::ReversedInterval);
        }
        Ok(Self { start, end })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FreeBusyError {
    InvalidTime(String),
    ReversedInterval,
    InvalidWindow,
}

impl fmt::Display for FreeBusyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTime(value) => write!(f, "invalid calendar time {value:?}"),
            Self::ReversedInterval => write!(f, "free/busy interval end must follow start"),
            Self::InvalidWindow => write!(f, "free/busy window must have a start before its end"),
        }
    }
}

impl std::error::Error for FreeBusyError {}

/// Clip intervals to a half-open window, subtract exclusions, merge overlaps,
/// and return deterministic normalized basic date-time values.
///
/// No recurrence expansion, timezone conversion, or external scheduling is
/// performed. Values with UTC (`Z`), floating, TZID-associated, or date-only
/// syntax are compared by their literal wall-clock value after normalization.
pub fn project(
    intervals: &[FreeBusyInterval],
    window_start: impl AsRef<str>,
    window_end: impl AsRef<str>,
    exclusions: &[FreeBusyInterval],
) -> Result<Vec<FreeBusyInterval>, FreeBusyError> {
    let window_start = normalize_time(window_start.as_ref())?;
    let window_end = normalize_time(window_end.as_ref())?;
    if window_start >= window_end {
        return Err(FreeBusyError::InvalidWindow);
    }

    let mut exclusions = exclusions.to_vec();
    exclusions.sort();
    let mut clipped = Vec::new();
    for interval in intervals {
        let start = max(interval.start.clone(), window_start.clone());
        let end = min(interval.end.clone(), window_end.clone());
        if start >= end {
            continue;
        }
        let mut cursor = start;
        for exclusion in &exclusions {
            if exclusion.end <= cursor {
                continue;
            }
            if exclusion.start >= end {
                break;
            }
            if exclusion.start > cursor {
                clipped.push(FreeBusyInterval {
                    start: cursor.clone(),
                    end: min(exclusion.start.clone(), end.clone()),
                });
            }
            cursor = max(cursor, exclusion.end.clone());
            if cursor >= end {
                break;
            }
        }
        if cursor < end {
            clipped.push(FreeBusyInterval { start: cursor, end });
        }
    }

    clipped.sort();
    let mut merged: Vec<FreeBusyInterval> = Vec::new();
    for interval in clipped {
        if let Some(previous) = merged.last_mut() {
            if interval.start <= previous.end {
                if interval.end > previous.end {
                    previous.end = interval.end;
                }
                continue;
            }
        }
        merged.push(interval);
    }
    Ok(merged)
}

fn normalize_time(value: &str) -> Result<String, FreeBusyError> {
    let value = value.trim();
    let compact = if value.len() == 8 && value.bytes().all(|byte| byte.is_ascii_digit()) {
        format!("{value}000000")
    } else if value.len() == 14 && value.bytes().all(|byte| byte.is_ascii_digit()) {
        value.to_owned()
    } else if value.len() == 15
        && value.as_bytes()[8] == b'T'
        && value[..8].bytes().all(|byte| byte.is_ascii_digit())
        && value[9..].bytes().all(|byte| byte.is_ascii_digit())
    {
        value.replace('T', "")
    } else if value.len() == 16
        && value.ends_with('Z')
        && value.as_bytes()[8] == b'T'
        && value[..8].bytes().all(|byte| byte.is_ascii_digit())
        && value[9..15].bytes().all(|byte| byte.is_ascii_digit())
    {
        value[..15].replace('T', "")
    } else if value.len() == 19
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value.as_bytes()[10] == b'T'
        && value.as_bytes()[13] == b':'
        && value.as_bytes()[16] == b':'
        && value[..4].bytes().all(|byte| byte.is_ascii_digit())
        && value[5..7].bytes().all(|byte| byte.is_ascii_digit())
        && value[8..10].bytes().all(|byte| byte.is_ascii_digit())
        && value[11..13].bytes().all(|byte| byte.is_ascii_digit())
        && value[14..16].bytes().all(|byte| byte.is_ascii_digit())
        && value[17..].bytes().all(|byte| byte.is_ascii_digit())
    {
        format!(
            "{}{}{}{}{}{}",
            &value[..4],
            &value[5..7],
            &value[8..10],
            &value[11..13],
            &value[14..16],
            &value[17..]
        )
    } else if value.len() == 20 && value.ends_with('Z') {
        return normalize_time(&value[..19]);
    } else {
        return Err(FreeBusyError::InvalidTime(value.to_owned()));
    };
    let year = compact[0..4].parse::<u16>().ok();
    let month = compact[4..6].parse::<u8>().ok();
    let day = compact[6..8].parse::<u8>().ok();
    let hour = compact[8..10].parse::<u8>().ok();
    let minute = compact[10..12].parse::<u8>().ok();
    let second = compact[12..14].parse::<u8>().ok();
    let valid = match (year, month, day, hour, minute, second) {
        (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) => {
            let month_days = match month {
                1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
                4 | 6 | 9 | 11 => 30,
                2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
                2 => 28,
                _ => 0,
            };
            day > 0 && day <= month_days && hour < 24 && minute < 60 && second < 60
        }
        _ => false,
    };
    valid
        .then_some(compact)
        .ok_or_else(|| FreeBusyError::InvalidTime(value.to_owned()))
}

/// Normalize a supported iCalendar date/date-time value without constructing
/// an interval.  Time zone conversion remains intentionally out of scope.
pub fn normalize(value: impl AsRef<str>) -> Result<String, FreeBusyError> {
    normalize_time(value.as_ref())
}
