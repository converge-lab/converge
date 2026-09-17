use std::fmt;

use anyhow::{Context, Result};
use testcontainers_modules::testcontainers::core::{CmdWaitFor, ExecCommand};
use testcontainers_modules::testcontainers::{ContainerAsync, Image};

/// Run `argv` to completion inside a container and collect what it said.
/// Shared by the agent (where scenarios run) and the server (where the
/// harness mints its own credentials).
pub(crate) async fn exec<I: Image>(
    container: &ContainerAsync<I>,
    argv: &[String],
    what: &str,
) -> Result<CommandResult> {
    let mut result = container
        .exec(
            ExecCommand::new(argv.iter().map(String::as_str))
                .with_cmd_ready_condition(CmdWaitFor::exit()),
        )
        .await
        .with_context(|| format!("execute in a container: {what}"))?;

    let exit_code = result
        .exit_code()
        .await
        .with_context(|| format!("read the exit code of {what}"))?
        .with_context(|| format!("{what} never exited"))?;

    let stdout = read(result.stdout_to_vec().await, what, "stdout")?;
    let stderr = read(result.stderr_to_vec().await, what, "stderr")?;

    Ok(CommandResult {
        outcome: exit_code.into(),
        stdout,
        stderr,
    })
}

fn read(
    bytes: Result<Vec<u8>, impl std::error::Error + Send + Sync + 'static>,
    what: &str,
    stream: &str,
) -> Result<String> {
    let bytes = bytes.with_context(|| format!("read the {stream} of {what}"))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub enum Command {
    Run(Vec<String>),
    Pipe {
        base: Box<Command>,
        next: Box<Command>,
    },
}

impl Command {
    pub fn run<I, S>(argv: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::Run(argv.into_iter().map(Into::into).collect())
    }

    pub fn piped_into<I, S>(self, argv: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::Pipe {
            base: Box::new(self),
            next: Box::new(Self::run(argv)),
        }
    }

    pub(crate) fn program(&self) -> &str {
        match self {
            Self::Run(argv) => argv.first().map(String::as_str).unwrap_or("(empty)"),
            Self::Pipe { base, .. } => base.program(),
        }
    }

    pub(crate) fn argv(&self) -> Vec<String> {
        if let Self::Run(argv) = self {
            return argv.clone();
        }

        let mut parameters = Vec::new();
        let script = self.script(&mut parameters);

        let mut invocation = vec![
            "sh".to_owned(),
            "-c".to_owned(),
            script,
            "converge-e2e".to_owned(),
        ];
        invocation.extend(parameters);
        invocation
    }

    fn script(&self, parameters: &mut Vec<String>) -> String {
        match self {
            Self::Run(argv) => argv
                .iter()
                .map(|argument| {
                    parameters.push(argument.clone());
                    format!("\"${{{}}}\"", parameters.len())
                })
                .collect::<Vec<_>>()
                .join(" "),
            Self::Pipe { base, next } => {
                format!("{} | {}", base.script(parameters), next.script(parameters))
            }
        }
    }
}

pub struct CommandResult {
    pub outcome: Outcome,
    pub stdout: String,
    pub stderr: String,
}

impl CommandResult {
    pub fn succeeded(&self) -> bool {
        self.outcome == Outcome::Ok
    }
}

impl fmt::Debug for CommandResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "\n{:?}", self.outcome)?;
        for (stream, text) in [("stdout", &self.stdout), ("stderr", &self.stderr)] {
            if text.trim().is_empty() {
                continue;
            }
            writeln!(f, "── {stream} ──")?;
            for line in text.lines() {
                writeln!(f, "{line}")?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    Failed(i64),
    NotExecutable,
    NotFound,
    Killed(i64),
}

impl Outcome {
    pub fn code(self) -> i64 {
        match self {
            Self::Ok => 0,
            Self::NotExecutable => 126,
            Self::NotFound => 127,
            Self::Killed(signal) => 128 + signal,
            Self::Failed(code) => code,
        }
    }
}

impl From<i64> for Outcome {
    fn from(exit_code: i64) -> Self {
        match exit_code {
            0 => Self::Ok,
            126 => Self::NotExecutable,
            127 => Self::NotFound,
            129..=192 => Self::Killed(exit_code - 128),
            other => Self::Failed(other),
        }
    }
}
