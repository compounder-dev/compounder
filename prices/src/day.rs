//! A calendar day, kept as a count of days since 1970-01-01.

use std::{fmt, str::FromStr};

const SECONDS_IN_A_DAY: i64 = 86_400;

/// One calendar day. It orders and compares as a whole number, and is four
/// bytes wide, so a long history stays small and quick to search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Day(i32);

/// 1990-01-02, the first day a history is asked for. The series we check our
/// numbers against begin here, however far back a ticker really goes.
pub const HISTORY_START: Day = Day(7_306);

impl Day {
    /// The day `number` days after 1970-01-01.
    pub const fn from_number(number: i32) -> Day {
        Day(number)
    }

    /// Days since 1970-01-01.
    pub const fn number(self) -> i32 {
        self.0
    }

    /// The given date, or nothing when the calendar has no such date.
    pub fn from_ymd(year: i32, month: u32, day: u32) -> Option<Day> {
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        let length = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap => 29,
            2 => 28,
            _ => return None,
        };
        if day == 0 || day > length {
            return None;
        }
        // Count in years that start in March, so the leap day comes last.
        let year = if month <= 2 { year - 1 } else { year };
        let era = year.div_euclid(400);
        let year_of_era = year.rem_euclid(400);
        let day_of_year = (153 * ((month as i32 + 9) % 12) + 2) / 5 + day as i32 - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        Some(Day(era * 146_097 + day_of_era - 719_468))
    }

    /// Year, month and day of the month.
    pub fn ymd(self) -> (i32, u32, u32) {
        let shifted = self.0 + 719_468;
        let era = shifted.div_euclid(146_097);
        let day_of_era = shifted.rem_euclid(146_097);
        let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let march_month = (5 * day_of_year + 2) / 153;
        let day = day_of_year - (153 * march_month + 2) / 5 + 1;
        let month = if march_month < 10 { march_month + 3 } else { march_month - 9 };
        let year = year_of_era + era * 400 + i32::from(month <= 2);
        (year, month as u32, day as u32)
    }

    /// The day a moment falls in by the clock in Greenwich, where `seconds`
    /// counts from the start of 1970. A daily bar is stamped at the New York
    /// open, which is the same date in both places all year.
    pub fn from_unix(seconds: i64) -> Day {
        Day(seconds.div_euclid(SECONDS_IN_A_DAY) as i32)
    }

    /// Midnight in Greenwich at the start of this day, in seconds since 1970.
    pub fn unix(self) -> i64 {
        i64::from(self.0) * SECONDS_IN_A_DAY
    }
}

/// Written as `2026-10-09`.
impl fmt::Display for Day {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (year, month, day) = self.ymd();
        write!(formatter, "{year:04}-{month:02}-{day:02}")
    }
}

impl FromStr for Day {
    type Err = String;

    fn from_str(text: &str) -> Result<Day, String> {
        let bad = || format!("{text:?} is not a day written as 2026-10-09");
        let bytes = text.as_bytes();
        if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
            return Err(bad());
        }
        let number = |from: usize, to: usize| {
            bytes[from..to].iter().try_fold(0u32, |sum, byte| byte.is_ascii_digit().then(|| sum * 10 + u32::from(byte - b'0')))
        };
        match (number(0, 4), number(5, 7), number(8, 10)) {
            (Some(year), Some(month), Some(day)) => Day::from_ymd(year as i32, month, day).ok_or_else(bad),
            _ => Err(bad()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_count_from_1970() {
        assert_eq!(Day::from_ymd(1970, 1, 1), Some(Day::from_number(0)));
        assert_eq!(Day::from_ymd(1969, 12, 31), Some(Day::from_number(-1)));
        assert_eq!(Day::from_ymd(1990, 1, 2), Some(HISTORY_START));
        assert_eq!(Day::from_ymd(2026, 10, 9).unwrap().number(), 20_735);
    }

    #[test]
    fn every_day_reads_back_as_the_date_it_was_made_from() {
        let mut expected = Day::from_ymd(1899, 12, 31).unwrap().number();
        for year in 1900..=2100 {
            for month in 1..=12 {
                for day in 1..=31 {
                    let Some(made) = Day::from_ymd(year, month, day) else { continue };
                    expected += 1;
                    assert_eq!(made.number(), expected, "{year}-{month}-{day} does not follow the day before");
                    assert_eq!(made.ymd(), (year, month, day));
                }
            }
        }
    }

    #[test]
    fn leap_days_exist_only_in_leap_years() {
        assert!(Day::from_ymd(2024, 2, 29).is_some());
        assert!(Day::from_ymd(2000, 2, 29).is_some());
        assert!(Day::from_ymd(2023, 2, 29).is_none());
        assert!(Day::from_ymd(1900, 2, 29).is_none());
        assert!(Day::from_ymd(2025, 4, 31).is_none());
        assert!(Day::from_ymd(2025, 13, 1).is_none());
        assert!(Day::from_ymd(2025, 0, 1).is_none());
        assert!(Day::from_ymd(2025, 1, 0).is_none());
    }

    #[test]
    fn days_are_written_and_read_as_iso_dates() {
        let day = Day::from_ymd(2012, 8, 13).unwrap();
        assert_eq!(day.to_string(), "2012-08-13");
        assert_eq!("2012-08-13".parse::<Day>(), Ok(day));
        for wrong in ["2012-8-13", "2012/08/13", "2012-02-30", "12-08-2013", "2012-08-1x", ""] {
            assert!(wrong.parse::<Day>().unwrap_err().contains("is not a day"), "{wrong}");
        }
    }

    #[test]
    fn a_moment_falls_in_the_day_it_belongs_to() {
        // The New York open on 2012-08-13, in summer time and so 13:30 in Greenwich.
        assert_eq!(Day::from_unix(1_344_864_600).to_string(), "2012-08-13");
        assert_eq!(Day::from_unix(-1).to_string(), "1969-12-31");
        let day = Day::from_ymd(2026, 10, 9).unwrap();
        assert_eq!(Day::from_unix(day.unix()), day);
        assert_eq!(Day::from_unix(day.unix() + 86_399), day);
    }
}
