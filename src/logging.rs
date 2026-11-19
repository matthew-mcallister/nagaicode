#[cfg(not(test))]
pub use self::real::init_logging;
#[cfg(test)]
pub use self::mock::init_logging;

#[cfg(not(test))]
mod real {
    use std::io::Write;
    use std::path::Path;
    use std::sync::OnceLock;

    use chrono::Local;
    use env_logger::{Env, Target};

    use crate::config::Dirs;
    use crate::error::AnyResult;

    static INIT: OnceLock<()> = OnceLock::new();

    /// Returns the path of the log file for this run.
    // FIXME: this should be state_dir() not data_dir()
    fn log_file_path(dirs: &Dirs) -> AnyResult<String> {
        let mut dir = dirs.data().to_path_buf();
        dir.push("log");
        std::fs::create_dir_all(&dir)?;
        let timestamp = Local::now().format("%Y-%m-%dT%H%M%S");
        let prefix = (|| -> Option<String> {
            let path = std::env::args().next()?;
            Some(Path::new(&path).file_stem()?.to_str()?.to_owned())
        })().unwrap_or("nagai".to_owned());
        let path = dir.join(format!("{prefix}-{timestamp}.log"));
        Ok(path.to_str().unwrap().to_string())
    }

    /// Configures the global logger to write to a timestamped file.
    // FIXME: Logs aren't being written until after the app closes, but the
    // env_logger crate does appear to flush after each write
    pub fn init_logging(dirs: &Dirs) -> AnyResult<()> {
        if INIT.get().is_some() {
            return Ok(());
        }
        let path = log_file_path(dirs)?;
        let target = Box::new(std::fs::File::create(&path)?);
        env_logger::Builder::from_env(Env::default())
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
        let _ = INIT.set(());
        Ok(())
    }
}

#[cfg(test)]
mod mock {
    use crate::config::Dirs;
    use crate::error::AnyResult;

    /// No-op in tests so log output never touches the filesystem.
    pub fn init_logging(_dirs: &Dirs) -> AnyResult<()> {
        Ok(())
    }
}