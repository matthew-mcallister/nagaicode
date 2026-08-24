#[cfg(not(test))]
pub use self::real::{init_logging, log_file_path};
#[cfg(test)]
pub use self::mock::{init_logging, log_file_path};

#[cfg(not(test))]
mod real {
    use std::io::Write;

    use chrono::Local;
    use env_logger::Target;
    use log::LevelFilter;

    use crate::config::data_dir;
    use crate::error::AnyResult;

    const LOG_FILE_PREFIX: &str = "nagai";

    /// Returns the path of the log file for this run.
    pub fn log_file_path() -> AnyResult<String> {
        let mut dir = data_dir()?;
        dir.push("log");
        std::fs::create_dir_all(&dir)?;
        let timestamp = Local::now().format("%Y-%m-%dT%H%M%S");
        let path = dir.join(format!("{LOG_FILE_PREFIX}-{timestamp}.log"));
        Ok(path.to_str().unwrap().to_string())
    }

    /// Configures the global logger to write to a timestamped file.
    pub fn init_logging() -> AnyResult<()> {
        let path = log_file_path()?;
        let target = Box::new(std::fs::File::create(&path)?);
        env_logger::Builder::new()
            .filter_level(LevelFilter::Debug)
            .target(Target::Pipe(target))
            .format(|buf, record| {
                writeln!(
                    buf,
                    "[{} {}] {}",
                    record.level(),
                    record.target(),
                    record.args(),
                )
            })
            .init();
        Ok(())
    }
}

#[cfg(test)]
mod mock {
    use crate::error::AnyResult;

    /// Returns the path of the log file for this run.
    pub fn log_file_path() -> AnyResult<String> {
        let tmpdir = std::env::var("TMPDIR").unwrap_or_default();
        Ok(format!("{tmpdir}/nagai.log"))
    }

    /// No-op in tests so log output never touches the filesystem.
    pub fn init_logging() -> AnyResult<()> {
        Ok(())
    }
}