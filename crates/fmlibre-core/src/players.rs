use std::collections::HashMap;

use fmlibre_mem::{Module, Process, is_user_pointer};
use serde::Serialize;

use crate::calibrate::ArrayRef;
use crate::date::FmDate;
use crate::details::{ATTRIBUTE_GROUPS, Attribute, POSITIONS};
use crate::filter::attribute_keys;
use crate::index::{Listed, SearchIndex};
use crate::profile::{Layout, PlayerLayout};
use crate::text::{Memo, display_name, is_name_like, read_fm_string, read_name_entry};

/// The fields shown in player lists.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerSummary {
    pub uid: u32,
    /// Display name: the common name if set, otherwise first and last name.
    pub name: String,
    /// Full birth name, when FM stores one.
    pub full_name: Option<String>,
    pub date_of_birth: Option<FmDate>,
    /// Age on the in-game date, when both dates are known.
    pub age: Option<u16>,
    pub nation: Option<String>,
    /// The nation's UID, e.g. for its flag.
    pub nation_uid: Option<u32>,
    /// Address of the nation object, which the nation field points at.
    #[serde(skip)]
    pub nation_address: u64,
    pub club: Option<String>,
    /// The club's UID, e.g. for its logo.
    pub club_uid: Option<u32>,
    /// Positions rated 15 or more (see [`crate::POSITIONS`] keys), best first.
    pub positions: Vec<&'static str>,
    pub contract_expires: Option<FmDate>,
    pub current_ability: u16,
    pub potential_ability: i16,
    /// All 15 position ratings, in [`crate::POSITIONS`] order.
    #[serde(skip)]
    pub position_ratings: [u8; 15],
    /// The attributes as FM shows them (1–20), in [`crate::ATTRIBUTE_GROUPS`] order, for the list's filters; 0 for
    /// one the layout does not place.
    #[serde(skip)]
    pub attributes: Vec<u8>,
    /// Whether his club has put him on the transfer list, and on the loan list.
    #[serde(skip)]
    pub transfer_listed: bool,
    #[serde(skip)]
    pub loan_listed: bool,
    /// His transfer value in pounds, where the layout places it (see [`PlayerLayout::transfer_value`]).
    #[serde(skip)]
    pub transfer_value: Option<i32>,
    /// Whether this is a woman, for the list's filter; unknown where the layout does not place the gender flags.
    #[serde(skip)]
    pub woman: Option<bool>,
    /// Address of the player object in the game process.
    #[serde(skip)]
    pub address: u64,
    /// How far this object's person block is shifted (see [`PlayerLayout::person_block_shifts`]).
    #[serde(skip)]
    pub shift: u64,
}

impl Listed for PlayerSummary {
    fn uid(&self) -> u32 {
        self.uid
    }

    fn names(&self) -> String {
        match &self.full_name {
            Some(full) => format!("{} {full}", self.name),
            None => self.name.clone(),
        }
    }

    fn display_name(&self) -> &str {
        &self.name
    }

    fn rank(&self) -> i32 {
        i32::from(self.current_ability)
    }
}

/// Players with a search index over their names.
pub type PlayerIndex = SearchIndex<PlayerSummary>;

/// Reads player objects one at a time, with the memoisation a full pass uses. `read_players` reads the players array
/// with it; the transfers agreed for a later day use it to read the players those offers name.
pub(crate) struct PlayerReader<'a> {
    process: &'a Process,
    layout: &'a Layout,
    today: Option<FmDate>,
    plain_len: usize,
    buf: Vec<u8>,
    contract_buf: Vec<u8>,
    attribute_offsets: Vec<Option<u64>>,
    /// The person-block shift is a property of the object's class, so decide it once per vtable.
    shifts: HashMap<u64, u64>,
    /// Many players share first names, clubs and nations; resolve each pointer once.
    names: Memo,
    clubs: Memo,
    club_uids: HashMap<u64, Option<u32>>,
    nations: Memo,
    nation_uids: HashMap<u64, Option<u32>>,
}

impl<'a> PlayerReader<'a> {
    pub(crate) fn new(process: &'a Process, layout: &'a Layout, today: Option<FmDate>) -> Self {
        let p = &layout.player;
        Self {
            process,
            layout,
            today,
            plain_len: p.end() as usize,
            buf: Vec::with_capacity(p.end() as usize + p.max_shift() as usize),
            contract_buf: vec![0u8; layout.contract.end() as usize],
            attribute_offsets: attribute_keys(ATTRIBUTE_GROUPS).map(|key| p.attributes.get(key).copied()).collect(),
            shifts: HashMap::new(),
            names: Memo::default(),
            clubs: Memo::default(),
            club_uids: HashMap::new(),
            nations: Memo::default(),
            nation_uids: HashMap::new(),
        }
    }

    /// Reads the player at `object`; `None` when the object cannot be read.
    pub(crate) fn read(&mut self, object: u64) -> Option<PlayerSummary> {
        let process = self.process;
        let layout = self.layout;
        let p = &layout.player;
        let c = &layout.contract;
        let plain_len = self.plain_len;

        self.buf.resize(plain_len, 0);
        if process.read_into(object, &mut self.buf).is_err() {
            return None;
        }
        let class = u64::from_le_bytes(self.buf[..8].try_into().expect("8 bytes"));
        let shift = *self.shifts.entry(class).or_insert_with(|| person_shift(process, p, object));
        if shift > 0 {
            self.buf.resize(plain_len + shift as usize, 0);
            if process.read_into(object, &mut self.buf).is_err() {
                return None;
            }
        }

        // Read the plain fields first so the buffer borrow ends before the memos are used.
        let (
            uid,
            full_name_ptr,
            common_ptr,
            first_ptr,
            last_ptr,
            team,
            nation_ptr,
            date_of_birth,
            position_ratings,
            attributes,
            has_contract,
            current_ability,
            potential_ability,
            transfer_value,
            woman,
            listing,
        ) = {
            let buf = &self.buf;
            let at = |offset: u64| p.at(offset, shift) as usize;
            let u16_at = |offset: u64| u16::from_le_bytes([buf[at(offset)], buf[at(offset) + 1]]);
            let u32_at = |offset: u64| u32::from_le_bytes(buf[at(offset)..at(offset) + 4].try_into().expect("4 bytes"));
            let u64_at = |offset: u64| u64::from_le_bytes(buf[at(offset)..at(offset) + 8].try_into().expect("8 bytes"));
            let mut position_ratings = [0u8; POSITIONS.len()];
            position_ratings.copy_from_slice(&buf[at(p.positions)..at(p.positions) + POSITIONS.len()]);
            let attributes: Vec<u8> = self
                .attribute_offsets
                .iter()
                .map(|offset| offset.map_or(0, |offset| Attribute::from_raw(buf[at(offset)]).value))
                .collect();
            // The contract in one read: its end date, and the listings.
            let contract = u64_at(p.contract);
            let has_contract = is_user_pointer(contract) && process.read_into(contract, &mut self.contract_buf).is_ok();
            let listing = if has_contract { self.contract_buf[c.listing as usize] } else { 0 };
            (
                u32_at(p.uid),
                u64_at(p.full_name),
                u64_at(p.common_name),
                u64_at(p.first_name),
                u64_at(p.last_name),
                u64_at(p.team),
                u64_at(p.nation),
                FmDate::from_raw(u16_at(p.date_of_birth), u16_at(p.date_of_birth + 2)),
                position_ratings,
                attributes,
                has_contract,
                u16_at(p.current_ability),
                u16_at(p.potential_ability),
                p.transfer_value.map(|offset| u32_at(offset) as i32),
                p.gender.map(|offset| is_woman(buf[at(offset)])),
                listing,
            )
        };

        let full_name = read_fm_string(process, full_name_ptr);
        let mut name_part = |offset| self.names.get(offset, |entry| read_name_entry(process, entry));
        let (common, first, last) = (name_part(common_ptr), name_part(first_ptr), name_part(last_ptr));
        let name = display_name(common, first, last, full_name.as_ref(), uid);

        let club = self.clubs.get(team, |team| club_of_team(process, layout, team));
        let club_uid = *self.club_uids.entry(team).or_insert_with(|| club_uid_of_team(process, layout, team));
        let nation = self.nations.get(nation_ptr, |nation| nation_name(process, layout, nation));
        let nation_uid = *self.nation_uids.entry(nation_ptr).or_insert_with(|| nation_uid(process, layout, nation_ptr));

        let contract_u16 = |offset: u64| {
            u16::from_le_bytes([self.contract_buf[offset as usize], self.contract_buf[offset as usize + 1]])
        };
        let contract_expires =
            has_contract.then(|| FmDate::from_raw(contract_u16(c.expires), contract_u16(c.expires + 2))).flatten();

        Some(PlayerSummary {
            uid,
            name,
            full_name,
            date_of_birth,
            age: date_of_birth.zip(self.today).and_then(|(born, today)| born.age_on(today)),
            nation_uid: nation.as_ref().and(nation_uid),
            nation,
            nation_address: nation_ptr,
            club_uid: club.as_ref().and(club_uid),
            club,
            positions: natural_positions(&position_ratings),
            contract_expires,
            current_ability,
            potential_ability: potential_ability as i16,
            position_ratings,
            attributes,
            transfer_listed: listing & 1 != 0,
            loan_listed: listing & 2 != 0,
            transfer_value,
            woman,
            address: object,
            shift,
        })
    }
}

/// Reads every player in the players array; ages are computed for `today`. Only reads memory.
pub fn read_players(
    process: &Process,
    module: &Module,
    array: &ArrayRef,
    layout: &Layout,
    today: Option<FmDate>,
) -> fmlibre_mem::Result<Vec<PlayerSummary>> {
    let (begin, count) = array.locate(process, module)?;
    let pointers = process.read_bytes(begin, (count * 8) as usize)?;
    let mut reader = PlayerReader::new(process, layout, today);
    let mut players = Vec::with_capacity(count as usize);
    for chunk in pointers.as_chunks::<8>().0 {
        let object = u64::from_le_bytes(*chunk);
        if !is_user_pointer(object) {
            continue;
        }
        if let Some(player) = reader.read(object) {
            players.push(player);
        }
    }
    Ok(players)
}

/// A nation as the editor offers it for a person's nation field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Nation {
    /// Address of the nation object: what a nation field holds.
    pub address: u64,
    /// For its flag.
    pub uid: Option<u32>,
    pub name: String,
}

/// The nations `people` belong to, given as (nation address, name, UID): each once, by name, accents and case
/// ignored. A nation without a name is left out; the game names every nation that has people.
pub fn nations<'a>(people: impl IntoIterator<Item = (u64, Option<&'a str>, Option<u32>)>) -> Vec<Nation> {
    let mut seen: HashMap<u64, Nation> = HashMap::new();
    for (address, name, uid) in people {
        if let Some(name) = name.filter(|_| is_user_pointer(address)) {
            seen.entry(address).or_insert_with(|| Nation { address, uid, name: name.to_owned() });
        }
    }
    let mut out: Vec<Nation> = seen.into_values().collect();
    out.sort_by_cached_key(|nation| (crate::text::fold(&nation.name), nation.address));
    out
}

/// Whether the gender flags of a player or member of staff (see [`crate::PlayerLayout::gender`]) are a woman's. The
/// flag 0x10 is set for every player of a women's club and for no player of a men's club; among staff, for women, and
/// not for the men who work for women's clubs. The other flags are not known.
pub fn is_woman(flags: u8) -> bool {
    flags & 0x10 != 0
}

/// Rating from which FM calls a position one the player can play ("accomplished").
pub const PLAYABLE_RATING: u8 = 15;

/// Positions rated at least [`PLAYABLE_RATING`], best first.
pub(crate) fn natural_positions(ratings: &[u8; POSITIONS.len()]) -> Vec<&'static str> {
    let mut playable: Vec<(u8, usize)> =
        ratings.iter().enumerate().filter(|&(_, &r)| r >= PLAYABLE_RATING).map(|(i, &r)| (r, i)).collect();
    playable.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    playable.into_iter().map(|(_, i)| POSITIONS[i]).collect()
}

/// The name of the club a team belongs to.
pub(crate) fn club_of_team(process: &Process, layout: &Layout, team: u64) -> Option<String> {
    let club = process.read_u64(team + layout.team.club).ok()?;
    read_fm_string(process, process.read_u64(club + layout.club.name).ok()?)
}

/// The UID of the club a team belongs to.
pub(crate) fn club_uid_of_team(process: &Process, layout: &Layout, team: u64) -> Option<u32> {
    if !is_user_pointer(team) {
        return None;
    }
    let club = process.read_u64(team + layout.team.club).ok().filter(|&club| is_user_pointer(club))?;
    process.read_u32(club + layout.club.uid).ok()
}

/// The name of a nation object.
pub(crate) fn nation_name(process: &Process, layout: &Layout, nation: u64) -> Option<String> {
    read_fm_string(process, process.read_u64(nation + layout.nation.name).ok()?)
}

/// The UID of a nation object, e.g. for its flag.
pub(crate) fn nation_uid(process: &Process, layout: &Layout, nation: u64) -> Option<u32> {
    is_user_pointer(nation).then(|| process.read_u32(nation + layout.nation.uid).ok()).flatten()
}

/// Finds where `object` keeps its person block: the candidate shift under which most
/// of the player's name decodes, preferring the smaller shift on a tie.
pub(crate) fn person_shift(process: &Process, p: &PlayerLayout, object: u64) -> u64 {
    let mut best = (0, 0);
    for &shift in &p.person_block_shifts {
        let score = name_score(process, p, object, shift);
        if score > best.0 {
            best = (score, shift);
        }
    }
    best.1
}

/// How many of a player's name parts (full name, first, last) decode under `shift`;
/// zero when the UID is unset.
pub(crate) fn name_score(process: &Process, p: &PlayerLayout, object: u64, shift: u64) -> u32 {
    if !process.read_u32(object + p.at(p.uid, shift)).is_ok_and(|uid| uid != 0) {
        return 0;
    }
    let pointer_at = |offset: u64| process.read_u64(object + p.at(offset, shift)).unwrap_or(0);
    let named = |text: Option<String>| u32::from(text.is_some_and(|t| is_name_like(&t)));
    named(read_fm_string(process, pointer_at(p.full_name)))
        + named(read_name_entry(process, pointer_at(p.first_name)))
        + named(read_name_entry(process, pointer_at(p.last_name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nations_are_listed_once_by_name() {
        let people = [
            (0x2_0000, Some("Türkiye"), Some(1)),
            (0x3_0000, Some("Brazil"), Some(2)),
            (0x2_0000, Some("Türkiye"), Some(1)),
            (0x4_0000, None, Some(3)),
            (0, Some("Nowhere"), None),
            (0x5_0000, Some("Österreich"), Some(4)),
        ];
        let names: Vec<String> = nations(people).into_iter().map(|n| n.name).collect();
        assert_eq!(names, ["Brazil", "Österreich", "Türkiye"]);
    }
}
