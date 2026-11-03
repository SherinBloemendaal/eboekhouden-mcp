# Roadmap

Milestones follow `SPEC.md` in `~/projects/eb-mcp-design`.

## Done (2026-11-03)

- **M0 skeleton:** tray panel, main window, local MCP server with token, Host and Origin checks.
- **M1 accounts and sign-in:** account list, 1Password and system stores (Touch ID, Windows Hello), hidden sign-in with e-Boekhouden's own page, two-step code, alert and confirm capture, unknown-step stop, sessions per account, idle sign-out, sign-out on quit.
- **M2 tools and request log:** twelve read-only tools with query parameters from e-Boekhouden's Angular app, the `get` allow-list, rate limit, request log with linked calls and masked secrets, Verzoeken screen.
- **M3 Dashboard, Koppelen, updates:** Dashboard per run, Koppelen for Claude (bridge) and Cursor with preview, backup and read-back, token renewal, Sparkle and the Tauri updater, release and CI workflows.
- **M4 Windows (code):** Windows paths, Credential Manager with Windows Hello, tray above the taskbar, Mica and Acrylic, a Fluent skin (`src/styles/windows.css`).

## Waiting on Sherin

1. **Real sign-in.** Agents never sign in. Try an account with 1Password and one with the Keychain, with and without two-step verification. Things to look at: whether `auth-token` appears for `secure20.e-boekhouden.nl`, whether the API accepts the cookies plus `Authorization: Bearer`, and what `administratie/uwgegevens` returns (the administration name).
2. **Tool parameters against the real API.** Taken from the public Angular bundle; unsure are the date format of `mutaties` (`datumVan`), the btw `periodType` letters (M, K, J) and `periodType=F` for winst en verlies.
3. **GitHub repository and update keys.** Create the repository, generate the Sparkle EdDSA key and the Tauri updater key, put the private halves in 1Password and in the repository secrets, the public halves in the variables (README, Releases).
4. **Windows.** The Windows code compiles and passes clippy with `-D warnings` for `x86_64-pc-windows-gnu` (MinGW on this Mac); the MSVC build and running it on Windows happen in CI and on a Windows machine. The Fluent boards are drawn (EB MCP canvas, page Windows) and wait for approval.
5. **Signing.** Apple Developer ID and notarization, and a Windows code-signing certificate, once they exist.
6. **Open design questions** in `BOARDS.md` (accent colour, appearance control, Claude Code, idle default, rate limit, tool names, Linux).
