//! Ages in years and tenths, from a birth date and the game's date.

use chrono::{Months, NaiveDate};

/// A calendar date as the game keeps it: a year, a month and a day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ymd {
    pub year: u16,
    pub month: u8,
    pub day: u8,
}

impl Ymd {
    pub fn new(year: u16, month: u8, day: u8) -> Self {
        Self { year, month, day }
    }
}

/// His age in years and tenths: the completed years, plus the days since his last birthday over 365.25, truncated to
/// a tenth. `None` before the birth date, or for a birth date the game cannot hold.
pub fn age_tenths_on(birth: Ymd, today: Ymd) -> Option<f32> {
    if !(1901..2100).contains(&birth.year) {
        return None;
    }
    let birth = naive(birth)?;
    let today = naive(today)?;
    // `years_since` is `None` before the birth date.
    let whole = today.years_since(birth)?;
    // A birthday on 29 February falls on the 28th in a year without one, as `Months` clamps to the month's last day.
    let last_birthday = birth.checked_add_months(Months::new(whole * 12))?;
    let since = (today - last_birthday).num_days();
    let tenths = (f64::from(whole) + since as f64 / 365.25) * 10.0;
    Some((tenths as i32) as f32 / 10.0)
}

fn naive(date: Ymd) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(i32::from(date.year), u32::from(date.month), u32::from(date.day))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_carries_tenths_and_handles_leap_days() {
        let today = Ymd::new(2026, 7, 18);
        let age = |birth: Ymd| age_tenths_on(birth, today);
        assert!((age(Ymd::new(2000, 7, 21)).unwrap() - 25.9).abs() < 0.05); // three days short of 26
        assert!((age(Ymd::new(2000, 7, 18)).unwrap() - 26.0).abs() < 0.05);
        assert_eq!(age(Ymd::new(2027, 1, 1)), None);

        // A birthday on 29 February falls on the 28th in a common year.
        let leap = Ymd::new(2000, 2, 29);
        let age = age_tenths_on(leap, Ymd::new(2025, 2, 28)).unwrap();
        assert!((age - 24.9).abs() < 0.05, "{age}");
        let age = age_tenths_on(leap, Ymd::new(2025, 3, 1)).unwrap();
        assert!((age - 25.0).abs() < 0.05, "{age}");

        // A date the calendar does not have.
        assert_eq!(age_tenths_on(Ymd::new(2025, 2, 30), today), None);
    }
}
