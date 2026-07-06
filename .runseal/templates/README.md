# santi ops templates

Seeds for the gitignored `.local/` that santi's remote-ops wrappers read. santi owns its
**high-frequency ops** (reach its host, run the client against the live edge, deploy new betas); the
infra repo owns the **cold, one-time buildout** (cluster, authentik, window nginx). The box + edge
are consumed as running services, so nothing here depends on the infra repo.

## Wrappers

- `runseal :ssh <host> [-- <cmd>...]` — reach a santi-operated host.
- `runseal :santi <args...>` — the santi HTTP client against the live edge.
- `runseal :deploy [<version>]` — fetch the latest (or given) beta `.deb` on the box and run
  `santi upgrade`, then verify schema + soul-memory continuity + health. (Cut the release first:
  `runseal :release` / the release-beta workflow.)

## Provision `.local/` (once per checkout: local + claude.host)

```
mkdir -p .local/ssh .local/secrets

# 1. SSH: copy the seed, fill the real HostName(s); install the private key.
cp .runseal/templates/ssh/config .local/ssh/config     # then edit HostName
cp <private-ops-key> .local/ssh/id_santi_ops && chmod 600 .local/ssh/id_santi_ops

# 2. Client: copy the seed, fill auth_client_id + auth_password.
cp .runseal/templates/secrets/santi.toml .local/secrets/santi.toml   # then edit
```

`.local/` is gitignored — real IPs, keys, and credentials never leave it.
