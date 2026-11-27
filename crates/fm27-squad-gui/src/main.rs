//! A standalone window for FM27 squad depth: ACA/APA, the gaps a formation leaves, and whom the sell rules allow.
//!
//! It connects by itself, reads every 30 seconds, and only reads the game. The rules live in `fm27-growth-model` beside
//! the growth window's, and both windows share one settings file.
//!
//! ```text
//! cargo run -p fm27-squad-gui
//! ```

#![cfg_attr(windows, windows_subsystem = "windows")]

mod worker;

use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use eframe::egui;
use fm27_growth_model::{
    DEFAULT_FORMATION, GrowthSettings, formation_text, load_formation, load_settings, parse_formation, position_counts,
    save_formation,
};

use worker::{Command, Event, SquadView};

const GAP_COLOR: egui::Color32 = egui::Color32::from_rgb(0xb0, 0x00, 0x20);
const COVERED_COLOR: egui::Color32 = egui::Color32::from_rgb(0x1a, 0x7f, 0x37);
const MUTED_COLOR: egui::Color32 = egui::Color32::from_rgb(0x5f, 0x63, 0x68);

/// The positions table, grouped: keepers, defenders, midfield, attack. Every one of the 15 positions appears once.
const POSITION_GROUPS: [(&str, &[&str]); 4] = [
    ("Goalkeepers", &["GK", "SW"]),
    ("Defenders", &["DL", "DC", "DR", "WBL", "WBR"]),
    ("Midfield", &["DM", "MC"]),
    ("Attack", &["ML", "MR", "AML", "AMC", "AMR", "ST"]),
];

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1120.0, 820.0]).with_min_inner_size([900.0, 620.0]),
        ..Default::default()
    };
    eframe::run_native(
        "FM27 Squad Gaps",
        options,
        Box::new(|cc| {
            install_fonts(&cc.egui_ctx);
            Ok(Box::new(App::new()))
        }),
    )
}

/// Adds a Windows CJK font as a fallback, so Chinese club and player names do not render as boxes.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for path in [r"C:\Windows\Fonts\msyh.ttc", r"C:\Windows\Fonts\simhei.ttf", r"C:\Windows\Fonts\simsun.ttc"] {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        fonts.font_data.insert("cjk".to_owned(), egui::FontData::from_owned(bytes).into());
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts.families.entry(family).or_default().push("cjk".to_owned());
        }
        break;
    }
    ctx.set_fonts(fonts);
}

struct App {
    commands: Sender<Command>,
    events: Receiver<Event>,
    settings: GrowthSettings,
    /// The starters of every position, in the game's order; edited in the positions table.
    counts: Vec<(String, u8)>,
    formation_error: Option<String>,
    view: Option<SquadView>,
    status: String,
    error: Option<String>,
    paused: bool,
    show_rules: bool,
}

impl App {
    fn new() -> Self {
        let mut settings = load_settings();
        // The formation file is the source; an older settings file and then the default fill the gap.
        let counts = load_formation()
            .or_else(|| parse_formation(&settings.formation).ok().map(|formation| position_counts(&formation)))
            .unwrap_or_else(default_counts);
        // Keep the worker's copy valid and in the game's order.
        settings.formation = formation_text(&counts);
        let (commands, events) = worker::spawn(settings.clone());
        Self {
            commands,
            events,
            counts,
            settings,
            formation_error: None,
            view: None,
            status: "Looking for the game…".to_owned(),
            error: None,
            paused: false,
            show_rules: false,
        }
    }

    fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    /// Applies the edited starters when they add up to 11; otherwise the last valid formation stays in use.
    fn apply_counts(&mut self) {
        let starters: u32 = self.counts.iter().map(|(_, count)| u32::from(*count)).sum();
        if starters != 11 {
            self.formation_error =
                Some(format!("the starters must add up to 11, not {starters}; the previous formation stays in use"));
            return;
        }
        self.formation_error = None;
        self.settings.formation = formation_text(&self.counts);
        self.send(Command::Settings(self.settings.clone()));
    }

    fn on_event(&mut self, event: Event) {
        match event {
            Event::Status(status) => self.status = status,
            Event::Connected => {}
            Event::Report(Ok(view)) => {
                self.error = None;
                self.view = Some(view);
            }
            Event::Report(Err(error)) => self.error = Some(error),
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let pause = if self.paused { "Resume" } else { "Pause" };
            if ui.button(pause).clicked() {
                self.paused = !self.paused;
                self.send(Command::Pause(self.paused));
            }
            if ui.add_enabled(!self.paused, egui::Button::new("Refresh now")).clicked() {
                self.send(Command::Refresh);
            }
            if ui.button("Squad rules").clicked() {
                self.show_rules = true;
            }
            ui.separator();
            if let Some(error) = &self.error {
                ui.colored_label(GAP_COLOR, error);
            } else if let Some(view) = &self.view {
                let floor = view.squad.floor.map_or_else(|| "-".to_owned(), |floor| floor.to_string());
                let date = view.date.clone().unwrap_or_else(|| "-".to_owned());
                let text = format!(
                    "Updated {} | {} players | {} in squad | floor {floor} | {} loaned out | date {date}",
                    view.updated,
                    view.players,
                    view.squad.unique,
                    view.squad.loaned_out.len()
                );
                ui.label(egui::RichText::new(text).color(MUTED_COLOR));
            } else {
                ui.spinner();
                ui.label(&self.status);
            }
        });
    }

    fn positions_panel(&mut self, ui: &mut egui::Ui) {
        let starters: u32 = self.counts.iter().map(|(_, count)| u32::from(*count)).sum();
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(
                    "Edit Starters in place; the counts must add up to 11. A count of 0 leaves the position unused.",
                )
                .color(MUTED_COLOR),
            );
            let color = if starters == 11 { COVERED_COLOR } else { GAP_COLOR };
            ui.colored_label(color, format!("{starters}/11 starters"));
        });
        if let Some(error) = &self.formation_error {
            ui.colored_label(GAP_COLOR, format!("Formation: {error}"));
        }
        if let Some(view) = &self.view {
            let depth: Vec<String> =
                view.squad.depth.iter().map(|(position, quota)| format!("{position} {quota}")).collect();
            ui.label(egui::RichText::new(format!("Depth: {}", depth.join(", "))).color(MUTED_COLOR));
        }
        ui.add_space(4.0);

        struct Row {
            filled: usize,
            quota: usize,
            highest: Option<i32>,
            weakest: Option<i32>,
            weak: usize,
            missing: usize,
            covered: bool,
            status: String,
        }
        // The last report, matched to the positions in editing order, so the counts can change without borrowing the
        // view while the table is edited.
        let rows: Vec<Option<Row>> = self
            .counts
            .iter()
            .map(|(position, _)| {
                self.view
                    .as_ref()
                    .and_then(|view| view.squad.positions.iter().find(|line| &line.position == position))
                    .map(|line| Row {
                        filled: line.count(),
                        quota: line.quota,
                        highest: line.highest_apa(),
                        weakest: line.weakest_apa(),
                        weak: line.weak,
                        missing: line.missing,
                        covered: !line.covers.is_empty(),
                        status: line.status(),
                    })
            })
            .collect();

        // Keep one line free under the table, so the summary stays visible instead of being pushed out of the window.
        let gaps: Vec<String> = self
            .counts
            .iter()
            .zip(rows.iter())
            .filter_map(|((position, _), row)| {
                let row = row.as_ref()?;
                (row.missing > 0).then(|| format!("{position} x{}", row.missing))
            })
            .collect();
        let footer = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
        let mut changed = false;
        egui::ScrollArea::both()
            .id_salt("positions")
            .auto_shrink([false, true])
            .max_height((ui.available_height() - footer).max(80.0))
            .show(ui, |ui| {
                egui::Grid::new("positions").num_columns(6).striped(true).spacing([12.0, 3.0]).show(ui, |ui| {
                    for heading in ["Position", "Starters", "Filled", "Highest APA", "Weakest APA", "Status"] {
                        ui.strong(heading);
                    }
                    ui.end_row();
                    for (group, positions) in POSITION_GROUPS {
                        ui.label(egui::RichText::new(group).strong().color(MUTED_COLOR));
                        ui.end_row();
                        for position in positions {
                            let Some(index) = self.counts.iter().position(|(key, _)| key == position) else {
                                continue;
                            };
                            ui.label(*position);
                            changed |= ui.add(egui::DragValue::new(&mut self.counts[index].1).range(0..=11)).changed();
                            match &rows[index] {
                                Some(row) => {
                                    ui.label(format!("{}/{}", row.filled, row.quota));
                                    ui.label(row.highest.map_or_else(|| "-".to_owned(), |apa| apa.to_string()));
                                    ui.label(row.weakest.map_or_else(|| "-".to_owned(), |apa| apa.to_string()));
                                    let color = if row.quota == 0 {
                                        MUTED_COLOR
                                    } else if row.missing > 0 || row.filled == 0 || row.filled < row.quota {
                                        GAP_COLOR
                                    } else if row.weak > 0 && row.covered {
                                        COVERED_COLOR
                                    } else {
                                        egui::Color32::PLACEHOLDER
                                    };
                                    if color == egui::Color32::PLACEHOLDER {
                                        ui.label(row.status.as_str());
                                    } else {
                                        ui.colored_label(color, row.status.as_str());
                                    }
                                }
                                None => {
                                    ui.label("-");
                                    ui.label("-");
                                    ui.label("-");
                                    ui.colored_label(MUTED_COLOR, "waiting");
                                }
                            }
                            ui.end_row();
                        }
                    }
                });
            });
        if changed {
            self.apply_counts();
        }

        ui.add_space(6.0);
        if self.view.is_none() {
            ui.colored_label(MUTED_COLOR, "Needs depth: waiting for a game read.");
            return;
        }
        if gaps.is_empty() {
            ui.colored_label(COVERED_COLOR, "Needs depth: none");
        } else {
            ui.colored_label(GAP_COLOR, format!("Needs depth: {}", gaps.join(", ")));
        }
    }

    fn sell_panel(&mut self, ui: &mut egui::Ui) {
        let Some(view) = &self.view else {
            return;
        };
        let squad = &view.squad;
        ui.horizontal_wrapped(|ui| {
            ui.strong(format!("Sell candidates ({})", squad.sell.len()));
            ui.label(egui::RichText::new("Players outside the squad whom the sell rules allow").color(MUTED_COLOR));
        });
        egui::ScrollArea::both().id_salt("sell").auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("sell").num_columns(7).striped(true).spacing([16.0, 4.0]).show(ui, |ui| {
                for heading in ["Name", "Age", "CA", "PA", "ACA", "APA", "Reason"] {
                    ui.strong(heading);
                }
                ui.end_row();
                if squad.sell.is_empty() {
                    ui.label("nobody");
                    ui.end_row();
                }
                for sale in &squad.sell {
                    let player = &sale.player;
                    ui.label(&player.name);
                    ui.label(player.age.map_or_else(|| "-".to_owned(), |age| format!("{age:.1}")));
                    ui.label(player.ca.to_string());
                    ui.label(player.pa.to_string());
                    ui.label(player.aca.to_string());
                    ui.label(player.apa.to_string());
                    ui.label(&sale.reason);
                    ui.end_row();
                }
            });
        });
    }

    fn rules_body(&self, ui: &mut egui::Ui) {
        let settings = &self.settings;
        ui.label(
            "The window reads the human manager's clubs, their teams and B teams, and never writes to the game. \
             It refreshes every 30 seconds; Pause stops the reads, Refresh now reads at once.",
        );
        ui.add_space(6.0);
        ui.strong("ACA and APA");
        ui.label(format!(
            "CA after N years = round(CA + {} × 12 × N). An unknown age or an age at or above {} is just CA.",
            settings.monthly_gain, settings.max_age
        ));
        ui.label(format!(
            "APA = min(PA, CA after (max_age − age), CA after {} years).",
            number(settings.apa_shortfall_years)
        ));
        ui.label(format!(
            "ACA = max(CA, min(CA after {} years, the age cap, APA)). The age cap is the same stepped one the growth window uses.",
            number(settings.aca_growth_years)
        ));
        ui.add_space(6.0);
        ui.strong("Depth");
        ui.label(
            "Edit the Starters column; the counts must add up to 11. Every position is shown, and a count of 0 leaves it unused. \
             The goalkeeper needs 2 players; every other used position gets 2 per starter plus 2. \
             Players are ranked by ACA, then APA; one player can fill every position he can play (rated 15 or more).",
        );
        ui.label(
            "The floor is the highest of the lowest 20% of the selected players' APA. A selected player at or below it is weak. \
             A weak player is covered by a left-over player who can play the position with a higher APA.",
        );
        ui.add_space(6.0);
        ui.strong("Sell");
        ui.label(format!(
            "A left-over player can be sold when he is older than {}, or older than {} with an APA at or below the floor.",
            settings.max_age,
            number(f32::from(settings.max_age) - settings.apa_shortfall_years)
        ));
        ui.label("Players on loan at another club are left out of the squad; players on loan here stay.");
        ui.add_space(6.0);
        ui.strong("Settings");
        ui.label(
            "The formation is read from `%APPDATA%\\fm27-growth\\formation.json` at start and saved there when the \
             window closes. The growth settings are shared with the growth window and are edited there.",
        );
    }
}

fn number(value: f32) -> String {
    if value.fract() == 0.0 { format!("{value:.0}") } else { format!("{value}") }
}

/// The default formation as editable counts.
fn default_counts() -> Vec<(String, u8)> {
    parse_formation(DEFAULT_FORMATION).map_or_else(|_| Vec::new(), |formation| position_counts(&formation))
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        while let Ok(event) = self.events.try_recv() {
            self.on_event(event);
        }

        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
        // Half the width for the sell candidates on the right, the other half for the positions.
        let half = (ui.available_width() / 2.0).max(260.0);
        egui::Panel::right("sell_half").exact_size(half).show(ui, |ui| {
            self.sell_panel(ui);
        });
        egui::CentralPanel::default_margins().show(ui, |ui| self.positions_panel(ui));

        if self.show_rules {
            let mut open = self.show_rules;
            egui::Window::new("Squad rules")
                .open(&mut open)
                .collapsible(false)
                .resizable(true)
                .default_width(560.0)
                .show(ui.ctx(), |ui| self.rules_body(ui));
            self.show_rules = open;
        }

        ui.ctx().request_repaint_after(Duration::from_millis(200));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Save the last valid formation, never an unfinished edit. It is read back at the next start.
        let counts = parse_formation(&self.settings.formation)
            .map_or_else(|_| default_counts(), |formation| position_counts(&formation));
        let _ = save_formation(&counts);
    }
}
