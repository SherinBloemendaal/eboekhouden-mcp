# Roadmap

Milestones follow `SPEC.md` in `~/projects/eb-mcp-design`. Each one ends with the checks green and Sherin trying it.

## M0: skeleton (done, 2026-11-03)

- Tauri 2 app with the menu-bar icon (template image), a borderless panel under it and the main window with its sidebar: Dashboard, Accounts, Verzoeken, Koppelen. Native material behind both (vibrancy on macOS, Mica or Acrylic on Windows).
- Rust MCP server on `127.0.0.1:47821/mcp`: token per installation in the credential store, bearer check, loopback Host only, any `Origin` refused. One read-only tool, `status`.
- Koppelen shows the address and the masked token. Pages without data show their empty state.

## M1: accounts and sign-in out of sight

- Account list on disk (label, username or `op://` reference, administration, store), never a password.
- Credential stores: Keychain with Touch ID, Windows Credential Manager with Windows Hello, 1Password CLI.
- Hidden webview that fills e-Boekhouden's own login page, the progress sheet, our own two-step code field, the unknown-step stop, `auth-token` capture, `refreshjwt` renewal, sign-out on quit and idle.
- Tested against mocked login pages only.

## M2: tools and the request log

- Read-only tools from the API map: relaties, mutaties, open posten, grootboek, balans, winst en verlies, btw, uren, digitaal archief and a `get` limited to GET paths on the map; an `account` parameter.
- Request log in memory (last 1000), MCP calls linked to the e-Boekhouden calls they caused, secrets masked. Rate limit.

## M3: Dashboard, Koppelen, updates

- Dashboard figures per run.
- Koppelen with the confirm sheet for Claude (through `eb-mcp-bridge`) and Cursor; token renewal.
- Sparkle on macOS and the Tauri updater on Windows, signed releases, the update states from the boards.

## M4: Windows

- Fluent boards first, then the Windows skin, tray position, credential store and installer signing.

## Open questions

See `BOARDS.md` in the design folder (accent colour, appearance control, Claude Desktop route, token in client files, idle sign-out, rate limit, tool names, restart for updates, Windows and Linux timing).
