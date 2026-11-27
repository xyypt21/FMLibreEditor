//! The monthly CA growth of the human manager's players, and the plan of what a settlement would change.
//!
//! Each in-game month change adds [`GrowthSettings::monthly_gain`] CA to every player of every team the human
//! managers run, as long as he is younger than [`GrowthSettings::max_age`] and the gain stays within his age cap. The
//! cap is a share of PA in steps: [`CapRule::ratio1`] of PA through [`CapRule::age1`], [`CapRule::ratio2`] of PA before
//! [`CapRule::age2`], and PA itself from there on. The rules follow the FM27 growth tool (`20261118-fm27-tool`), except
//! that the cap no longer rises linearly between the two ages.
//!
//! This module keeps only the reads and writes the window needs; the growth rules themselves live in
//! `fm27-growth-model`, shared with the squad window, while the targeted roster reads it uses are `fmlibre-core`'s own
//! (`Session::human_clubs`, `team_objects_at`, `linked_team_addresses` and `team_players`). It neither scans the staff,
//! the club list nor the player array.
//!
//! The human managers' clubs come from the training manager's list, which keeps one object per human manager and names
//! his club.

use std::collections::BTreeSet;

use fm27_growth_model::{Ymd, age_tenths_on, growth_cap, growth_target};
use fmlibre_core::{FmDate, Session, TeamPlayer};

pub use fm27_growth_model::GrowthSettings;

/// The in-game month as a number that orders as the months do; see [`fm27_growth_model::month_key`].
pub fn month_key(date: FmDate) -> i32 {
    fm27_growth_model::month_key(date.year, date.month)
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

/// The plan for every human manager's club: each team's players, deduplicated by UID. Only reads memory.
pub fn plan_growth(session: &Session, settings: &GrowthSettings) -> Result<GrowthPlan, String> {
    settings.validate()?;

    let date = session.game_date();
    let clubs = session.human_clubs()?;

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
            (Some(birth), Some(today)) => age_tenths_on(ymd(birth), ymd(today)),
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

fn ymd(date: FmDate) -> Ymd {
    Ymd::new(date.year, date.month, date.day)
}
