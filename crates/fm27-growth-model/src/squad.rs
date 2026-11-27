//! ACA and APA, the depth quotas of a formation, which players fill them, and whom the sell rules allow.

use crate::settings::{GrowthSettings, growth_cap};

/// The position keys a formation may name, as the game stores them, upper case for the window and the settings text.
const POSITIONS: [&str; 15] =
    ["GK", "SW", "DL", "DC", "DR", "DM", "ML", "MC", "MR", "AML", "AMC", "AMR", "ST", "WBL", "WBR"];

/// The bottom share of the selected players' APA that marks a position short of depth.
const WEAK_APA_SHARE: f64 = 0.2;

/// A goalkeeper needs one backup; every other position gets two players per starter plus two.
const GOALKEEPER_DEPTH: usize = 2;

/// A player as the game read him, before ACA and APA are worked out.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub uid: u32,
    pub name: String,
    /// Age in years and tenths; `None` when the birth date or the game date is unknown.
    pub age: Option<f32>,
    pub ca: u16,
    pub pa: i16,
    /// The positions he can play, in any case; compared upper case.
    pub positions: Vec<String>,
    /// He plays for another club on loan, so he is not part of the squad here.
    pub loaned_out: bool,
}

/// A candidate with his ACA and APA worked out.
#[derive(Debug, Clone, PartialEq)]
pub struct SquadEntry {
    pub uid: u32,
    pub name: String,
    pub age: Option<f32>,
    pub ca: u16,
    pub pa: i16,
    pub positions: Vec<String>,
    pub loaned_out: bool,
    pub aca: i32,
    pub apa: i32,
}

/// One position of the formation: its quota, the players who filled it, and what is missing.
#[derive(Debug, Clone)]
pub struct PositionLine {
    /// The game's position key, upper case.
    pub position: String,
    pub quota: usize,
    /// The players placed here, best ACA first.
    pub chosen: Vec<SquadEntry>,
    /// How many of `chosen` are in the bottom APA slice of the squad.
    pub weak: usize,
    /// Left-over players above the floor who can play here, at most one per weak player.
    pub covers: Vec<SquadEntry>,
    /// How many weak players still have no cover.
    pub missing: usize,
}

impl PositionLine {
    pub fn count(&self) -> usize {
        self.chosen.len()
    }

    pub fn highest_apa(&self) -> Option<i32> {
        self.chosen.iter().map(|player| player.apa).max()
    }

    pub fn weakest_apa(&self) -> Option<i32> {
        self.chosen.iter().map(|player| player.apa).min()
    }

    pub fn weakest_aca(&self) -> Option<i32> {
        self.chosen.last().map(|player| player.aca)
    }

    /// The status the window shows: `not used`, `empty`, `NEEDS DEPTH xN`, `covered by …`, `N short`, or `ok`.
    pub fn status(&self) -> String {
        if self.quota == 0 {
            return "not used".to_owned();
        }
        if self.chosen.is_empty() {
            return "empty".to_owned();
        }
        if self.missing > 0 {
            return format!("NEEDS DEPTH x{}", self.missing);
        }
        if self.weak > 0 && !self.covers.is_empty() {
            return format!("covered by {}", cover_text(&self.covers));
        }
        if self.count() < self.quota {
            return format!("{} short", self.quota - self.count());
        }
        "ok".to_owned()
    }
}

/// One player and the rule that allows selling him.
#[derive(Debug, Clone)]
pub struct Sale {
    pub player: SquadEntry,
    pub reason: String,
}

/// Everything the squad window shows of one read.
#[derive(Debug, Clone)]
pub struct FilledSquad {
    /// The depth quota of every position, in the game's order; a position the formation leaves out has a quota of 0.
    pub depth: Vec<(String, usize)>,
    pub floor: Option<i32>,
    pub positions: Vec<PositionLine>,
    /// Players who filled no quota, best ACA first.
    pub unused: Vec<SquadEntry>,
    pub loaned_out: Vec<SquadEntry>,
    pub sell: Vec<Sale>,
    /// How many distinct players filled at least one position.
    pub unique: usize,
}

/// Works out the whole report: formation, squad, floor, gaps and sell candidates.
pub fn build_squad(candidates: &[Candidate], settings: &GrowthSettings) -> Result<FilledSquad, String> {
    settings.validate()?;
    let formation = parse_formation(&settings.formation)?;
    let depth = depth_quotas(&formation)?;
    let entries = prepare(candidates, settings);

    let (loaned_out, available): (Vec<SquadEntry>, Vec<SquadEntry>) =
        entries.into_iter().partition(|player| player.loaned_out);
    let (filled, unused) = fill_squad(available, &depth);

    let members: Vec<&SquadEntry> = filled.iter().flatten().collect();
    let floor = apa_floor(members.iter().map(|player| player.apa));
    let mut positions = Vec::with_capacity(depth.len());
    for (index, (position, quota)) in depth.iter().enumerate() {
        let chosen = filled[index].clone();
        if chosen.is_empty() {
            positions.push(PositionLine {
                position: position.clone(),
                quota: *quota,
                chosen,
                weak: 0,
                covers: Vec::new(),
                missing: 0,
            });
            continue;
        }
        let weak = match floor {
            Some(floor) => chosen.iter().filter(|player| player.apa <= floor).count(),
            None => 0,
        };
        let covers = internal_cover(position, &unused, floor, weak);
        let missing = weak.saturating_sub(covers.len());
        positions.push(PositionLine { position: position.clone(), quota: *quota, chosen, weak, covers, missing });
    }

    let unique = {
        let mut uids: Vec<u32> = members.iter().map(|player| player.uid).collect();
        uids.sort_unstable();
        uids.dedup();
        uids.len()
    };
    let mut sell = sell_candidates(&unused, floor, settings);
    sell.sort_by(|a, b| b.player.age.unwrap_or(0.0).total_cmp(&a.player.age.unwrap_or(0.0)));

    Ok(FilledSquad { depth, floor, positions, unused, loaned_out, sell, unique })
}

/// Parses the formation text: `POSITION COUNT` pairs separated by commas, semicolons or newlines. The count is the
/// number of starters and may be 0, which leaves the position unused. `=` may stand in for the space. Keys are compared
/// upper case.
pub fn parse_formation(text: &str) -> Result<Vec<(String, u8)>, String> {
    let mut formation = Vec::new();
    for chunk in text.split([',', ';', '\n']) {
        let chunk = chunk.trim();
        if chunk.is_empty() {
            continue;
        }
        let (key, count) = chunk
            .split_once('=')
            .or_else(|| {
                let mut parts = chunk.split_whitespace();
                let key = parts.next()?;
                let count = parts.next()?;
                parts.next().is_none().then_some((key, count))
            })
            .ok_or_else(|| format!("write each position as `GK 1`, not `{chunk}`"))?;
        let key = key.trim().to_uppercase();
        if !POSITIONS.contains(&key.as_str()) {
            return Err(format!("unknown position `{key}`"));
        }
        if formation.iter().any(|(seen, _): &(String, u8)| seen == &key) {
            return Err(format!("`{key}` is named twice"));
        }
        let count: u8 = count.trim().parse().map_err(|_| format!("`{chunk}` needs a whole number of starters"))?;
        formation.push((key, count));
    }
    if formation.is_empty() {
        return Err("the formation is empty".to_owned());
    }
    Ok(formation)
}

/// The starters of every position in the game's order; a position the text leaves out gets 0. Use this to edit a
/// formation one position at a time.
pub fn position_counts(formation: &[(String, u8)]) -> Vec<(String, u8)> {
    POSITIONS
        .iter()
        .map(|position| {
            let count = formation.iter().find(|(key, _)| key == position).map_or(0, |(_, count)| *count);
            ((*position).to_owned(), count)
        })
        .collect()
}

/// Ordered counts for all 15 positions from position/count pairs, as a formation file stores them. A missing position
/// gets 0; unknown positions are an error, and the counts must add up to 11.
pub fn counts_from_map(counts: impl IntoIterator<Item = (String, u8)>) -> Result<Vec<(String, u8)>, String> {
    let mut ordered = position_counts(&[]);
    for (key, count) in counts {
        let key = key.trim().to_uppercase();
        let Some(slot) = ordered.iter_mut().find(|(position, _)| position == &key) else {
            return Err(format!("unknown position `{key}`"));
        };
        slot.1 = count;
    }
    let starters: u32 = ordered.iter().map(|(_, count)| u32::from(*count)).sum();
    if starters != 11 {
        return Err(format!("the starters must add up to 11, not {starters}"));
    }
    Ok(ordered)
}

/// The formation text for counts, in the game's order, leaving out positions with no starters: `GK 1, DC 3, ...`.
pub fn formation_text(counts: &[(String, u8)]) -> String {
    counts
        .iter()
        .filter(|(_, count)| *count > 0)
        .map(|(position, count)| format!("{position} {count}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The depth quota of every position in the game's order: the goalkeeper needs two players, and every other position
/// gets two per starter plus two. A position with no starters gets no quota. The formation must have eleven starters.
pub fn depth_quotas(formation: &[(String, u8)]) -> Result<Vec<(String, usize)>, String> {
    let starters: u32 = formation.iter().map(|(_, count)| u32::from(*count)).sum();
    if starters != 11 {
        return Err(format!("the formation must have 11 starters, not {starters}"));
    }
    Ok(POSITIONS
        .iter()
        .map(|position| {
            let count = formation.iter().find(|(key, _)| key == position).map_or(0, |(_, count)| usize::from(*count));
            let quota = if count == 0 {
                0
            } else if *position == "GK" {
                GOALKEEPER_DEPTH
            } else {
                count * 2 + 2
            };
            ((*position).to_owned(), quota)
        })
        .collect())
}

/// The CA a player reaches after `years` of monthly gains, rounded to an integer.
pub fn ca_after_years(ca: u16, years: f32, monthly_gain: u16) -> i32 {
    (f64::from(ca) + f64::from(monthly_gain) * 12.0 * f64::from(years)).round() as i32
}

/// CA projected to `max_age`, capped by PA and by `apa_shortfall_years`: the shorter projection wins.
pub fn predict_apa(ca: u16, pa: i16, age: Option<f32>, settings: &GrowthSettings) -> i32 {
    let Some(age) = age else {
        return i32::from(ca);
    };
    if age >= f32::from(settings.max_age) {
        return i32::from(ca);
    }
    let to_max_age = ca_after_years(ca, f32::from(settings.max_age) - age, settings.monthly_gain);
    let within_limit = ca_after_years(ca, settings.apa_shortfall_years, settings.monthly_gain);
    i32::from(pa).min(to_max_age).min(within_limit)
}

/// CA plus `aca_growth_years` of growth, capped by the age cap and APA.
pub fn predict_aca(ca: u16, pa: i16, age: Option<f32>, settings: &GrowthSettings) -> i32 {
    let Some(age) = age else {
        return i32::from(ca);
    };
    if age >= f32::from(settings.max_age) {
        return i32::from(ca);
    }
    let cap = growth_cap(f64::from(age), pa, &settings.cap);
    let after_growth = ca_after_years(ca, settings.aca_growth_years, settings.monthly_gain);
    i32::from(ca).max(after_growth.min(cap).min(predict_apa(ca, pa, Some(age), settings)))
}

/// Works out ACA and APA for every candidate, best ACA first, then best APA.
pub fn prepare(candidates: &[Candidate], settings: &GrowthSettings) -> Vec<SquadEntry> {
    let mut entries: Vec<SquadEntry> = candidates
        .iter()
        .map(|candidate| SquadEntry {
            uid: candidate.uid,
            name: candidate.name.clone(),
            age: candidate.age,
            ca: candidate.ca,
            pa: candidate.pa,
            positions: candidate.positions.iter().map(|position| position.to_uppercase()).collect(),
            loaned_out: candidate.loaned_out,
            aca: predict_aca(candidate.ca, candidate.pa, candidate.age, settings),
            apa: predict_apa(candidate.ca, candidate.pa, candidate.age, settings),
        })
        .collect();
    entries.sort_by(|a, b| b.aca.cmp(&a.aca).then(b.apa.cmp(&a.apa)).then(a.uid.cmp(&b.uid)));
    entries
}

/// Fills every quota from the best player down. One player may fill several positions. Returns the filled positions,
/// in the quota order, and the players who filled none.
fn fill_squad(entries: Vec<SquadEntry>, depth: &[(String, usize)]) -> (Vec<Vec<SquadEntry>>, Vec<SquadEntry>) {
    let mut filled: Vec<Vec<SquadEntry>> = depth.iter().map(|_| Vec::new()).collect();
    let mut unused = Vec::new();
    for entry in entries {
        let mut placed = false;
        for (index, (position, quota)) in depth.iter().enumerate() {
            if !entry.positions.contains(position) || filled[index].len() >= *quota {
                continue;
            }
            filled[index].push(entry.clone());
            placed = true;
        }
        if !placed {
            unused.push(entry);
        }
    }
    (filled, unused)
}

/// The APA that separates the bottom slice: the highest of the lowest `ceil(share × count)` APAs. Ties at the floor
/// are all included. `None` without players.
fn apa_floor(values: impl IntoIterator<Item = i32>) -> Option<i32> {
    let mut values: Vec<i32> = values.into_iter().collect();
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let slice = (WEAK_APA_SHARE * values.len() as f64).ceil() as usize;
    Some(values[slice.saturating_sub(1)])
}

/// Left-over players who can play `position` with an APA above the floor, best APA first, at most `wanted`.
fn internal_cover(position: &str, unused: &[SquadEntry], floor: Option<i32>, wanted: usize) -> Vec<SquadEntry> {
    let Some(floor) = floor else {
        return Vec::new();
    };
    if wanted == 0 {
        return Vec::new();
    }
    let mut covers: Vec<SquadEntry> = unused
        .iter()
        .filter(|player| player.positions.iter().any(|can| can == position) && player.apa > floor)
        .cloned()
        .collect();
    covers.sort_by_key(|player| std::cmp::Reverse(player.apa));
    covers.truncate(wanted);
    covers
}

/// The left-over players the sell rules allow, with the reason: older than `max_age`, or older than
/// `max_age - apa_shortfall_years` with an APA at or below the floor. Players without a known age are never listed.
fn sell_candidates(unused: &[SquadEntry], floor: Option<i32>, settings: &GrowthSettings) -> Vec<Sale> {
    let cutoff = f32::from(settings.max_age) - settings.apa_shortfall_years;
    unused
        .iter()
        .filter_map(|player| {
            let age = player.age?;
            let reason = if age > f32::from(settings.max_age) {
                format!("over {} and not in the squad", number(f32::from(settings.max_age)))
            } else if age > cutoff && floor.is_some_and(|floor| player.apa <= floor) {
                format!("over {}, APA {} <= floor {}", number(cutoff), player.apa, floor.unwrap_or_default())
            } else {
                return None;
            };
            Some(Sale { player: player.clone(), reason })
        })
        .collect()
}

/// A number without a trailing `.0`, so the reason reads `over 21` rather than `over 21.0`.
fn number(value: f32) -> String {
    if value.fract() == 0.0 { format!("{value:.0}") } else { format!("{value}") }
}

fn cover_text(covers: &[SquadEntry]) -> String {
    covers.iter().map(|player| format!("{} (APA {})", player.name, player.apa)).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{CapRule, DEFAULT_FORMATION};

    fn settings() -> GrowthSettings {
        GrowthSettings::default()
    }

    fn candidate(uid: u32, name: &str, age: Option<f32>, ca: u16, pa: i16, positions: &[&str]) -> Candidate {
        Candidate {
            uid,
            name: name.to_owned(),
            age,
            ca,
            pa,
            positions: positions.iter().map(|position| (*position).to_owned()).collect(),
            loaned_out: false,
        }
    }

    #[test]
    fn the_default_formation_fills_eleven_starters() {
        let formation = parse_formation(DEFAULT_FORMATION).unwrap();
        let quotas: Vec<_> = depth_quotas(&formation).unwrap();
        assert_eq!(quotas.len(), POSITIONS.len()); // every position is shown, even an unused one
        assert_eq!(quotas.iter().map(|(_, quota)| quota).sum::<usize>(), 2 + 4 + 4 + 8 + 6 + 4 + 4 + 4);
        assert!(quotas.contains(&("GK".to_owned(), 2)));
        assert!(quotas.contains(&("DC".to_owned(), 8)));
        assert!(quotas.contains(&("AMC".to_owned(), 0)));
    }

    #[test]
    fn the_formation_is_checked() {
        assert!(parse_formation("GK 1, DC 3, MC 2, ST 5").is_ok()); // 11 starters
        assert!(parse_formation("GK 1, DC 3, MC 2, ST 5, AML 0").is_ok()); // an explicit zero changes nothing
        assert!(parse_formation("GK 1, DC 3, MC 2, ST 4").and_then(|f| depth_quotas(&f)).is_err()); // 10
        assert!(parse_formation("GK 1, GK 1, DC 3, MC 2, ST 4").is_err()); // twice
        assert!(parse_formation("GK 1, XX 10").is_err()); // unknown
        assert!(parse_formation("GK=1, DC=3, MC=2, ST=5").is_ok());
        assert!(parse_formation("GK, DC 3, MC 2, ST 6").is_err());
        assert!(parse_formation("").is_err());
    }

    #[test]
    fn counts_and_text_round_trip_in_the_game_order() {
        let formation = parse_formation("GK 1, DC 3, MC 2, AML 0, AMC 1, ST 4").unwrap();
        let counts = position_counts(&formation);
        assert_eq!(counts.len(), POSITIONS.len());
        assert_eq!(counts.iter().find(|(position, _)| position == "AML").unwrap().1, 0);
        assert_eq!(counts.iter().find(|(position, _)| position == "AMC").unwrap().1, 1);
        // Zeros are left out of the text; the positions that remain stay in the game's order.
        assert_eq!(formation_text(&counts), "GK 1, DC 3, MC 2, AMC 1, ST 4");
        assert_eq!(position_counts(&parse_formation(&formation_text(&counts)).unwrap()), counts);
    }

    #[test]
    fn counts_from_a_file_are_checked_and_ordered() {
        let counts = counts_from_map([
            ("aml".to_owned(), 0),
            ("AMC".to_owned(), 1),
            ("GK".to_owned(), 1),
            ("DC".to_owned(), 3),
            ("MC".to_owned(), 2),
            ("ST".to_owned(), 4),
        ])
        .unwrap();
        assert_eq!(counts.len(), POSITIONS.len());
        assert_eq!(formation_text(&counts), "GK 1, DC 3, MC 2, AMC 1, ST 4");
        assert!(counts_from_map([("GK".to_owned(), 1), ("XX".to_owned(), 10)]).is_err());
        assert!(counts_from_map([("GK".to_owned(), 1), ("ST".to_owned(), 3)]).is_err());
    }

    #[test]
    fn aca_and_apa_follow_the_growth_rules() {
        let settings = settings(); // gain 2, max_age 23, shortfall 2, growth 0.5
        // 24 CA a year at gain 2.
        assert_eq!(predict_apa(100, 150, Some(21.0), &settings), 148); // two years, under PA
        assert_eq!(predict_apa(100, 130, Some(18.0), &settings), 130); // PA wins
        assert_eq!(predict_apa(100, 150, Some(23.0), &settings), 100); // too old to project
        assert_eq!(predict_apa(100, 150, None, &settings), 100); // unknown age

        assert_eq!(predict_aca(100, 150, Some(18.0), &settings), 100); // cap at 18 is 90, already above
        assert_eq!(predict_aca(80, 150, Some(18.0), &settings), 90); // cap wins over half a year
        assert_eq!(predict_aca(80, 200, Some(21.0), &settings), 92); // half a year is 12 CA, under PA
        assert_eq!(predict_aca(140, 150, Some(21.0), &settings), 150); // limited by PA and APA
        assert_eq!(predict_aca(80, 150, None, &settings), 80);
    }

    #[test]
    fn one_versatile_player_can_fill_several_positions() {
        let settings = settings();
        let players = vec![candidate(1, "Best", Some(20.0), 100, 150, &["mc", "amc", "dm"])];
        let report = build_squad(&players, &settings).unwrap();
        let filled: Vec<_> = report.positions.iter().map(|line| line.count()).collect();
        // Only the positions in the default formation can be filled, and every one by the same player.
        let mc = report.positions.iter().find(|line| line.position == "MC").unwrap();
        let amc = report.positions.iter().find(|line| line.position == "AMC").unwrap();
        assert_eq!(mc.count(), 1);
        assert_eq!(amc.quota, 0);
        assert_eq!(amc.status(), "not used");
        assert_eq!(report.unique, 1);
        assert_eq!(filled.iter().sum::<usize>(), 1);
        assert!(report.unused.is_empty());
    }

    #[test]
    fn a_position_with_no_starters_needs_nobody() {
        let settings = GrowthSettings { formation: "GK 1, DC 4, MC 4, ST 2, AML 0".to_owned(), ..settings() };
        let players = vec![candidate(1, "Winger", Some(20.0), 100, 150, &["aml"])];
        let report = build_squad(&players, &settings).unwrap();
        let aml = report.positions.iter().find(|line| line.position == "AML").unwrap();
        assert_eq!(aml.quota, 0);
        assert_eq!(aml.count(), 0);
        assert_eq!(aml.missing, 0);
        assert_eq!(aml.status(), "not used");
        // He only plays the unused position, so the squad has no room for him.
        assert_eq!(report.unused.len(), 1);
        assert_eq!(report.unique, 0);
    }

    #[test]
    fn players_are_ranked_by_aca_then_apa() {
        let settings = settings();
        let players = vec![
            candidate(1, "Low", Some(22.0), 100, 150, &["mc"]),
            candidate(2, "High", Some(20.0), 120, 150, &["mc"]),
        ];
        let entries = prepare(&players, &settings);
        assert_eq!(entries[0].name, "High");
        assert_eq!(entries[1].name, "Low");
        assert!(entries[0].aca >= entries[1].aca);
    }

    #[test]
    fn the_floor_is_the_bottom_fifth() {
        assert_eq!(apa_floor([10, 20, 30, 40, 50]), Some(10));
        assert_eq!(apa_floor([10, 20, 30, 40, 50, 60]), Some(20)); // ceil(1.2) = 2
        assert_eq!(apa_floor([7]), Some(7));
        assert_eq!(apa_floor(std::iter::empty::<i32>()), None);
        assert_eq!(apa_floor([5, 5, 5, 5, 5]), Some(5)); // ties at the floor are all weak
    }

    #[test]
    fn a_weak_player_needs_one_cover_and_two_need_two() {
        // The goalkeeper quota is 2 whatever the formation, so this leaves one weak player and one cover.
        let settings = GrowthSettings { formation: "GK 11".to_owned(), ..settings() };
        let players = vec![
            candidate(1, "Selected A", Some(20.0), 120, 200, &["gk"]),
            candidate(2, "Selected B", Some(20.0), 115, 150, &["gk"]),
            candidate(3, "Cover", Some(20.0), 104, 160, &["gk"]),
        ];
        let report = build_squad(&players, &settings).unwrap();
        let gk = report.positions.iter().find(|line| line.position == "GK").unwrap();
        assert_eq!(gk.count(), 2);
        assert_eq!(gk.weak, 1); // the floor is the lower of the two APAs
        assert_eq!(gk.covers.len(), 1);
        assert_eq!(gk.missing, 0);
        assert!(gk.status().starts_with("covered by"));
    }

    #[test]
    fn an_empty_position_is_reported_and_loans_are_split() {
        let settings = settings();
        let mut loaned = candidate(9, "Loaned out", Some(20.0), 100, 150, &["st"]);
        loaned.loaned_out = true;
        let report = build_squad(&[loaned], &settings).unwrap();
        let st = report.positions.iter().find(|line| line.position == "ST").unwrap();
        assert_eq!(st.status(), "empty");
        assert_eq!(report.loaned_out.len(), 1);
        assert_eq!(report.unique, 0);
    }

    #[test]
    fn sell_rules() {
        let settings = settings(); // max_age 23, shortfall 2 → cutoff 21
        let mut players = Vec::new();
        // Fill the squad so everyone else is left over.
        for (uid, position) in (1..=8).zip(["gk", "gk", "dc", "dc", "dc", "dc", "mc", "st"]) {
            players.push(candidate(uid, "Squad", Some(20.0), 150, 200, &[position]));
        }
        players.push(candidate(20, "Old", Some(24.0), 100, 150, &["sw"]));
        players.push(candidate(21, "Cutoff", Some(22.0), 20, 60, &["sw"]));
        players.push(candidate(22, "Young", Some(21.0), 20, 60, &["sw"]));
        players.push(candidate(23, "Unknown", None, 20, 60, &["sw"]));
        let report = build_squad(&players, &settings).unwrap();
        let names: Vec<_> = report.sell.iter().map(|sale| sale.player.name.as_str()).collect();
        assert!(names.contains(&"Old"));
        assert!(names.contains(&"Cutoff"));
        assert!(!names.contains(&"Young"));
        assert!(!names.contains(&"Unknown"));
        assert!(
            report.sell[0].player.age.unwrap_or_default() >= report.sell.last().unwrap().player.age.unwrap_or_default()
        );
    }

    #[test]
    fn a_bad_formation_is_an_error() {
        let settings = GrowthSettings { formation: "GK 1".to_owned(), cap: CapRule::default(), ..Default::default() };
        assert!(build_squad(&[], &settings).is_err());
    }
}
