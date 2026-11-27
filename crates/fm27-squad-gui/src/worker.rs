//! The thread that owns the game connection. The window sends [`Command`]s and reads [`Event`]s; no memory is read on
//! the window's thread, so a slow read never freezes it.

use std::collections::HashSet;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use fm27_growth_model::{
    Candidate, FilledSquad, GrowthSettings, Ymd, age_tenths_on, build_squad, depth_quotas, parse_formation,
};
use fmlibre_core::{ArraySource, FmDate, Session, find_game};
use fmlibre_mem::Process;

/// How often the squad is read while the window is not paused.
pub const REFRESH: Duration = Duration::from_secs(30);

/// How long to wait before looking for the game again.
const RECONNECT: Duration = Duration::from_secs(3);

/// How often the worker wakes to connect, refresh or take a command.
const TICK: Duration = Duration::from_millis(500);

pub enum Command {
    Refresh,
    Pause(bool),
    Settings(GrowthSettings),
}

pub enum Event {
    Status(String),
    Connected,
    Report(Result<SquadView, String>),
}

/// One report, with the game date and the counts the status line shows.
pub struct SquadView {
    pub squad: FilledSquad,
    pub date: Option<String>,
    /// How many players the controlled teams list, each once.
    pub players: usize,
    pub updated: String,
}

struct Read {
    date: Option<FmDate>,
    candidates: Vec<Candidate>,
}

/// Starts the worker thread and gives the window its ends of the two channels.
pub fn spawn(settings: GrowthSettings) -> (Sender<Command>, Receiver<Event>) {
    let (commands, inbox) = std::sync::mpsc::channel();
    let (events, outbox) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("fm27-squad".to_owned())
        .spawn(move || run(inbox, events, settings))
        .expect("the worker thread starts");
    (commands, outbox)
}

fn run(commands: Receiver<Command>, events: Sender<Event>, mut settings: GrowthSettings) {
    let mut session: Option<Session> = None;
    let mut read: Option<Read> = None;
    let mut paused = false;
    let mut next_connect = Instant::now();
    let mut next_refresh = Instant::now();
    let mut last_connect_error: Option<String> = None;

    loop {
        if session.is_none() && Instant::now() >= next_connect {
            connect(&mut session, &events, &mut last_connect_error);
            if session.is_some() {
                next_refresh = Instant::now();
            } else {
                next_connect = Instant::now() + RECONNECT;
            }
        }

        if session.is_some() && !paused && Instant::now() >= next_refresh {
            read_now(session.as_ref(), &settings, &events, &mut read);
            next_refresh = Instant::now() + REFRESH;
        }

        let command = match commands.recv_timeout(TICK) {
            Ok(command) => Some(command),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break,
        };

        if let Some(command) = command {
            match command {
                Command::Refresh => {
                    if session.is_some() {
                        read_now(session.as_ref(), &settings, &events, &mut read);
                        next_refresh = Instant::now() + REFRESH;
                    }
                }
                Command::Pause(value) => {
                    paused = value;
                    if !paused {
                        next_refresh = Instant::now();
                    }
                }
                Command::Settings(new) => {
                    settings = new;
                    // A formation change does not need a new memory read: the players are the same.
                    if let Some(read) = read.as_ref() {
                        report(read, &settings, &events);
                    }
                }
            }
        }

        // Keep the wait between reads short so the window's own commands are taken promptly.
        if !paused && session.is_some() && Instant::now() >= next_refresh {
            read_now(session.as_ref(), &settings, &events, &mut read);
            next_refresh = Instant::now() + REFRESH;
        }
    }
}

fn connect(session: &mut Option<Session>, events: &Sender<Event>, last_error: &mut Option<String>) {
    match open() {
        Ok(connected) => {
            let summary = summarize(&connected);
            *session = Some(connected);
            *last_error = None;
            let _ = events.send(Event::Connected);
            let _ = events.send(Event::Status(summary));
        }
        Err(error) => {
            *session = None;
            // Retrying every few seconds must not flood the window with the same failure.
            if last_error.as_deref() != Some(error.as_str()) {
                let _ = events.send(Event::Connected);
                let _ = events.send(Event::Status(error.clone()));
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
    let mut lines = vec![format!("Connected: {}", session.game.name())];
    lines.push(match session.game_date() {
        Some(date) => format!("In-game date: {}", date_text(date)),
        None => "In-game date: not readable (is a save loaded?)".to_owned(),
    });
    match session.human_clubs() {
        Ok(clubs) if clubs.is_empty() => lines.push("Human manager clubs: none".to_owned()),
        Ok(clubs) => {
            let names: Vec<&str> = clubs.iter().map(|club| club.name.as_str()).collect();
            lines.push(format!("Human manager clubs: {}", names.join(", ")));
        }
        Err(error) => lines.push(format!("Human manager clubs: {error}")),
    }
    lines.join(" | ")
}

fn read_now(session: Option<&Session>, settings: &GrowthSettings, events: &Sender<Event>, cache: &mut Option<Read>) {
    let Some(session) = session else {
        return;
    };
    match read_squad(session) {
        Ok(read) => {
            report(&read, settings, events);
            *cache = Some(read);
        }
        Err(error) => {
            let _ = events.send(Event::Report(Err(error)));
        }
    }
}

/// Reads the human managers' clubs, their teams and their B teams, with their players once. Only reads memory.
fn read_squad(session: &Session) -> Result<Read, String> {
    let date = session.game_date();
    let clubs = session.human_clubs()?;
    let mut teams: Vec<u64> = Vec::new();
    for club in &clubs {
        teams.extend(session.team_objects_at(club.address, club.uid).unwrap_or_default());
        teams.extend(session.linked_team_addresses(club.address));
    }
    teams.sort_unstable();
    teams.dedup();
    let own: HashSet<u64> = teams.iter().copied().collect();

    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for team in teams {
        for player in session.team_players(team).unwrap_or_default() {
            if !seen.insert(player.uid) {
                continue;
            }
            // FM keeps PA as a range with negative values; the Python tool skips those records too.
            if !(1..=200).contains(&i32::from(player.current_ability))
                || !(1..=200).contains(&i32::from(player.potential_ability))
            {
                continue;
            }
            let age = match (player.date_of_birth, date) {
                (Some(birth), Some(today)) => age_tenths_on(
                    Ymd::new(birth.year, birth.month, birth.day),
                    Ymd::new(today.year, today.month, today.day),
                ),
                _ => None,
            };
            // A player whose current team is not one of ours plays for another club on loan.
            let loaned_out = player.team.is_some_and(|team| !own.contains(&team));
            candidates.push(Candidate {
                uid: player.uid,
                name: player.name,
                age,
                ca: player.current_ability,
                pa: player.potential_ability,
                positions: player.positions.iter().map(|position| (*position).to_owned()).collect(),
                loaned_out,
            });
        }
    }
    Ok(Read { date, candidates })
}

fn report(read: &Read, settings: &GrowthSettings, events: &Sender<Event>) {
    // The window validates the formation as it is typed; this is the last line of defence.
    let formation =
        parse_formation(&settings.formation).and_then(|formation| depth_quotas(&formation).map(|_| formation));
    if let Err(error) = formation {
        let _ = events.send(Event::Report(Err(format!("Formation: {error}"))));
        return;
    }
    match build_squad(&read.candidates, settings) {
        Ok(squad) => {
            let _ = events.send(Event::Report(Ok(SquadView {
                date: read.date.map(date_text),
                players: read.candidates.len(),
                updated: chrono::Local::now().format("%H:%M:%S").to_string(),
                squad,
            })));
        }
        Err(error) => {
            let _ = events.send(Event::Report(Err(error)));
        }
    }
}

fn date_text(date: FmDate) -> String {
    format!("{:04}-{:02}-{:02}", date.year, date.month, date.day)
}
