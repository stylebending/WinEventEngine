use iced::Theme;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppTheme {
    #[default]
    Dark,
    Light,
    System,
}

impl AppTheme {
    pub fn to_iced_theme(&self) -> Theme {
        match self {
            AppTheme::Dark => Theme::Dark,
            AppTheme::Light => Theme::Light,
            AppTheme::System => {
                // For now, default to Dark. System detection can be added later.
                Theme::Dark
            }
        }
    }
}

impl fmt::Display for AppTheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppTheme::Dark => write!(f, "Dark"),
            AppTheme::Light => write!(f, "Light"),
            AppTheme::System => write!(f, "System"),
        }
    }
}

// Custom colors for our app
pub mod colors {
    use iced::Color;

    pub const SUCCESS: Color = Color::from_rgb(0.2, 0.8, 0.2);
    pub const ERROR: Color = Color::from_rgb(0.9, 0.2, 0.2);
    pub const WARNING: Color = Color::from_rgb(0.9, 0.7, 0.1);
    pub const INFO: Color = Color::from_rgb(0.2, 0.6, 0.9);
}
