use clap::Command;

pub const COMMAND_NAME: &str = "down";

pub fn command() -> Command {
    Command::new(COMMAND_NAME).args(args())
}

fn args() -> Vec<clap::Arg> {
    vec![]
}

pub(crate) async fn run_sub_command(_args: &clap::ArgMatches) -> anyhow::Result<()> {
    // run warp server
    Ok(())
}
