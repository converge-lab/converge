use converge_e2e::command::Command;

pub fn version() -> Command {
    Command::run(["codex", "--version"])
}

pub fn get_mcp(server: &str) -> Command {
    Command::run(["codex", "mcp", "get", server])
}
