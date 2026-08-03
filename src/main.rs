use crate::error::AnyResult;

mod arena;
mod db;
pub mod error;
mod render;
mod style;
mod text;
mod ui;

fn main() -> AnyResult<()> {
    Ok(())
}
