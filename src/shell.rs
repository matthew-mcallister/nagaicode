// XXX: Could support alternative shells if there's a good use case
use tokio::process::Command;

#[cfg(not(windows))]
const PROGRAM: &str = "sh";
#[cfg(windows)]
const PROGRAM: &str = "powershell";

#[cfg(not(windows))]
const COMMAND_FLAG: &str = "-c";
#[cfg(windows)]
const COMMAND_FLAG: &str = "-C";

/// Returns the name of the host shell executable.
pub fn program() -> &'static str {
    PROGRAM
}

/// Creates a command to invoke the system shell on a given input.
pub fn command(command: &str) -> Command {
    let mut cmd = Command::new(PROGRAM);
    cmd.arg(COMMAND_FLAG);
    cmd.arg(command);
    cmd
}
