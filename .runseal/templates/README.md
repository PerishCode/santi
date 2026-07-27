# santi ops templates

Seeds for the gitignored `.local/` that santi's remote-ops wrappers read. santi owns its complete
service lifecycle: reach its host, run the client, deploy new betas, cold service-user/kube wiring,
and the Santi-specific edge/authentik integration. The infra repo supplies only the generic host,
k3s, DNS, and shared middleware substrate. Nothing here depends on an infra checkout.

## Wrappers

- `runseal :ssh <host> [-- <cmd>...]` — reach a santi-operated host.
- `runseal :scp <source> <destination>` — copy through the same declared-host SSH boundary.
- `runseal :santi <args...>` — the santi HTTP client against the live edge.
- `runseal :deploy [<version>]` — fetch the latest (or given) beta `.deb` on the box and run the
  streamed host transaction, then verify schema + soul-memory continuity + health and arm a recovery
  capsule. (Cut the release first: `runseal :release` / the release-beta workflow.)
- `.runseal/ops/` — cold, idempotent host access wiring and edge manifests/recipes. These are
  intentionally service-owned even though they operate platform APIs.

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
