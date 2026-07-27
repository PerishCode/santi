# santi

`santi` is a standalone agent runtime.

It keeps the architecture deliberately small:

```text
crates/
  api/             # the `santi-api` server binary: config, bootstrap, and local ops
  santi-core/      # soul runtime: sessions, turns, context assembly, store, objects, workspace
  santi-provider/  # provider-agnostic ProviderClient boundary (OpenAI Responses, chat-completions)
  santi-api/       # HTTP/SSE + OpenAPI server library over santi-core
  santi/           # the `santi` transport-only HTTP client binary
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
- `api` — the `santi-api` server executable. Owns config, bootstrap, serving,
  OpenAPI export, and local runtime operations.
- `santi` — the transport-only HTTP client. It reaches the runtime only over
  HTTP and does not link the runtime crates.

## Running locally

```sh
cp santi.example.toml santi.toml   # fill in a provider api_key + model
cp .env.example .env               # SANTI_PATHS_DATABASE / SANTI_LISTEN_HOST / SANTI_LISTEN_PORT

cargo run -p api -- serve
```

With no `.env`/config at all, santi-api runs zero-config from its home directory
(`SANTI_HOME`, default `~/.santi`): it reads `~/.santi/santi.toml` and creates
`~/.santi/{runtime,execution}` automatically.

Then, against a running server:

```sh
cargo run -p santi -- health
cargo run -p santi -- strand create
cargo run -p santi -- strand send <strand_id> "hello"
cargo run -p santi -- strand events <strand_id>
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

After the cause of a `turn_failed` receipt is cleared, an explicit
`santi strand drive <strand_id>` starts a recovery turn even when no new inbox
message exists. A context compact that resolves its incident does the same.
Ordinary boot/completion pokes do not retry failed receipts, and recovery reuses
durable confirmed effect results rather than replaying them automatically.

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
cargo run -p api -- export-openapi
```

Local runtime operations stay on the server entry:

```sh
santi-api doctor
SANTI_STRAND_ID=<strand_id> santi-api inbox seed "come look"
```

## Cross-host downstreams

A downstream owns one non-overlapping label zone such as `stim:`. Create a
high-entropy token, register only its SHA-256 digest through the trusted
management path, and retain the token in the downstream:

```sh
TOKEN=$(openssl rand -hex 32)
DIGEST=$(printf %s "$TOKEN" | sha256sum | cut -d ' ' -f1)
curl -X POST http://127.0.0.1:43307/api/v1/downstreams \
  -H 'Content-Type: application/json' \
  -d "{\"id\":\"stim\",\"label_prefix\":\"stim:\",\"credential_sha256\":\"$DIGEST\"}"
```

The credential digest is stored but never returned by the management API.
Registrations are idempotent when all three input fields match. Prefix overlap,
credential reuse, or reuse of an id with different values is rejected. Upgrading
a v31 database intentionally clears the old environment-variable registrations;
register digest credentials before starting a remote consumer.

The downstream submits every request with a stable idempotency key. Repeating the
same key and payload returns the original receipt; changing the payload produces
`409 Conflict`:

```sh
curl -X POST https://santi.liberte.top/api/v1/ingest \
  -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"soul_id":"soul_default","label":"stim:alice","text":"hello","request_id":"message-42"}'
```

Completed turns are pulled with the same credential. The response is
`{"cursor":...,"events":[...]}` and includes only the registered zone. Persist
the returned cursor even when `events` is empty. The cursor is a global opaque
high-water mark, so it reveals aggregate activity volume but no other zone's
labels or payloads. The SSE endpoint is only a lossy wake-up signal; always use
cursor backfill as the authority:

```sh
curl -H "Authorization: Bearer $TOKEN" \
  'https://santi.liberte.top/api/v1/turn-events?since=0'
curl -N -H "Authorization: Bearer $TOKEN" \
  https://santi.liberte.top/api/v1/turn-events/stream
```

## Configuration

`santi.toml` (gitignored) holds real provider credentials. Start from
`santi.example.toml`.

Everything anchors on the santi home — `SANTI_HOME`, default `~/.santi` — so the
runtime works with zero explicit configuration. Each path can be overridden by
its own variable (configuration precedence is `--flag` > environment > config
file > defaults):

| Variable | Default | Purpose |
| --- | --- | --- |
| `SANTI_HOME` | `~/.santi` | Anchor for the defaults below |
| `SANTI_CONFIG` | `$SANTI_HOME/santi.toml` | Provider config file (`--config` overrides) |
| `SANTI_PATHS_DATABASE` | `$SANTI_HOME/runtime/db` | SQLite store |
| `SANTI_PATHS_RUNTIME_ROOT` | `$SANTI_HOME/runtime` | Soul/session memory, objects |
| `SANTI_PATHS_EXECUTION_ROOT` | `$SANTI_HOME/execution` | Shell tool working area |
| `SANTI_PROVIDER` | `openai` | Selected provider profile |
| `SANTI_LISTEN_HOST` / `SANTI_LISTEN_PORT` | `127.0.0.1` / `43307` | Bind address |
| `SANTI_API_KEY` | unset | Transitional static bearer sent by the CLI (`--api-key` overrides). The runtime has no global API-key gate; edge Authentik protects management paths, while downstream data paths use registered zone credentials. |
| `SANTI_API_URL` | `http://127.0.0.1:43307` | Client target (`--base-url` overrides) |

A `.env` in the working directory is loaded and overrides the process
environment (via `dotenvy::dotenv_override`).

## Distribution

The current release gate supports Linux x86_64 only. R2 publishes both a
standalone tarball and the Debian package used by the streamed host deployment
path:

```sh
curl -fsSL https://releases.santi.perish.uk/manage.sh | sh -s -- install --channel beta
```

Forgejo `PerishFire/santi` is the canonical source and automation target. The
public GitHub repository is retained as historical context without reverse
synchronization.

## License

MIT. See [LICENSE](LICENSE).
