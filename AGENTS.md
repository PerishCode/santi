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

## Conventions

- Edition 2024, MIT. Workspace dependencies are pinned in the root
  `Cargo.toml`; crates reference them with `.workspace = true`.
- Secrets live in `santi.toml` (gitignored). Never commit live credentials;
  `santi.example.toml` is the tracked template.
