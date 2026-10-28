use staw_e2e::command::Command;

pub fn version() -> Command {
    Command::run(["opencode", "--version"])
}

/// Load the installed plugin the way opencode would: an ES module whose
/// `server` export is a function. A syntax error would otherwise only
/// ever surface as "staw silently does nothing".
pub fn load_plugin() -> Command {
    Command::run([
        "node",
        "--input-type=module",
        "-e",
        "import(process.env.HOME + '/.config/opencode/plugin/staw.js')\
         .then(m => { if (typeof m.server !== 'function') { process.exit(3) } })\
         .catch(e => { console.error(e); process.exit(4) })",
    ])
}
