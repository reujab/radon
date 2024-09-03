use std::{path::PathBuf, time::Duration};

use anyhow::{anyhow, bail, Error, Result};
use regex::Regex;
use tokio::time::{interval, Interval};
use toml::{Table, Value};

pub struct Config {
    pub monitors: Vec<MonitorConfig>,
    pub variables: Table,
}

pub struct MonitorConfig {
    pub name: String,
    pub mutates_globals: bool,

    pub every: Option<Interval>,
    pub log: Option<PathBuf>,

    pub cooldown: Option<Duration>,
    pub match_log: Option<Regex>,

    pub exec: Option<Exec>,
    pub set: Table,
    pub push: Table,
}

pub enum Exec {
    Shell(String),
    Spawn(Vec<String>),
}

pub fn parse(doc: &str) -> Result<Config> {
    let mut table = doc
        .parse::<Table>()
        .map_err(|err| map_to_readable_syntax_err(doc, err))?;

    let variables = match table.remove("var") {
        Some(var) => match var {
            Value::Table(var) => var,
            _ => bail!("Key `var` must be a table."),
        },
        None => Table::new(),
    };

    // Validate and parse monitors.
    let monitor_configs = match table.remove("monitor") {
        None => bail!("No monitors found!"),
        Some(monitors) => match monitors {
            Value::Table(monitors) => {
                let mut monitor_configs = Vec::with_capacity(monitors.len());
                for (name, monitor) in monitors {
                    let monitor_table = match monitor {
                        Value::Table(monitor) => monitor,
                        _ => bail!("Key `monitor.{name}` must be a table."),
                    };
                    monitor_configs.push(
                        parse_monitor_config(name.clone(), monitor_table)
                            .map_err(|err| anyhow!("Monitor `{name}`: {err}"))?,
                    );
                }
                monitor_configs
            }
            _ => bail!("Key `monitor` must be a table."),
        },
    };

    assert_table_is_empty(table)?;

    Ok(Config {
        monitors: monitor_configs,
        variables,
    })
}

/// Turns a `toml::de::Error` into a human-readable error message.
fn map_to_readable_syntax_err(doc: &str, err: toml::de::Error) -> Error {
    let mut message = err.message().to_owned();
    // Print lines where error occurred.
    if let Some(err_range) = err.span() {
        let mut line_start_byte;
        let mut line_end_byte = 0;
        message += "\n";
        for (i, line) in doc.lines().enumerate() {
            line_start_byte = line_end_byte;
            // Account for new line.
            line_end_byte = line_start_byte + line.len() + 1;
            // Only print the last line.
            if line_end_byte < err_range.end {
                continue;
            }
            message += &format!("\n{}:\t{line}", i + 1);
            message += &format!(
                "\n\t{}{}",
                " ".repeat(err_range.start - line_start_byte),
                "^".repeat(err_range.len())
            );
            break;
        }
    }
    anyhow!("{message}")
}

fn parse_monitor_config(name: String, mut monitor_table: Table) -> Result<MonitorConfig> {
    let every = match monitor_table.remove("every") {
        None => None,
        Some(every) => match every {
            Value::String(every_str) => Some(interval(
                duration_str::parse(every_str).map_err(|err| anyhow!("Key `every`:\n{err}"))?,
            )),
            _ => bail!("Key `every` must be a string."),
        },
    };

    let log = match monitor_table.remove("log") {
        None => None,
        Some(log) => match log {
            Value::String(log_str) => Some(log_str.into()),
            _ => bail!("Key `log` must be a string."),
        },
    };

    let cooldown = match monitor_table.remove("cooldown") {
        None => None,
        Some(cooldown) => match cooldown {
            Value::String(cooldown_str) => Some(
                duration_str::parse(cooldown_str)
                    .map_err(|err| anyhow!("Invalid cooldown:\n{err}"))?,
            ),
            _ => bail!("Key `cooldown` must be a string."),
        },
    };

    let match_log = match monitor_table.remove("match_log") {
        None => None,
        Some(match_log) => {
            let log_regex_str = match_log
                .as_str()
                .ok_or(anyhow!("Key `match_log` must be a string."))?;
            let log_regex = Regex::new(log_regex_str)
                .map_err(|err| anyhow!("Failed to parse match_log: {err}"))?;
            Some(log_regex)
        }
    };

    // Determine whether we'll need a write lock or a read lock to the global state later on.
    let mut mutates_globals = false;

    let set = match monitor_table.remove("set") {
        None => Table::new(),
        Some(set) => match set {
            Value::Table(set) => {
                mutates_globals = true;
                set
            }
            _ => bail!("Key `set` must be a table."),
        },
    };

    let push = match monitor_table.remove("push") {
        None => Table::new(),
        Some(push) => match push {
            Value::Table(push) => {
                mutates_globals = true;
                push
            }
            _ => bail!("Key `push` must be a table."),
        },
    };

    let exec = match monitor_table.remove("exec") {
        None => None,
        Some(exec) => match exec {
            Value::String(exec) => Some(Exec::Shell(exec)),
            Value::Array(args) => match args.is_empty() {
                true => bail!("Key `exec` must not be empty."),
                false => {
                    mutates_globals = true;
                    Some(Exec::Spawn(args.into_iter().map(value_to_string).collect()))
                }
            },
            _ => bail!("Key `exec` must be a string or an array of strings."),
        },
    };

    assert_table_is_empty(monitor_table)?;

    Ok(MonitorConfig {
        name,
        mutates_globals,

        log,
        every,

        cooldown,
        match_log,
        exec,
        set,
        push,
    })
}

pub fn value_to_string(value: Value) -> String {
    match value {
        Value::String(string) => string,
        v => v.to_string(),
    }
}

fn assert_table_is_empty(table: Table) -> Result<()> {
    for key in table.keys() {
        bail!("Invalid key `{key}`");
    }
    Ok(())
}
