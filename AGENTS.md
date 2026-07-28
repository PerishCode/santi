# Agent guide

`santi` is a standalone agent runtime. Treat this repo as runtime-first: there
is no product layer here, and none should be added speculatively.

## Layout

```text
crates/
  api/             # the `santi-api` binary: config, bootstrap, serve, local ops
  santi-core/      # runtime model + service (turns, assembly, objects, workspace)
  santi-estate/    # Keel graph + durable ceremonies and projections
  santi-provider/  # ProviderClient boundary; keeps santi-core provider-agnostic
  santi-api/       # HTTP/SSE/OpenAPI server library over santi-core
  santi/           # the `santi` transport-only HTTP client binary
.runseal/ops/       # retained operator-owned host/edge artifacts
```

## Boundaries

- `santi-core` is provider-agnostic. Provider specifics live behind
  `santi-provider::ProviderClient`.
- `santi-api` is the only network boundary. Browser/host-facing shapes are
  owned here, not in `santi-core`.
- `api` ships `santi-api`, the server entry. It owns config resolution,
  bootstrap, serving, OpenAPI export, and local runtime operations.
- `santi` is a transport-only HTTP client. It must never depend on or call
  `santi-api` or `santi-core` in process. HTTP stays the only way in.

## Build & checks

```sh
cargo fmt --all
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
```

CI (`.forgejo/workflows/guard.yml`) runs Ectropy syntax laws, Plumb repository
shape, and the Rust fmt/clippy/test triad on the self-hosted Linux runner.
Release workflows publish both executables in Linux x86_64 tar and Debian
artifacts to R2.

## Trigger a single turn locally (hot path)

To exercise a real end-to-end turn, prefer reusing the repo-root
`santi.toml` — it already configures a working provider, so no ad-hoc
config or env wiring is needed. `santi-api serve` reads `./santi.toml`
by default; drive one turn and stop when it lands:

```sh
santi-api serve &                                       # reads ./santi.toml
SID=$(santi strand create | jq -r .strand.id)
SANTI_STRAND_ID=$SID santi strand send 'Reply with exactly: OK' --watch
```

`--watch` follows the SSE stream and exits when the strand goes idle (after the
turn completes), so it doubles as the wait — no sleep/poll dance. It stays
robust when sends coalesce: a completed turn that spawns a follow-on is still
awaited to full idle. By default, watch output is filtered human-readable
milestones for interactive use.

For raw/debug automation, pass `--watch-format raw`; it relays event JSON (one
object per line, same payload shape as `strand events`). Distill the reply with
jq:

```sh
… strand send '…' --watch --watch-format raw \
  | jq -rc 'select(.payload.type=="message" and .payload.beat=="completed")
            | .payload.message.content_text'
```

`--strand`/`SANTI_STRAND_ID` set a default strand id; `--soul`/`SANTI_SOUL_ID`
pick a non-default soul (empty → the runtime's default soul; an unknown soul is
rejected, not silently created). To address a soul ad hoc without a default:
`santi --soul <id> strand send <strand_id> '…'`.

## Conventions

- Edition 2024, MIT. Workspace dependencies are pinned in the root
  `Cargo.toml`; crates reference them with `.workspace = true`.
- Forgejo (`PerishFire/santi`) is the canonical write target. The public GitHub
  repository is historical and is not reverse-synchronized.
- Santi's runtime boundary stops at its executables. Existing packaging,
  recovery, and system-service artifacts are operator-owned and are not runtime
  architecture. Infra owns only generic host, k3s, DNS, and shared middleware.
- Runtime secrets live in `santi.toml`; local release credentials live in
  `.forgejo/release.env`. Both are gitignored. Never commit live credentials;
  `santi.example.toml` and `.forgejo/release.env.example` are tracked templates.
