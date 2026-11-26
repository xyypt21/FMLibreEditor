//! The monthly CA growth of the human manager's players, and the plan of what a settlement would change.
//!
//! Each in-game month change adds [`GrowthSettings::monthly_gain`] CA to every player of every team the human
//! managers run, as long as he is younger than [`GrowthSettings::max_age`] and the gain stays within his age cap. The
//! rules are the ones of the FM27 growth tool (`20261118-fm27-tool`): the cap is a share of PA that grows with age,
//! reaching PA itself at `cap.end_age`.
//!
//! This module lives beside the window rather than in `fmlibre-core`, so the shared crate stays as upstream has it and
//! the next merge of upstream is the usual one. It reads only the public API of `fmlibre-core`: the layout says where
//! every field is, and [`Session::process`] reads and writes it. Planning walks the lists of the human managers' teams,
//! not every player in the game.
//!
//! The human managers' clubs come from the training manager's list, which keeps one object per human manager and names
//! his club, so neither the staff nor the club list is read.

use std::collections::{BTreeSet, HashMap};

use chrono::NaiveDate;
use fmlibre_core::{FmDate, Manager, PlayerLayout, Session, is_name_like, read_fm_string, read_name_entry};
use fmlibre_mem::{Process, is_user_pointer};

/// Most human managers a save may list; beyond this the list is not one the editor understands.
const MOST_MANAGERS: u64 = 64;

/// Most teams a club may list; beyond this the list is not one the editor understands.
const MOST_TEAMS: u64 = 16;

/// Most entries a list may hold; beyond this the list is not one the editor understands.
const MOST_POINTERS: u64 = 2_000;

/// The linear age cap: a share of PA from `start_age` to `end_age`, PA itself from `end_age` on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CapRule {
    pub start_age: f64,
    pub start_ratio: f64,
    pub end_age: f64,
    pub end_ratio: f64,
}

/// What a settlement does, and the cap it keeps to.
#[derive(Debug, Clone, PartialEq)]
pub struct GrowthSettings {
    /// CA added each in-game month.
    pub monthly_gain: u16,
    /// No growth at or above this age.
    pub max_age: f64,
    pub cap: CapRule,
}

impl Default for GrowthSettings {
    fn default() -> Self {
        Self {
            monthly_gain: 2,
            max_age: 23.0,
            cap: CapRule { start_age: 18.0, start_ratio: 0.6, end_age: 21.0, end_ratio: 0.9 },
        }
    }
}

impl GrowthSettings {
    /// The settings the tool accepts: a positive gain, a sane age cap, and positive growth spans.
    pub fn validate(&self) -> Result<(), String> {
        if self.monthly_gain == 0 {
            return Err("monthly_gain must be a positive integer".to_owned());
        }
        if !self.max_age.is_finite() || self.max_age < 0.0 {
            return Err("max_age must be a non-negative number".to_owned());
        }
        let cap = &self.cap;
        for (name, value) in [
            ("cap.start_age", cap.start_age),
            ("cap.end_ratio", cap.end_ratio),
            ("cap.start_ratio", cap.start_ratio),
            ("cap.end_age", cap.end_age),
        ] {
            if !value.is_finite() {
                return Err(format!("{name} must be a finite number"));
            }
        }
        if cap.start_age < 0.0 || cap.end_age <= cap.start_age {
            return Err("cap must have 0 <= start_age < end_age".to_owned());
        }
        for (name, ratio) in [("cap.start_ratio", cap.start_ratio), ("cap.end_ratio", cap.end_ratio)] {
            if !(ratio > 0.0 && ratio <= 1.0) {
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

/// The CA ceiling for this age: a share of PA up to `end_age`, PA itself from there on.
pub fn growth_cap(age: f64, pa: i16, cap: &CapRule) -> i32 {
    let ratio = if age <= cap.start_age {
        cap.start_ratio
    } else if age >= cap.end_age {
        1.0
    } else {
        let span = cap.end_age - cap.start_age;
        cap.start_ratio + (cap.end_ratio - cap.start_ratio) * (age - cap.start_age) / span
    };
    (f64::from(pa) * ratio).round().clamp(0.0, f64::from(pa)) as i32
}

/// His CA after a settlement, or `None` when he does not grow: at or past `max_age`, at his cap, or with abilities
/// that are not a real player record (FM keeps PA as a range with negative values).
pub fn growth_target(ca: u16, pa: i16, age: f64, settings: &GrowthSettings) -> Option<u16> {
    if !(1..=200).contains(&i32::from(ca)) || pa <= 0 || pa > 200 {
        return None;
    }
    if i32::from(ca) > i32::from(pa) || age >= settings.max_age {
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
    /// Age in whole years, from the in-game date; `None` when either date is unknown.
    pub age: Option<u16>,
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

/// A player read from a team's list: only what the growth rules need.
struct PlayerRow {
    uid: u32,
    name: String,
    date_of_birth: Option<FmDate>,
    current_ability: u16,
    potential_ability: i16,
    address: u64,
    shift: u64,
}

/// A club a human manager runs: what the plan needs of it, and where its object is.
pub struct ControlledClub {
    pub uid: u32,
    pub name: String,
    pub address: u64,
}

/// The clubs the human managers run, from the training manager's list: one object per human manager, each naming his
/// club at `familiarity.club`. The plan reads these clubs and their teams directly, so it never scans the staff or the
/// club list.
pub fn controlled_clubs(session: &Session) -> Result<Vec<ControlledClub>, String> {
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
    let (c, t) = (&session.layout.club, &session.layout.team);
    let mut clubs: Vec<ControlledClub> = Vec::new();
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
        let Some(club) = read_club(session, address) else {
            continue;
        };
        if clubs.iter().any(|seen| seen.address == club.address) {
            continue;
        }
        // A club lists teams of its own, and each points back at it: the same check the plan makes.
        let own_team = pointers(process, address + c.teams, MOST_TEAMS)
            .into_iter()
            .any(|team| is_user_pointer(team) && process.read_u64(team + t.club).ok() == Some(address));
        if own_team {
            clubs.push(club);
        }
    }
    if clubs.is_empty() && first != last {
        return Err("the training manager's clubs are not ones the editor understands".to_owned());
    }
    Ok(clubs)
}

/// One club object's UID and name, where it looks like a club.
fn read_club(session: &Session, address: u64) -> Option<ControlledClub> {
    let process = &session.process;
    let c = &session.layout.club;
    let uid = process.read_u32(address + c.uid).ok().filter(|&uid| uid != 0)?;
    let name = read_fm_string(process, process.read_u64(address + c.name).ok()?)?;
    is_name_like(&name).then_some(ControlledClub { uid, name, address })
}

/// The plan for every human manager's club: each team's players, deduplicated by UID. Only reads memory.
pub fn plan_growth(session: &Session, settings: &GrowthSettings) -> Result<GrowthPlan, String> {
    settings.validate()?;

    let date = session.game_date();
    let clubs = controlled_clubs(session)?;

    let mut shifts = HashMap::new();
    let mut seen = BTreeSet::new();
    let mut changes = Vec::new();
    for club in &clubs {
        for team in team_objects(session, club.address, club.uid) {
            let roster = team_players(session, team, &mut shifts);
            collect_players(&roster, date, settings, &mut seen, &mut changes);
        }
        // A club's B team is a club of its own, listed among its linked teams.
        for team in linked_team_objects(session, club.address) {
            let roster = team_players(session, team, &mut shifts);
            collect_players(&roster, date, settings, &mut seen, &mut changes);
        }
    }

    changes.sort_by_key(|change| change.age.unwrap_or(u16::MAX));
    Ok(GrowthPlan { date, considered: seen.len(), changes })
}

fn collect_players(
    roster: &[PlayerRow],
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
            (Some(birth), Some(today)) => age_on(birth, today),
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

/// The player objects of a club's teams, in the club's order. A club whose object or list is not what the layout says
/// has no teams.
fn team_objects(session: &Session, address: u64, uid: u32) -> Vec<u64> {
    let process = &session.process;
    let (c, t) = (&session.layout.club, &session.layout.team);
    if process.read_u32(address + c.uid).ok() != Some(uid) {
        return Vec::new();
    }
    let teams = pointers(process, address + c.teams, MOST_TEAMS);
    if teams.len() as u64 > MOST_TEAMS {
        return Vec::new();
    }
    teams
        .into_iter()
        .filter(|&team| is_user_pointer(team) && process.read_u64(team + t.club).ok() == Some(address))
        .collect()
}

/// The senior teams of other clubs that `club` runs (its B team), from its linked team list. A team is left out unless
/// it belongs to another club and that club lists it among its own teams.
fn linked_team_objects(session: &Session, club: u64) -> Vec<u64> {
    let process = &session.process;
    let (c, t) = (&session.layout.club, &session.layout.team);
    let Some(offset) = c.linked_teams else {
        return Vec::new();
    };
    pointers(process, club + offset, MOST_TEAMS)
        .into_iter()
        .filter(|&team| {
            if !is_user_pointer(team) {
                return false;
            }
            let Ok(owner) = process.read_u64(team + t.club) else {
                return false;
            };
            is_user_pointer(owner)
                && owner != club
                && process.read_u32(owner + c.uid).is_ok_and(|uid| uid != 0)
                && pointers(process, owner + c.teams, MOST_TEAMS).contains(&team)
        })
        .collect()
}

/// The players a team lists, read from their objects.
fn team_players(session: &Session, team: u64, shifts: &mut HashMap<u64, u64>) -> Vec<PlayerRow> {
    pointers(&session.process, team + session.layout.team.players, MOST_POINTERS)
        .into_iter()
        .filter_map(|object| read_player(session, object, shifts))
        .collect()
}

/// The pointers a list at `header` holds. A header that does not look like one of the game's lists holds none.
fn pointers(process: &Process, header: u64, longest: u64) -> Vec<u64> {
    let Ok(bytes) = process.read_bytes(header, 24) else {
        return Vec::new();
    };
    let word = |i: usize| u64::from_le_bytes(bytes[i * 8..i * 8 + 8].try_into().expect("8 bytes"));
    let (begin, end, capacity) = (word(0), word(1), word(2));
    let sane = is_user_pointer(begin)
        && begin <= end
        && end <= capacity
        && (end - begin).is_multiple_of(8)
        && (capacity - begin).is_multiple_of(8)
        && capacity - begin <= 8 * longest;
    if !sane || end == begin {
        return Vec::new();
    }
    let Ok(bytes) = process.read_bytes(begin, (end - begin) as usize) else {
        return Vec::new();
    };
    bytes.as_chunks::<8>().0.iter().map(|chunk| u64::from_le_bytes(*chunk)).collect()
}

/// One player's row: the fields the growth rules read, and where his object is. `None` for a pointer that is not a
/// player, or an object whose UID is unset.
fn read_player(session: &Session, object: u64, shifts: &mut HashMap<u64, u64>) -> Option<PlayerRow> {
    if !is_user_pointer(object) {
        return None;
    }
    let process = &session.process;
    let p = &session.layout.player;
    let class = process.read_u64(object).ok()?;
    let shift = *shifts.entry(class).or_insert_with(|| person_shift(process, p, object));

    let at = |offset: u64| object + p.at(offset, shift);
    let u16_at =
        |offset: u64| process.read_bytes(at(offset), 2).ok().map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]));
    let u64_at = |offset: u64| process.read_u64(at(offset)).unwrap_or(0);

    let uid = process.read_u32(at(p.uid)).ok()?;
    if uid == 0 {
        return None;
    }
    let full_name = read_fm_string(process, u64_at(p.full_name));
    let (common, first, last) = (
        read_name_entry(process, u64_at(p.common_name)),
        read_name_entry(process, u64_at(p.first_name)),
        read_name_entry(process, u64_at(p.last_name)),
    );
    let name = display_name(common, first, last, full_name.as_ref(), uid);
    let date_of_birth = FmDate::from_raw(u16_at(p.date_of_birth)?, u16_at(p.date_of_birth + 2)?);
    let current_ability = u16_at(p.current_ability)?;
    let potential_ability = u16_at(p.potential_ability)? as i16;
    Some(PlayerRow { uid, name, date_of_birth, current_ability, potential_ability, address: object, shift })
}

/// The name FM shows for a person: the common name if set, otherwise first and last name, otherwise the full name.
fn display_name(
    common: Option<String>,
    first: Option<String>,
    last: Option<String>,
    full: Option<&String>,
    uid: u32,
) -> String {
    common
        .or_else(|| match (first, last) {
            (Some(first), Some(last)) => Some(format!("{first} {last}")),
            (first, last) => first.or(last),
        })
        .or_else(|| full.cloned())
        .unwrap_or_else(|| format!("UID {uid}"))
}

/// How far this object's person block is shifted (see [`PlayerLayout::person_block_shifts`]).
fn person_shift(process: &Process, p: &PlayerLayout, object: u64) -> u64 {
    let mut best = (0, 0);
    for &shift in &p.person_block_shifts {
        let score = name_score(process, p, object, shift);
        if score > best.0 {
            best = (score, shift);
        }
    }
    best.1
}

/// How many of a player's name parts (full name, first, last) decode under `shift`; zero when the UID is unset.
fn name_score(process: &Process, p: &PlayerLayout, object: u64, shift: u64) -> u32 {
    if !process.read_u32(object + p.at(p.uid, shift)).is_ok_and(|uid| uid != 0) {
        return 0;
    }
    let pointer_at = |offset: u64| process.read_u64(object + p.at(offset, shift)).unwrap_or(0);
    let named = |text: Option<String>| u32::from(text.is_some_and(|t| is_name_like(&t)));
    named(read_fm_string(process, pointer_at(p.full_name)))
        + named(read_name_entry(process, pointer_at(p.first_name)))
        + named(read_name_entry(process, pointer_at(p.last_name)))
}

/// His age in whole years: the completed years, so he turns 26 on his birthday, not before. `None` before the birth
/// date, or for a birth date the game cannot hold.
fn age_on(birth: FmDate, today: FmDate) -> Option<u16> {
    if !(1901..2100).contains(&birth.year) {
        return None;
    }
    let birth = naive(birth)?;
    let today = naive(today)?;
    // `years_since` is `None` before the birth date; a birthday on 29 February falls on the 28th in a year without one.
    today.years_since(birth).map(|years| years as u16)
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
            max_age: 23.0,
            cap: CapRule { start_age: 18.0, start_ratio: 0.6, end_age: 21.0, end_ratio: 0.9 },
        }
    }

    #[test]
    fn cap_follows_age() {
        let cap = settings().cap;
        assert_eq!(growth_cap(17.0, 200, &cap), 120); // 200 × 0.6
        assert_eq!(growth_cap(18.0, 200, &cap), 120);
        assert_eq!(growth_cap(19.5, 200, &cap), 150); // halfway from 0.6 to 0.9
        assert_eq!(growth_cap(21.0, 200, &cap), 200); // end_ratio is not the ceiling from end_age on
        assert_eq!(growth_cap(30.0, 150, &cap), 150);
    }

    #[test]
    fn cap_rounds_half_up() {
        // 0.5 CA is not a real value, but the rounding rule is: up, not to even.
        let cap = CapRule { start_age: 0.0, start_ratio: 0.5, end_age: 1.0, end_ratio: 0.5 };
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
    fn age_is_whole_years_and_handles_leap_days() {
        let today = FmDate { year: 2026, month: 7, day: 18 };
        let age = |birth: FmDate| age_on(birth, today);
        assert_eq!(age(FmDate { year: 2000, month: 7, day: 21 }), Some(25)); // three days short of 26
        assert_eq!(age(FmDate { year: 2000, month: 7, day: 18 }), Some(26));
        assert_eq!(age(FmDate { year: 2027, month: 1, day: 1 }), None);

        // A birthday on 29 February falls on the 28th in a common year.
        let leap = FmDate { year: 2000, month: 2, day: 29 };
        assert_eq!(age_on(leap, FmDate { year: 2025, month: 2, day: 28 }), Some(24));
        assert_eq!(age_on(leap, FmDate { year: 2025, month: 3, day: 1 }), Some(25));

        // A date the calendar does not have.
        assert_eq!(age_on(FmDate { year: 2025, month: 2, day: 30 }, today), None);
    }
}
