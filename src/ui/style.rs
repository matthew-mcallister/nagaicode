use crossterm::Command;
use crossterm::style::{
    Attribute, Attributes, Color as CrosstermColor, ContentStyle, SetAttribute, SetBackgroundColor,
    SetForegroundColor, SetStyle,
};

pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb { r, g, b }
}

/// Port of Color from crossterm
#[derive(Copy, Clone, Debug, PartialEq, Eq, Ord, PartialOrd, Hash)]
pub enum Color {
    Black,
    DarkGrey,
    Red,
    DarkRed,
    Green,
    DarkGreen,
    Yellow,
    DarkYellow,
    Blue,
    DarkBlue,
    Magenta,
    DarkMagenta,
    Cyan,
    DarkCyan,
    White,
    Grey,
    Rgb { r: u8, g: u8, b: u8 },
    AnsiValue(u8),
}

impl From<Color> for CrosstermColor {
    fn from(value: Color) -> Self {
        match value {
            Color::Black => CrosstermColor::Black,
            Color::DarkGrey => CrosstermColor::DarkGrey,
            Color::Red => CrosstermColor::Red,
            Color::DarkRed => CrosstermColor::DarkRed,
            Color::Green => CrosstermColor::Green,
            Color::DarkGreen => CrosstermColor::DarkGreen,
            Color::Yellow => CrosstermColor::Yellow,
            Color::DarkYellow => CrosstermColor::DarkYellow,
            Color::Blue => CrosstermColor::Blue,
            Color::DarkBlue => CrosstermColor::DarkBlue,
            Color::Magenta => CrosstermColor::Magenta,
            Color::DarkMagenta => CrosstermColor::DarkMagenta,
            Color::Cyan => CrosstermColor::Cyan,
            Color::DarkCyan => CrosstermColor::DarkCyan,
            Color::White => CrosstermColor::White,
            Color::Grey => CrosstermColor::Grey,
            Color::Rgb { r, g, b } => CrosstermColor::Rgb { r, g , b },
            Color::AnsiValue(v) => CrosstermColor::AnsiValue(v),
        }
    }
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
            foreground_color: Some(value.fg_color.into()),
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
        style.background_color = Some(value.bg_color.into());
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

impl std::fmt::Display for Style {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        SetStyle((*self).into()).write_ansi(f)
    }
}

pub trait CommandWriter: Sized {
    type Error;

    fn write(&mut self, cmd: &impl Command) -> Result<(), Self::Error>;
}

impl<'a, W> CommandWriter for &'a mut W
    where W: CommandWriter
{
    type Error = W::Error;

    fn write(&mut self, cmd: &impl Command) -> Result<(), Self::Error> {
        <W as CommandWriter>::write(*self, cmd)
    }
}

#[derive(Debug)]
pub struct AnsiCommandWriter<'a, W: std::fmt::Write>(pub &'a mut W);

impl<'a, W: std::fmt::Write> CommandWriter for AnsiCommandWriter<'a, W> {
    type Error = std::fmt::Error;

    fn write(&mut self, cmd: &impl Command) -> std::fmt::Result {
        cmd.write_ansi(self.0)
    }
}

/// Wrapper around write_ansi/execute_winapi.
#[derive(Clone, Copy, Debug, Default)]
#[cfg(windows)]
pub struct WinapiCommandWriter;

#[cfg(windows)]
impl CommandWriter for WinapiCommandWriter {
    type Error = std::io::Error;

    fn write(&mut self, cmd: &impl Command) -> std::io::Result<()> {
        cmd.execute_winapi()
    }
}

/// Cross-platform version of [`crossterm::Command`]`.
pub trait CrossPlatformCommand {
    fn write<W: CommandWriter>(&self, w: W) -> Result<(), W::Error>;
}

macro_rules! cross_platform_command {
    ($ty:ty) => {
        impl crossterm::Command for $ty {
            fn write_ansi(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
                use crate::ui::style::CommandWriter;
                crate::ui::style::AnsiCommandWriter(f).write(self)
            }

            #[cfg(windows)]
            fn execute_winapi(&self) -> std::io::Result<()> {
                use crate::ui::style::CommandWriter;
                crate::ui::style::WinapiCommandWriter.write(self)
            }
        }
    }
}

pub(crate) use cross_platform_command;

/// Tuple `(old, new)`, replaces terminal text style
#[derive(Clone, Copy, Debug)]
pub struct UpdateStyle(pub Style, pub Style);

impl CrossPlatformCommand for UpdateStyle {
    fn write<W: CommandWriter>(&self, mut w: W) -> Result<(), W::Error> {
        let UpdateStyle(old_style, new_style) = self;

        let (old, new) = (old_style.text, new_style.text);
        if old.fg_color != new.fg_color {
            w.write(&SetForegroundColor(new.fg_color.into()))?;
        }
        if old.bold != new.bold {
            w.write(&SetAttribute(if new.bold {
                Attribute::Bold
            } else {
                Attribute::NormalIntensity
            }))?;
        }
        if old.italic != new.italic {
            w.write(&SetAttribute(if new.italic {
                Attribute::Italic
            } else {
                Attribute::NoItalic
            }))?;
        }
        if old.underline != new.underline {
            w.write(&SetAttribute(if new.underline {
                Attribute::Underlined
            } else {
                Attribute::NoUnderline
            }))?;
        }
        if old_style.bg_color != new_style.bg_color {
            w.write(&SetBackgroundColor(new_style.bg_color.into()))?;
        }

        Ok(())
    }
}

cross_platform_command!(UpdateStyle);

impl std::fmt::Display for UpdateStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.write(AnsiCommandWriter(f))
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
    pub text_thought: TextStyle,
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
#[allow(dead_code)] const GREY_50: Color = rgb(250, 250, 249);
#[allow(dead_code)] const GREY_100: Color = rgb(245, 245, 244);
#[allow(dead_code)] const GREY_200: Color = rgb(231, 229, 228);
#[allow(dead_code)] const GREY_300: Color = rgb(214, 211, 209);
const GREY_400: Color = rgb(168, 162, 158);
#[allow(dead_code)] const GREY_500: Color = rgb(120, 113, 108);
const GREY_600: Color = rgb(87, 83, 78);
#[allow(dead_code)] const GREY_700: Color = rgb(68, 64, 60);
const GREY_800: Color = rgb(41, 37, 36);
const GREY_900: Color = rgb(28, 25, 23);
const GREY_950: Color = rgb(12, 10, 9);
#[allow(dead_code)] const BLACK: Color = Color::Black;

const YELLOW_200: Color = rgb(254, 240, 138);

const RED_400: Color = rgb(248, 113, 113);

const SKY_400: Color = rgb(56, 189, 248);

const EMERALD_400: Color = rgb(52, 211, 153);
#[allow(dead_code)] const EMERALD_600: Color = rgb(5, 150, 105);

/// Only available theme for now
pub const THEME_DARK: Theme = Theme {
    text_base: text_style! { fg_color: WHITE },
    text_header: text_style! { fg_color: WHITE, bold: true },
    text_subtle: text_style! { fg_color: GREY_400 },
    text_quote: text_style! { fg_color: GREY_400, italic: true },
    text_code: text_style! { fg_color: YELLOW_200 },
    text_math: text_style! { fg_color: YELLOW_200, italic: true },
    text_error: text_style! { fg_color: RED_400 },
    text_thought: text_style! { fg_color: EMERALD_400 },
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

/// Styling helpers for tests
#[cfg(test)]
pub mod testing {
    use crossterm::style::{Attribute, SetAttribute};

    use super::*;

    #[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
    pub struct SetItalic;

    impl CrossPlatformCommand for SetItalic {
        fn write<W: CommandWriter>(&self, mut w: W) -> Result<(), W::Error> {
            w.write(&SetAttribute(Attribute::Italic))
        }
    }

    cross_platform_command!(SetItalic);

    impl std::fmt::Display for SetItalic {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            self.write(AnsiCommandWriter(f))
        }
    }

    #[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
    pub struct ResetItalic;

    impl CrossPlatformCommand for ResetItalic {
        fn write<W: CommandWriter>(&self, mut w: W) -> Result<(), W::Error> {
            w.write(&SetAttribute(Attribute::NoItalic))
        }
    }

    cross_platform_command!(ResetItalic);

    impl std::fmt::Display for ResetItalic {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            self.write(AnsiCommandWriter(f))
        }
    }

    #[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
    pub struct SetBold;

    impl CrossPlatformCommand for SetBold {
        fn write<W: CommandWriter>(&self, mut w: W) -> Result<(), W::Error> {
            w.write(&SetAttribute(Attribute::Bold))
        }
    }

    cross_platform_command!(SetBold);

    impl std::fmt::Display for SetBold {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            self.write(AnsiCommandWriter(f))
        }
    }

    #[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
    pub struct ResetBold;

    impl CrossPlatformCommand for ResetBold {
        fn write<W: CommandWriter>(&self, mut w: W) -> Result<(), W::Error> {
            w.write(&SetAttribute(Attribute::NormalIntensity))
        }
    }

    cross_platform_command!(ResetBold);

    impl std::fmt::Display for ResetBold {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            self.write(AnsiCommandWriter(f))
        }
    }
}
