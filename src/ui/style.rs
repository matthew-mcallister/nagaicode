#![allow(dead_code)]

use crossterm::Command;
use crossterm::style::{
    Attribute, Attributes, Color, ContentStyle, SetAttribute, SetBackgroundColor, SetForegroundColor
};

pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb { r, g, b }
}

macro_rules! text_style {
    ($($attr:ident: $value:expr),*$(,)?) => {{
        TextStyle {
            $($attr: $value,)*
            ..TextStyle::default()
        }
    }};
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextStyle {
    pub fg_color: Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
}

impl From<TextStyle> for ContentStyle {
    fn from(value: TextStyle) -> Self {
        let mut attributes = Attributes::none();
        if value.bold {
            attributes.set(Attribute::Bold);
        } else {
            attributes.unset(Attribute::NormalIntensity);
        }
        if value.italic {
            attributes.set(Attribute::Italic);
        } else {
            attributes.unset(Attribute::NoItalic);
        }
        if value.strikethrough {
            attributes.set(Attribute::CrossedOut);
        } else {
            attributes.unset(Attribute::NotCrossedOut);
        }
        Self {
            foreground_color: Some(value.fg_color),
            background_color: None,
            underline_color: None,
            attributes,
        }
    }
}

impl TextStyle {
    pub const fn default() -> Self {
        Self {
            fg_color: WHITE,
            bold: false,
            italic: false,
            underline: false,
            strikethrough: false,
        }
    }

    pub const fn bolded(mut self) -> Self {
        self.bold = true;
        self
    }

    pub const fn italicized(mut self) -> Self {
        self.italic = true;
        self
    }

    pub const fn underlined(mut self) -> Self {
        self.underline = true;
        self
    }

    pub const fn struck_out(mut self) -> Self {
        self.strikethrough = true;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Style {
    pub text: TextStyle,
    pub bg_color: Color,
}

impl From<Style> for ContentStyle {
    fn from(value: Style) -> Self {
        let mut style: ContentStyle = value.text.into();
        style.background_color = Some(value.bg_color);
        style
    }
}

impl Style {
    pub fn new(text: TextStyle, bg_color: Color) -> Self {
        Self { text, bg_color }
    }

    pub fn with_text(self, text: TextStyle) -> Self {
        Self {
            text,
            ..self
        }
    }

    pub fn with_bg_color(self, bg_color: Color) -> Self {
        Self {
            bg_color,
            ..self
        }
    }

    pub const fn bolded(mut self) -> Self {
        self.text = self.text.bolded();
        self
    }

    pub const fn italicized(mut self) -> Self {
        self.text = self.text.italicized();
        self
    }

    pub const fn underlined(mut self) -> Self {
        self.text = self.text.underlined();
        self
    }

    pub const fn struck_out(mut self) -> Self {
        self.text = self.text.struck_out();
        self
    }
}

/// Tuple `(old, new)`, replaces terminal text style
#[derive(Clone, Copy, Debug)]
pub struct UpdateStyle(pub Style, pub Style);

impl Command for UpdateStyle {
    fn write_ansi(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        let UpdateStyle(old_style, new_style) = self;

        let (old, new) = (old_style.text, new_style.text);
        if old.fg_color != new.fg_color {
            SetForegroundColor(new.fg_color).write_ansi(f)?;
        }
        if old.bold != new.bold {
            SetAttribute(if new.bold {
                Attribute::Bold
            } else {
                Attribute::NormalIntensity
            })
            .write_ansi(f)?;
        }
        if old.italic != new.italic {
            SetAttribute(if new.italic {
                Attribute::Italic
            } else {
                Attribute::NoItalic
            })
            .write_ansi(f)?;
        }
        if old.underline != new.underline {
            SetAttribute(if new.underline {
                Attribute::Underlined
            } else {
                Attribute::NoUnderline
            })
            .write_ansi(f)?;
        }
        if old_style.bg_color != new_style.bg_color {
            SetBackgroundColor(new_style.bg_color).write_ansi(f)?;
        }

        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub bg_base: Color,
    pub bg_prompt: Color,
    pub bg_input_box: Color,
    pub text_base: TextStyle,
    pub text_header: TextStyle,
    pub text_subtle: TextStyle,
    pub text_quote: TextStyle,
    pub text_code: TextStyle,
    pub text_math: TextStyle,
    pub text_error: TextStyle,
    pub text_scroll_bar_track: TextStyle,
    pub text_scroll_bar_focused: TextStyle,
    pub text_scroll_bar_unfocused: TextStyle,
}

impl Theme {
    pub fn base_style(&self) -> Style {
        Style { text: self.text_base, bg_color: self.bg_base }
    }
}

// Stock Tailwind CSS colors
const WHITE: Color = Color::White;
const GREY_50: Color = rgb(250, 250, 249);
const GREY_100: Color = rgb(245, 245, 244);
const GREY_200: Color = rgb(231, 229, 228);
const GREY_300: Color = rgb(214, 211, 209);
const GREY_400: Color = rgb(168, 162, 158);
const GREY_500: Color = rgb(120, 113, 108);
const GREY_600: Color = rgb(87, 83, 78);
const GREY_700: Color = rgb(68, 64, 60);
const GREY_800: Color = rgb(41, 37, 36);
const GREY_900: Color = rgb(28, 25, 23);
const GREY_950: Color = rgb(12, 10, 9);
const BLACK: Color = Color::Black;

const YELLOW_200: Color = rgb(254, 240, 138);

const RED_400: Color = rgb(248, 113, 113);

const SKY_400: Color = rgb(56, 189, 248);

/// Only available theme for now
pub const THEME_DARK: Theme = Theme {
    text_base: text_style! { fg_color: WHITE },
    text_header: text_style! { fg_color: WHITE, bold: true },
    text_subtle: text_style! { fg_color: GREY_400 },
    text_quote: text_style! { fg_color: GREY_400, italic: true },
    text_code: text_style! { fg_color: YELLOW_200 },
    text_math: text_style! { fg_color: YELLOW_200, italic: true },
    text_error: text_style! { fg_color: RED_400 },
    text_scroll_bar_track: text_style! { fg_color: GREY_950 },
    text_scroll_bar_focused: text_style! { fg_color: SKY_400 },
    text_scroll_bar_unfocused: text_style! { fg_color: GREY_600 },
    bg_base: GREY_950,
    bg_prompt: GREY_900,
    bg_input_box: GREY_800,
};

#[derive(Debug)]
pub struct StyleSettings {
    pub theme: Theme,
    pub max_width: u32,
}
