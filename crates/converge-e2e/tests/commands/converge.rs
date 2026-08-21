use converge_e2e::command::Command;

/// Where agent.Dockerfile drops the workspace-built binary. Absolute
/// because the hooks `init` writes record whatever path the CLI was
/// invoked from, and the assertions read them back.
pub const BIN: &str = "/usr/local/bin/converge";

pub fn version() -> Command {
    Command::run([BIN, "--version"])
}

pub fn init(server: &str, token: &str) -> Command {
    Command::run(["printf", "%s\n%s\n\n", server, token]).piped_into([BIN, "init"])
}
