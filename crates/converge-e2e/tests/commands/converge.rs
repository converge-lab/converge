use converge_e2e::command::Command;

pub const INSTALLER: &str =
    "https://raw.githubusercontent.com/converge-lab/converge/main/install.sh";

pub const BIN: &str = "/home/e2e/.converge/bin/converge";
pub const LINK: &str = "/home/e2e/.local/bin/converge";

pub fn install() -> Command {
    Command::run([
        "sh",
        "-c",
        r#"curl -fsSL "$1" -o /tmp/install.sh && sh /tmp/install.sh"#,
        "converge-e2e",
        INSTALLER,
    ])
}

pub fn version() -> Command {
    Command::run([LINK, "--version"])
}

pub fn init(server: &str, token: &str) -> Command {
    Command::run(["printf", "%s\n%s\n\n", server, token]).piped_into([LINK, "init"])
}
