// TODO: Explicitly mark the dead code we want to keep
#![allow(dead_code)]

use crate::error::AnyResult;

mod arena;
mod canvas;
mod db;
pub mod error;
mod style;
mod text;
mod ui;

fn main() -> AnyResult<()> {
    ui::run()
}
