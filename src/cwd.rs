#[cfg(test)]
pub use self::test::{Cwd, cwd};
#[cfg(not(test))]
pub use self::real::{Cwd, cwd};

#[cfg(not(test))]
mod real {
    pub type Cwd = std::path::PathBuf;

    /// Returns the current working directory.
    pub fn cwd() -> Cwd {
        match std::env::current_dir() {
            Ok(dir) => dir,
            Err(err) => panic!("failed to get current directory: {err}"),
        }
    }
}

#[cfg(test)]
mod test {
    use std::ops::Deref;
    use std::path::{Path, PathBuf};

    /// Scoped temporary directory; deletes the directory when dropped.
    #[derive(Debug)]
    pub struct Cwd {
        path: PathBuf,
    }

    impl Cwd {
        /// Creates a new temporary directory under the system temp dir.
        pub fn new() -> Self {
            let path = unique_temp_dir();
            std::fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }

        /// Returns the temporary directory path.
        pub fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Default for Cwd {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Deref for Cwd {
        type Target = Path;

        fn deref(&self) -> &Self::Target {
            &self.path
        }
    }

    impl Drop for Cwd {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// Returns a temporary working directory for use in tests.
    pub fn cwd() -> Cwd {
        Cwd::new()
    }

    fn unique_temp_dir() -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};

        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("nagai-test-{}-{n}", std::process::id()))
    }
}
