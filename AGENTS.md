# Agent guide

`santi` is a standalone agent runtime. Treat this repo as runtime-first: there
is no product layer here, and none should be added speculatively.

## Layout

```text
crates/
  santi-core/      # runtime model + service (store, turns, assembly, objects, workspace)
  santi-provider/  # ProviderClient boundary; keeps santi-core provider-agnostic
  santi-api/       # HTTP/SSE/OpenAPI server library over santi-core
  santi/           # the `santi` binary: `service` runs the server; else HTTP client
```

## Boundaries

- `santi-core` is provider-agnostic. Provider specifics live behind
  `santi-provider::ProviderClient`.
- `santi-api` is the only network boundary. Browser/host-facing shapes are
  owned here, not in `santi-core`.
- `santi` ships one binary with two faces: `santi service ...` runs the server
  in-process (links `santi-core` via `santi-api`); every other command is a
  transport-only HTTP client. The client path must reach the runtime only over
  HTTP — never call `santi-core` in-process. HTTP stays the only way in.

## Build & checks

```sh
cargo fmt --all
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
```

CI (`.github/workflows/guard.yml`) runs `flavor check` plus the Rust
fmt/clippy/test triad on PRs and a 3-OS matrix on `main`.

## Trigger a single turn locally (hot path)

To exercise a real end-to-end turn, prefer reusing the repo-root
`santi.toml` — it already configures a working provider, so no ad-hoc
config or env wiring is needed. `santi service serve` reads `./santi.toml`
by default; drive one turn and stop when it lands:

```sh
santi service serve &                                   # reads ./santi.toml
SID=$(santi session create | jq -r .session.session.id)
SANTI_SESSION=$SID santi session send 'Reply with exactly: OK' --watch
```

`--watch` follows the SSE stream and exits when the soul_session goes idle
(after the turn completes), so it doubles as the wait — no sleep/poll dance.
It stays robust when sends coalesce: a completed turn that spawns a follow-on
is still awaited to full idle.

`--watch` relays raw SSE frames (one JSON object per line), same shape as
`session events`. Distill the reply with jq:

```sh
… send '…' --watch | jq -rc 'select(.payload.type=="message_completed")
                             | .payload.message.content_text'
```

`--session`/`SANTI_SESSION` set a default session id; `--soul`/`SANTI_SOUL`
pick a non-default soul (empty → the runtime's default soul; an unknown soul
is rejected, not silently created). To address a soul ad hoc without a
default: `santi --soul <id> session send <sid> '…'`.

## Conventions

- Edition 2024, MIT. Workspace dependencies are pinned in the root
  `Cargo.toml`; crates reference them with `.workspace = true`.
- Secrets live in `santi.toml` (gitignored). Never commit live credentials;
  `santi.example.toml` is the tracked template.
