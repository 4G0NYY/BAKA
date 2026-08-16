//! The accent colour, in the two forms the views need it.

use ratatui::style::{Color, Modifier, Style};

use crate::config::Accent;

pub fn colour(accent: Accent) -> Color {
    match accent {
        Accent::Pink => Color::Rgb(255, 121, 198),
        Accent::Cyan => Color::Rgb(80, 220, 230),
        Accent::Green => Color::Rgb(120, 220, 120),
        Accent::Amber => Color::Rgb(255, 184, 76),
        Accent::Plain => Color::Reset,
    }
}

pub fn highlight(accent: Accent) -> Style {
    match accent {
        // Plain has no colour to paint with, so the cursor row swaps foreground and
        // background instead of picking one.
        Accent::Plain => Style::new().add_modifier(Modifier::REVERSED),
        accent => Style::new().bg(colour(accent)).fg(Color::Black),
    }
}

pub fn dim() -> Style {
    Style::new().add_modifier(Modifier::DIM)
}

pub fn heading(accent: Accent) -> Style {
    Style::new().fg(colour(accent)).add_modifier(Modifier::BOLD)
}
