use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "nagai", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Parser)]
pub enum Command {
    Provider,
}

impl Cli {
    pub fn parse_args() -> Self {
        Self::parse()
    }
}

pub fn run_command(text: &str) -> String {
    let Ok(args) = shellwords::split(text) else {
        return "Error: unclosed \"".into();
    };
    let _cmd = match Command::try_parse_from(args) {
        Ok(cmd) => cmd,
        Err(e) => return e.to_string(),
    };
    "".to_owned()
}
