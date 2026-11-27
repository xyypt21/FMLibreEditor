//! The rules the FM27 tools share: monthly CA growth, the age cap, and the squad-depth report.
//!
//! Nothing here reads the game or opens a window. The windows translate what they read into [`squad::Candidate`]s and
//! the model answers what would grow and how the squad is filled, so both windows follow the same rules and the rules
//! can be tested without FM27.

pub mod date;
pub mod settings;
pub mod squad;

pub use date::{Ymd, age_tenths_on};
pub use settings::{
    CapRule, DEFAULT_FORMATION, GrowthSettings, formation_path, growth_cap, growth_target, load_formation,
    load_settings, month_key, save_formation, save_settings, settings_path, update_settings,
};
pub use squad::{
    Candidate, FilledSquad, PositionLine, Sale, SquadEntry, build_squad, counts_from_map, depth_quotas, formation_text,
    parse_formation, position_counts, predict_aca, predict_apa, prepare,
};
