use crossterm::style::{Attribute, Color, ContentStyle};

pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb { r, g, b }
}

macro_rules! text_style {
    (
        $($attrib:ident: $value:expr),*
        $(; $($deco:expr),*)?
        $(,)?
    ) => {{
        let mut style = crossterm::style::ContentStyle {
            foreground_color: None,
            background_color: None,
            underline_color: None,
            attributes: crossterm::style::Attributes::none(),
        };
        $(style.$attrib = Some($value);)*
        $($(style.attributes = style.attributes.with($deco);)*)?
        style
    }};
}

#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub enum BackgroundColorName {
    Base,
    Collapsible,
    CollapsibleHover,
    InputBox,
}

#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub enum TextStyleName {
    Base,
    Header,
    Subtle,
}

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub bg_base: Color,
    pub bg_collapsible: Color,
    pub bg_collapsible_hover: Color,
    pub bg_input_box: Color,
    pub text_base: ContentStyle,
    pub text_header: ContentStyle,
    pub text_subtle: ContentStyle,
}

impl Theme {
    pub fn get_text_style(&self, name: TextStyleName) -> ContentStyle {
        match name {
            TextStyleName::Base => self.text_base,
            TextStyleName::Header => self.text_header,
            TextStyleName::Subtle => self.text_subtle,
        }
    }

    pub fn get_background_color(&self, name: BackgroundColorName) -> Color {
        match name {
            BackgroundColorName::Base => self.bg_base,
            BackgroundColorName::Collapsible => self.bg_collapsible,
            BackgroundColorName::CollapsibleHover => self.bg_collapsible_hover,
            BackgroundColorName::InputBox => self.bg_input_box,
        }
    }
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
    text_base: text_style! { foreground_color: WHITE },
    text_header: text_style! { foreground_color: WHITE; Attribute::Bold },
    text_subtle: text_style! { foreground_color: GREY_400 },
    bg_base: GREY_950,
    bg_collapsible: GREY_950,
    bg_collapsible_hover: GREY_900,
    bg_input_box: GREY_800,
};

#[derive(Debug)]
pub struct StyleSettings {
    pub theme: Theme,
    pub max_width: u32,
}
