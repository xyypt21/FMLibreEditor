//! The thread that owns the game connection. The window sends [`Command`]s and reads [`Event`]s; no memory is read on
//! the window's thread, so a slow read never freezes it.
//!
//! On each new in-game month the watcher plans the settlement, writes it and reports what it wrote.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use fmlibre_core::{ArraySource, FmDate, Session, find_game};
use fmlibre_mem::Process;

use crate::growth::{GrowthChange, GrowthSettings, plan_growth, settle};

/// How often the in-game date is checked while the watcher runs.
pub const POLL: Duration = Duration::from_secs(2);

pub enum Command {
    Connect,
    Preview(GrowthSettings),
    Start(GrowthSettings),
    Stop,
}

pub enum Event {
    Log(String),
    Connected(Result<String, String>),
    Plan(Result<PlanView, String>),
    Running(bool),
}

/// What the window shows of one plan.
pub struct PlanView {
    pub date: Option<String>,
    /// How many players the controlled teams list, each once.
    pub considered: usize,
    pub changes: Vec<GrowthChange>,
    /// Whether these changes have been written, as on a settlement, or are only planned.
    pub written: bool,
}

struct Watch {
    settings: GrowthSettings,
    last_month: Option<i32>,
    loads: Option<u64>,
}

/// Starts the worker thread and gives the window its ends of the two channels.
pub fn spawn() -> (Sender<Command>, Receiver<Event>) {
    let (commands, inbox) = std::sync::mpsc::channel();
    let (events, outbox) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("fm27-growth".to_owned())
        .spawn(move || run(inbox, events))
        .expect("the worker thread starts");
    (commands, outbox)
}

fn run(commands: Receiver<Command>, events: Sender<Event>) {
    let mut session: Option<Session> = None;
    let mut watch: Option<Watch> = None;
    loop {
        // While the watcher runs, wake up every couple of seconds to look at the date; otherwise wait for a command.
        let command = if watch.is_some() {
            match commands.recv_timeout(POLL) {
                Ok(command) => Some(command),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match commands.recv() {
                Ok(command) => Some(command),
                Err(_) => break,
            }
        };

        if let Some(command) = command {
            match command {
                Command::Connect => connect(&mut session, &events),
                Command::Preview(settings) => preview(session.as_ref(), &settings, &events),
                Command::Start(settings) => start(session.as_ref(), settings, &mut watch, &events),
                Command::Stop => {
                    if watch.take().is_some() {
                        log(&events, "Stopped.");
                        let _ = events.send(Event::Running(false));
                    }
                }
            }
        }
        if let Some(watch) = watch.as_mut() {
            tick(session.as_ref(), watch, &events);
        }
    }
}

fn connect(session: &mut Option<Session>, events: &Sender<Event>) {
    match open() {
        Ok(connected) => {
            let summary = summarize(&connected);
            *session = Some(connected);
            let _ = events.send(Event::Connected(Ok(summary)));
        }
        Err(error) => {
            *session = None;
            let _ = events.send(Event::Connected(Err(error)));
        }
    }
}

fn open() -> Result<Session, String> {
    let pid = find_game(None).map_err(|error| error.to_string())?;
    let process = Process::open_read_write(pid).map_err(|error| error.to_string())?;
    Session::connect(process, ArraySource::ProfileFirst).map_err(|error| error.to_string())
}

fn summarize(session: &Session) -> String {
    let mut lines = vec![format!("Game: {}", session.game.name())];
    lines.push(match session.game_date() {
        Some(date) => format!("In-game date: {}", date_text(date)),
        None => "In-game date: not readable".to_owned(),
    });
    lines.push(match session.loads() {
        Some(loads) => format!("Saves loaded since the game started: {loads}"),
        None => "Saves loaded: not readable".to_owned(),
    });
    match crate::growth::controlled_clubs(session) {
        Ok(clubs) if clubs.is_empty() => lines.push("Human manager clubs: none (is a save loaded?)".to_owned()),
        Ok(clubs) => {
            let names: Vec<&str> = clubs.iter().map(|club| club.name.as_str()).collect();
            lines.push(format!("Human manager clubs: {}", names.join(", ")));
        }
        Err(error) => lines.push(format!("Staff: {error}")),
    }
    lines.join("\n")
}

fn preview(session: Option<&Session>, settings: &GrowthSettings, events: &Sender<Event>) {
    let Some(session) = session else {
        let _ = events.send(Event::Plan(Err("Connect to the game first.".to_owned())));
        return;
    };
    match plan_growth(session, settings) {
        Ok(plan) => {
            let _ = events.send(Event::Plan(Ok(PlanView {
                date: plan.date.map(date_text),
                considered: plan.considered,
                changes: plan.changes,
                written: false,
            })));
        }
        Err(error) => {
            let _ = events.send(Event::Plan(Err(error)));
        }
    }
}

fn start(session: Option<&Session>, settings: GrowthSettings, watch: &mut Option<Watch>, events: &Sender<Event>) {
    let Some(session) = session else {
        log(events, "Connect to the game first.");
        return;
    };
    if let Err(error) = settings.validate() {
        log(events, &format!("Settings are not valid: {error}"));
        return;
    }
    let Some(date) = session.game_date() else {
        log(events, "Cannot read the in-game date.");
        return;
    };
    let Some(loads) = session.loads() else {
        log(events, "Cannot read the save-load counter.");
        return;
    };
    *watch = Some(Watch { settings, last_month: Some(GrowthSettings::month_key(date)), loads: Some(loads) });
    log(events, &format!("Game date {}; watching. The next month will write CA.", date_text(date)));
    let _ = events.send(Event::Running(true));
}

fn tick(session: Option<&Session>, watch: &mut Watch, events: &Sender<Event>) {
    let Some(session) = session else {
        return;
    };

    // A load is recognised by the game's own counter; the date is not used, as normal play moves it too.
    if let (Some(then), Some(now)) = (watch.loads, session.loads())
        && now != then
    {
        watch.loads = Some(now);
        if let Some(date) = session.game_date() {
            watch.last_month = Some(GrowthSettings::month_key(date));
            log(events, &format!("Save loaded at {}; resetting state.", date_text(date)));
        }
        return;
    }

    let Some(now) = session.game_date() else {
        return;
    };
    let key = GrowthSettings::month_key(now);
    match watch.last_month {
        None => watch.last_month = Some(key),
        Some(last) if key == last => {}
        Some(last) if key < last => {
            watch.last_month = Some(key);
            log(events, &format!("Save loaded at {}; resetting state.", date_text(now)));
        }
        Some(_) => {
            match plan_growth(session, &watch.settings) {
                Ok(plan) => {
                    let count = plan.changes.len();
                    let written = match settle(session, &plan) {
                        Ok((written, failed)) => {
                            log(
                                events,
                                &format!(
                                    "{}: {count} due to grow, {} written, {} failed.",
                                    date_text(now),
                                    written.len(),
                                    failed.len()
                                ),
                            );
                            for line in failed.iter().take(5) {
                                log(events, &format!("  {line}"));
                            }
                            if failed.len() > 5 {
                                log(events, &format!("  ...and {} more failures.", failed.len() - 5));
                            }
                            true
                        }
                        Err(error) => {
                            log(events, &format!("{}: cannot write CA: {error}.", date_text(now)));
                            false
                        }
                    };
                    let _ = events.send(Event::Plan(Ok(PlanView {
                        date: plan.date.map(date_text),
                        considered: plan.considered,
                        changes: plan.changes,
                        written,
                    })));
                }
                Err(error) => log(events, &format!("Cannot plan growth ({error}); retrying.")),
            }
            watch.last_month = Some(key);
        }
    }
}

fn log(events: &Sender<Event>, line: &str) {
    let _ = events.send(Event::Log(line.to_owned()));
}

fn date_text(date: FmDate) -> String {
    format!("{:04}-{:02}-{:02}", date.year, date.month, date.day)
}
