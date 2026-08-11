use std::error::Error;

use dedent::dedent;
use diesel::QueryDsl;
use diesel::RunQueryDsl;
use diesel::expression_methods::ExpressionMethods;

use crate::db;
use crate::provider::Provider;
use crate::schema::provider::dsl;

#[derive(Debug, Eq, PartialEq)]
pub enum Command {
    Provider(ProviderCommand),
    Quit,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ProviderCommand {
    Ls,
    New {
        name: String,
        interface: String,
        api_key: String,
        base_url: Option<String>,
    },
    Rm(String),
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

fn unknown_command(usage: &'static str, command: &str) -> Box<dyn Error> {
    From::from(CliParseError {
        error: Some(format!("Unknown command: '{}'", command)),
        usage,
    })
}

fn unexpected_argument(usage: &'static str, argument: &str) -> Box<dyn Error> {
    From::from(CliParseError {
        error: Some(format!("Unexpected argument: '{}'", argument)),
        usage,
    })
}

fn missing_argument(usage: &'static str, argument: &str) -> Box<dyn Error> {
    From::from(CliParseError {
        error: Some(format!("Missing argument: '{}'", argument)),
        usage,
    })
}

fn show_usage(usage: &'static str) -> Box<dyn Error> {
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
) -> Result<(), Box<dyn Error>> {
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

    fn expect(&mut self) -> Result<&'a str, Box<dyn Error>> {
        if let Some((arg, rest)) = self.args.split_first() {
            self.args = rest;
            Ok(arg)
        } else {
            Err(show_usage(self.usage))
        }
    }

    fn expect_empty(&mut self) -> Result<(), Box<dyn Error>> {
        if let Some(first) = self.args.first() {
            Err(From::from(unexpected_argument(self.usage, first)))
        } else {
            Ok(())
        }
    }

    fn expect_key_value(&mut self) -> Result<(&'a str, &'a str), Box<dyn Error>> {
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

fn parse_args<'a>(args: Vec<String>) -> Result<Command, Box<dyn Error>> {
    let args_: Vec<&str> = args.iter().map(|s| &s[..]).collect();

    const USAGE: &str = dedent!("
        List of commands:

          /provider     [/p]
          /help         [/h]
          /quit         [/q]
    ");
    let mut parser = Parser::new(USAGE, &args_[..], &[]);
    match parser.expect()? {
        "p" | "provider" => parse_provider(parser.args),
        "q" | "quit" => parse_quit(parser.args),
        "h" | "help" => Err(show_usage(USAGE)),
        command => Err(unknown_command(USAGE, command)),
    }
}

fn parse_quit(args: &[&str]) -> Result<Command, Box<dyn Error>> {
    const USAGE: &str = dedent!("
        Usage:

            /quit
    ");
    let mut parser = Parser::new(USAGE, &args[..], &[]);
    parser.expect_empty()?;
    Ok(Command::Quit)
}

fn parse_provider(args: &[&str]) -> Result<Command, Box<dyn Error>> {
    const USAGE: &str = dedent!("
        List of commands:

          /provider new
          /provider ls
          /provider rm
    ");
    let mut parser = Parser::new(USAGE, &args[..], &[]);
    match parser.expect()? {
        "new" => parse_provider_new(parser.args),
        "ls" => parse_provider_ls(parser.args),
        "rm" => parse_provider_rm(parser.args),
        command => Err(unknown_command(USAGE, command)),
    }
}

fn parse_provider_ls(args: &[&str]) -> Result<Command, Box<dyn Error>> {
    const USAGE: &str = dedent!("
        Usage:

          /provider ls
    ");
    let mut parser = Parser::new(USAGE, &args[..], &[]);
    parser.expect_empty()?;
    Ok(Command::Provider(ProviderCommand::Ls))
}

fn parse_provider_rm(args: &[&str]) -> Result<Command, Box<dyn Error>> {
    const USAGE: &str = dedent!("
        Usage:

          /provider rm 'provider-name-here'
    ");
    let mut parser = Parser::new(USAGE, &args[..], &[]);
    let arg = parser.expect()?;
    parser.expect_empty()?;
    Ok(Command::Provider(ProviderCommand::Rm(arg.into())))
}

fn parse_provider_new(args: &[&str]) -> Result<Command, Box<dyn Error>> {
    const USAGE: &str = dedent!("
        Usage:

            /provider new
                -name 'provider-name-here'
                -interface 'openai'
                -api-key 'sk-api-key-here'
                [-base-url 'https://base.url/here']
    ");
    let mut parser = Parser::new(USAGE, &args[..], &["-name", "-interface", "-api-key", "-base-url"]);

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
    let interface = interface.ok_or_else(|| missing_argument(USAGE, "-interface"))?;

    Ok(Command::Provider(ProviderCommand::New {
        name,
        interface,
        api_key,
        base_url,
    }))
}

pub fn parse_command(text: &str) -> Result<Command, Box<dyn Error>> {
    let args = shellwords::split(text)?;
    parse_args(args)
}

pub fn run_provider_command(command: ProviderCommand) -> Result<String, Box<dyn Error>> {
    let mut conn = db::open()?;
    match command {
        ProviderCommand::Ls => {
            let providers: Vec<Provider> = dsl::provider
                .order(dsl::name.asc())
                .load(&mut conn)?;
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
            Provider::create(&mut conn, &name, &interface, &api_key, base_url_ref)?;
            Ok(format!("Created provider \"{name}\""))
        }
        ProviderCommand::Rm(name) => {
            let deleted = Provider::delete_by_name(&mut conn, &name)?;
            if deleted {
                Ok(format!("Deleted provider \"{name}\""))
            } else {
                Err(format!("no provider named '{name}' found").into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_help() {
        let expected = "List of commands:\n\n  /provider     [/p]\n  /help         [/h]\n  /quit         [/q]";
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
                assert_eq!(interface, "openai");
                assert_eq!(api_key, "sk-123");
                assert_eq!(base_url, None);
            }
            other => panic!("expected New, got {:?}", other),
        }
    }
}
