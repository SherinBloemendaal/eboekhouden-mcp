# EB MCP

A menu-bar app (macOS, and Windows from the same code) that signs in to e-Boekhouden and offers its internal API to AI clients (Claude, Cursor) as a local MCP server: read-only, unless you turn writing on for an account.

- Product spec and design boards: `~/projects/eb-mcp-design` (`SPEC.md`, `BOARDS.md`, the EB MCP canvas).
- Roadmap and what is left: [docs/ROADMAP.md](docs/ROADMAP.md).

## How it works

- **Accounts** (`src-tauri/src/accounts.rs`): label, username or `op://` item, administration and which store holds the password. Never a password in the app's files. There is no default account: with more than one, every tool call must name its account, so an AI client never reads the wrong books.
- **Credentials** (`creds.rs`): 1Password through the `op` CLI, or the system store (Keychain with Touch ID, Credential Manager with Windows Hello), read only at sign-in.
- **Sign-in as the browser does it** (`signin.rs`, `ebapi.rs`): EB MCP fetches e-Boekhouden's login page (`bh/inloggen.asp`) and posts its form with the username and password from 1Password or the Keychain. The answer sets the `auth-token` cookie that e-Boekhouden's API reads; EB MCP then checks the session with `POST auth`, as their app does at startup. Token and cookies stay in memory only. Two-step verification on this form is not built yet: the sign-in stops and its diagnosis names the page's fields. (0.1.9 to 0.1.11 used the `auth/login` of their new app, which answers 500 for ordinary accounts.)
- **API** (`ebapi.rs`): calls under `/v1/api/` with the session's cookies, 60 a minute and 2 at a time, and a growing pause after errors. Reads are GETs; a change is a POST, PUT or DELETE, never retried (a quote with lines, or a template with its layout, takes several).
- **MCP** (`mcp.rs`, `tools.rs`): Streamable HTTP on `http://127.0.0.1:47821/mcp`, behind a bearer token, loopback Host only and no browser `Origin`. Twenty-seven tools that read (among them a search in the KvK register through e-Boekhouden) and twenty-seven that change something. The generic `get` reads only the 334 plain paths in `data/get-paths.txt` (`scripts/gen_get_paths.py`).
- **Writing** (`write.rs`, `approvals.rs`, `hub.rs`): off for every account until the user turns it on (Accounts, per account): with approval, or without. With approval a tool's change is shown in EB MCP first and goes to e-Boekhouden only after Goedkeuren; deleting, crediting, invoicing subscriptions, mailing an invoice and changing where an e-mail template sends its mail always ask. The tools book and edit mutaties of all seven kinds, upload files to the digitaal archief and link them to a mutatie, make, edit, credit, delete and e-mail invoices (and fetch one as a PDF into Downloads), make, edit and delete subscriptions and e-mail templates, and delete a mutatie or an archive file. Deleting a mutatie always asks and says that a correcting entry is the better bookkeeping; deleting a file follows the account's setting, and always asks, with a warning, when it is still linked. Relaties can be added, edited and deleted; a change of IBAN, BIC or an e-mail address always asks. Quotes can be made, edited, deleted and fetched as a PDF. Templates: e-mail and dunning templates as text, and the layout of invoice and quote templates through the API of e-Boekhouden's own template editor (`editor.e-boekhouden.nl`): a tool reads the layout document, and a changed one is saved after the user has seen what changes per element. Templates that are an uploaded RTF are left alone. The request shapes were taken from e-Boekhouden's own web app and checked against a test administration (2026-11-11); mailing an invoice is the one that was built from the web form's code and first sent after the release. "Facturen maken" for subscriptions runs on e-Boekhouden's SignalR hub over a WebSocket, as their page does.
- **Request log and Dashboard** (`reqlog.rs`, `logstore.rs`): the last 1000 calls, MCP calls linked to the e-Boekhouden calls they caused, secrets masked, each with the app version that made it. Log and statistics are kept in `request-log.json.gz` in the app's config folder (user-only, written atomically), so they survive restarts and updates until Leegmaken. Verzoeken can export everything to one `eb-mcp-debug-*.jsonl.gz` in Downloads for debugging.
- **Koppelen** (`connect.rs`, `bridge/`): writes the entry into Claude Desktop's or Cursor's config after a confirm, with a backup and a read-back. Claude Desktop starts only stdio servers, so it gets `eb-mcp-bridge`, a small sidecar that forwards to the HTTP server.
- **Updates** (`updates.rs`, ported from Tresso): Sparkle on macOS, Tauri's updater on Windows, signed releases from `.github/workflows/release.yml`.

## Develop

```bash
npm install
npm run tauri dev
```

`tauri dev` first fetches Sparkle (pinned, SHA-256 checked) and builds the bridge. The MCP token lives in a file in the app's config folder that only the user can read: `token.json` in release builds, `dev-token.json` in debug builds. It is not in the Keychain, because the ad-hoc signed app would otherwise ask for the Keychain password at every start after each update, and the bridge reads the same file without a prompt. A release that finds a token from an older version in the Keychain moves it to the file once.

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
