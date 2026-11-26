# Development, Testing, Self-Bootstrapping, & Release Guide

Follow this guide when modifying Meta-Harness itself (including when using Meta-Harness group chats inside the `meta-harness` repo!).

---

## 1. Environment Prerequisites (Windows, macOS, Linux)

Verify tool versions before building:

```sh
node -v    # Node.js 24 recommended
npm -v
git --version
cargo --version
```

- **Windows:** Requires either Visual Studio C++ Build Tools (`stable-x86_64-pc-windows-msvc`) or WinLibs MinGW-w64 (`stable-x86_64-pc-windows-gnu` with `gcc.exe` on `PATH` for `rusqlite`'s bundled SQLite build).
- **macOS:** `xcode-select --install` + `rustup`.
- **Linux:** `build-essential` (or equivalent C toolchain) + `rustup`.

Install frontend dependencies once:

```sh
npm --prefix web ci
```

---

## 2. Self-Bootstrapping Tips (Running Meta-Harness on Meta-Harness)

Because Meta-Harness uses Git worktrees (`.meta-harness/worktrees/<session_id>/_integration` and `_exec_<handle>`):

1. **Test against ephemeral ports & isolated dirs:** When running `cargo run` or launching a test binary while an existing `meta-harness` instance is serving your UI on `:4410`, ALWAYS pass distinct `--bind 127.0.0.1:<FREE_PORT>`, `--data-dir <TEMP_DIR>`, `--no-open`, `--no-tray` (or set `MH_DATA_DIR`, `MH_CONFIG_DIR`, `MH_WORKSPACE_DIR`) so you do not overwrite your active database or conflict on port `4410`.
2. **Rebuild `web/dist` before embedding:** `crates/mh-server/src/lib.rs` embeds `../../web/dist` via `RustEmbed` at compile time. Whenever files under `web/` change, run `npm --prefix web run build` BEFORE `cargo build` or `cargo test --locked --workspace`.
3. **Mirror DTO changes across Rust & TypeScript:** Keep serde field names in `crates/mh-core/src/domain.rs` (and `crates/mh-core/src/skills.rs`) aligned 1:1 with `web/src/types/api.ts` and `web/src/api/client.ts`.
4. **Normalize path & newline behavior across OSes:** Use `/` when storing relative paths in SQLite/JSON; normalize CRLF -> LF when hashing files so Windows `core.autocrlf` checkouts behave identically to macOS/Linux.

---

## 3. Deterministic Offline Testing

Meta-Harness supports deterministic integration testing without external AI subscriptions or network access:

- `crates/mh-acp/src/mock_agent.rs` speaks ACP stdio JSON-RPC 2.0 through `mock-acp-agent` and `mh-test-fixture mock-agent`. It supports prompt directives after stripping context XML blocks such as `<available_skills>` and `<group_chat_updates>`:
  - `[MOCK:REPLY text="..."]` — Return fixed final output.
  - `[MOCK:WRITE path="src/foo.rs" content="..."]` — Exercise `fs/write_text_file` & unified diff capture.
  - `[MOCK:SH cmd="..."]` — Exercise brokered `terminal/create` + `terminal/wait_for_exit`.
  - `[MOCK:MCP tool="skill_read" args={"skill":"meta-harness-docs"}]` (with optional `permission=required`) — Exercise `meta-harness mcp-bridge` tool calls & auto-approval checks.
  - `[MOCK:ORCH_AUTO]`, `[MOCK:CURATE]`, `[MOCK:SLEEP ms=... until="..."]`, `[MOCK:429]`, `[MOCK:CHILD_UPDATE]` — Cover quota-aware dispatch, memory curation, `<250 ms` cancellation, 429 failover, and child-session filtering.
- See `crates/mh-server/tests/e2e_switch.rs` and `crates/mh-acp/tests/acp_integration.rs` for full end-to-end fixtures.
- Server integration tests launch the mock agent and bridge from `mh-test-fixture`. Release builds package only the `meta-harness` executable and enforce the startup privilege check.

---

## 4. Mandatory Quality & Verification Gates

Run every check before considering work complete (zero warnings/failures allowed):

### Frontend (`web/`)
```sh
npm --prefix web run typecheck   # tsc --noEmit (strict mode, zero `any`)
npm --prefix web run test        # vitest run
npm --prefix web run build       # updates web/dist for RustEmbed
```

### Rust (`crates/*`)
```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
```

### Versioning, Changelog, & Release Packaging
- **Single SemVer bump per commit (when committing):**
  ```sh
  node scripts/version.mjs <patch|minor|major>
  node scripts/version.mjs --check
  ```
  *(Keeps `Cargo.toml`, `Cargo.lock`, `web/package.json`, `desktop/package.json`, and lockfiles synchronized. Omit `--locked` only intentionally when updating `Cargo.lock`.)*
- Record notable changes under `## Unreleased` in `CHANGELOG.md`. Move them to the versioned section for milestone releases, and update `docs/SPECIFICATION.md`, `docs/USAGE.md`, and this bundled skill when their workflows change.
- **Release packages:**
  - `node scripts/build.mjs` -> Electron desktop installer/archive (`dist/desktop`) + standalone binary (`dist/cli`).
  - `node scripts/build.mjs --cli-only` -> Standalone binary (`dist/cli`).
  - Pass `--unpacked` during local packaging verification.
  - Run the **Release build** workflow from the matching version tag with publishing enabled to attach verified Windows and macOS update assets to a GitHub release.
