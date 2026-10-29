# Upgrading from converge

Staw was called converge until v0.2.0. `converge update` cannot cross the
rename (the release assets changed names), so install staw (v0.2.1 or
later) and move the old pieces over:

```sh
curl -fsSL https://raw.githubusercontent.com/staw-ai/staw/main/install.sh | sh
mv ~/.config/converge ~/.config/staw
mv ~/.local/state/converge ~/.local/state/staw
# the hosted server moved; a self-hosted one keeps its address
sed -i 's#app\.converge\.expert#app.staw.ai#' ~/.config/staw/cli.toml
staw init
rm -r ~/.converge ~/.local/bin/converge
```

`staw init` moves the hooks over in place. It leaves each harness's
`converge` MCP server, which points at the old address; a redirect drops
the token there, so it fails to connect. Remove it:

- Claude Code: `claude mcp remove --scope user converge`
- Codex: `[mcp_servers.converge]` in `~/.codex/config.toml`
- Cursor: `mcpServers.converge` in `~/.cursor/mcp.json`
- opencode: `mcp.converge` in `opencode.json`, and `plugin/converge.js`
  beside it

`CONVERGE_SERVER` and `CONVERGE_TOKEN` are now `STAW_SERVER` and
`STAW_TOKEN`. In each repository, `git mv .converge .staw` and commit;
until then the repository looks unmapped and the next session asks to
bind it again.
