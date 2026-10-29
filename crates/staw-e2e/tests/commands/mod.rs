// Each suite is its own binary and uses its own subset of these.
#![allow(dead_code)]

pub mod claude;
pub mod codex;
pub mod opencode;
pub mod staw;

use staw_e2e::command::Command;

/// A shell one-liner in the agent container, as the agent's user.
pub fn sh(script: &str) -> Command {
    Command::run(["sh", "-c", script])
}
