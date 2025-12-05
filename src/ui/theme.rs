/// Color themes and styling
use ratatui::style::Color;

#[derive(Debug, Clone)]
pub struct Theme {
    pub primary: Color,
    pub secondary: Color,
    pub accent: Color,
    pub background: Color,
    pub foreground: Color,
    pub error: Color,
    pub success: Color,
}

impl Theme {
    pub fn default_dark() -> Self {
        Self {
            primary: Color::Cyan,
            secondary: Color::Magenta,
            accent: Color::Yellow,
            background: Color::Black,
            foreground: Color::White,
            error: Color::Red,
            success: Color::Green,
        }
    }

    pub fn default_light() -> Self {
        Self {
            primary: Color::Blue,
            secondary: Color::Magenta,
            accent: Color::Red,
            background: Color::White,
            foreground: Color::Black,
            error: Color::Red,
            success: Color::Green,
        }
    }

    pub fn spotify() -> Self {
        Self {
            primary: Color::Green,
            secondary: Color::Black,
            accent: Color::White,
            background: Color::Black,
            foreground: Color::White,
            error: Color::Red,
            success: Color::Green,
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::default_dark()
    }
}
