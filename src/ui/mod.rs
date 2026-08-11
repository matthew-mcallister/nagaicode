// TODO maybe: write a diffing system that compares each row with the row from
// the previous draw and only redraws the portions which changed.

pub mod chat;
pub mod component;
pub mod input_box;
pub mod history;
pub mod markdown;
pub mod padded;
pub mod stacked_view;
pub mod style;
pub mod text;

use std::fmt;

use text::SPACES;

pub use component::Component;

pub(crate) fn write_spaces(f: &mut impl fmt::Write, count: usize) -> fmt::Result {
    let mut remaining = count;
    while remaining != 0 {
        let n = remaining.min(SPACES.len());
        f.write_str(&SPACES[..n])?;
        remaining -= n;
    }
    Ok(())
}
