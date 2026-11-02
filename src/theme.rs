use ratatui::style::{Color, Modifier, Style};

/// Every color the UI uses. Widgets read from here, never hardcode colors.
///
/// The colors are the terminal's palette slots, not fixed RGB values, so
/// needledrop wears whatever theme the terminal has and recolors the moment
/// it changes, like tmux or lazygit do with default colors.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub accent: Color,
    pub text: Color,
    pub muted: Color,
    pub border: Color,
    pub selection_bg: Color,
    pub playing: Color,
    pub error: Color,
    /// Visualizer colors, bottom to top.
    pub spectrum: [Color; 3],
}

impl Theme {
    pub const TERMINAL: Theme = Theme {
        accent: Color::Magenta,
        text: Color::Reset,
        muted: Color::DarkGray,
        border: Color::DarkGray,
        selection_bg: Color::DarkGray,
        playing: Color::Green,
        error: Color::Red,
        spectrum: [Color::Blue, Color::Magenta, Color::LightMagenta],
    };

    pub fn text(&self) -> Style {
        Style::new().fg(self.text)
    }

    pub fn muted(&self) -> Style {
        Style::new().fg(self.muted)
    }

    pub fn accent(&self) -> Style {
        Style::new().fg(self.accent)
    }

    pub fn border(&self, focused: bool) -> Style {
        Style::new().fg(if focused { self.accent } else { self.border })
    }

    pub fn title(&self, focused: bool) -> Style {
        let style = Style::new().add_modifier(Modifier::BOLD);
        if focused { style.fg(self.accent) } else { style.fg(self.muted) }
    }

    /// The highlighted row. Sets the text color too, because dimmed text
    /// would vanish against the bright-black highlight.
    pub fn selection(&self) -> Style {
        Style::new().bg(self.selection_bg).fg(self.text).add_modifier(Modifier::BOLD)
    }
}
