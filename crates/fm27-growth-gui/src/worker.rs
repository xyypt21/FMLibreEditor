//! The thread that owns the game connection. The window sends [`Command`]s and reads [`Event`]s; no memory is read on
//! the window's thread, so a slow read never freezes it.
//!
//! The worker connects on its own, retries until the game is running, previews once as soon as it connects, and, on
//! each new in-game month, plans the settlement, writes it and reports what it wrote.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use fmlibre_core::{ArraySource, FmDate, Session, find_game};
use fmlibre_mem::Process;

use crate::growth::{GrowthChange, GrowthSettings, month_key, plan_growth, settle};

/// How often the in-game date is checked while the watcher runs.
pub const POLL: Duration = Duration::from_secs(2);

/// How long to wait before looking for the game again.
const RECONNECT: Duration = Duration::from_secs(3);

pub enum Command {
    Preview(GrowthSettings),
    Settings(GrowthSettings),
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
pub fn spawn(settings: GrowthSettings) -> (Sender<Command>, Receiver<Event>) {
    let (commands, inbox) = std::sync::mpsc::channel();
    let (events, outbox) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("fm27-growth".to_owned())
        .spawn(move || run(inbox, events, settings))
        .expect("the worker thread starts");
    (commands, outbox)
}

fn run(commands: Receiver<Command>, events: Sender<Event>, mut settings: GrowthSettings) {
    let mut session: Option<Session> = None;
    let mut watch: Option<Watch> = None;
    let mut next_connect = Instant::now();
    let mut last_connect_error: Option<String> = None;
    let mut last_plan_error: Option<String> = None;
    let mut last_start_error: Option<String> = None;
    loop {
        // Connect on the first pass and retry while the game is not running.
        if session.is_none() && Instant::now() >= next_connect {
            connect(&mut session, &events, &mut last_connect_error);
            if session.is_some() {
                // The one automatic preview: at startup, once the game is found.
                preview(session.as_ref(), &settings, &events, &mut last_plan_error);
            } else {
                next_connect = Instant::now() + RECONNECT;
            }
        }

        // Start watching by itself as soon as a save can be watched; retry while the game is at its menus.
        if session.is_some() && watch.is_none() {
            match start(session.as_ref(), settings.clone(), &mut watch) {
                Ok(date) => {
                    last_start_error = None;
                    log(&events, &format!("Game date {}; watching. The next month will write CA.", date_text(date)));
                    let _ = events.send(Event::Running(true));
                }
                Err(error) => {
                    if last_start_error.as_deref() != Some(error.as_str()) {
                        log(&events, &format!("Waiting to watch: {error}."));
                    }
                    last_start_error = Some(error);
                }
            }
        }

        // Wake every couple of seconds to retry connecting or starting, and to poll the in-game date.
        let command = match commands.recv_timeout(POLL) {
            Ok(command) => Some(command),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break,
        };

        if let Some(command) = command {
            match command {
                Command::Preview(new) => {
                    settings = new.clone();
                    preview(session.as_ref(), &new, &events, &mut last_plan_error);
                }
                Command::Settings(new) => {
                    settings = new.clone();
                    if let Some(watch) = watch.as_mut() {
                        watch.settings = new;
                    }
                }
            }
        }

        tick(session.as_ref(), &mut watch, &events);
    }
}

fn connect(session: &mut Option<Session>, events: &Sender<Event>, last_error: &mut Option<String>) {
    match open() {
        Ok(connected) => {
            let summary = summarize(&connected);
            *session = Some(connected);
            *last_error = None;
            let _ = events.send(Event::Connected(Ok(summary)));
        }
        Err(error) => {
            *session = None;
            // Retrying every few seconds must not flood the log with the same failure.
            if last_error.as_deref() != Some(error.as_str()) {
                let _ = events.send(Event::Connected(Err(error.clone())));
            }
            *last_error = Some(error);
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
    match session.human_clubs() {
        Ok(clubs) if clubs.is_empty() => lines.push("Human manager clubs: none (is a save loaded?)".to_owned()),
        Ok(clubs) => {
            let names: Vec<&str> = clubs.iter().map(|club| club.name.as_str()).collect();
            lines.push(format!("Human manager clubs: {}", names.join(", ")));
        }
        Err(error) => lines.push(format!("Human manager clubs: {error}")),
    }
    lines.join("\n")
}

fn preview(
    session: Option<&Session>,
    settings: &GrowthSettings,
    events: &Sender<Event>,
    last_error: &mut Option<String>,
) {
    let Some(session) = session else {
        return;
    };
    match plan_growth(session, settings) {
        Ok(plan) => {
            *last_error = None;
            let _ = events.send(Event::Plan(Ok(PlanView {
                date: plan.date.map(date_text),
                considered: plan.considered,
                changes: plan.changes,
                written: false,
            })));
        }
        Err(error) => {
            // A repeated manual preview must not flood the log with the same failure.
            if last_error.as_deref() != Some(error.as_str()) {
                let _ = events.send(Event::Plan(Err(error.clone())));
            }
            *last_error = Some(error);
        }
    }
}

fn start(session: Option<&Session>, settings: GrowthSettings, watch: &mut Option<Watch>) -> Result<FmDate, String> {
    let session = session.ok_or("not connected")?;
    settings.validate()?;
    let date = session.game_date().ok_or("cannot read the in-game date")?;
    let loads = session.loads().ok_or("cannot read the save-load counter")?;
    *watch = Some(Watch { settings, last_month: Some(month_key(date)), loads: Some(loads) });
    Ok(date)
}

/// What one poll does, decided from plain values so that it can be tested without a game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Nothing to do: the month is unchanged or earlier, or the date cannot be read yet.
    Idle,
    /// The first month read: remember it, but do not write.
    Remember,
    /// The game's load counter changed: forget the month and wait for the next one.
    Reset,
    /// The in-game month moved forward: plan, write and remember the new month.
    Settle,
}

/// The poll's decision. A load is recognised only by the game's own counter; a month that moves back without one
/// writes nothing and is simply ignored.
fn decide(loads: (Option<u64>, Option<u64>), last_month: Option<i32>, key: Option<i32>) -> Step {
    if let (Some(then), Some(now)) = loads
        && now != then
    {
        return Step::Reset;
    }
    match (last_month, key) {
        (None, Some(_)) => Step::Remember,
        (Some(last), Some(key)) if key > last => Step::Settle,
        _ => Step::Idle,
    }
}

/// Checks the in-game date and, on a new month, settles the plan.
fn tick(session: Option<&Session>, watch: &mut Option<Watch>, events: &Sender<Event>) {
    let Some(session) = session else {
        return;
    };
    let Some(state) = watch.as_mut() else {
        return;
    };

    let now_loads = session.loads();
    let date = session.game_date();
    let key = date.map(month_key);
    match decide((state.loads, now_loads), state.last_month, key) {
        Step::Idle => {}
        Step::Remember => {
            state.last_month = key;
        }
        Step::Reset => {
            state.loads = now_loads;
            if let (Some(date), Some(key)) = (date, key) {
                state.last_month = Some(key);
                log(events, &format!("Save loaded at {}; resetting state.", date_text(date)));
            }
        }
        Step::Settle => {
            let Some(now) = date else {
                return;
            };
            let Some(key) = key else {
                return;
            };
            match plan_growth(session, &state.settings) {
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
            state.last_month = Some(key);
        }
    }
}

fn log(events: &Sender<Event>, line: &str) {
    let _ = events.send(Event::Log(line.to_owned()));
}

fn date_text(date: FmDate) -> String {
    format!("{:04}-{:02}-{:02}", date.year, date.month, date.day)
}

#[cfg(test)]
mod tests {
    use super::{Step, decide};

    /// November 2026 as `decide` sees a month.
    const MONTH: i32 = 2026 * 12 + 11;

    #[test]
    fn a_same_month_does_nothing() {
        assert_eq!(decide((Some(3), Some(3)), Some(MONTH), Some(MONTH)), Step::Idle);
    }

    #[test]
    fn the_first_month_is_remembered_without_writing() {
        assert_eq!(decide((Some(3), Some(3)), None, Some(MONTH)), Step::Remember);
    }

    #[test]
    fn a_new_month_settles() {
        assert_eq!(decide((Some(3), Some(3)), Some(MONTH), Some(MONTH + 1)), Step::Settle);
    }

    #[test]
    fn a_backward_month_without_a_load_does_nothing() {
        assert_eq!(decide((Some(3), Some(3)), Some(MONTH + 1), Some(MONTH)), Step::Idle);
    }

    #[test]
    fn a_load_resets_the_month() {
        assert_eq!(decide((Some(3), Some(4)), Some(MONTH), Some(MONTH)), Step::Reset);
    }

    #[test]
    fn a_load_beats_the_month() {
        assert_eq!(decide((Some(3), Some(4)), Some(MONTH), Some(MONTH + 1)), Step::Reset);
    }

    #[test]
    fn an_unreadable_date_does_nothing() {
        assert_eq!(decide((Some(3), Some(3)), Some(MONTH), None), Step::Idle);
    }

    #[test]
    fn an_unreadable_counter_falls_back_to_the_date() {
        assert_eq!(decide((None, None), Some(MONTH), Some(MONTH + 1)), Step::Settle);
    }
}
