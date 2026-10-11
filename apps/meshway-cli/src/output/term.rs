//! Terminal capabilities: colors and box tables only on a TTY without
//! `NO_COLOR`; every other destination gets plain aligned text.
use anstyle::{AnsiColor, Style};
use std::fmt::Display;
use std::io::IsTerminal;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    Plain,
    Good,
    Warn,
    Bad,
    Dim,
    Bold,
    Accent,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Term {
    /// Box tables and screen redraws.
    pub rich: bool,
    pub color: bool,
}

impl Term {
    pub fn stdout() -> Self {
        let rich = std::io::stdout().is_terminal()
            && std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty());
        Self { rich, color: rich }
    }

    #[cfg(test)]
    pub fn plain() -> Self {
        Self {
            rich: false,
            color: false,
        }
    }

    #[cfg(test)]
    pub fn rich() -> Self {
        Self {
            rich: true,
            color: true,
        }
    }

    pub fn paint(&self, role: Role, text: impl Display) -> String {
        let style = match role {
            _ if !self.color => return text.to_string(),
            Role::Plain => return text.to_string(),
            Role::Good => Style::new().fg_color(Some(AnsiColor::Green.into())),
            Role::Warn => Style::new().fg_color(Some(AnsiColor::Yellow.into())),
            Role::Bad => Style::new().fg_color(Some(AnsiColor::Red.into())),
            Role::Dim => Style::new().dimmed(),
            Role::Bold => Style::new().bold(),
            Role::Accent => Style::new().fg_color(Some(AnsiColor::Cyan.into())),
        };
        format!("{}{text}{}", style.render(), style.render_reset())
    }
}

/// Display width of terminal text, for CJK and other wide characters.
pub(crate) fn width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

pub(crate) fn pad(text: &str, width: usize) -> String {
    let fill = width.saturating_sub(self::width(text));
    format!("{text}{}", " ".repeat(fill))
}
