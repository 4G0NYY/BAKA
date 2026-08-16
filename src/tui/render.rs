//! Everything drawn on screen. These functions read state and draw it, and that is all
//! they do.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Clear, Paragraph, Row, Table, TableState, Tabs, Wrap};

use super::theme;
use super::{App, Mode, Tab};
use crate::config::Accent;
use crate::engine::{Progress, State, human_eta};
use crate::search::human_size;

const ART: &str = include_str!("../../stuff/ascii-art.txt");

const WORDMARK: &str = r"
 ____    _    _  __    _
| __ )  / \  | |/ /   / \
|  _ \ / _ \ | ' /   / _ \
| |_) / ___ \| . \  / ___ \
|____/_/   \_\_|\_\/_/   \_\
";

const TAGLINE: &str = "BitTorrent Acquisition & Keyword Aggregator";

pub fn draw(frame: &mut Frame, app: &mut App) {
    let [top, body, bottom] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    tab_bar(frame, app, top);
    match app.tab {
        Tab::Search => search(frame, app, body),
        Tab::Downloads => downloads(frame, app, body),
        Tab::Seeding => seeding(frame, app, body),
        Tab::Settings => settings(frame, app, body),
    }
    status(frame, app, bottom);

    if app.prompt.is_some() {
        prompt(frame, app);
    }
    if app.question.is_some() {
        question(frame, app);
    }
    if app.helping {
        help(frame, app);
    }
}

fn tab_bar(frame: &mut Frame, app: &App, area: Rect) {
    let accent = app.settings.interface.accent;
    let [name, rest] = Layout::horizontal([Constraint::Length(6), Constraint::Min(0)]).areas(area);

    frame.render_widget(Paragraph::new("BAKA").style(theme::heading(accent)), name);

    if app.settings_only {
        frame.render_widget(
            Paragraph::new("Settings").style(theme::highlight(accent)),
            rest,
        );
        return;
    }

    let titles = Tab::ALL.map(Tab::title);
    frame.render_widget(
        Tabs::new(titles)
            .select(app.tab.index())
            .highlight_style(theme::highlight(accent))
            .divider(" "),
        rest,
    );
}

fn search(frame: &mut Frame, app: &App, area: Rect) {
    let accent = app.settings.interface.accent;
    let failures = u16::from(!app.failures.is_empty());
    let [box_area, warnings, results] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(failures),
        Constraint::Min(0),
    ])
    .areas(area);

    let typing = app.mode() == Mode::Typing;
    let mut line = app.query.clone();
    if typing {
        line.push('\u{258f}');
    }
    let border = match typing {
        true => Style::new().fg(theme::colour(accent)),
        false => Style::new(),
    };
    frame.render_widget(
        Paragraph::new(line).block(
            Block::bordered()
                .border_style(border)
                .title(" Search ")
                .title_style(theme::heading(accent)),
        ),
        box_area,
    );

    if failures == 1 {
        frame.render_widget(
            Paragraph::new(app.failures.join("   ")).style(theme::dim()),
            warnings,
        );
    }

    if app.results.is_empty() {
        welcome(frame, app, results);
        return;
    }

    let rows = app.results.iter().map(|torrent| {
        let mark = match super::is_risky(app, torrent) {
            true => "! ",
            false => "",
        };
        Row::new(vec![
            Cell::from(torrent.source),
            Cell::from(torrent.seeders.to_string()),
            Cell::from(human_size(torrent.size_bytes)),
            Cell::from(format!("{mark}{}", torrent.title)),
        ])
    });

    let table = Table::new(
        rows,
        [
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(10),
            Constraint::Min(10),
        ],
    )
    .header(header(accent, ["SOURCE", "SEED", "SIZE", "TITLE"]))
    .row_highlight_style(theme::highlight(accent));

    frame.render_stateful_widget(table, results, &mut selection(app));
}

/// The empty state carries the branding. It is the one place BAKA gets to be loud.
fn welcome(frame: &mut Frame, app: &App, area: Rect) {
    let accent = app.settings.interface.accent;
    let mut lines = Vec::new();

    if area.height >= 30 {
        lines.extend(block(ART, Style::new()));
        lines.push(Line::default());
    }
    if area.height >= 9 {
        lines.extend(block(WORDMARK, theme::heading(accent)));
        lines.push(Line::default());
    }
    lines.push(Line::from(TAGLINE).centered().style(theme::dim()));
    lines.push(Line::default());

    let hint = match app.searching {
        true => "Asking every source.",
        false => "Type to search. Press ? for the keys.",
    };
    lines.push(Line::from(hint).centered());

    frame.render_widget(Paragraph::new(lines), area);
}

/// Padding every line to the widest one is what keeps art centred as one picture
/// rather than as a stack of separately centred lines.
fn block(art: &str, style: Style) -> Vec<Line<'static>> {
    let lines: Vec<&str> = art.trim_matches('\n').lines().collect();
    let width = lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0);
    lines
        .into_iter()
        .map(|line| {
            Line::from(format!("{line:<width$}"))
                .centered()
                .style(style)
        })
        .collect()
}

fn downloads(frame: &mut Frame, app: &App, area: Rect) {
    let accent = app.settings.interface.accent;
    let list = app.downloads();
    if list.is_empty() {
        frame.render_widget(empty("Nothing is downloading."), area);
        return;
    }

    let rows = list.iter().map(|torrent| {
        Row::new(vec![
            Cell::from(name_of(torrent)),
            Cell::from(bar(torrent)),
            Cell::from(human_size(torrent.total_bytes)),
            Cell::from(format!("{}/s", human_size(torrent.download_bps))),
            Cell::from(human_eta(torrent.eta)),
            Cell::from(torrent.peers.to_string()),
            Cell::from(label(torrent.state)),
        ])
    });

    let table = Table::new(
        rows,
        [
            Constraint::Min(16),
            Constraint::Length(18),
            Constraint::Length(9),
            Constraint::Length(11),
            Constraint::Length(11),
            Constraint::Length(5),
            Constraint::Length(8),
        ],
    )
    .header(header(
        accent,
        ["NAME", "PROGRESS", "SIZE", "DOWN", "LEFT", "PEERS", "STATE"],
    ))
    .row_highlight_style(theme::highlight(accent));

    frame.render_stateful_widget(table, area, &mut selection(app));
}

fn seeding(frame: &mut Frame, app: &App, area: Rect) {
    let accent = app.settings.interface.accent;
    let list = app.seeds();
    if list.is_empty() {
        frame.render_widget(empty("Nothing is seeding yet."), area);
        return;
    }

    let rows = list.iter().map(|torrent| {
        Row::new(vec![
            Cell::from(name_of(torrent)),
            Cell::from(format!("{:.2}", torrent.ratio)),
            Cell::from(human_size(torrent.uploaded_bytes)),
            Cell::from(format!("{}/s", human_size(torrent.upload_bps))),
            Cell::from(torrent.peers.to_string()),
            Cell::from(label(torrent.state)),
        ])
    });

    let table = Table::new(
        rows,
        [
            Constraint::Min(16),
            Constraint::Length(7),
            Constraint::Length(10),
            Constraint::Length(11),
            Constraint::Length(5),
            Constraint::Length(8),
        ],
    )
    .header(header(
        accent,
        ["NAME", "RATIO", "SHARED", "UP", "PEERS", "STATE"],
    ))
    .row_highlight_style(theme::highlight(accent));

    frame.render_stateful_widget(table, area, &mut selection(app));
}

fn settings(frame: &mut Frame, app: &mut App, area: Rect) {
    let accent = app.settings.interface.accent;
    let at = app.at();
    let editor = app.editor.clone();
    let [list, footer] = Layout::vertical([Constraint::Min(0), Constraint::Length(3)]).areas(area);

    let fields = app.settings.fields();
    let mut group = "";
    let mut rows = Vec::new();
    for (index, field) in fields.iter().enumerate() {
        let heading = match field.group == group {
            true => String::new(),
            false => {
                group = field.group;
                field.group.to_string()
            }
        };
        let value = match (index == at, &editor) {
            (true, Some(text)) => format!("{text}\u{258f}"),
            _ => field.display(),
        };
        rows.push(Row::new(vec![
            Cell::from(heading).style(theme::heading(accent)),
            Cell::from(field.label.to_string()),
            Cell::from(value),
        ]));
    }

    let note = fields.get(at).map(|field| {
        let restart = match field.needs_restart {
            true => "  Takes effect on the next start.",
            false => "",
        };
        format!("{}{restart}", field.description)
    });

    let table = Table::new(
        rows,
        [
            Constraint::Length(11),
            Constraint::Length(26),
            Constraint::Min(10),
        ],
    )
    .row_highlight_style(theme::highlight(accent));

    let mut state = TableState::new().with_selected(Some(at));
    frame.render_stateful_widget(table, list, &mut state);
    frame.render_widget(
        Paragraph::new(note.unwrap_or_default())
            .wrap(Wrap { trim: true })
            .block(Block::bordered().border_style(theme::dim())),
        footer,
    );
}

fn status(frame: &mut Frame, app: &App, area: Rect) {
    let accent = app.settings.interface.accent;
    let left = match &app.notice {
        Some(notice) => Span::from(notice.clone()).style(Style::new().fg(theme::colour(accent))),
        None => Span::from(hint(app)).style(theme::dim()),
    };

    let counts = format!(
        "{} downloading  {} seeding",
        app.downloads().len(),
        app.seeds().len()
    );

    let [message, tally] = Layout::horizontal([
        Constraint::Min(0),
        Constraint::Length(counts.len() as u16 + 1),
    ])
    .areas(area);
    frame.render_widget(Paragraph::new(Line::from(left)), message);
    frame.render_widget(
        Paragraph::new(counts)
            .style(theme::dim())
            .alignment(Alignment::Right),
        tally,
    );
}

fn hint(app: &App) -> &'static str {
    match (app.mode(), app.tab) {
        (Mode::Typing, _) => "Enter searches. A magnet, an infohash or a file path downloads.",
        (Mode::Editing, _) => "Type a value, Enter keeps it, Esc leaves it alone.",
        (Mode::Asking, _) => "Type a folder, Enter starts the download, Esc cancels.",
        (Mode::Confirming, _) => "y to go ahead, anything else to leave it running.",
        (Mode::Helping, _) => "Any key closes this.",
        (_, Tab::Search) => "d download   D download to   c copy magnet   / search   ? keys",
        (_, Tab::Settings) => "arrows change   Enter types a value   ? keys",
        (_, _) => "p pause or resume   x stop   c copy magnet   ? keys",
    }
}

fn prompt(frame: &mut Frame, app: &App) {
    let Some(prompt) = &app.prompt else {
        return;
    };
    let accent = app.settings.interface.accent;
    let area = popup(frame.area(), 70, 5);
    let body = vec![
        Line::from(prompt.title.clone()).style(theme::dim()),
        Line::from(format!("{}\u{258f}", prompt.folder)),
    ];
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(body).block(
            Block::bordered()
                .title(" Download into ")
                .title_style(theme::heading(accent))
                .border_style(Style::new().fg(theme::colour(accent))),
        ),
        area,
    );
}

fn question(frame: &mut Frame, app: &App) {
    let Some(question) = &app.question else {
        return;
    };
    let accent = app.settings.interface.accent;
    let area = popup(frame.area(), 70, 5);
    let body = vec![
        Line::from(question.text.clone()),
        Line::default(),
        Line::from("y to stop it, anything else to leave it alone.").style(theme::dim()),
    ];
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(body).wrap(Wrap { trim: true }).block(
            Block::bordered()
                .title(" Stop ")
                .title_style(theme::heading(accent))
                .border_style(Style::new().fg(theme::colour(accent))),
        ),
        area,
    );
}

fn help(frame: &mut Frame, app: &App) {
    const KEYS: [[&str; 2]; 13] = [
        ["/", "Focus the search box"],
        ["Enter", "Search, or download the selected result"],
        ["Tab", "Next tab, Shift Tab for the previous one"],
        ["j k", "Move, arrows work too"],
        ["d", "Download to the default folder"],
        ["D", "Download to a folder you pick"],
        ["p", "Pause or resume"],
        ["x", "Stop, files stay on disk"],
        ["c", "Copy the magnet link"],
        ["s", "Settings"],
        ["arrows", "Change a setting, Enter types a value"],
        ["?", "This list"],
        ["q", "Quit"],
    ];

    let accent = app.settings.interface.accent;
    let area = popup(frame.area(), 56, KEYS.len() as u16 + 2);
    let rows = KEYS.map(|[key, what]| {
        Row::new(vec![
            Cell::from(key).style(theme::heading(accent)),
            Cell::from(what),
        ])
    });

    frame.render_widget(Clear, area);
    frame.render_widget(
        Table::new(rows, [Constraint::Length(8), Constraint::Min(10)]).block(
            Block::bordered()
                .title(" Keys ")
                .title_style(theme::heading(accent))
                .border_style(Style::new().fg(theme::colour(accent))),
        ),
        area,
    );
}

fn header<'a, const N: usize>(accent: Accent, titles: [&'a str; N]) -> Row<'a> {
    Row::new(titles.map(Cell::from)).style(theme::heading(accent))
}

fn selection(app: &App) -> TableState {
    TableState::new().with_selected(Some(app.at()))
}

fn empty(message: &str) -> Paragraph<'_> {
    Paragraph::new(message).style(theme::dim()).centered()
}

fn name_of(torrent: &Progress) -> String {
    torrent
        .name
        .clone()
        .unwrap_or_else(|| format!("{}...", &torrent.info_hash[..8]))
}

fn bar(torrent: &Progress) -> String {
    const WIDTH: usize = 10;
    let share = match torrent.total_bytes {
        0 => 0.0,
        total => torrent.done_bytes as f64 / total as f64,
    };
    let filled = (share * WIDTH as f64).round() as usize;
    format!(
        "{}{} {:>5.1}%",
        "\u{2588}".repeat(filled.min(WIDTH)),
        "\u{2591}".repeat(WIDTH - filled.min(WIDTH)),
        share * 100.0
    )
}

fn label(state: State) -> &'static str {
    match state {
        State::Checking => "checking",
        State::Active => "running",
        State::Queued => "queued",
        State::Paused => "paused",
        State::Failed => "failed",
    }
}

fn popup(area: Rect, width: u16, height: u16) -> Rect {
    let [row] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    let [cell] = Layout::horizontal([Constraint::Length(width.min(area.width))])
        .flex(Flex::Center)
        .areas(row);
    cell
}
