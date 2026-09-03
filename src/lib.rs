pub mod agent;
pub mod app;
pub mod arena;
pub mod command;
pub mod config;
pub mod cwd;
pub mod db;
pub mod error;
pub mod interface;
pub mod item;
pub mod ui;
pub mod logging;
pub mod model;
pub mod provider;
pub mod query;
pub mod request;
pub mod schema;
pub mod session;
pub mod settings;
pub mod task;
pub mod terminal;
pub mod testing;
pub mod tools;

#[cfg(test)]
mod tests;

#[macro_export]
macro_rules! try_nested {
    ($expr:expr) => {
        match $expr {
            Ok(Some(x)) => x,
            Ok(None) => return Ok(None),
            Err(e) => return Err(e.into()),
        }
    }
}

