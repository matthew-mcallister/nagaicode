#![feature(iter_macro, yield_expr)]

use std::io::{self, stdout};

use crossterm;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode,
    enable_raw_mode,
    EnterAlternateScreen,
    LeaveAlternateScreen,
};

mod app;
mod style;
mod db;
mod render;

use app::{App, AppEvent, AppResult};

fn main() -> AppResult<()> {
    Ok(())
}
