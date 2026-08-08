#![allow(dead_code)]

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

// Stock Tailwind CSS colors
const WHITE: Color = rgb(255, 255, 255);
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
