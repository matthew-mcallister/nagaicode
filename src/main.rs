// TODO: Explicitly mark the dead code we want to keep
#![allow(dead_code)]

use crate::error::AnyResult;

mod arena;
mod db;
pub mod error;
mod render;
mod style;
mod text;
mod ui;

fn main() -> AnyResult<()> {
    ui::run()
}
