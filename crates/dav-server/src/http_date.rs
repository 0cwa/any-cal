//! Small, dependency-free IMF-fixdate parser/formatter for DAV conditions.
//! HTTP dates are compared at whole UTC-second precision.

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub(crate) fn parse(value: &str) -> Option<u64> {
    let parts: Vec<_> = value.split_ascii_whitespace().collect();
    if parts.len() != 6 || parts[0].len() != 4 || !parts[0].ends_with(',') || parts[5] != "GMT" {
        return None;
    }
    let day = parts[1].parse::<u32>().ok()?;
    let month = MONTHS.iter().position(|month| *month == parts[2])? as u32 + 1;
    let year = parts[3].parse::<i32>().ok()?;
    // The date and time are split around the month/year tokens in IMF-fixdate:
    // `Wed, 21 Oct 2015 07:28:00 GMT`.
    let time = parts[4];
    let [hour, minute, second] = parse_clock(time)?;
    if !(1..=9999).contains(&year) || !(1..=31).contains(&day) {
        return None;
    }
    let days = days_from_civil(year, month, day)?;
    let seconds = days
        .checked_mul(86_400)?
        .checked_add(u64::from(hour) * 3_600)?
        .checked_add(u64::from(minute) * 60)?
        .checked_add(u64::from(second))?;
    Some(seconds)
}

fn parse_clock(value: &str) -> Option<[u32; 3]> {
    let mut fields = value.split(':');
    let hour = fields.next()?.parse::<u32>().ok()?;
    let minute = fields.next()?.parse::<u32>().ok()?;
    let second = fields.next()?.parse::<u32>().ok()?;
    if fields.next().is_some() || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    Some([hour, minute, second.min(59)])
}

pub(crate) fn format(seconds: u64) -> Option<String> {
    let days = seconds / 86_400;
    let remainder = seconds % 86_400;
    let days = i64::try_from(days).ok()?;
    let (year, month, day) = civil_from_days(days);
    if !(1..=9999).contains(&year) {
        return None;
    }
    let hour = remainder / 3_600;
    let minute = remainder % 3_600 / 60;
    let second = remainder % 60;
    let weekday = match days.rem_euclid(7) {
        0 => "Thu",
        1 => "Fri",
        2 => "Sat",
        3 => "Sun",
        4 => "Mon",
        5 => "Tue",
        _ => "Wed",
    };
    Some(format!(
        "{weekday}, {day:02} {} {year:04} {hour:02}:{minute:02}:{second:02} GMT",
        MONTHS[month as usize - 1]
    ))
}

// Howard Hinnant's proleptic Gregorian civil-date conversion, adapted to
// return days relative to 1970-01-01 without external time dependencies.
fn days_from_civil(year: i32, month: u32, day: u32) -> Option<u64> {
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => return None,
    };
    if day > max_day {
        return None;
    }
    let y = i64::from(year) - i64::from(month <= 2);
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let year_of_era = y - era * 400;
    let month = i64::from(month);
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    u64::try_from(days).ok()
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    (year as i32, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_epoch_and_known_http_date() {
        let known = "Wed, 21 Oct 2015 07:28:00 GMT";
        let seconds = parse(known).unwrap();
        assert_eq!(format(seconds).as_deref(), Some(known));
        assert_eq!(parse("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
    }

    #[test]
    fn malformed_and_out_of_range_dates_are_ignored() {
        for value in [
            "not a date",
            "Wed, 31 Feb 2015 07:28:00 GMT",
            "Wed, 21 Oct 2015 25:28:00 GMT",
            "Wed, 21 Oct 1969 07:28:00 GMT",
            "Wed, 21 Oct 2015 07:28:00 UTC",
        ] {
            assert_eq!(parse(value), None, "{value}");
        }
    }
}
