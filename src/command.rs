// TODO: Should execute the correct command as long as you type any prefix that
// uniquely determines that command
use std::sync::Arc;

use anyhow::anyhow;

use dedent::dedent;
use diesel::QueryDsl;
use diesel::RunQueryDsl;
use diesel::expression_methods::ExpressionMethods;

use crate::app::App;
use crate::app::AppEvent;
use crate::error::{AnyError, AnyResult};
use crate::interface::InterfaceId;
use crate::model::Model;
use crate::provider::Provider;
use crate::schema::provider::dsl;
use crate::session::Session;
use crate::task::{Task, TaskContext};
use crate::ui::text::truncate_line;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    Provider(ProviderCommand),
    Model(ModelCommand),
    Session(SessionCommand),
    Quit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderCommand {
    Ls,
    New {
        name: String,
        interface: InterfaceId,
        api_key: String,
        base_url: Option<String>,
    },
    Rm(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelCommand {
    Ls,
    Switch { provider: String, model: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionCommand {
    Ls,
    New,
    Switch(i32),
}

#[derive(Clone, Debug)]
struct CliParseError {
    error: Option<String>,
    usage: &'static str,
}

impl std::fmt::Display for CliParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(error) = &self.error {
            writeln!(f, "{}\n", error)?;
        }
        write!(f, "{}", self.usage)
    }
}

impl std::error::Error for CliParseError {}

fn unknown_command(usage: &'static str, command: &str) -> AnyError {
    From::from(CliParseError {
        error: Some(format!("Unknown command: '{}'", command)),
        usage,
    })
}

fn unexpected_argument(usage: &'static str, argument: &str) -> AnyError {
    From::from(CliParseError {
        error: Some(format!("Unexpected argument: '{}'", argument)),
        usage,
    })
}

fn missing_argument(usage: &'static str, argument: &str) -> AnyError {
    From::from(CliParseError {
        error: Some(format!("Missing argument: '{}'", argument)),
        usage,
    })
}

fn show_usage(usage: &'static str) -> AnyError {
    From::from(CliParseError {
        error: None,
        usage,
    })
}

fn set_arg<T>(
    usage: &'static str,
    var: &mut Option<T>,
    key: &str,
    value: impl Into<T>,
) -> Result<(), AnyError> {
    if var.is_some() {
        Err(From::from(CliParseError {
            error: Some(format!("Repeated argument: '{}'", key)),
            usage,
        }))
    } else {
        *var = Some(value.into());
        Ok(())
    }
}

#[derive(Debug)]
struct Parser<'a> {
    usage: &'static str,
    args: &'a [&'a str],
    keys: &'static [&'static str],
}

impl<'a> Parser<'a> {
    fn new(usage: &'static str, args: &'a [&'a str], keys: &'static [&'static str]) -> Self {
        Parser { usage, args, keys }
    }

    fn is_empty(&self) -> bool {
        self.args.is_empty()
    }

    fn expect(&mut self) -> Result<&'a str, AnyError> {
        if let Some((arg, rest)) = self.args.split_first() {
            self.args = rest;
            Ok(arg)
        } else {
            Err(show_usage(self.usage))
        }
    }

    fn expect_empty(&mut self) -> Result<(), AnyError> {
        if let Some(first) = self.args.first() {
            Err(unexpected_argument(self.usage, first))
        } else {
            Ok(())
        }
    }

    fn expect_key_value(&mut self) -> Result<(&'a str, &'a str), AnyError> {
        let first = self.expect()?;
        let second = self.expect()?;
        if !first.starts_with('-') {
            return Err(unexpected_argument(self.usage, first));
        }
        if !self.keys.contains(&first) {
            return Err(unexpected_argument(self.usage, first));
        }
        Ok((first, second))
    }
}

fn parse_args(args: Vec<String>) -> Result<Command, AnyError> {
    let args_: Vec<&str> = args.iter().map(|s| &s[..]).collect();

    const USAGE: &str = dedent!("
        List of commands:

          /provider     [/p]
          /model        [/m]
          /session      [/s]
          /help         [/h]
          /quit         [/q]
    ");
    let mut parser = Parser::new(USAGE, &args_[..], &[]);
    match parser.expect()? {
        "p" | "provider" => parse_provider(parser.args),
        "m" | "model" => parse_model(parser.args),
        "s" | "session" => parse_session(parser.args),
        "q" | "quit" => parse_quit(parser.args),
        "h" | "help" => Err(show_usage(USAGE)),
        command => Err(unknown_command(USAGE, command)),
    }
}

fn parse_quit(args: &[&str]) -> Result<Command, AnyError> {
    const USAGE: &str = dedent!("
        Usage:

            /quit
    ");
    let mut parser = Parser::new(USAGE, args, &[]);
    parser.expect_empty()?;
    Ok(Command::Quit)
}

fn parse_provider(args: &[&str]) -> Result<Command, AnyError> {
    const USAGE: &str = dedent!("
        List of commands:

          /provider new
          /provider ls
          /provider rm
    ");
    let mut parser = Parser::new(USAGE, args, &[]);
    match parser.expect()? {
        "new" => parse_provider_new(parser.args),
        "ls" => parse_provider_ls(parser.args),
        "rm" => parse_provider_rm(parser.args),
        command => Err(unknown_command(USAGE, command)),
    }
}

fn parse_provider_ls(args: &[&str]) -> Result<Command, AnyError> {
    const USAGE: &str = dedent!("
        Usage:

          /provider ls
    ");
    let mut parser = Parser::new(USAGE, args, &[]);
    parser.expect_empty()?;
    Ok(Command::Provider(ProviderCommand::Ls))
}

fn parse_provider_rm(args: &[&str]) -> Result<Command, AnyError> {
    const USAGE: &str = dedent!("
        Usage:

          /provider rm 'provider-name-here'
    ");
    let mut parser = Parser::new(USAGE, args, &[]);
    let arg = parser.expect()?;
    parser.expect_empty()?;
    Ok(Command::Provider(ProviderCommand::Rm(arg.into())))
}

fn parse_provider_new(args: &[&str]) -> Result<Command, AnyError> {
    const USAGE: &str = dedent!("
        Usage:

            /provider new
                -name 'provider-name-here'
                -interface 'openai'
                -api-key 'sk-api-key-here'
                [-base-url 'https://base.url/here']
    ");
    let mut parser = Parser::new(USAGE, args, &["-name", "-interface", "-api-key", "-base-url"]);

    let mut name: Option<String> = None;
    let mut interface: Option<String> = None;
    let mut api_key: Option<String> = None;
    let mut base_url: Option<String> = None;

    while !parser.is_empty() {
        let (k, v) = parser.expect_key_value()?;
        match k {
            "-name" => set_arg(USAGE, &mut name, k, v)?,
            "-interface" => set_arg(USAGE, &mut interface, k, v)?,
            "-api-key" => set_arg(USAGE, &mut api_key, k, v)?,
            "-base-url" => set_arg(USAGE, &mut base_url, k, v)?,
            _ => unreachable!(),
        }
    }

    let name = name.ok_or_else(|| missing_argument(USAGE, "-name"))?;
    let api_key = api_key.ok_or_else(|| missing_argument(USAGE, "-api-key"))?;
    let interface = interface.ok_or_else(|| missing_argument(USAGE, "-interface"))?.parse()?;

    Ok(Command::Provider(ProviderCommand::New {
        name,
        interface,
        api_key,
        base_url,
    }))
}

fn parse_model(args: &[&str]) -> Result<Command, AnyError> {
    const USAGE: &str = dedent!("
        List of commands:

          /model ls
          /model switch
    ");
    let mut parser = Parser::new(USAGE, args, &[]);
    match parser.expect()? {
        "ls" => parse_model_ls(parser.args),
        "switch" => parse_model_switch(parser.args),
        command => Err(unknown_command(USAGE, command)),
    }
}

fn parse_model_ls(args: &[&str]) -> Result<Command, AnyError> {
    const USAGE: &str = dedent!("
        Usage:

          /model ls
    ");
    let mut parser = Parser::new(USAGE, args, &[]);
    parser.expect_empty()?;
    Ok(Command::Model(ModelCommand::Ls))
}

fn parse_model_switch(args: &[&str]) -> Result<Command, AnyError> {
    const USAGE: &str = dedent!("
        Usage:

          /model switch 'provider-name-here' 'model-id-here'
    ");
    let mut parser = Parser::new(USAGE, args, &[]);
    let provider = parser.expect()?;
    let model = parser.expect()?;
    parser.expect_empty()?;
    Ok(Command::Model(ModelCommand::Switch {
        provider: provider.into(),
        model: model.into(),
    }))
}

fn parse_session(args: &[&str]) -> Result<Command, AnyError> {
    if args.is_empty() {
        return Ok(Command::Session(SessionCommand::Ls));
    }
    const USAGE: &str = dedent!("
        Usage:

          /session ls
          /session new
          /session switch <id>
    ");
    let mut parser = Parser::new(USAGE, &args, &[]);
    match parser.expect()? {
        "ls" => {
            parser.expect_empty()?;
            Ok(Command::Session(SessionCommand::Ls))
        }
        "new" => {
            parser.expect_empty()?;
            Ok(Command::Session(SessionCommand::New))
        }
        "switch" => {
            let arg = parser.expect()?;
            parser.expect_empty()?;
            let id: i32 = arg.parse().map_err(|_| CliParseError {
                error: Some(format!("Invalid session id: '{arg}'")),
                usage: USAGE,
            })?;
            Ok(Command::Session(SessionCommand::Switch(id)))
        }
        command => Err(unknown_command(USAGE, command)),
    }
}

pub fn parse_command(text: &str) -> Result<Command, AnyError> {
    let args = shellwords::split(text)?;
    parse_args(args)
}

pub fn run_provider_command(
    app: &mut App,
    command: ProviderCommand,
) -> Result<String, AnyError> {
    match command {
        ProviderCommand::Ls => {
            let providers: Vec<Provider> = dsl::provider
                .order(dsl::name.asc())
                .load(app.conn())?;
            if providers.is_empty() {
                return Ok("No providers configured.".into());
            }
            let mut out = String::new();
            for p in providers {
                out.push_str(&format!(
                    "{:<20} {:<10} {}\n",
                    p.name, p.interface, p.base_url.unwrap_or_default()
                ));
            }
            Ok(out)
        }
        ProviderCommand::New {
            name,
            interface,
            api_key,
            base_url,
        } => {
            let base_url_ref = base_url.as_deref();
            Provider::create(app.conn(), &name, interface, &api_key, base_url_ref)?;
            // FIXME urgent: Need to revalidate models
            Ok(format!("Created provider \"{name}\""))
        }
        ProviderCommand::Rm(name) => {
            let deleted = Provider::delete_by_name(app.conn(), &name)?;
            if deleted {
                app.on_provider_deleted(&name)?;
                Ok(format!("Deleted provider \"{name}\""))
            } else {
                Err(anyhow!("no provider named '{name}' found"))
            }
        }
    }
}

pub fn run_model_command(
    app: &mut App,
    command: ModelCommand,
) -> Result<String, AnyError> {    match command {
        ModelCommand::Ls => {
            use crate::schema::model::dsl as model_dsl;

            let selected = app
                .selected_model()
                .map(|(sp, sm)| (sp.id, sm.id.clone()));
            let rows: Vec<(Provider, Model)> = dsl::provider
                .inner_join(model_dsl::model)
                .order((dsl::id, model_dsl::id))
                .load(app.conn())?;
            if rows.is_empty() {
                return Ok("No models available. Add a provider by typing /provider".into());
            }
            let mut out = String::new();
            for (p, m) in rows {
                let selected_here =
                    matches!(&selected, Some((spid, smid)) if *spid == p.id && *smid == m.id);
                let marker = if selected_here { "* " } else { "  " };
                out.push_str(&format!("{:<20} {}{}\n", p.name, marker, m.id));
            }
            Ok(out)
        }
        ModelCommand::Switch { provider, model } => {
            let p = Provider::get_by_name(app.conn(), &provider)?
                .ok_or_else(|| anyhow!("No provider '{provider}'"))?;
            let m = Model::get(app.conn(), p.id, &model)?
                .ok_or_else(|| anyhow!("No model '{provider}:{model}''"))?;
            app.switch_model(p, m)?;
            Ok(format!("Using '{provider}:{model}'"))
        }
    }
}

pub async fn run_session_command(
    app: &mut App,
    command: SessionCommand,
) -> Result<String, AnyError> {
    match command {
        SessionCommand::Ls => {
            const SESSION_LIST_NAME_WIDTH: usize = 30;
            let sessions = Session::all(app.conn())?;
            if sessions.is_empty() {
                return Ok("No sessions yet.".into());
            }
            // TODO: colored output
            let active_id = app.session().map(|s| s.id);
            let mut out = String::new();
            for s in sessions {
                let marker = if Some(s.id) == active_id { "* " } else { "  " };
                let name = truncate_line(SESSION_LIST_NAME_WIDTH, &s.name)
                    .to_padded_string(SESSION_LIST_NAME_WIDTH);
                let created = s.created_at.format("%Y-%m-%d %H:%M");
                let id = s.id;
                out.push_str(&format!("{marker}{id:>4}  {name}  {created}\n"));
            }
            Ok(out)
        }
        SessionCommand::New => {
            app.new_session().await?;
            Ok(String::new())
        }
        SessionCommand::Switch(id) => {
            app.switch_session(id).await?;
            Ok(String::new())
        }
    }
}

/// Executes a host command in the working directory, reporting the result to
/// the UI.
pub struct BangCommand {
    command: String,
}

impl BangCommand {
    /// Creates a task that runs `command` on the host.
    pub fn new(command: String) -> Self {
        Self { command }
    }

    async fn process(self, context: &mut TaskContext) -> AnyResult<()> {
        context.send(AppEvent::CommandPrompt(format!("$ {}", self.command)));
        let cwd = Arc::clone(context.cwd());
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&self.command)
            .current_dir(&**cwd)
            .output()
            .await
            .map_err(|e| anyhow!("failed to run 'sh': {e}"))?;
        let return_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if return_code == 0 {
            context.send(AppEvent::CommandOutput(format!("{stdout}{stderr}")));
            Ok(())
        } else {
            Err(anyhow!("command exited with code {return_code}: {stdout}{stderr}"))
        }
    }
}

impl Task for BangCommand {
    type Output = ();

    async fn run(self, context: &mut TaskContext) {
        if let Err(e) = self.process(context).await {
            context.send(AppEvent::ErrorMessage(e.to_string()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_help() {
        let expected = "List of commands:\n\n  /provider     [/p]\n  /model        [/m]\n  /session      [/s]\n  /help         [/h]\n  /quit         [/q]";
        let result = parse_args(vec!["help".into()]).unwrap_err().to_string();
        assert_eq!(result, expected);
    }

    #[test]
    fn test_provider() {
        let cmd = parse_args(vec!["provider".into(), "ls".into()]).expect("parse ls failed");
        assert_eq!(cmd, Command::Provider(ProviderCommand::Ls));

        let cmd = parse_args(vec!["provider".into(), "rm".into(), "foo".into()])
            .expect("parse rm failed");
        assert_eq!(cmd, Command::Provider(ProviderCommand::Rm("foo".into())));

        let args = "provider new -name foo -interface openai -api-key sk-123";
        let cmd = parse_args(shellwords::split(args).unwrap()).unwrap();
        match cmd {
            Command::Provider(ProviderCommand::New {
                name,
                interface,
                api_key,
                base_url,
            }) => {
                assert_eq!(name, "foo");
                assert_eq!(interface, InterfaceId::Openai);
                assert_eq!(api_key, "sk-123");
                assert_eq!(base_url, None);
            }
            other => panic!("expected New, got {:?}", other),
        }
    }

    #[test]
    fn test_model() {
        let cmd = parse_args(shellwords::split("model ls").unwrap()).unwrap();
        assert_eq!(cmd, Command::Model(ModelCommand::Ls));

        let cmd = parse_args(shellwords::split("m ls").unwrap()).unwrap();
        assert_eq!(cmd, Command::Model(ModelCommand::Ls));

        let extra = parse_args(shellwords::split("model ls extra").unwrap());
        assert!(extra.is_err(), "expected error, got {:?}", extra.unwrap());

        let cmd = parse_args(shellwords::split("model switch openai gpt-4").unwrap()).unwrap();
        assert_eq!(
            cmd,
            Command::Model(ModelCommand::Switch {
                provider: "openai".into(),
                model: "gpt-4".into(),
            })
        );

        let missing = parse_args(shellwords::split("model switch openai").unwrap());
        assert!(missing.is_err(), "expected error, got {:?}", missing.unwrap());

        let extra = parse_args(shellwords::split("model switch openai gpt-4 x").unwrap());
        assert!(extra.is_err(), "expected error, got {:?}", extra.unwrap());
    }

    #[test]
    fn test_session() {
        let cmd = parse_args(vec!["session".into()]).expect("parse bare failed");
        assert_eq!(cmd, Command::Session(SessionCommand::Ls));

        let cmd = parse_args(vec!["s".into()]).expect("parse alias failed");
        assert_eq!(cmd, Command::Session(SessionCommand::Ls));

        let cmd = parse_args(vec!["s".into(), "ls".into()]).expect("parse ls failed");
        assert_eq!(cmd, Command::Session(SessionCommand::Ls));

        let cmd = parse_args(vec!["session".into(), "new".into()]).expect("parse new failed");
        assert_eq!(cmd, Command::Session(SessionCommand::New));

        let cmd = parse_args(vec!["s".into(), "new".into()]).expect("parse alias new failed");
        assert_eq!(cmd, Command::Session(SessionCommand::New));

        let cmd = parse_args(vec!["s".into(), "switch".into(), "3".into()])
            .expect("parse switch failed");
        assert_eq!(cmd, Command::Session(SessionCommand::Switch(3)));

        let bad = parse_args(vec!["s".into(), "switch".into(), "abc".into()]);
        assert!(bad.is_err(), "expected error, got {:?}", bad.unwrap());

        let missing = parse_args(vec!["s".into(), "switch".into()]);
        assert!(missing.is_err(), "expected error, got {:?}", missing.unwrap());

        let extra = parse_args(vec!["s".into(), "switch".into(), "3".into(), "4".into()]);
        assert!(extra.is_err(), "expected error, got {:?}", extra.unwrap());

        let extra = parse_args(vec!["session".into(), "new".into(), "x".into()]);
        assert!(extra.is_err(), "expected error, got {:?}", extra.unwrap());

        let extra = parse_args(vec!["session".into(), "ls".into(), "x".into()]);
        assert!(extra.is_err(), "expected error, got {:?}", extra.unwrap());

        let unknown = parse_args(vec!["session".into(), "foo".into()]);
        assert!(unknown.is_err(), "expected error, got {:?}", unknown.unwrap());
    }
}
