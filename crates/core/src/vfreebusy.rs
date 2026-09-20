//! Bounded, provider-free VFREEBUSY wire and projection helpers.
//!
//! This module deliberately handles only explicit FREEBUSY periods.  It does
//! not expand recurrence rules, resolve time zones, or deliver scheduling
//! messages.

use crate::etag::{typed_etag_for_bytes, ETag};
use crate::freebusy::{normalize, project, FreeBusyError, FreeBusyInterval};
use crate::ical::{Calendar, Property};
use crate::repository::ModifiedAt;
use std::fmt;

pub const DEFAULT_MAX_BYTES: usize = 64 * 1024;
pub const DEFAULT_MAX_INTERVALS: usize = 512;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum FreeBusyType {
    Busy,
    BusyTentative,
    BusyUnavailable,
    Free,
    Other(String),
}

impl FreeBusyType {
    fn parse(value: &str) -> Self {
        match value.to_ascii_uppercase().as_str() {
            "BUSY" => Self::Busy,
            "BUSY-TENTATIVE" => Self::BusyTentative,
            "BUSY-UNAVAILABLE" => Self::BusyUnavailable,
            "FREE" => Self::Free,
            _ => Self::Other(value.to_ascii_uppercase()),
        }
    }

    fn as_str(&self) -> &str {
        match self {
            Self::Busy => "BUSY",
            Self::BusyTentative => "BUSY-TENTATIVE",
            Self::BusyUnavailable => "BUSY-UNAVAILABLE",
            Self::Free => "FREE",
            Self::Other(value) => value.as_str(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FreeBusyPeriod {
    pub kind: FreeBusyType,
    pub interval: FreeBusyInterval,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VFreeBusy {
    pub dtstart: Option<String>,
    pub dtend: Option<String>,
    pub organizer: Option<String>,
    pub url: Option<String>,
    pub tzid: Option<String>,
    pub periods: Vec<FreeBusyPeriod>,
    pub modified_at: ModifiedAt,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VFreeBusyError {
    Malformed(String),
    InvalidInterval(FreeBusyError),
    TooLarge,
    TooManyIntervals,
}

impl fmt::Display for VFreeBusyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(reason) => write!(f, "malformed VFREEBUSY: {reason}"),
            Self::InvalidInterval(error) => write!(f, "invalid VFREEBUSY interval: {error}"),
            Self::TooLarge => f.write_str("VFREEBUSY exceeds byte limit"),
            Self::TooManyIntervals => f.write_str("VFREEBUSY exceeds interval limit"),
        }
    }
}

impl std::error::Error for VFreeBusyError {}

impl From<FreeBusyError> for VFreeBusyError {
    fn from(error: FreeBusyError) -> Self {
        Self::InvalidInterval(error)
    }
}

impl VFreeBusy {
    pub fn parse(input: &str) -> Result<Self, VFreeBusyError> {
        Self::parse_bounded(input, DEFAULT_MAX_BYTES, DEFAULT_MAX_INTERVALS)
    }

    pub fn parse_bounded(
        input: &str,
        max_bytes: usize,
        max_intervals: usize,
    ) -> Result<Self, VFreeBusyError> {
        if input.len() > max_bytes {
            return Err(VFreeBusyError::TooLarge);
        }
        let calendar =
            Calendar::parse(input).map_err(|error| VFreeBusyError::Malformed(error.to_string()))?;
        let components: Vec<_> = calendar.root().components_named("VFREEBUSY").collect();
        if components.len() != 1 || calendar.root().components_named("VFREEBUSY").count() != 1 {
            return Err(VFreeBusyError::Malformed(
                "expected one VFREEBUSY component".into(),
            ));
        }
        let component = components[0];
        let first = |name: &str| component.properties_named(name).next();
        let dtstart = first("DTSTART").map(normalize_time_property).transpose()?;
        let dtend = first("DTEND").map(normalize_time_property).transpose()?;
        let organizer = first("ORGANIZER").map(|property| property.value.clone());
        let url = first("URL").map(|property| property.value.clone());
        let tzid = first("DTSTART")
            .and_then(|property| property.params.get("TZID"))
            .and_then(|values| values.first())
            .cloned();
        let mut periods = Vec::new();
        for property in component.properties_named("FREEBUSY") {
            let kind = property
                .params
                .get("FBTYPE")
                .and_then(|values| values.first())
                .map_or(FreeBusyType::Busy, |value| FreeBusyType::parse(value));
            for raw_period in property.value.split(',') {
                let (start, end) = raw_period.split_once('/').ok_or_else(|| {
                    VFreeBusyError::Malformed("FREEBUSY requires start/end".into())
                })?;
                // Duration forms are intentionally rejected until a duration
                // model is added; silently guessing would change availability.
                if end.starts_with('P') || end.starts_with('-') {
                    return Err(VFreeBusyError::Malformed(
                        "FREEBUSY duration is unsupported".into(),
                    ));
                }
                periods.push(FreeBusyPeriod {
                    kind: kind.clone(),
                    interval: FreeBusyInterval::new(start, end)?,
                });
                if periods.len() > max_intervals {
                    return Err(VFreeBusyError::TooManyIntervals);
                }
            }
        }
        periods.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then(left.interval.cmp(&right.interval))
        });
        Ok(Self {
            dtstart,
            dtend,
            organizer,
            url,
            tzid,
            periods,
            modified_at: ModifiedAt::UNIX_EPOCH,
        })
    }

    pub fn serialize(&self) -> Result<String, VFreeBusyError> {
        self.serialize_bounded(DEFAULT_MAX_BYTES)
    }

    pub fn serialize_bounded(&self, max_bytes: usize) -> Result<String, VFreeBusyError> {
        let mut output = String::from("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VFREEBUSY\r\n");
        if let Some(value) = &self.dtstart {
            if let Some(tzid) = &self.tzid {
                output.push_str("DTSTART;TZID=");
                output.push_str(tzid);
                output.push(':');
            } else {
                output.push_str("DTSTART:");
            }
            output.push_str(value);
            output.push_str("\r\n");
        }
        if let Some(value) = &self.dtend {
            output.push_str("DTEND:");
            output.push_str(value);
            output.push_str("\r\n");
        }
        if let Some(value) = &self.organizer {
            output.push_str("ORGANIZER:");
            output.push_str(value);
            output.push_str("\r\n");
        }
        if let Some(value) = &self.url {
            output.push_str("URL:");
            output.push_str(value);
            output.push_str("\r\n");
        }
        for kind in [
            FreeBusyType::Busy,
            FreeBusyType::BusyTentative,
            FreeBusyType::BusyUnavailable,
            FreeBusyType::Free,
        ] {
            let values: Vec<_> = self
                .periods
                .iter()
                .filter(|period| period.kind == kind)
                .collect();
            if !values.is_empty() {
                append_periods(&mut output, &kind, &values);
            }
        }
        for period in self
            .periods
            .iter()
            .filter(|period| matches!(period.kind, FreeBusyType::Other(_)))
        {
            append_periods(&mut output, &period.kind, std::slice::from_ref(&period));
        }
        output.push_str("END:VFREEBUSY\r\nEND:VCALENDAR\r\n");
        if output.len() > max_bytes {
            return Err(VFreeBusyError::TooLarge);
        }
        Ok(output)
    }

    pub fn etag(&self) -> Result<ETag, VFreeBusyError> {
        Ok(typed_etag_for_bytes(self.serialize()?.as_bytes()))
    }

    pub fn project(
        &self,
        window_start: &str,
        window_end: &str,
        max_intervals: usize,
    ) -> Result<Self, VFreeBusyError> {
        let mut projected = Vec::new();
        let mut kinds = self
            .periods
            .iter()
            .map(|period| period.kind.clone())
            .collect::<Vec<_>>();
        kinds.sort();
        kinds.dedup();
        for kind in kinds {
            let source: Vec<_> = self
                .periods
                .iter()
                .filter(|period| period.kind == kind)
                .map(|period| period.interval.clone())
                .collect();
            for interval in project(&source, window_start, window_end, &[])? {
                projected.push(FreeBusyPeriod {
                    kind: kind.clone(),
                    interval,
                });
            }
        }
        if projected.len() > max_intervals {
            return Err(VFreeBusyError::TooManyIntervals);
        }
        projected.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then(left.interval.cmp(&right.interval))
        });
        Ok(Self {
            dtstart: Some(window_start.to_owned()),
            dtend: Some(window_end.to_owned()),
            organizer: self.organizer.clone(),
            url: self.url.clone(),
            tzid: self.tzid.clone(),
            periods: projected,
            modified_at: self.modified_at,
        })
    }
}

fn normalize_time_property(property: &Property) -> Result<String, VFreeBusyError> {
    Ok(normalize(&property.value)?)
}

fn append_periods(output: &mut String, kind: &FreeBusyType, periods: &[&FreeBusyPeriod]) {
    output.push_str("FREEBUSY;FBTYPE=");
    output.push_str(kind.as_str());
    output.push(':');
    output.push_str(
        &periods
            .iter()
            .map(|period| format!("{}/{}", period.interval.start, period.interval.end))
            .collect::<Vec<_>>()
            .join(","),
    );
    output.push_str("\r\n");
}
