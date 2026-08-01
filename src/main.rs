#![feature(iter_macro, yield_expr)]

use crate::error::AnyResult;

mod db;
pub mod error;
mod render;
mod style;
mod ui;

fn main() -> AnyResult<()> {
    Ok(())
}
