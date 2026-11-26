//! A standalone window for the FM27 CA growth: attach to the running game, preview the next monthly settlement and
//! watch the in-game date.
//!
//! It connects and previews by itself, retrying while the game is not running. The watcher starts on request and then
//! writes on each new in-game month.
//!
//! It borrows all of its game knowledge from `fmlibre-core` (build profiles, layouts, players, teams, dates), so it
//! has no offsets of its own and builds without Tauri, the web front end or npm. The growth rules live in
//! [`growth`], beside the window, so `fmlibre-core` stays as upstream has it:
//!
//! ```text
//! cargo run -p fm27-growth-gui
//! ```
//!
//! The workspace's `dev` profile leaves out debug information, so a local build is not the hundreds of megabytes a
//! default debug build of eframe would be.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod growth;
mod worker;

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use eframe::egui;

use growth::GrowthSettings;
use worker::{Command, Event, PlanView};

/// The newest log lines kept: the watcher may run for a long time.
const MOST_LOG_LINES: usize = 2000;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1060.0, 740.0]).with_min_inner_size([840.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native("FM27 CA Growth", options, Box::new(|_cc| Ok(Box::new(App::new()))))
}

struct App {
    commands: Sender<Command>,
    events: Receiver<Event>,
    settings: GrowthSettings,
    sent_settings: GrowthSettings,
    connected: bool,
    running: bool,
    summary: String,
    plan: Option<PlanView>,
    log: VecDeque<String>,
    error: Option<String>,
    show_rules: bool,
}

impl App {
    fn new() -> Self {
        let (commands, events) = worker::spawn();
        let mut app = Self {
            commands,
            events,
            settings: GrowthSettings::default(),
            sent_settings: GrowthSettings::default(),
            connected: false,
            running: false,
            summary: "Waiting for the game…".to_owned(),
            plan: None,
            log: VecDeque::new(),
            error: None,
            show_rules: false,
        };
        app.push_log("Waiting for the game…".to_owned());
        app
    }

    fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    fn push_log(&mut self, line: String) {
        if self.log.len() == MOST_LOG_LINES {
            self.log.pop_front();
        }
        self.log.push_back(line);
    }

    fn on_event(&mut self, event: Event) {
        match event {
            Event::Log(line) => self.push_log(line),
            Event::Connected(Ok(summary)) => {
                self.connected = true;
                self.error = None;
                self.summary = summary;
                self.push_log("Connected.".to_owned());
            }
            Event::Connected(Err(error)) => {
                self.connected = false;
                self.running = false;
                self.plan = None;
                self.summary = "Waiting for the game…".to_owned();
                self.push_log(format!("Connect failed: {error}"));
                self.error = Some(error);
            }
            Event::Plan(Ok(plan)) => {
                self.error = None;
                self.plan = Some(plan);
            }
            Event::Plan(Err(error)) => {
                self.push_log(format!("Plan failed: {error}"));
                self.error = Some(error);
            }
            Event::Running(running) => self.running = running,
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.add_enabled(self.connected && !self.running, egui::Button::new("Start watcher (writes CA)")).clicked()
            {
                self.send(Command::Start(self.settings.clone()));
            }
            if ui.button("Growth rules").clicked() {
                self.show_rules = true;
            }
            ui.separator();
            if self.running {
                ui.colored_label(egui::Color32::from_rgb(0x1a, 0x7f, 0x37), "watching the in-game month");
            } else if self.connected {
                ui.label("connected; preview refreshes automatically");
            } else {
                ui.spinner();
                ui.label("looking for the game…");
            }
            if ui.button("Clear log").clicked() {
                self.log.clear();
            }
        });
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::from_rgb(0xb0, 0x00, 0x20), error);
        }
    }

    fn settings_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Growth settings");
        ui.add_space(4.0);
        egui::Grid::new("settings").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label("Monthly CA gain");
            ui.add(egui::DragValue::new(&mut self.settings.monthly_gain).range(1..=50));
            ui.end_row();

            ui.label("Max age (no growth at or above)");
            ui.add(egui::DragValue::new(&mut self.settings.max_age).speed(1.0).range(0..=60));
            ui.end_row();

            ui.label("Age 1 (ratio 1 through this age)");
            ui.add(egui::DragValue::new(&mut self.settings.cap.age1).speed(1.0).range(0..=60));
            ui.end_row();

            ui.label("Ratio 1 (of PA)");
            ui.add(egui::DragValue::new(&mut self.settings.cap.ratio1).speed(0.1).range(0.01..=1.0).max_decimals(1));
            ui.end_row();

            ui.label("Age 2 (PA from this age)");
            ui.add(egui::DragValue::new(&mut self.settings.cap.age2).speed(1.0).range(0..=60));
            ui.end_row();

            ui.label("Ratio 2 (of PA before age 2)");
            ui.add(egui::DragValue::new(&mut self.settings.cap.ratio2).speed(0.1).range(0.01..=1.0).max_decimals(1));
            ui.end_row();
        });

        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(
                "The watcher writes the planned CA on each new in-game month. The preview refreshes by itself and only reads.",
            )
            .small()
            .weak(),
        );

        ui.separator();
        ui.heading("Connection");
        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
            ui.label(egui::RichText::new(&self.summary).monospace());
        });
    }

    fn plan_panel(&mut self, ui: &mut egui::Ui) {
        let Some(plan) = &self.plan else {
            ui.centered_and_justified(|ui| {
                ui.label("Waiting for the game. The preview refreshes automatically.");
            });
            return;
        };

        let heading = if plan.written { "Settlement changes" } else { "Planned changes" };
        ui.heading(format!("{heading}: {}", plan.changes.len()));
        let mut meta = format!("{} players in the controlled teams", plan.considered);
        if let Some(date) = &plan.date {
            meta.push_str(&format!(" · in-game date {date}"));
        }
        ui.label(egui::RichText::new(meta).weak());
        ui.separator();

        if plan.changes.is_empty() {
            ui.label("No player can grow on the next settlement.");
            return;
        }

        egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("plan").num_columns(6).striped(true).spacing([16.0, 4.0]).show(ui, |ui| {
                for heading in ["Name", "Age", "CA", "New CA", "PA", "Cap"] {
                    ui.strong(heading);
                }
                ui.end_row();

                for change in &plan.changes {
                    ui.label(&change.name);
                    ui.label(change.age.map_or_else(|| "-".to_owned(), |age| format!("{age:.1}")));
                    ui.label(change.ca.to_string());
                    ui.label(change.new_ca.to_string());
                    ui.label(change.pa.to_string());
                    ui.label(change.cap.to_string());
                    ui.end_row();
                }
            });
        });
    }

    fn rules_body(&self, ui: &mut egui::Ui) {
        let settings = &self.settings;
        let cap = &settings.cap;
        ui.label(
            "Players of the human manager's clubs are checked automatically. The preview only reads; the watcher \
             writes when the in-game month changes.",
        );
        ui.add_space(6.0);
        ui.strong("One settlement");
        ui.label(format!(
            "Adds {} CA to each player who can grow, once per in-game month; not on start and not on a save load.",
            settings.monthly_gain
        ));
        ui.add_space(6.0);
        ui.strong("Age cap (share of PA)");
        egui::Grid::new("rules_cap").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
            ui.label(format!("Age {}.0 and below", cap.age1));
            ui.label(format!("{:.0}% of PA", cap.ratio1 * 100.0));
            ui.end_row();
            ui.label(format!("Age {}.1 to {}.9", cap.age1, cap.age2.saturating_sub(1)));
            ui.label(format!("{:.0}% of PA", cap.ratio2 * 100.0));
            ui.end_row();
            ui.label(format!("Age {}.0 and above", cap.age2));
            ui.label("PA itself (100%)");
            ui.end_row();
        });
        ui.label(
            egui::RichText::new(format!(
                "Only exactly {}.0 takes the first step: {}.1 is already in the second. The cap does not interpolate.",
                cap.age1, cap.age1
            ))
            .small()
            .weak(),
        );
        ui.add_space(6.0);
        ui.strong("Other limits");
        ui.label(format!("No growth at or above {} years old.", settings.max_age));
        ui.label("CA and PA must be 1 to 200, and CA must not be above PA.");
        ui.label("A player at his PA does not grow.");
        ui.label("If CA + the monthly gain would pass the cap, that player is skipped; CA is not clamped.");
        ui.add_space(6.0);
        ui.strong("Editing");
        ui.label(
            "Settings apply to the next preview and the next settlement. The window connects and previews by itself.",
        );
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        while let Ok(event) = self.events.try_recv() {
            self.on_event(event);
        }

        // Settings edits reach the worker at once, so the automatic preview reflects them on its next pass.
        if self.settings != self.sent_settings {
            self.sent_settings = self.settings.clone();
            self.send(Command::Settings(self.settings.clone()));
        }

        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("log").resizable(true).default_size(190.0).min_size(80.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong("Log");
                ui.label(egui::RichText::new(format!("{} lines", self.log.len())).weak());
            });
            egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                for line in &self.log {
                    ui.monospace(line);
                }
            });
        });
        egui::Panel::left("settings").resizable(true).default_size(330.0).show(ui, |ui| self.settings_panel(ui));
        egui::CentralPanel::default_margins().show(ui, |ui| self.plan_panel(ui));

        if self.show_rules {
            let mut open = self.show_rules;
            egui::Window::new("Growth rules")
                .open(&mut open)
                .collapsible(false)
                .resizable(true)
                .default_width(470.0)
                .show(ui.ctx(), |ui| self.rules_body(ui));
            self.show_rules = open;
        }

        // The worker's events are polled through a channel: keep looking even when there is no input.
        ui.ctx().request_repaint_after(Duration::from_millis(200));
    }
}
