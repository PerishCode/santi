# santi

`santi` is a standalone agent runtime.

It keeps the architecture deliberately small:

```text
crates/
  santi-core/      # soul runtime: sessions, turns, context assembly, store, objects, workspace
  santi-provider/  # provider-agnostic ProviderClient boundary (OpenAI Responses, chat-completions)
  santi-api/       # HTTP/SSE + OpenAPI boundary over santi-core (the canonical entry)
  santi-cli/       # thin HTTP wrapper over santi-api (no santi-core linkage)
```

The runtime owns soul identity, per-session runtime state, turn execution with
streaming events (thinking / text / tool calls / tool results), context
assembly into provider input, a local object protocol (`santi://`), and
workspace/memory. The only way in from outside is HTTP.

## Crates

- `santi-core` — runtime model and service. SQLite-backed store, turn
  execution, context assembly, `santi://` object store, soul/session
  workspaces and memory.
- `santi-provider` — the `ProviderClient` trait and its OpenAI Responses /
  chat-completions implementations. `santi-core` stays provider-agnostic
  behind this boundary.
- `santi-api` — Axum HTTP server, SSE streaming, and OpenAPI export. The
  canonical foreground entry: `cargo run -p santi-api`.
- `santi-cli` — a transport-only HTTP client for `santi-api`. Every command
  maps to one endpoint; it never links `santi-core`.

## Running locally

```sh
cp santi.example.toml santi.toml   # fill in a provider api_key + model
cp .env.example .env               # SANTI_DB / SANTI_HOST / SANTI_PORT

mkdir -p .tmp
cargo run -p santi-api -- serve
```

Then, against a running server:

```sh
cargo run -p santi-cli -- health
cargo run -p santi-cli -- session create
cargo run -p santi-cli -- session send <session_id> "hello"
cargo run -p santi-cli -- session events <session_id>
```

Export the OpenAPI document:

```sh
cargo run -p santi-api -- export-openapi
```

## Configuration

- `santi.toml` (gitignored) holds real provider credentials. Start from
  `santi.example.toml`.
- Runtime is configured by environment (`.env`): `SANTI_DB` is required;
  `SANTI_HOST` / `SANTI_PORT` default to `127.0.0.1:43307`.

## License

MIT. See [LICENSE](LICENSE).
