//! When a recording was made, in local time: its `created` field, its
//! automatic name and its file name all come from it.

use std::fmt;

/// A local date and time to the millisecond, with its offset from UTC.
///
/// Written in one form, `2026-11-09T18:04:05.123+01:00`, and read back only
/// in that form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Created {
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    millis: u16,
    /// Minutes east of UTC.
    offset_minutes: i16,
}

/// The parts of a [`Created`], as a clock gives them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i64,
    /// 1 to 12.
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    /// 0 to 59; a leap second (60) is taken as 59.
    pub second: u32,
    pub millis: u32,
    /// Seconds east of UTC.
    pub offset_seconds: i64,
}

/// The largest offset from UTC the format writes: under a day.
const MAX_OFFSET_MINUTES: i16 = 23 * 60 + 59;

impl Created {
    /// The time `local` names, or `None` when it is no real date and time,
    /// lies outside the years 1970 to 9999, or has an offset of a day or
    /// more. An offset that is not whole minutes is cut to whole minutes.
    pub fn new(local: LocalTime) -> Option<Created> {
        let year = u16::try_from(local.year)
            .ok()
            .filter(|year| (1970..=9999).contains(year))?;
        let month = u8::try_from(local.month)
            .ok()
            .filter(|m| (1..=12).contains(m))?;
        let day = u8::try_from(local.day)
            .ok()
            .filter(|&d| d >= 1 && d <= days_in_month(year, month))?;
        let hour = u8::try_from(local.hour).ok().filter(|&h| h < 24)?;
        let minute = u8::try_from(local.minute).ok().filter(|&m| m < 60)?;
        let second = u8::try_from(local.second.min(59)).ok()?;
        let millis = u16::try_from(local.millis).ok().filter(|&ms| ms < 1000)?;
        let widest = i64::from(MAX_OFFSET_MINUTES);
        let offset_minutes = Some(local.offset_seconds / 60)
            .filter(|offset| (-widest..=widest).contains(offset))
            .and_then(|offset| i16::try_from(offset).ok())?;
        Some(Created {
            year,
            month,
            day,
            hour,
            minute,
            second,
            millis,
            offset_minutes,
        })
    }

    /// Reads the one form [`Created`] is written in.
    pub fn parse(text: &str) -> Option<Created> {
        let bytes = text.as_bytes();
        if bytes.len() != 29 {
            return None;
        }
        let punctuation = [
            (4, b'-'),
            (7, b'-'),
            (10, b'T'),
            (13, b':'),
            (16, b':'),
            (19, b'.'),
            (26, b':'),
        ];
        if punctuation
            .iter()
            .any(|&(at, expected)| bytes.get(at) != Some(&expected))
        {
            return None;
        }
        let number = |from: usize, to: usize| -> Option<u32> {
            let digits = bytes.get(from..to)?;
            digits.iter().try_fold(0_u32, |value, &byte| {
                byte.is_ascii_digit()
                    .then(|| value * 10 + u32::from(byte - b'0'))
            })
        };
        let sign = match bytes.get(23) {
            Some(b'+') => 1,
            Some(b'-') => -1,
            _ => return None,
        };
        let offset_minutes = i64::from(number(24, 26)? * 60 + number(27, 29)?);
        if number(27, 29)? >= 60 {
            return None;
        }
        Created::new(LocalTime {
            year: i64::from(number(0, 4)?),
            month: number(5, 7)?,
            day: number(8, 10)?,
            hour: number(11, 13)?,
            minute: number(14, 16)?,
            second: number(17, 19).filter(|&s| s < 60)?,
            millis: number(20, 23)?,
            offset_seconds: sign * offset_minutes * 60,
        })
    }

    /// The automatic name of a recording made at this time: "Recording
    /// 2026-11-09 18:04".
    pub fn default_name(&self) -> String {
        format!(
            "Recording {:04}-{:02}-{:02} {:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute
        )
    }

    /// The file name, without extension, of a recording made at this time:
    /// "2026-11-09_18-04-05.123". It sorts in time order within one offset.
    pub fn file_stem(&self) -> String {
        format!(
            "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}.{:03}",
            self.year, self.month, self.day, self.hour, self.minute, self.second, self.millis
        )
    }
}

impl fmt::Display for Created {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.offset_minutes < 0 { '-' } else { '+' };
        let offset = self.offset_minutes.unsigned_abs();
        write!(
            f,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}{sign}{:02}:{:02}",
            self.year,
            self.month,
            self.day,
            self.hour,
            self.minute,
            self.second,
            self.millis,
            offset / 60,
            offset % 60
        )
    }
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn is_leap(year: u16) -> bool {
    let century = year.is_multiple_of(100);
    year.is_multiple_of(4) && (!century || year.is_multiple_of(400))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(offset_seconds: i64) -> LocalTime {
        LocalTime {
            year: 2026,
            month: 11,
            day: 9,
            hour: 18,
            minute: 4,
            second: 5,
            millis: 123,
            offset_seconds,
        }
    }

    #[test]
    fn writes_one_form_and_reads_it_back() {
        let cases = [
            (3600, "2026-11-09T18:04:05.123+01:00"),
            (0, "2026-11-09T18:04:05.123+00:00"),
            (-(5 * 3600 + 30 * 60), "2026-11-09T18:04:05.123-05:30"),
        ];
        for (offset, text) in cases {
            let created = Created::new(local(offset)).unwrap();
            assert_eq!(created.to_string(), text);
            assert_eq!(Created::parse(text), Some(created), "{text}");
        }
    }

    #[test]
    fn names_and_file_stems_use_the_local_time() {
        let created = Created::new(local(3600)).unwrap();
        assert_eq!(created.default_name(), "Recording 2026-11-09 18:04");
        assert_eq!(created.file_stem(), "2026-11-09_18-04-05.123");
    }

    #[test]
    fn other_spellings_and_impossible_times_are_refused() {
        for text in [
            "2026-11-09T18:04:05.123Z",
            "2026-11-09T18:04:05+01:00",
            "2026-11-09 18:04:05.123+01:00",
            "2026-11-09T18:04:05.123+0100",
            "2026-02-29T18:04:05.123+01:00",
            "2026-04-31T18:04:05.123+01:00",
            "2026-13-09T18:04:05.123+01:00",
            "2026-11-09T24:04:05.123+01:00",
            "2026-11-09T18:60:05.123+01:00",
            "2026-11-09T18:04:60.123+01:00",
            "2026-11-09T18:04:05.123+24:00",
            "2026-11-09T18:04:05.123+01:60",
            "2026-11-09T18:04:05.123±01:00",
            "1969-11-09T18:04:05.123+01:00",
            "２026-11-09T18:04:05.123+01:00",
            "",
        ] {
            assert_eq!(Created::parse(text), None, "{text}");
        }
        assert!(Created::parse("2028-02-29T00:00:00.000+00:00").is_some());
        assert!(Created::parse("2000-02-29T00:00:00.000+00:00").is_some());
        assert!(Created::parse("2100-02-29T00:00:00.000+00:00").is_none());
    }

    #[test]
    fn a_leap_second_counts_as_the_one_before() {
        let leap = LocalTime {
            second: 60,
            ..local(0)
        };
        assert_eq!(
            Created::new(leap).unwrap().to_string(),
            "2026-11-09T18:04:59.123+00:00"
        );
    }

    #[test]
    fn a_day_long_offset_is_refused_and_seconds_are_cut() {
        assert_eq!(Created::new(local(24 * 3600)), None);
        assert_eq!(Created::new(local(-24 * 3600)), None);
        assert_eq!(Created::new(local(i64::from(i16::MIN) * 60)), None);
        assert_eq!(Created::new(local(i64::MIN)), None);
        let widest = Created::new(local(-(23 * 3600 + 59 * 60))).unwrap();
        assert_eq!(widest.to_string(), "2026-11-09T18:04:05.123-23:59");
        let odd = Created::new(local(3600 + 59)).unwrap();
        assert_eq!(odd.to_string(), "2026-11-09T18:04:05.123+01:00");
    }
}
