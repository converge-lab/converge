use converge_e2e::command::Command;

pub fn version() -> Command {
    Command::run(["claude", "--version"])
}

pub fn get_mcp(server: &str) -> Command {
    Command::run(["claude", "mcp", "get", server])
}

pub fn session(cwd: &str, prompt: &str, tools: &[&str]) -> Command {
    Command::run([
        "sh",
        "-c",
        r#"cd "$1" && claude -p "$2" --allowedTools "$3" --output-format json"#,
        "converge-e2e",
        cwd,
        prompt,
        &tools.join(","),
    ])
}
