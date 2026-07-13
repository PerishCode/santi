# santi

`santi` is a standalone agent runtime.

It keeps the architecture deliberately small:

```text
crates/
  santi-core/      # soul runtime: sessions, turns, context assembly, store, objects, workspace
  santi-provider/  # provider-agnostic ProviderClient boundary (OpenAI Responses, chat-completions)
  santi-api/       # HTTP/SSE + OpenAPI server library over santi-core
  santi/           # the `santi` binary: `service` runs the server; other commands are an HTTP client
```

The runtime owns soul identity, per-session runtime state, turn execution with
streaming events (thinking / text / tool calls / tool results), context
assembly into provider input, a local object protocol (`santi://`), and
workspace/memory. The only way into the runtime is HTTP.

## Crates

- `santi-core` — runtime model and service. SQLite-backed store, turn
  execution, context assembly, `santi://` object store, soul/session
  workspaces and memory.
- `santi-provider` — the `ProviderClient` trait and its OpenAI Responses /
  chat-completions implementations. `santi-core` stays provider-agnostic
  behind this boundary.
- `santi-api` — Axum HTTP server, SSE streaming, and OpenAPI export as a
  library. Owns the HTTP boundary and links `santi-core`.
- `santi` — the single binary. `santi service ...` runs the server in-process
  (via `santi-api`); every other command is a transport-only HTTP client that
  reaches the runtime only over HTTP.

## Running locally

```sh
cp santi.example.toml santi.toml   # fill in a provider api_key + model
cp .env.example .env               # SANTI_DB / SANTI_HOST / SANTI_PORT

cargo run -p santi -- service serve
```

With no `.env`/config at all, santi runs zero-config from its home directory
(`SANTI_HOME`, default `~/.santi`): it reads `~/.santi/santi.toml` and creates
`~/.santi/{runtime,execution}` automatically.

Then, against a running server:

```sh
cargo run -p santi -- health
cargo run -p santi -- session create
cargo run -p santi -- session send <session_id> "hello"
cargo run -p santi -- session events <session_id>
```

Every accepted send returns a durable `receipt.inbox_id`. Query its obligation
state and state-transition evidence without replaying the message timeline:

```sh
santi receipt <inbox_id>
```

Receipt completion means an assistant turn completed and was persisted. Driver
recovery or incident resolution alone never marks the receipt completed.
Migration-reconstructed transitions expose `reconstructed_from`; live
transitions leave it unset. A v24 drain is completed only when its linked turn
is durably completed, never merely because the inbox item was drained.

Shell commands also create a durable effect attempt. Receipt completion still
only proves the assistant turn was persisted; inspect the linked effect before
claiming that an external action occurred:

```sh
santi effect query <effect_id>
santi effect resolve <effect_id> \
  --outcome applied \
  --evidence "operator found the target marker"
```

`prepared` means dispatch has not begun. An interrupted `dispatching` attempt
becomes `unknown`, because the runtime cannot prove whether the command took
effect, and is never replayed automatically. A mechanically rejected spawn is
`not_dispatched`; a durably captured command result is `confirmed`. Only an
`unknown` attempt accepts an explicit `applied` or `not-applied` operator
resolution, and resolution records evidence without retrying the command or
changing its turn/receipt state.

Export the OpenAPI document:

```sh
cargo run -p santi -- service export-openapi
```

## Configuration

`santi.toml` (gitignored) holds real provider credentials. Start from
`santi.example.toml`.

Everything anchors on the santi home — `SANTI_HOME`, default `~/.santi` — so the
runtime works with zero explicit configuration. Each path can be overridden by
its own variable (and the provider config follows `--flag` > config file > env):

| Variable | Default | Purpose |
| --- | --- | --- |
| `SANTI_HOME` | `~/.santi` | Anchor for the defaults below |
| `SANTI_CONFIG` | `$SANTI_HOME/santi.toml` | Provider config file (`--config` overrides) |
| `SANTI_DB` | `$SANTI_HOME/runtime/db` | SQLite store |
| `SANTI_RUNTIME_ROOT` | `$SANTI_HOME/runtime` | Soul/session memory, objects |
| `SANTI_EXECUTION_ROOT` | `$SANTI_HOME/execution` | Shell tool working area |
| `SANTI_PROVIDER` | `openai` | Selected provider profile |
| `SANTI_HOST` / `SANTI_PORT` | `127.0.0.1` / `43307` | Bind address |
| `SANTI_API_KEY` | unset (open) | Optional bearer key. Server: when set, every endpoint except `/health` requires `Authorization: Bearer <key>`. Client: sent on requests (`--api-key` overrides). |
| `SANTI_API_URL` | `http://127.0.0.1:43307` | Client target (`--base-url` overrides) |

A `.env` in the working directory is loaded and overrides the process
environment (via `dotenvy::dotenv_override`).

## Distribution

The current release gate supports Linux x86_64 only. R2 publishes both a
standalone tarball and the Debian package used by the self-upgrade path:

```sh
curl -fsSL https://releases.santi.perish.uk/manage.sh | sh -s -- install --channel beta
```

Forgejo `PerishFire/santi` is the canonical source and automation target. The
public GitHub repository is retained as historical context without reverse
synchronization.

## License

MIT. See [LICENSE](LICENSE).
