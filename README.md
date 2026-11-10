# EB MCP

A menu-bar app (macOS, and Windows from the same code) that signs in to e-Boekhouden and offers its internal API to AI clients (Claude, Cursor) as a local, read-only MCP server.

- Product spec and design boards: `~/projects/eb-mcp-design` (`SPEC.md`, `BOARDS.md`, the EB MCP canvas).
- Roadmap and what is left: [docs/ROADMAP.md](docs/ROADMAP.md).

## How it works

- **Accounts** (`src-tauri/src/accounts.rs`): label, username or `op://` item, administration and which store holds the password. Never a password in the app's files.
- **Credentials** (`creds.rs`): 1Password through the `op` CLI, or the system store (Keychain with Touch ID, Credential Manager with Windows Hello), read only at sign-in.
- **Sign-in as e-Boekhouden's own app does it** (`signin.rs`, `ebapi.rs`): the username and password from 1Password or the Keychain go to `auth/login` on e-Boekhouden's API, which answers with the session token. A two-step code is asked in EB MCP's own field and sent with a second login. EB MCP then checks the session with `POST auth`, as their app does at startup. Token and cookies stay in memory only. (Until 0.1.8 a hidden webview filled their classic login page; that signs in to the old ASP pages only, which their API never accepted.)
- **API** (`ebapi.rs`): GET only under `/v1/api/`, with the session's cookies, 30 reads a minute and 2 at a time, and a growing pause after errors.
- **MCP** (`mcp.rs`, `tools.rs`): Streamable HTTP on `http://127.0.0.1:47821/mcp`, behind a bearer token, loopback Host only and no browser `Origin`. Twelve read-only tools; the generic `get` reads only the 334 plain paths in `data/get-paths.txt` (`scripts/gen_get_paths.py`).
- **Request log and Dashboard** (`reqlog.rs`): the last 1000 calls in memory, MCP calls linked to the e-Boekhouden calls they caused, secrets masked. Statistics per run.
- **Koppelen** (`connect.rs`, `bridge/`): writes the entry into Claude Desktop's or Cursor's config after a confirm, with a backup and a read-back. Claude Desktop starts only stdio servers, so it gets `eb-mcp-bridge`, a small sidecar that forwards to the HTTP server.
- **Updates** (`updates.rs`, ported from Tresso): Sparkle on macOS, Tauri's updater on Windows, signed releases from `.github/workflows/release.yml`.

## Develop

```bash
npm install
npm run tauri dev
```

`tauri dev` first fetches Sparkle (pinned, SHA-256 checked) and builds the bridge. Debug builds keep the MCP token in `dev-token.json` in the app's config folder, because every rebuild would otherwise ask for Keychain access again. Release builds keep it in the Keychain.

## Checks

Run them one at a time:

```bash
npm run typecheck
npm test
cd src-tauri && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

Agents never sign in to e-Boekhouden or enter passwords. Tests use mocked pages and answers; the real sign-in is tested by hand.

## Releases

A tag `v1.2.3` builds macOS (universal) and Windows and publishes a GitHub release with `appcast.xml` and `latest.json`. The repository needs:

- variables `SPARKLE_PUBLIC_KEY` and `TAURI_UPDATER_PUBKEY`;
- secrets `SPARKLE_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

Keep the private keys in 1Password and never rotate them: installed apps trust only these keys.
