//! The growth settings and the rules that write CA: the age cap in steps, what one settlement adds, and when a player
//! is too old or too close to his PA to grow.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The formation the squad window starts with: eleven starters in the game's position keys, as the user writes them.
pub const DEFAULT_FORMATION: &str = "GK 1, WBL 1, WBR 1, DC 3, MC 2, AML 1, AMR 1, ST 1";

/// The age cap in steps: `ratio1` of PA through `age1`, `ratio2` of PA before `age2`, PA itself from `age2` on. Only the
/// exact age `age1` (and younger) takes `ratio1`; a tenth more takes `ratio2`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CapRule {
    pub age1: u16,
    pub ratio1: f64,
    pub age2: u16,
    pub ratio2: f64,
}

impl Default for CapRule {
    fn default() -> Self {
        Self { age1: 18, ratio1: 0.6, age2: 21, ratio2: 0.9 }
    }
}

/// What a settlement does, the cap it keeps to, and the projections the squad window shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GrowthSettings {
    /// CA added each in-game month.
    pub monthly_gain: u16,
    /// No growth at or above this age.
    pub max_age: u16,
    /// How many years of growth APA may promise, however far `max_age` is.
    pub apa_shortfall_years: f32,
    /// How many years of growth ACA shows now.
    pub aca_growth_years: f32,
    pub cap: CapRule,
    /// The squad window's formation, one `POSITION COUNT` pair per position; a count of 0 leaves it unused.
    pub formation: String,
}

impl Default for GrowthSettings {
    fn default() -> Self {
        Self {
            monthly_gain: 2,
            max_age: 23,
            apa_shortfall_years: 2.0,
            aca_growth_years: 0.5,
            cap: CapRule::default(),
            formation: DEFAULT_FORMATION.to_owned(),
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
        if cap.age2 <= cap.age1 {
            return Err("cap must have age1 < age2".to_owned());
        }
        for (name, ratio) in [("cap.ratio1", cap.ratio1), ("cap.ratio2", cap.ratio2)] {
            if !ratio.is_finite() || !(ratio > 0.0 && ratio <= 1.0) {
                return Err(format!("{name} must be in (0, 1]"));
            }
        }
        for (name, years) in
            [("apa_shortfall_years", self.apa_shortfall_years), ("aca_growth_years", self.aca_growth_years)]
        {
            if !years.is_finite() || years <= 0.0 {
                return Err(format!("{name} must be a positive number"));
            }
        }
        Ok(())
    }
}

/// The in-game month as a number that orders as the months do, e.g. 2026 × 12 + 7 for July 2026.
pub fn month_key(year: u16, month: u8) -> i32 {
    i32::from(year) * 12 + i32::from(month)
}

/// The CA ceiling for this age: `ratio1` of PA through `age1`, `ratio2` until `age2`, PA itself from `age2` on.
pub fn growth_cap(age: f64, pa: i16, cap: &CapRule) -> i32 {
    let ratio = if age <= f64::from(cap.age1) {
        cap.ratio1
    } else if age < f64::from(cap.age2) {
        cap.ratio2
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

/// Where the settings file lives: under `%APPDATA%`, or beside the working directory without it.
pub fn settings_path() -> PathBuf {
    match std::env::var_os("APPDATA") {
        Some(dir) => PathBuf::from(dir).join("fm27-growth").join("settings.json"),
        None => PathBuf::from("fm27-growth-settings.json"),
    }
}

/// The settings on disk, or the defaults when the file is missing or unreadable.
pub fn load_settings() -> GrowthSettings {
    std::fs::read_to_string(settings_path()).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

/// Writes the settings, creating their directory when needed.
pub fn save_settings(settings: &GrowthSettings) {
    let path = settings_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string_pretty(settings) {
        let _ = std::fs::write(path, text);
    }
}

/// Re-reads the settings file, changes what `edit` changes, and writes it back. The two windows share one file, so a
/// window saves only the fields it owns and keeps the other window's.
pub fn update_settings(edit: impl FnOnce(&mut GrowthSettings)) {
    let mut settings = load_settings();
    edit(&mut settings);
    save_settings(&settings);
}

/// Where the squad window keeps its formation: beside the settings file.
pub fn formation_path() -> PathBuf {
    settings_path().with_file_name("formation.json")
}

/// The formation file as all 15 starters, or `None` when it is missing or not a valid formation.
pub fn load_formation() -> Option<Vec<(String, u8)>> {
    let text = std::fs::read_to_string(formation_path()).ok()?;
    let counts: BTreeMap<String, u8> = serde_json::from_str(&text).ok()?;
    crate::squad::counts_from_map(counts).ok()
}

/// Writes all 15 starters, zeros included, creating the directory when needed.
pub fn save_formation(counts: &[(String, u8)]) -> Result<(), String> {
    let path = formation_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    }
    let map: BTreeMap<&str, u8> = counts.iter().map(|(position, count)| (position.as_str(), *count)).collect();
    let text = serde_json::to_string_pretty(&map).map_err(|error| error.to_string())?;
    std::fs::write(path, text).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> GrowthSettings {
        GrowthSettings {
            monthly_gain: 2,
            max_age: 23,
            apa_shortfall_years: 2.0,
            aca_growth_years: 0.5,
            cap: CapRule { age1: 18, ratio1: 0.6, age2: 21, ratio2: 0.9 },
            formation: DEFAULT_FORMATION.to_owned(),
        }
    }

    #[test]
    fn cap_steps_by_age() {
        let cap = settings().cap;
        assert_eq!(growth_cap(17.0, 200, &cap), 120); // 200 × 0.6
        assert_eq!(growth_cap(18.0, 200, &cap), 120);
        assert_eq!(growth_cap(18.1, 200, &cap), 180); // 200 × 0.9, no interpolation
        assert_eq!(growth_cap(18.9, 200, &cap), 180);
        assert_eq!(growth_cap(20.9, 200, &cap), 180);
        assert_eq!(growth_cap(21.0, 200, &cap), 200); // ratio2 is not the ceiling from age2 on
        assert_eq!(growth_cap(30.0, 150, &cap), 150);
    }

    #[test]
    fn cap_rounds_half_up() {
        // 0.5 CA is not a real value, but the rounding rule is: up, not to even.
        let cap = CapRule { age1: 0, ratio1: 0.5, age2: 1, ratio2: 0.5 };
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
        bad.cap.age2 = bad.cap.age1;
        assert!(bad.validate().is_err());
        let mut bad = settings();
        bad.cap.ratio1 = 0.0;
        assert!(bad.validate().is_err());
        let mut bad = settings();
        bad.apa_shortfall_years = 0.0;
        assert!(bad.validate().is_err());
        let mut bad = settings();
        bad.aca_growth_years = f32::NAN;
        assert!(bad.validate().is_err());
    }

    #[test]
    fn settings_survive_json_and_missing_fields_take_defaults() {
        let settings = settings();
        let text = serde_json::to_string(&settings).unwrap();
        assert_eq!(serde_json::from_str::<GrowthSettings>(&text).unwrap(), settings);

        let parsed: GrowthSettings = serde_json::from_str(r#"{"monthly_gain": 3}"#).unwrap();
        assert_eq!(parsed.monthly_gain, 3);
        assert_eq!(parsed.max_age, GrowthSettings::default().max_age);
        assert_eq!(parsed.cap, CapRule::default());
        assert_eq!(parsed.apa_shortfall_years, 2.0);
        assert_eq!(parsed.aca_growth_years, 0.5);
        assert_eq!(parsed.formation, DEFAULT_FORMATION);
    }

    #[test]
    fn months_order_across_years() {
        assert_eq!(month_key(2026, 12), month_key(2027, 1) - 1);
        assert!(month_key(2026, 7) < month_key(2026, 8));
    }
}
