//! Command implementations.

mod workspace;

use crate::cli::{Cli, Command};
use crate::error::Result;
use crate::output::Output;
use crate::store::Dirs;

/// What every command needs.
pub struct Ctx {
    pub dirs: Dirs,
    pub out: Output,
}

pub fn run(cli: &Cli, out: Output) -> Result<()> {
    let ctx = Ctx {
        dirs: Dirs::from_env()?,
        out,
    };
    match &cli.command {
        Command::Workspace(cmd) => workspace::run(&ctx, cmd),
    }
}
