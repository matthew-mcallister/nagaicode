use crossterm::style::Color;

pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb { r, g, b }
}

/// Text appearance
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct TextStyle {
    pub color: Color,
    pub bold: bool,
}

/// Consistent set of color and style choices for content on a given
/// background. Used for UI, not code highlighting.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    /// Background color
    pub bg: Color,
    /// Default text
    pub base: TextStyle,
    /// Header
    pub header: TextStyle,
    /// Faint or faded text
    pub subtle: TextStyle,
}

#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub enum TextStyleName {
    Base,
    Header,
    Faded,
}

impl Palette {
    pub fn get_style(&self, name: TextStyleName) -> &TextStyle {
        match name {
            TextStyleName::Base => &self.base,
            TextStyleName::Header => &self.header,
            TextStyleName::Faded => &self.subtle,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub content: Palette,
    pub collapsible: Palette,
    pub input_box: Palette,
}

#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub enum ThemeStyleName {
    Content,
    Collapsible,
    InputBox,
}

// Stock Tailwind CSS colors
const WHITE: Color = rgb(255, 255, 255);
const GREY_50: Color = rgb(249, 250, 251);
const GREY_100: Color = rgb(243, 244, 246);
const GREY_200: Color = rgb(229, 231, 235);
const GREY_300: Color = rgb(209, 213, 219);
const GREY_400: Color = rgb(156, 163, 175);
const GREY_500: Color = rgb(107, 114, 128);
const GREY_600: Color = rgb(75, 85, 99);
const GREY_700: Color = rgb(55, 65, 81);
const GREY_800: Color = rgb(31, 41, 55);
const GREY_900: Color = rgb(17, 24, 39);
const GREY_950: Color = rgb(3, 7, 18);
const BLACK: Color = rgb(0, 0, 0);

/// Only available theme for now
pub const THEME_DARK: Theme = Theme {
    content: Palette {
        bg: BLACK,
        base: TextStyle { color: WHITE, bold: false },
        header: TextStyle { color: WHITE, bold: true },
        subtle: TextStyle { color: GREY_400, bold: false },
    },
    collapsible: Palette {
        bg: GREY_800,
        base: TextStyle { color: WHITE, bold: false },
        header: TextStyle { color: WHITE, bold: true },
        subtle: TextStyle { color: GREY_400, bold: false },
    },
    input_box: Palette {
        bg: GREY_600,
        base: TextStyle { color: WHITE, bold: false },
        header: TextStyle { color: WHITE, bold: true },
        subtle: TextStyle { color: GREY_400, bold: false },
    },
};

#[derive(Debug)]
pub struct StyleSettings {
    pub theme: Theme,
    pub max_width: u32,
}
