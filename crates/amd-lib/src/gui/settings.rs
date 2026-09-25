//! What the desktop board remembers between sessions.
//!
//! Both settings live in the shared config file under `ui.*`. egui can
//! persist its own memory, but only when eframe is built with its
//! `persistence` feature, and that stores the choice in eframe's blob rather
//! than the file the rest of agile-md uses — so the theme is kept here
//! instead, in the shared config.

use crate::settings::Config;

/// The theme the board opens with. `System` is the default because following
/// the desktop is what a user who has never touched the setting expects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    pub fn as_str(self) -> &'static str {
        match self {
            Theme::System => "system",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }

    fn parse(value: &str) -> Option<Theme> {
        match value.trim().to_ascii_lowercase().as_str() {
            "system" => Some(Theme::System),
            "light" => Some(Theme::Light),
            "dark" => Some(Theme::Dark),
            _ => None,
        }
    }
}

/// The accent colour applied over whichever base theme is active —
/// `egui::Visuals::selection`/`hyperlink_color`, which is what everything
/// already built on `ui.visuals()` (card fills, borders, the drop-target
/// highlight) picks up automatically, with no per-widget changes needed.
/// Deliberately not a per-element palette editor: an inventory of every
/// `Color32`/`.visuals()` call in `gui/mod.rs` found only two genuinely
/// hardcoded colours (the status-error red, the lease-badge amber), and
/// both are semantic rather than decorative — left untouched by design, so
/// an error still reads as an error whichever accent is chosen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Accent {
    /// Leave egui's own theme default alone — no override applied.
    #[default]
    Default,
    Blue,
    Green,
    Purple,
    Orange,
    Red,
}

impl Accent {
    pub const ALL: [Accent; 6] = [
        Accent::Default,
        Accent::Blue,
        Accent::Green,
        Accent::Purple,
        Accent::Orange,
        Accent::Red,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Accent::Default => "default",
            Accent::Blue => "blue",
            Accent::Green => "green",
            Accent::Purple => "purple",
            Accent::Orange => "orange",
            Accent::Red => "red",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Accent::Default => "Default",
            Accent::Blue => "Blue",
            Accent::Green => "Green",
            Accent::Purple => "Purple",
            Accent::Orange => "Orange",
            Accent::Red => "Red",
        }
    }

    fn parse(value: &str) -> Option<Accent> {
        match value.trim().to_ascii_lowercase().as_str() {
            "default" => Some(Accent::Default),
            "blue" => Some(Accent::Blue),
            "green" => Some(Accent::Green),
            "purple" => Some(Accent::Purple),
            "orange" => Some(Accent::Orange),
            "red" => Some(Accent::Red),
            _ => None,
        }
    }

    /// The colour to apply, or `None` to leave the active theme's own
    /// default selection/hyperlink colour alone.
    pub fn color(self) -> Option<egui::Color32> {
        match self {
            Accent::Default => None,
            Accent::Blue => Some(egui::Color32::from_rgb(66, 133, 244)),
            Accent::Green => Some(egui::Color32::from_rgb(52, 168, 83)),
            Accent::Purple => Some(egui::Color32::from_rgb(155, 89, 182)),
            Accent::Orange => Some(egui::Color32::from_rgb(230, 126, 34)),
            Accent::Red => Some(egui::Color32::from_rgb(192, 57, 43)),
        }
    }
}

/// Text size, as a multiple of the standard size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FontSize {
    #[default]
    Standard,
    Medium,
    Large,
}

impl FontSize {
    pub fn as_str(self) -> &'static str {
        match self {
            FontSize::Standard => "standard",
            FontSize::Medium => "medium",
            FontSize::Large => "large",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FontSize::Standard => "Standard",
            FontSize::Medium => "Medium",
            FontSize::Large => "Large",
        }
    }

    /// Medium is one and a half times standard, large is double.
    pub fn scale(self) -> f32 {
        match self {
            FontSize::Standard => 1.0,
            FontSize::Medium => 1.5,
            FontSize::Large => 2.0,
        }
    }

    fn parse(value: &str) -> Option<FontSize> {
        match value.trim().to_ascii_lowercase().as_str() {
            "standard" => Some(FontSize::Standard),
            "medium" => Some(FontSize::Medium),
            "large" => Some(FontSize::Large),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GuiSettings {
    pub theme: Theme,
    pub font: FontSize,
    pub accent: Accent,
}

impl GuiSettings {
    pub fn load() -> GuiSettings {
        GuiSettings::from_config(&Config::load())
    }

    pub fn from_config(config: &Config) -> GuiSettings {
        GuiSettings {
            theme: config
                .get("ui.theme")
                .and_then(Theme::parse)
                .unwrap_or_default(),
            font: config
                .get("ui.font")
                .and_then(FontSize::parse)
                .unwrap_or_default(),
            accent: config
                .get("ui.accent")
                .and_then(Accent::parse)
                .unwrap_or_default(),
        }
    }

    /// Write both keys, keeping everything else in the file — another writer's
    /// lives there too.
    pub fn save(&self) -> anyhow::Result<()> {
        let path = crate::settings::path().ok_or_else(|| anyhow::anyhow!("no config directory"))?;
        self.save_to(&path)
    }

    /// The half that takes a path, so the tests need no environment fiddling.
    pub fn save_to(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let mut config = Config::load_from(path);
        config.set("ui.theme", self.theme.as_str());
        config.set("ui.font", self.font.as_str());
        config.set("ui.accent", self.accent.as_str());
        config.save_to(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_system_theme_at_standard_size() {
        let settings = GuiSettings::from_config(&Config::default());
        assert_eq!(settings.theme, Theme::System);
        assert_eq!(settings.font, FontSize::Standard);
        assert_eq!(settings.font.scale(), 1.0);
        assert_eq!(settings.accent, Accent::Default);
    }

    #[test]
    fn the_sizes_are_the_ones_the_ticket_asked_for() {
        assert_eq!(FontSize::Standard.scale(), 1.0);
        assert_eq!(FontSize::Medium.scale(), 1.5, "medium is 1.5x standard");
        assert_eq!(FontSize::Large.scale(), 2.0, "large is double standard");
    }

    #[test]
    fn choices_survive_a_round_trip_through_the_config() {
        let mut config = Config::default();
        config.set("ui.theme", "light");
        config.set("ui.font", "large");
        let settings = GuiSettings::from_config(&config);
        assert_eq!(settings.theme, Theme::Light);
        assert_eq!(settings.font, FontSize::Large);
    }

    #[test]
    fn a_choice_is_still_there_the_next_time_the_board_opens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agile-md/config");
        // A config that has been written before, with a key we do not own.
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "theme = nord\n").unwrap();

        GuiSettings {
            theme: Theme::Light,
            font: FontSize::Large,
            accent: Accent::Purple,
        }
        .save_to(&path)
        .unwrap();

        // "Next session": read it back from scratch.
        let reopened = GuiSettings::from_config(&Config::load_from(&path));
        assert_eq!(reopened.theme, Theme::Light);
        assert_eq!(reopened.font, FontSize::Large);
        assert_eq!(reopened.accent, Accent::Purple);
        assert_eq!(
            Config::load_from(&path).get("theme"),
            Some("nord"),
            "another writer's key must not be collateral damage"
        );
    }

    #[test]
    fn accent_default_applies_no_colour_override() {
        assert_eq!(Accent::default(), Accent::Default);
        assert_eq!(Accent::Default.color(), None);
    }

    #[test]
    fn every_other_accent_has_a_colour() {
        for accent in Accent::ALL {
            if accent == Accent::Default {
                continue;
            }
            assert!(accent.color().is_some(), "{accent:?} should have a colour");
        }
    }

    #[test]
    fn accent_survives_a_round_trip_through_the_config() {
        let mut config = Config::default();
        config.set("ui.accent", "green");
        assert_eq!(GuiSettings::from_config(&config).accent, Accent::Green);
    }

    #[test]
    fn nonsense_falls_back_rather_than_failing() {
        let mut config = Config::default();
        config.set("ui.theme", "chartreuse");
        config.set("ui.font", "enormous");
        config.set("ui.accent", "chartreuse");
        let settings = GuiSettings::from_config(&config);
        assert_eq!(settings.theme, Theme::System);
        assert_eq!(settings.font, FontSize::Standard);
        assert_eq!(settings.accent, Accent::Default);
    }
}
