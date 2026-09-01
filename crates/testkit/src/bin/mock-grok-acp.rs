use std::io;

use agent_remote_testkit::{MockGrokAcp, RunOutcome, run_stdio};
use anyhow::Result;

fn main() -> Result<()> {
    let mut engine = MockGrokAcp::from_env()?;
    let outcome = run_stdio(&mut engine, io::stdin().lock(), io::stdout().lock())?;
    if let RunOutcome::Exit(code) = outcome {
        std::process::exit(code);
    }
    Ok(())
}
