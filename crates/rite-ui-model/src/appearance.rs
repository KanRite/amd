//! A user's colour scheme: light or dark (or follow the system), and an
//! accent colour for buttons, filters and highlights. Saved in the user's
//! own settings on this machine, never in the shared tickets repository.

use serde::{Deserialize, Serialize};

/// Light or dark, or whatever the desktop is set to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    pub const ALL: [Theme; 3] = [Theme::System, Theme::Light, Theme::Dark];

    pub fn key(self) -> &'static str {
        match self {
            Theme::System => "system",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Theme::System => "Follow the system",
            Theme::Light => "Light",
            Theme::Dark => "Dark",
        }
    }

    pub fn from_key(key: &str) -> Theme {
        Theme::ALL
            .into_iter()
            .find(|t| t.key() == key)
            .unwrap_or_default()
    }

    /// Whether the board should be dark, given whether the system is.
    pub fn is_dark(self, system_dark: bool) -> bool {
        match self {
            Theme::System => system_dark,
            Theme::Light => false,
            Theme::Dark => true,
        }
    }
}

/// The colour of buttons, filters and highlights. Red and orange aren't
/// offered (ticket 192): red reads as an error, and orange is kept for the
/// update bar, so neither can be mistaken for the accent. A saved `rose` or
/// `amber` reads back as the default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Accent {
    #[default]
    Indigo,
    Teal,
    Slate,
}

impl Accent {
    pub const ALL: [Accent; 3] = [Accent::Indigo, Accent::Teal, Accent::Slate];

    /// The `data-accent` value the stylesheet switches on.
    pub fn key(self) -> &'static str {
        match self {
            Accent::Indigo => "indigo",
            Accent::Teal => "teal",
            Accent::Slate => "slate",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Accent::Indigo => "Indigo",
            Accent::Teal => "Teal",
            Accent::Slate => "Slate",
        }
    }

    pub fn from_key(key: &str) -> Accent {
        Accent::ALL
            .into_iter()
            .find(|a| a.key() == key)
            .unwrap_or_default()
    }
}

/// A user's colour scheme. Saved as its two keys; anything unrecognised in
/// a settings file reads back as the default (see `from_keys`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Appearance {
    pub theme: Theme,
    pub accent: Accent,
}

impl Appearance {
    pub fn from_keys(theme: &str, accent: &str) -> Appearance {
        Appearance {
            theme: Theme::from_key(theme),
            accent: Accent::from_key(accent),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn red_and_orange_are_gone_and_read_back_as_the_default() {
        // Ticket 192: orange is the update bar's, red reads as an error.
        assert_eq!(Accent::from_key("rose"), Accent::Indigo);
        assert_eq!(Accent::from_key("amber"), Accent::Indigo);
        assert_eq!(Accent::from_key("teal"), Accent::Teal);
        assert!(
            Accent::ALL
                .iter()
                .all(|a| !matches!(a.key(), "rose" | "amber"))
        );
    }

    #[test]
    fn defaults_follow_the_system_in_indigo() {
        let a = Appearance::default();
        assert_eq!((a.theme, a.accent), (Theme::System, Accent::Indigo));
    }

    #[test]
    fn keys_round_trip_and_unknowns_fall_back() {
        for t in Theme::ALL {
            assert_eq!(Theme::from_key(t.key()), t);
        }
        for a in Accent::ALL {
            assert_eq!(Accent::from_key(a.key()), a);
        }
        assert_eq!(Theme::from_key("sepia"), Theme::System);
        assert_eq!(Accent::from_key("chartreuse"), Accent::Indigo);
    }

    #[test]
    fn darkness_follows_the_choice() {
        assert!(Theme::System.is_dark(true) && !Theme::System.is_dark(false));
        assert!(!Theme::Light.is_dark(true));
        assert!(Theme::Dark.is_dark(false));
    }

    #[test]
    fn reads_saved_keys_leniently() {
        let a = Appearance::from_keys("dark", "teal");
        assert_eq!((a.theme, a.accent), (Theme::Dark, Accent::Teal));
        assert_eq!(Appearance::from_keys("", "neon"), Appearance::default());
    }
}
