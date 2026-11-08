#[cfg(not(test))]
pub(crate) use self::real::init_logging;
#[cfg(test)]
pub(crate) use self::mock::init_logging;

#[cfg(not(test))]
mod real {
    use std::io::Write;
    use std::sync::OnceLock;

    use chrono::Local;
    use env_logger::{Env, Target};

    use crate::config::Dirs;
    use crate::error::AnyResult;

    const LOG_FILE_PREFIX: &str = "nagai";

    static INIT: OnceLock<()> = OnceLock::new();

    /// Returns the path of the log file for this run.
    // FIXME: this should be state_dir() not data_dir()
    fn log_file_path(dirs: &Dirs) -> AnyResult<String> {
        let mut dir = dirs.data().to_path_buf();
        dir.push("log");
        std::fs::create_dir_all(&dir)?;
        let timestamp = Local::now().format("%Y-%m-%dT%H%M%S");
        let path = dir.join(format!("{LOG_FILE_PREFIX}-{timestamp}.log"));
        Ok(path.to_str().unwrap().to_string())
    }

    /// Configures the global logger to write to a timestamped file.
    pub(crate) fn init_logging(dirs: &Dirs) -> AnyResult<()> {
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
    pub(crate) fn init_logging(_dirs: &Dirs) -> AnyResult<()> {
        Ok(())
    }
}