//! The monthly CA growth of the human manager's players, and the plan of what a settlement would change.
//!
//! Each in-game month change adds [`GrowthSettings::monthly_gain`] CA to every player of every team the human
//! managers run, as long as he is younger than [`GrowthSettings::max_age`] and the gain stays within his age cap. The
//! cap is a share of PA in steps: [`CapRule::start_ratio`] through [`CapRule::start_age`], [`CapRule::end_ratio`] before
//! [`CapRule::end_age`], and PA itself from there on. The rules follow the FM27 growth tool (`20261118-fm27-tool`),
//! except that the cap no longer rises linearly between the two ages.
//!
//! This module lives beside the window rather than in `fmlibre-core`: the growth rules are the tool's, while the
//! targeted roster reads it needs are `fmlibre-core`'s own (`Session::club_at`, `team_objects_at`,
//! `linked_team_addresses` and `team_players`). It neither scans the staff, the club list nor the player array.
//!
//! The human managers' clubs come from the training manager's list, which keeps one object per human manager and names
//! his club.

use std::collections::BTreeSet;

use chrono::{Months, NaiveDate};
use fmlibre_core::{ClubIdentity, FmDate, Manager, Session, TeamPlayer};
use fmlibre_mem::is_user_pointer;

/// Most human managers a save may list; beyond this the list is not one the editor understands.
const MOST_MANAGERS: u64 = 64;

/// The age cap in steps: `start_ratio` of PA through `start_age`, `end_ratio` of PA before `end_age`, PA itself from
/// `end_age` on. Ages count whole years (a player of 18 years and 11 months is in the 18 band).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CapRule {
    pub start_age: u16,
    pub start_ratio: f64,
    pub end_age: u16,
    pub end_ratio: f64,
}

/// What a settlement does, and the cap it keeps to.
#[derive(Debug, Clone, PartialEq)]
pub struct GrowthSettings {
    /// CA added each in-game month.
    pub monthly_gain: u16,
    /// No growth at or above this age.
    pub max_age: u16,
    pub cap: CapRule,
}

impl Default for GrowthSettings {
    fn default() -> Self {
        Self {
            monthly_gain: 2,
            max_age: 23,
            cap: CapRule { start_age: 18, start_ratio: 0.6, end_age: 21, end_ratio: 0.9 },
        }
    }
}

impl GrowthSettings {
    /// The settings the tool accepts: a positive gain, a sane age cap, and positive growth spans.
    pub fn validate(&self) -> Result<(), String> {
        if self.monthly_gain == 0 {
            return Err("monthly_gain must be a positive integer".to_owned());
        }
        let cap = &self.cap;
        if cap.end_age <= cap.start_age {
            return Err("cap must have start_age < end_age".to_owned());
        }
        for (name, ratio) in [("cap.start_ratio", cap.start_ratio), ("cap.end_ratio", cap.end_ratio)] {
            if !ratio.is_finite() || !(ratio > 0.0 && ratio <= 1.0) {
                return Err(format!("{name} must be in (0, 1]"));
            }
        }
        Ok(())
    }

    /// The in-game month as a number that orders as the months do, e.g. 202607 for July 2026.
    pub fn month_key(date: FmDate) -> i32 {
        i32::from(date.year) * 12 + i32::from(date.month)
    }
}

/// The CA ceiling for this age: `start_ratio` of PA through `start_age`, `end_ratio` until `end_age`, PA itself from
/// `end_age` on. Whole years decide the band, so a player of 18 years and 11 months is still capped at 0.6 of PA.
pub fn growth_cap(age: f64, pa: i16, cap: &CapRule) -> i32 {
    let age = age.floor();
    let ratio = if age <= f64::from(cap.start_age) {
        cap.start_ratio
    } else if age < f64::from(cap.end_age) {
        cap.end_ratio
    } else {
        1.0
    };
    (f64::from(pa) * ratio).round().clamp(0.0, f64::from(pa)) as i32
}

/// His CA after a settlement, or `None` when he does not grow: at or past `max_age`, at his cap, or with abilities
/// that are not a real player record (FM keeps PA as a range with negative values).
pub fn growth_target(ca: u16, pa: i16, age: f64, settings: &GrowthSettings) -> Option<u16> {
    if !(1..=200).contains(&i32::from(ca)) || pa <= 0 || pa > 200 {
        return None;
    }
    if i32::from(ca) > i32::from(pa) || age >= f64::from(settings.max_age) {
        return None;
    }
    let cap = growth_cap(age, pa, &settings.cap);
    let new_ca = i32::from(ca) + i32::from(settings.monthly_gain);
    (new_ca <= cap).then_some(new_ca as u16)
}

/// One player's planned change: who he is, what he has, and what the settlement would make of it.
#[derive(Debug, Clone)]
pub struct GrowthChange {
    pub uid: u32,
    pub name: String,
    /// Age in years and tenths, from the in-game date; `None` when either date is unknown.
    pub age: Option<f32>,
    pub ca: u16,
    pub new_ca: u16,
    pub pa: i16,
    /// The CA ceiling for his age.
    pub cap: i32,
    /// Where his object was read, so that the settlement writes without reading the whole player list again.
    pub address: u64,
    pub shift: u64,
}

/// What one settlement would change, read from the game now.
#[derive(Debug, Clone)]
pub struct GrowthPlan {
    pub date: Option<FmDate>,
    /// How many players the controlled teams list, each once, for the status line.
    pub considered: usize,
    /// The changes, youngest first; players whose age is unknown come last.
    pub changes: Vec<GrowthChange>,
}

/// The clubs the human managers run, from the training manager's list: one object per human manager, each naming his
/// club at `familiarity.club`. The plan reads these clubs and their teams directly, so it never scans the staff or the
/// club list.
pub fn controlled_clubs(session: &Session) -> Result<Vec<ClubIdentity>, String> {
    let familiarity = session.layout.familiarity.as_ref().ok_or("this build does not place the training manager")?;
    let place = session.manager_place(Manager::Training).ok_or("the training manager was not found")?;
    let process = &session.process;
    let root = process.read_u64(session.plugin.base + place.rva).map_err(|error| error.to_string())?;
    if !is_user_pointer(root) {
        return Err("the training manager's pointer is not one the editor understands".to_owned());
    }
    let first = process.read_u64(root + familiarity.managers).map_err(|error| error.to_string())?;
    let last = process.read_u64(root + familiarity.managers + 8).map_err(|error| error.to_string())?;
    if !is_user_pointer(first) || last < first || last - first > 8 * MOST_MANAGERS {
        return Err("the training manager's list of managers is not one the editor understands".to_owned());
    }
    let mut clubs: Vec<ClubIdentity> = Vec::new();
    for at in (first..last).step_by(8) {
        let Ok(keeper) = process.read_u64(at) else {
            continue;
        };
        if !is_user_pointer(keeper) {
            continue;
        }
        let Ok(address) = process.read_u64(keeper + familiarity.club) else {
            continue;
        };
        let Some(club) = session.club_at(address) else {
            continue;
        };
        if clubs.iter().any(|seen| seen.address == club.address) {
            continue;
        }
        // A club lists teams of its own: the ones the plan will walk.
        if session.team_objects_at(club.address, club.uid).is_ok_and(|teams| !teams.is_empty()) {
            clubs.push(club);
        }
    }
    if clubs.is_empty() && first != last {
        return Err("the training manager's clubs are not ones the editor understands".to_owned());
    }
    Ok(clubs)
}

/// The plan for every human manager's club: each team's players, deduplicated by UID. Only reads memory.
pub fn plan_growth(session: &Session, settings: &GrowthSettings) -> Result<GrowthPlan, String> {
    settings.validate()?;

    let date = session.game_date();
    let clubs = controlled_clubs(session)?;

    let mut seen = BTreeSet::new();
    let mut changes = Vec::new();
    for club in &clubs {
        for team in session.team_objects_at(club.address, club.uid).unwrap_or_default() {
            if let Ok(roster) = session.team_players(team) {
                collect_players(&roster, date, settings, &mut seen, &mut changes);
            }
        }
        // A club's B team is a club of its own, listed among its linked teams.
        for team in session.linked_team_addresses(club.address) {
            if let Ok(roster) = session.team_players(team) {
                collect_players(&roster, date, settings, &mut seen, &mut changes);
            }
        }
    }

    changes.sort_by(|a, b| a.age.unwrap_or(f32::MAX).total_cmp(&b.age.unwrap_or(f32::MAX)));
    Ok(GrowthPlan { date, considered: seen.len(), changes })
}

fn collect_players(
    roster: &[TeamPlayer],
    date: Option<FmDate>,
    settings: &GrowthSettings,
    seen: &mut BTreeSet<u32>,
    changes: &mut Vec<GrowthChange>,
) {
    for player in roster {
        if !seen.insert(player.uid) {
            continue;
        }
        let age = match (player.date_of_birth, date) {
            (Some(birth), Some(today)) => age_tenths_on(birth, today),
            _ => None,
        };
        let Some(age) = age else {
            continue;
        };
        let Some(new_ca) = growth_target(player.current_ability, player.potential_ability, f64::from(age), settings)
        else {
            continue;
        };
        changes.push(GrowthChange {
            uid: player.uid,
            name: player.name.clone(),
            age: Some(age),
            ca: player.current_ability,
            new_ca,
            pa: player.potential_ability,
            cap: growth_cap(f64::from(age), player.potential_ability, &settings.cap),
            address: player.address,
            shift: player.shift,
        });
    }
}

/// Writes the plan's new current abilities, verifying each write. Returns the changes that were written and the ones
/// that failed. Only writes when the session allows it. Each change carries the address it was read at, so no player
/// list is read again; a player who has moved or whose CA changed since the plan is left out.
pub fn settle(session: &Session, plan: &GrowthPlan) -> Result<(Vec<GrowthChange>, Vec<String>), String> {
    if !session.writes_allowed() {
        return Err("the editor cannot write to this build".to_owned());
    }
    let layout = &session.layout.player;
    let u16_at = |address: u64| {
        session.process.read_bytes(address, 2).ok().map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
    };
    let mut written = Vec::new();
    let mut failed = Vec::new();
    for change in &plan.changes {
        let uid_at = change.address + layout.at(layout.uid, change.shift);
        if session.process.read_u32(uid_at).ok() != Some(change.uid) {
            failed.push(format!("{}: no longer at that address", change.name));
            continue;
        }
        let at = change.address + layout.at(layout.current_ability, change.shift);
        if u16_at(at) != Some(change.ca) {
            failed.push(format!("{}: CA changed since the plan", change.name));
            continue;
        }
        let bytes = change.new_ca.to_le_bytes();
        let written_ok = session.process.write_bytes(at, &bytes).is_ok()
            && session.process.read_bytes(at, 2).is_ok_and(|read| read.as_slice() == bytes);
        if written_ok {
            written.push(change.clone());
        } else {
            failed.push(format!("{}: write failed", change.name));
        }
    }
    Ok((written, failed))
}

/// His age in years and tenths: the completed years, plus the days since his last birthday over 365.25, truncated to
/// a tenth. `None` before the birth date, or for a birth date the game cannot hold.
fn age_tenths_on(birth: FmDate, today: FmDate) -> Option<f32> {
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

fn naive(date: FmDate) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(i32::from(date.year), u32::from(date.month), u32::from(date.day))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> GrowthSettings {
        GrowthSettings {
            monthly_gain: 2,
            max_age: 23,
            cap: CapRule { start_age: 18, start_ratio: 0.6, end_age: 21, end_ratio: 0.9 },
        }
    }

    #[test]
    fn cap_steps_by_whole_age() {
        let cap = settings().cap;
        assert_eq!(growth_cap(17.0, 200, &cap), 120); // 200 × 0.6
        assert_eq!(growth_cap(18.0, 200, &cap), 120);
        assert_eq!(growth_cap(18.9, 200, &cap), 120); // still in the 18 band
        assert_eq!(growth_cap(19.0, 200, &cap), 180); // 200 × 0.9, no interpolation
        assert_eq!(growth_cap(20.9, 200, &cap), 180);
        assert_eq!(growth_cap(21.0, 200, &cap), 200); // end_ratio is not the ceiling from end_age on
        assert_eq!(growth_cap(30.0, 150, &cap), 150);
    }

    #[test]
    fn cap_rounds_half_up() {
        // 0.5 CA is not a real value, but the rounding rule is: up, not to even.
        let cap = CapRule { start_age: 0, start_ratio: 0.5, end_age: 1, end_ratio: 0.5 };
        assert_eq!(growth_cap(0.0, 1, &cap), 1);
    }

    #[test]
    fn only_players_who_can_grow_do() {
        let settings = settings();
        assert_eq!(growth_target(80, 150, 18.0, &settings), Some(82));
        assert_eq!(growth_target(100, 150, 18.0, &settings), None); // the cap at 18 is 90, below CA 100
        assert_eq!(growth_target(140, 150, 22.9, &settings), Some(142)); // cap is PA from 21
        assert_eq!(growth_target(150, 150, 20.0, &settings), None); // at PA
        assert_eq!(growth_target(100, -8, 18.0, &settings), None); // PA is a range
        assert_eq!(growth_target(100, 150, 23.0, &settings), None); // max_age
        assert_eq!(growth_target(180, 150, 18.0, &settings), None); // CA above PA
        assert_eq!(growth_target(0, 150, 18.0, &settings), None);
        assert_eq!(growth_target(100, 201, 18.0, &settings), None);
    }

    #[test]
    fn settings_are_validated() {
        assert!(settings().validate().is_ok());
        let mut bad = settings();
        bad.monthly_gain = 0;
        assert!(bad.validate().is_err());
        let mut bad = settings();
        bad.cap.end_age = bad.cap.start_age;
        assert!(bad.validate().is_err());
        let mut bad = settings();
        bad.cap.start_ratio = 0.0;
        assert!(bad.validate().is_err());
    }

    #[test]
    fn age_carries_tenths_and_handles_leap_days() {
        let today = FmDate { year: 2026, month: 7, day: 18 };
        let age = |birth: FmDate| age_tenths_on(birth, today);
        assert!((age(FmDate { year: 2000, month: 7, day: 21 }).unwrap() - 25.9).abs() < 0.05); // three days short of 26
        assert!((age(FmDate { year: 2000, month: 7, day: 18 }).unwrap() - 26.0).abs() < 0.05);
        assert_eq!(age(FmDate { year: 2027, month: 1, day: 1 }), None);

        // A birthday on 29 February falls on the 28th in a common year.
        let leap = FmDate { year: 2000, month: 2, day: 29 };
        let age = age_tenths_on(leap, FmDate { year: 2025, month: 2, day: 28 }).unwrap();
        assert!((age - 24.9).abs() < 0.05, "{age}");
        let age = age_tenths_on(leap, FmDate { year: 2025, month: 3, day: 1 }).unwrap();
        assert!((age - 25.0).abs() < 0.05, "{age}");

        // A date the calendar does not have.
        assert_eq!(age_tenths_on(FmDate { year: 2025, month: 2, day: 30 }, today), None);
    }
}
