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
use fm27_growth_model::{GrowthSettings, depth_quotas, load_settings, parse_formation, update_settings};

use worker::{Command, Event, SquadView};

const GAP_COLOR: egui::Color32 = egui::Color32::from_rgb(0xb0, 0x00, 0x20);
const COVERED_COLOR: egui::Color32 = egui::Color32::from_rgb(0x1a, 0x7f, 0x37);
const MUTED_COLOR: egui::Color32 = egui::Color32::from_rgb(0x5f, 0x63, 0x68);

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
    formation_text: String,
    formation_error: Option<String>,
    view: Option<SquadView>,
    status: String,
    error: Option<String>,
    paused: bool,
    show_rules: bool,
}

impl App {
    fn new() -> Self {
        let settings = load_settings();
        let (commands, events) = worker::spawn(settings.clone());
        Self {
            commands,
            events,
            formation_text: settings.formation.clone(),
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

    fn apply_formation(&mut self) {
        match parse_formation(&self.formation_text).and_then(|formation| depth_quotas(&formation)) {
            Ok(_) => {
                self.formation_error = None;
                self.settings.formation = self.formation_text.clone();
                self.send(Command::Settings(self.settings.clone()));
            }
            Err(error) => self.formation_error = Some(error),
        }
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

    fn formation_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Formation");
            let response = ui.add(egui::TextEdit::singleline(&mut self.formation_text).desired_width(560.0));
            if response.changed() {
                self.apply_formation();
            }
            if ui.button("Reset").clicked() {
                self.formation_text = fm27_growth_model::DEFAULT_FORMATION.to_owned();
                self.apply_formation();
            }
        });
        if let Some(error) = &self.formation_error {
            ui.colored_label(GAP_COLOR, format!("Formation: {error}"));
        } else {
            ui.label(
                egui::RichText::new(
                    "One `POSITION COUNT` pair per starter or `=`: e.g. `GK 1, WBL 1, WBR 1, DC 3, MC 2, AML 1, AMR 1, ST 1`. \
                     The goalkeeper's quota is 2; every other position gets two per starter plus two.",
                )
                .small()
                .color(MUTED_COLOR),
            );
        }
    }

    fn positions_panel(&mut self, ui: &mut egui::Ui) {
        let Some(view) = &self.view else {
            ui.centered_and_justified(|ui| {
                ui.label("Waiting for the game. The window connects and reads by itself.");
            });
            return;
        };
        let squad = &view.squad;
        let depth: Vec<String> = squad.depth.iter().map(|(position, quota)| format!("{position} {quota}")).collect();
        ui.label(egui::RichText::new(format!("Depth: {}", depth.join(", "))).color(MUTED_COLOR));
        ui.add_space(4.0);

        egui::ScrollArea::vertical().id_salt("positions").auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("positions").num_columns(5).striped(true).spacing([18.0, 5.0]).show(ui, |ui| {
                for heading in ["Position", "Filled", "Highest APA", "Weakest APA", "Status"] {
                    ui.strong(heading);
                }
                ui.end_row();
                for line in &squad.positions {
                    ui.label(&line.position);
                    ui.label(format!("{}/{}", line.count(), line.quota));
                    ui.label(line.highest_apa().map_or_else(|| "-".to_owned(), |apa| apa.to_string()));
                    ui.label(line.weakest_apa().map_or_else(|| "-".to_owned(), |apa| apa.to_string()));
                    let color = if line.missing > 0 || line.count() == 0 || line.count() < line.quota {
                        GAP_COLOR
                    } else if line.weak > 0 && !line.covers.is_empty() {
                        COVERED_COLOR
                    } else {
                        egui::Color32::PLACEHOLDER
                    };
                    let status = line.status();
                    if color == egui::Color32::PLACEHOLDER {
                        ui.label(status);
                    } else {
                        ui.colored_label(color, status);
                    }
                    ui.end_row();
                }
            });
        });

        ui.add_space(6.0);
        let gaps: Vec<String> = squad
            .positions
            .iter()
            .filter(|line| line.missing > 0)
            .map(|line| format!("{} x{}", line.position, line.missing))
            .collect();
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
        ui.horizontal(|ui| {
            ui.strong(format!("Sell candidates ({})", squad.sell.len()));
            ui.label(egui::RichText::new("Players outside the squad whom the sell rules allow").color(MUTED_COLOR));
        });
        egui::ScrollArea::vertical().id_salt("sell").auto_shrink([false, false]).show(ui, |ui| {
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
            "The formation must have 11 starters. The goalkeeper needs 2 players; every other position gets 2 per starter plus 2. \
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
            "The formation is saved when the window closes. The growth settings are shared with the growth window and are \
             edited there.",
        );
    }
}

fn number(value: f32) -> String {
    if value.fract() == 0.0 { format!("{value:.0}") } else { format!("{value}") }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        while let Ok(event) = self.events.try_recv() {
            self.on_event(event);
        }

        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
        egui::Panel::top("formation").show(ui, |ui| self.formation_panel(ui));
        egui::Panel::bottom("sell").resizable(true).default_size(260.0).min_size(120.0).show(ui, |ui| {
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
        // The growth window shares this file, so save only the formation this window owns.
        let formation = self.settings.formation.clone();
        update_settings(|saved| saved.formation = formation);
    }
}
