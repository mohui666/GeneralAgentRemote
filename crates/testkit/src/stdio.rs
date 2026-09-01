use std::io::{BufRead, Write};

use anyhow::{Context, Result};

use crate::{EngineOutput, JsonlEngine};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    EndOfInput,
    Exit(i32),
}

pub fn run_stdio<E, R, W>(engine: &mut E, reader: R, mut writer: W) -> Result<RunOutcome>
where
    E: JsonlEngine,
    R: BufRead,
    W: Write,
{
    for line in reader.lines() {
        let line = line.context("failed to read JSONL request")?;
        if line.trim().is_empty() {
            continue;
        }

        for output in engine.receive_line(&line)? {
            match output {
                EngineOutput::Json(message) => {
                    serde_json::to_writer(&mut writer, &message)
                        .context("failed to encode JSONL response")?;
                    writer.write_all(b"\n")?;
                    writer.flush()?;
                }
                EngineOutput::Exit(code) => return Ok(RunOutcome::Exit(code)),
            }
        }
    }

    Ok(RunOutcome::EndOfInput)
}
