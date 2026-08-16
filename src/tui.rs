//! ratatui views and key handling. The views render state and emit intents, and the
//! loop here is the only part that touches the engine or the disk.

mod render;
mod theme;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tokio::sync::mpsc::{self, UnboundedSender};

use crate::config::Settings;
use crate::engine::{DownloadId, Engine, Input, Progress, State};
use crate::search::{self, Category, Outcome, Torrent, magnet_link};

/// The whole product.
pub async fn run(settings: Settings) -> Result<()> {
    let engine = Arc::new(Engine::start(&settings).await?);
    let app = App::new(settings, Some(engine));
    drive(app).await
}

/// The Settings page on its own. It starts no session, so a server running BAKA
/// elsewhere keeps its port and its downloads while this edits the same file.
pub async fn settings_page(settings: Settings) -> Result<()> {
    let mut app = App::new(settings, None);
    app.tab = Tab::Settings;
    app.settings_only = true;
    // Written on the way out, so anyone who would rather use an editor has a file.
    app.unsaved = true;
    drive(app).await
}

async fn drive(app: App) -> Result<()> {
    let mut terminal = ratatui::try_init()?;
    let outcome = event_loop(&mut terminal, app).await;
    ratatui::restore();
    outcome
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Search,
    Downloads,
    Seeding,
    Settings,
}

impl Tab {
    const ALL: [Self; 4] = [Self::Search, Self::Downloads, Self::Seeding, Self::Settings];

    fn title(self) -> &'static str {
        match self {
            Self::Search => "Search",
            Self::Downloads => "Downloads",
            Self::Seeding => "Seeding",
            Self::Settings => "Settings",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|tab| *tab == self).unwrap_or(0)
    }

    fn shifted(self, by: isize) -> Self {
        let count = Self::ALL.len() as isize;
        Self::ALL[((self.index() as isize + by).rem_euclid(count)) as usize]
    }
}

/// What a key means depends only on what is currently open, so the mapping below can
/// be a plain function and can be tested without a terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Browsing,
    Typing,
    Editing,
    Asking,
    Confirming,
    Helping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Quit,
    NextTab,
    PrevTab,
    Go(Tab),
    Move(isize),
    FocusQuery,
    Submit,
    Download { pick_folder: bool },
    PauseResume,
    Stop,
    CopyMagnet,
    Nudge(bool),
    BeginEdit,
    Type(char),
    Backspace,
    Yes,
    Help,
    Dismiss,
}

enum Event {
    Key(KeyEvent),
    Redraw,
    Tick,
    Searched(Box<Result<Outcome>>),
    Notice(String),
}

struct Prompt {
    title: String,
    folder: String,
    input: Input,
}

struct Question {
    text: String,
    id: DownloadId,
}

struct App {
    settings: Settings,
    engine: Option<Arc<Engine>>,
    events: Option<UnboundedSender<Event>>,
    clipboard: Option<arboard::Clipboard>,
    settings_only: bool,
    tab: Tab,
    cursor: [usize; 4],
    typing: bool,
    query: String,
    results: Vec<Torrent>,
    failures: Vec<String>,
    searching: bool,
    torrents: Vec<Progress>,
    editor: Option<String>,
    prompt: Option<Prompt>,
    question: Option<Question>,
    helping: bool,
    notice: Option<String>,
    unsaved: bool,
    quit: bool,
}

impl App {
    fn new(settings: Settings, engine: Option<Arc<Engine>>) -> Self {
        Self {
            settings,
            engine,
            events: None,
            clipboard: None,
            settings_only: false,
            tab: Tab::Search,
            cursor: [0; 4],
            typing: true,
            query: String::new(),
            results: Vec::new(),
            failures: Vec::new(),
            searching: false,
            torrents: Vec::new(),
            editor: None,
            prompt: None,
            question: None,
            helping: false,
            notice: None,
            unsaved: false,
            quit: false,
        }
    }

    fn mode(&self) -> Mode {
        if self.helping {
            Mode::Helping
        } else if self.question.is_some() {
            Mode::Confirming
        } else if self.prompt.is_some() {
            Mode::Asking
        } else if self.editor.is_some() {
            Mode::Editing
        } else if self.typing {
            Mode::Typing
        } else {
            Mode::Browsing
        }
    }

    fn downloads(&self) -> Vec<&Progress> {
        self.torrents.iter().filter(|t| !t.finished).collect()
    }

    fn seeds(&self) -> Vec<&Progress> {
        self.torrents.iter().filter(|t| t.finished).collect()
    }

    fn rows(&mut self) -> usize {
        match self.tab {
            Tab::Search => self.results.len(),
            Tab::Downloads => self.downloads().len(),
            Tab::Seeding => self.seeds().len(),
            Tab::Settings => self.settings.fields().len(),
        }
    }

    fn at(&self) -> usize {
        self.cursor[self.tab.index()]
    }

    fn selected_torrent(&self) -> Option<&Progress> {
        let list = match self.tab {
            Tab::Downloads => self.downloads(),
            Tab::Seeding => self.seeds(),
            _ => return None,
        };
        list.get(self.at()).copied()
    }

    fn say(&mut self, message: impl Into<String>) {
        self.notice = Some(message.into());
    }
}

async fn event_loop(terminal: &mut DefaultTerminal, mut app: App) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    app.events = Some(tx.clone());

    let keys = tx.clone();
    std::thread::spawn(move || {
        while let Ok(event) = event::read() {
            let sent = match event {
                event::Event::Key(key) => keys.send(Event::Key(key)),
                _ => keys.send(Event::Redraw),
            };
            if sent.is_err() {
                break;
            }
        }
    });

    let ticks = tx.clone();
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_secs(1));
        while ticks.send(Event::Tick).is_ok() {
            timer.tick().await;
        }
    });

    terminal.draw(|frame| render::draw(frame, &mut app))?;
    while let Some(event) = rx.recv().await {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if let Some(action) = map_key(key, app.mode(), app.tab) {
                    act(&mut app, action).await;
                }
            }
            Event::Key(_) | Event::Redraw => {}
            Event::Tick => tick(&mut app).await,
            Event::Searched(outcome) => finish_search(&mut app, *outcome),
            Event::Notice(message) => app.say(message),
        }
        if app.quit {
            break;
        }
        terminal.draw(|frame| render::draw(frame, &mut app))?;
    }

    save(&mut app);
    if let Some(engine) = &app.engine {
        engine.shutdown().await;
    }
    Ok(())
}

fn map_key(key: KeyEvent, mode: Mode, tab: Tab) -> Option<Action> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Some(Action::Quit);
    }

    match mode {
        Mode::Helping => Some(Action::Dismiss),
        Mode::Confirming => match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => Some(Action::Yes),
            _ => Some(Action::Dismiss),
        },
        Mode::Editing | Mode::Asking => match key.code {
            KeyCode::Esc => Some(Action::Dismiss),
            KeyCode::Enter => Some(Action::Submit),
            KeyCode::Backspace => Some(Action::Backspace),
            KeyCode::Char(typed) => Some(Action::Type(typed)),
            _ => None,
        },
        Mode::Typing => match key.code {
            KeyCode::Esc => Some(Action::Dismiss),
            KeyCode::Enter => Some(Action::Submit),
            KeyCode::Backspace => Some(Action::Backspace),
            KeyCode::Tab => Some(Action::NextTab),
            KeyCode::BackTab => Some(Action::PrevTab),
            KeyCode::Up => Some(Action::Move(-1)),
            KeyCode::Down => Some(Action::Move(1)),
            KeyCode::Char(typed) => Some(Action::Type(typed)),
            _ => None,
        },
        Mode::Browsing => browsing_key(key, tab),
    }
}

fn browsing_key(key: KeyEvent, tab: Tab) -> Option<Action> {
    let shared = match key.code {
        KeyCode::Char('q') => Some(Action::Quit),
        KeyCode::Tab => Some(Action::NextTab),
        KeyCode::BackTab => Some(Action::PrevTab),
        KeyCode::Char('?') => Some(Action::Help),
        KeyCode::Char('/') => Some(Action::FocusQuery),
        KeyCode::Char('s') => Some(Action::Go(Tab::Settings)),
        KeyCode::Esc => Some(Action::Dismiss),
        KeyCode::Char('j') | KeyCode::Down => Some(Action::Move(1)),
        KeyCode::Char('k') | KeyCode::Up => Some(Action::Move(-1)),
        KeyCode::PageDown => Some(Action::Move(10)),
        KeyCode::PageUp => Some(Action::Move(-10)),
        _ => None,
    };
    shared.or_else(|| tab_key(key, tab))
}

fn tab_key(key: KeyEvent, tab: Tab) -> Option<Action> {
    match tab {
        Tab::Settings => match key.code {
            KeyCode::Char('h') | KeyCode::Left => Some(Action::Nudge(false)),
            KeyCode::Char('l') | KeyCode::Right => Some(Action::Nudge(true)),
            KeyCode::Enter => Some(Action::BeginEdit),
            _ => None,
        },
        Tab::Search => match key.code {
            KeyCode::Char('d') | KeyCode::Enter => Some(Action::Download { pick_folder: false }),
            KeyCode::Char('D') => Some(Action::Download { pick_folder: true }),
            KeyCode::Char('c') => Some(Action::CopyMagnet),
            _ => None,
        },
        Tab::Downloads | Tab::Seeding => match key.code {
            KeyCode::Char('p') => Some(Action::PauseResume),
            KeyCode::Char('x') => Some(Action::Stop),
            KeyCode::Char('c') => Some(Action::CopyMagnet),
            _ => None,
        },
    }
}

async fn act(app: &mut App, action: Action) {
    app.notice = None;
    match action {
        Action::Quit => app.quit = true,
        Action::Help => app.helping = true,
        Action::NextTab => switch(app, app.tab.shifted(1)),
        Action::PrevTab => switch(app, app.tab.shifted(-1)),
        Action::Go(tab) => switch(app, tab),
        Action::Move(by) => move_cursor(app, by),
        Action::FocusQuery => {
            switch(app, Tab::Search);
            app.typing = true;
        }
        Action::Dismiss => dismiss(app),
        Action::Type(typed) => {
            if let Some(text) = typing_into(app) {
                text.push(typed);
            }
        }
        Action::Backspace => {
            if let Some(text) = typing_into(app) {
                text.pop();
            }
        }
        Action::Submit => submit(app),
        Action::BeginEdit => begin_edit(app),
        Action::Nudge(up) => nudge(app, up),
        Action::Download { pick_folder } => download(app, pick_folder),
        Action::CopyMagnet => copy_magnet(app),
        Action::PauseResume => pause_resume(app).await,
        Action::Stop => stop(app).await,
        Action::Yes => confirm(app).await,
    }
}

/// Three text boxes, one at a time, and which one is open is what the mode says.
fn typing_into(app: &mut App) -> Option<&mut String> {
    match app.mode() {
        Mode::Typing => Some(&mut app.query),
        Mode::Editing => app.editor.as_mut(),
        Mode::Asking => app.prompt.as_mut().map(|prompt| &mut prompt.folder),
        _ => None,
    }
}

fn switch(app: &mut App, tab: Tab) {
    if app.settings_only || tab == app.tab {
        return;
    }
    save(app);
    app.tab = tab;
    app.typing = tab == Tab::Search && app.results.is_empty();
}

fn move_cursor(app: &mut App, by: isize) {
    // A search box you can arrow out of is how you reach the results you just asked for.
    if app.typing {
        if by <= 0 || app.results.is_empty() {
            return;
        }
        app.typing = false;
        return;
    }

    let rows = app.rows();
    if rows == 0 {
        return;
    }
    if app.tab == Tab::Settings {
        save(app);
    }
    let moved = (app.at() as isize + by).clamp(0, rows as isize - 1);
    app.cursor[app.tab.index()] = moved as usize;
}

fn dismiss(app: &mut App) {
    if app.helping {
        app.helping = false;
    } else if app.question.is_some() {
        app.question = None;
    } else if app.prompt.is_some() {
        app.prompt = None;
    } else if app.editor.is_some() {
        app.editor = None;
    } else if app.typing {
        app.typing = false;
    }
}

fn submit(app: &mut App) {
    match app.mode() {
        Mode::Typing => start_search(app),
        Mode::Editing => commit_edit(app),
        Mode::Asking => {
            if let Some(prompt) = app.prompt.take() {
                let folder = PathBuf::from(prompt.folder.trim());
                start_add(app, prompt.input, folder);
            }
        }
        _ => {}
    }
}

/// A magnet, an infohash or a torrent file in the box is a download, not a query.
/// That is what pasting one into a search box is asking for. An empty box is a
/// browse, which is what an empty box is asking for.
fn start_search(app: &mut App) {
    let typed = app.query.trim().to_string();
    if let Ok(input) = Input::parse(&typed) {
        app.query.clear();
        let folder = app.settings.downloads.folder.clone();
        start_add(app, input, folder);
        return;
    }

    app.searching = true;
    app.typing = false;
    ask(app, typed);
}

/// The sources are asked in a task, so typing never waits on the slowest of them.
fn ask(app: &App, query: String) {
    let Some(events) = app.events.clone() else {
        return;
    };
    let settings = app.settings.search.clone();
    tokio::spawn(async move {
        let outcome = search::run(&settings, &query, None).await;
        let _ = events.send(Event::Searched(Box::new(outcome)));
    });
}

fn finish_search(app: &mut App, outcome: Result<Outcome>) {
    app.searching = false;
    app.cursor[Tab::Search.index()] = 0;
    match outcome {
        Err(e) => {
            app.results.clear();
            app.failures.clear();
            app.say(format!("{e:#}"));
        }
        Ok(outcome) => {
            app.failures = outcome
                .failures
                .iter()
                .map(|failure| format!("{} skipped: {}", failure.source, failure.reason))
                .collect();
            app.results = outcome.torrents;
            if app.results.is_empty() {
                app.typing = true;
                app.say("Nothing found.");
            }
        }
    }
}

fn download(app: &mut App, pick_folder: bool) {
    let Some(result) = app.results.get(app.at()) else {
        app.say("No result is selected.");
        return;
    };
    let input = Input::Magnet(result.magnet.clone());
    let title = result.title.clone();
    let folder = app.settings.downloads.folder.clone();

    if pick_folder || app.settings.downloads.ask_for_folder {
        app.prompt = Some(Prompt {
            title,
            folder: folder.display().to_string(),
            input,
        });
        return;
    }
    start_add(app, input, folder);
}

fn start_add(app: &mut App, input: Input, folder: PathBuf) {
    let (Some(engine), Some(events)) = (app.engine.clone(), app.events.clone()) else {
        app.say("This page is running without a torrent session.");
        return;
    };
    app.say("Adding. A magnet waits for peers to hand over the file list.");
    tokio::spawn(async move {
        let message = match engine.add(&input, &folder).await {
            Ok(_) => format!("Downloading into {}", folder.display()),
            Err(e) => format!("{e:#}"),
        };
        let _ = events.send(Event::Notice(message));
    });
}

fn copy_magnet(app: &mut App) {
    let magnet = match app.tab {
        Tab::Search => app
            .results
            .get(app.at())
            .map(|result| result.magnet.clone()),
        _ => app
            .selected_torrent()
            .map(|torrent| magnet_link(&torrent.info_hash, torrent.name.as_deref().unwrap_or(""))),
    };
    let Some(magnet) = magnet else {
        app.say("Nothing is selected.");
        return;
    };

    let clipboard = match &mut app.clipboard {
        Some(clipboard) => Ok(clipboard),
        slot => match arboard::Clipboard::new() {
            Ok(fresh) => Ok(slot.insert(fresh)),
            Err(e) => Err(e.to_string()),
        },
    };
    match clipboard.and_then(|clipboard| clipboard.set_text(magnet).map_err(|e| e.to_string())) {
        Ok(()) => app.say("Magnet link copied."),
        Err(e) => app.say(format!("No clipboard here: {e}")),
    }
}

async fn pause_resume(app: &mut App) {
    let Some(torrent) = app.selected_torrent() else {
        return;
    };
    let (id, running) = (torrent.id, torrent.state != State::Paused);
    let Some(engine) = app.engine.clone() else {
        return;
    };
    let outcome = match running {
        true => engine.pause(id).await,
        false => engine.resume(id).await,
    };
    match outcome {
        Ok(()) if running => app.say("Paused."),
        Ok(()) => app.say("Running again."),
        Err(e) => app.say(format!("{e:#}")),
    }
}

async fn stop(app: &mut App) {
    let Some(torrent) = app.selected_torrent() else {
        return;
    };
    let id = torrent.id;
    let name = torrent
        .name
        .clone()
        .unwrap_or_else(|| torrent.info_hash.clone());

    if app.settings.interface.confirm_before_removing {
        app.question = Some(Question {
            text: format!("Stop {name}? Files already on disk stay where they are."),
            id,
        });
        return;
    }
    remove(app, id).await;
}

async fn confirm(app: &mut App) {
    let Some(question) = app.question.take() else {
        return;
    };
    remove(app, question.id).await;
}

async fn remove(app: &mut App, id: DownloadId) {
    let Some(engine) = app.engine.clone() else {
        return;
    };
    match engine.remove(id).await {
        Ok(()) => app.say("Stopped. The files are still on disk."),
        Err(e) => app.say(format!("{e:#}")),
    }
    app.torrents = engine.snapshot();
}

fn begin_edit(app: &mut App) {
    let at = app.at();
    let typed = app
        .settings
        .fields()
        .get(at)
        .and_then(|field| field.typed());
    match typed {
        Some(text) => app.editor = Some(text),
        None => app.say("This one changes with the left and right arrows."),
    }
}

fn commit_edit(app: &mut App) {
    let Some(text) = app.editor.take() else {
        return;
    };
    let at = app.at();
    let outcome = app
        .settings
        .fields()
        .get_mut(at)
        .map(|field| field.accept(&text));
    match outcome {
        Some(Err(complaint)) => {
            app.say(complaint);
            app.editor = Some(text);
        }
        _ => changed(app),
    }
}

fn nudge(app: &mut App, up: bool) {
    let at = app.at();
    if let Some(field) = app.settings.fields().get_mut(at) {
        field.nudge(up);
    }
    changed(app);
}

/// Live where the engine allows it, on disk when the row is left. A setting that only
/// takes hold on the next start says so on its own row.
fn changed(app: &mut App) {
    app.unsaved = true;
    if let Some(engine) = &app.engine {
        engine.apply(&app.settings);
    }
}

fn save(app: &mut App) {
    if !app.unsaved {
        return;
    }
    app.unsaved = false;
    if let Err(e) = app.settings.save() {
        app.say(format!("{e}"));
    }
}

async fn tick(app: &mut App) {
    let Some(engine) = app.engine.clone() else {
        return;
    };
    app.torrents = engine.snapshot();
    if let Err(e) = engine.enforce(&app.settings).await {
        app.say(format!("{e:#}"));
    }
    let (downloading, seeding) = (app.downloads().len(), app.seeds().len());
    clamp(app, Tab::Downloads, downloading);
    clamp(app, Tab::Seeding, seeding);
}

fn clamp(app: &mut App, tab: Tab, rows: usize) {
    let at = &mut app.cursor[tab.index()];
    *at = (*at).min(rows.saturating_sub(1));
}

/// Games are executables and can run code. Video and subtitles cannot.
fn is_risky(app: &App, torrent: &Torrent) -> bool {
    app.settings.interface.game_warnings && torrent.category == Category::Games
}

#[cfg(test)]
mod tests {
    use super::*;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn screen(app: &mut App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| render::draw(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    fn found(title: &str) -> Torrent {
        Torrent {
            title: title.to_string(),
            size_bytes: 3_285_649_981,
            seeders: 42,
            leechers: 1,
            category: Category::Movies,
            source: "yts",
            info_hash: "cab507494d02ebb1178b38f2e9d7be299c86b862".to_string(),
            magnet: "magnet:?xt=urn:btih:cab507494d02ebb1178b38f2e9d7be299c86b862".to_string(),
        }
    }

    fn running(name: &str, finished: bool) -> Progress {
        Progress {
            id: 1,
            name: Some(name.to_string()),
            info_hash: "cab507494d02ebb1178b38f2e9d7be299c86b862".to_string(),
            folder: PathBuf::from("downloads"),
            state: State::Active,
            finished,
            done_bytes: 512,
            total_bytes: 1024,
            uploaded_bytes: 2048,
            download_bps: 1024,
            upload_bps: 512,
            peers: 9,
            ratio: 2.0,
            eta: Some(Duration::from_secs(90)),
            error: None,
        }
    }

    #[test]
    fn typing_in_the_search_box_never_triggers_a_shortcut() {
        let typed = map_key(press(KeyCode::Char('d')), Mode::Typing, Tab::Search);
        assert_eq!(typed, Some(Action::Type('d')));
    }

    #[test]
    fn the_same_key_downloads_once_the_box_is_left() {
        let pressed = map_key(press(KeyCode::Char('d')), Mode::Browsing, Tab::Search);
        assert_eq!(pressed, Some(Action::Download { pick_folder: false }));
    }

    #[test]
    fn a_capital_d_asks_where_to_put_it() {
        let pressed = map_key(press(KeyCode::Char('D')), Mode::Browsing, Tab::Search);
        assert_eq!(pressed, Some(Action::Download { pick_folder: true }));
    }

    #[test]
    fn an_open_overlay_swallows_every_key_it_does_not_use() {
        assert_eq!(
            map_key(press(KeyCode::Char('q')), Mode::Helping, Tab::Search),
            Some(Action::Dismiss)
        );
        assert_eq!(
            map_key(press(KeyCode::Char('q')), Mode::Confirming, Tab::Search),
            Some(Action::Dismiss)
        );
    }

    #[test]
    fn ctrl_c_quits_from_anywhere_including_a_text_box() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(map_key(key, Mode::Typing, Tab::Search), Some(Action::Quit));
        assert_eq!(
            map_key(key, Mode::Editing, Tab::Settings),
            Some(Action::Quit)
        );
    }

    #[test]
    fn the_settings_tab_uses_the_arrows_rather_than_the_download_keys() {
        assert_eq!(
            map_key(press(KeyCode::Left), Mode::Browsing, Tab::Settings),
            Some(Action::Nudge(false))
        );
        assert_eq!(
            map_key(press(KeyCode::Char('d')), Mode::Browsing, Tab::Settings),
            None
        );
    }

    #[test]
    fn tabs_wrap_in_both_directions() {
        assert_eq!(Tab::Settings.shifted(1), Tab::Search);
        assert_eq!(Tab::Search.shifted(-1), Tab::Settings);
    }

    #[test]
    fn a_magnet_pasted_into_the_search_box_is_downloaded_not_searched() {
        let mut app = App::new(Settings::default(), None);
        app.query = "magnet:?xt=urn:btih:cab507494d02ebb1178b38f2e9d7be299c86b862".to_string();
        start_search(&mut app);
        assert!(!app.searching);
        assert!(app.query.is_empty());
    }

    #[test]
    fn an_empty_box_browses_rather_than_complaining() {
        let mut app = App::new(Settings::default(), None);
        start_search(&mut app);
        assert!(app.searching);
        assert!(app.notice.is_none());
    }

    #[test]
    fn a_typed_setting_that_does_not_parse_keeps_the_editor_open() {
        let mut app = App::new(Settings::default(), None);
        app.tab = Tab::Settings;
        app.cursor[Tab::Settings.index()] = 1;
        app.editor = Some("three".to_string());
        commit_edit(&mut app);
        assert!(app.editor.is_some());
        assert_eq!(app.settings.downloads.max_concurrent, 3);
    }

    #[test]
    fn a_typed_setting_that_parses_is_kept_and_marked_unsaved() {
        let mut app = App::new(Settings::default(), None);
        app.tab = Tab::Settings;
        app.cursor[Tab::Settings.index()] = 1;
        app.editor = Some("7".to_string());
        commit_edit(&mut app);
        assert!(app.editor.is_none());
        assert_eq!(app.settings.downloads.max_concurrent, 7);
        assert!(app.unsaved);
    }

    #[test]
    fn an_empty_search_tab_carries_the_branding() {
        let mut app = App::new(Settings::default(), None);
        let drawn = screen(&mut app);
        assert!(drawn.contains("BAKA"));
        assert!(drawn.contains("BitTorrent Acquisition & Keyword Aggregator"));
    }

    #[test]
    fn results_are_drawn_with_their_source_shelf_and_size() {
        let mut app = App::new(Settings::default(), None);
        app.results = vec![found("Dune Part Two")];
        app.typing = false;
        let drawn = screen(&mut app);
        assert!(drawn.contains("Dune Part Two"));
        assert!(drawn.contains("3.1 GiB"));
        assert!(drawn.contains("yts"));
        assert!(drawn.contains("movies"));
    }

    #[test]
    fn a_browse_names_the_shelf_every_result_came_from() {
        let mut app = App::new(Settings::default(), None);
        let mut game = found("Some Repack");
        game.category = Category::Games;
        game.source = "fitgirl";
        app.results = vec![found("Dune Part Two"), game];
        app.typing = false;
        let drawn = screen(&mut app);
        assert!(drawn.contains("games"));
        assert!(drawn.contains("movies"));
    }

    #[test]
    fn a_game_result_is_flagged_as_something_that_can_run_code() {
        let mut app = App::new(Settings::default(), None);
        let mut game = found("Some Game");
        game.category = Category::Games;
        app.results = vec![game];
        app.typing = false;
        assert!(screen(&mut app).contains("! Some Game"));

        app.settings.interface.game_warnings = false;
        assert!(!screen(&mut app).contains("! Some Game"));
    }

    #[test]
    fn the_downloads_tab_shows_progress_speed_and_time_left() {
        let mut app = App::new(Settings::default(), None);
        app.torrents = vec![running("debian.iso", false)];
        app.tab = Tab::Downloads;
        app.typing = false;
        let drawn = screen(&mut app);
        assert!(drawn.contains("debian.iso"));
        assert!(drawn.contains("50.0%"));
        assert!(drawn.contains("1m 30s"));
        assert!(drawn.contains("running"));
    }

    #[test]
    fn a_finished_torrent_moves_from_downloads_to_seeding() {
        let mut app = App::new(Settings::default(), None);
        app.torrents = vec![running("debian.iso", true)];
        app.typing = false;

        app.tab = Tab::Downloads;
        assert!(screen(&mut app).contains("Nothing is downloading."));

        app.tab = Tab::Seeding;
        let drawn = screen(&mut app);
        assert!(drawn.contains("debian.iso"));
        assert!(drawn.contains("2.00"));
    }

    #[test]
    fn the_settings_tab_lists_every_group_and_every_source() {
        let mut app = App::new(Settings::default(), None);
        app.tab = Tab::Settings;
        app.typing = false;
        let drawn = screen(&mut app);
        for group in [
            crate::config::DOWNLOADS,
            crate::config::SEEDING,
            crate::config::NETWORK,
            crate::config::SEARCH,
            crate::config::INTERFACE,
        ] {
            assert!(drawn.contains(group), "{group}");
        }
        for source in search::source_names() {
            assert!(drawn.contains(source), "{source}");
        }
    }

    #[test]
    fn a_setting_that_needs_a_restart_says_so_on_its_own_row() {
        let mut app = App::new(Settings::default(), None);
        app.tab = Tab::Settings;
        app.typing = false;
        let at = app
            .settings
            .fields()
            .iter()
            .position(|field| field.label == "Listen port")
            .unwrap();
        app.cursor[Tab::Settings.index()] = at;
        assert!(screen(&mut app).contains("Takes effect on the next start."));
    }

    #[test]
    fn the_help_overlay_covers_the_keys_the_readme_promises() {
        let mut app = App::new(Settings::default(), None);
        app.helping = true;
        let drawn = screen(&mut app);
        assert!(drawn.contains("Keys"));
        assert!(drawn.contains("Copy the magnet link"));
    }

    #[test]
    fn a_terminal_too_small_for_any_of_this_still_draws() {
        for (width, height) in [(20, 5), (40, 10), (80, 3)] {
            let mut app = App::new(Settings::default(), None);
            app.results = vec![found("Dune Part Two")];
            app.torrents = vec![running("debian.iso", false)];
            app.helping = true;
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            for tab in Tab::ALL {
                app.tab = tab;
                terminal
                    .draw(|frame| render::draw(frame, &mut app))
                    .unwrap_or_else(|e| panic!("{width}x{height} {}: {e}", tab.title()));
            }
        }
    }
}
