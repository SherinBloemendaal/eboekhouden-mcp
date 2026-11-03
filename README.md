# EB MCP

A menu-bar app (macOS first, Windows from the same code) that signs in to e-Boekhouden and offers its internal API to AI clients (Claude, Cursor) as a local, read-only MCP server.

- Product spec and design boards: `~/projects/eb-mcp-design` (`SPEC.md`, `BOARDS.md`, the EB MCP canvas).
- Roadmap: [docs/ROADMAP.md](docs/ROADMAP.md).

## Stack

Tauri 2 with a Rust core (`src-tauri/`) and a Vue 3 + TypeScript UI (`src/`). The MCP server is `rmcp` over Streamable HTTP on `http://127.0.0.1:47821/mcp`, behind a bearer token kept in the system credential store, a loopback-only Host check and a refusal of any browser `Origin`.

## Develop

```bash
npm install
npm run tauri dev
```

The first start stores a new MCP token in the Keychain (service "EB MCP"). The app lives in the menu bar; closing the main window keeps it running.

## Checks

Run them one at a time:

```bash
npm run typecheck
npm test
cd src-tauri && cargo test && cargo clippy --all-targets
```

Agents never sign in to e-Boekhouden or enter passwords. Tests use mocked pages and answers; real sign-in is tested by hand.
