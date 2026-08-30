// TODO maybe: write a diffing system that compares each row with the row from
// the previous draw and only redraws the portions which changed.

pub mod canvas;
pub mod chat;
pub mod command_editor;
pub mod component;
pub mod input_box;
pub mod history;
pub mod history_view;
pub mod markdown;
pub mod padded;
pub mod render_item;
pub mod scroll_bar;
pub mod stacked_view;
pub mod style;
pub mod styled_string;
pub mod text;
pub mod tool_render_item;

pub use component::Component;
