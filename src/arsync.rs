use crate::{arsync, commands, config};
use clap::{arg, Command};

pub fn command() -> Command {
    Command::new("arsync")
        .args(args())
        .subcommand(crate::commands::config::command())
        .subcommand(crate::commands::drives::command())
        .subcommand(crate::commands::down::command())
        .subcommand(crate::commands::up::command())
}

fn args() -> Vec<clap::Arg> {
    vec![arg!(-c --config <CONFIG_FILE_PATH> "Path to the config file path, like `config.toml`")]
}

pub async fn run_command() -> anyhow::Result<()> {
    let matches = arsync::command().get_matches();
    let config_path: Option<&String> = matches.get_one("config");
    if let Some((command_name, args)) = matches.subcommand() {
        if let Some(config_path) = config_path {
            config::set_path(config_path.as_str()).await?;
            match command_name {
                commands::config::COMMAND_NAME => {
                    commands::config::run_sub_command(args).await?;
                }
                commands::drives::COMMAND_NAME => {
                    commands::drives::run_sub_command(args).await?;
                }
                commands::down::COMMAND_NAME => {
                    commands::down::run_sub_command(args).await?;
                }
                commands::up::COMMAND_NAME => {
                    commands::up::run_sub_command(args).await?;
                }
                _ => {
                    arsync::command().print_help()?;
                }
            }
        } else {
            arsync::command().print_help()?;
            eprintln!("\nError: config file path is required when using subcommands\n\n");
        }
    } else {
        arsync::command().print_help()?;
    }
    Ok(())
}
