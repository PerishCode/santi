# Santi Debian package

The Linux release job builds one package containing the `santi` HTTP client, the `santi-api` runtime
entry, and `santi.service`. Host deployment belongs to the streamed operator program at
`.runseal/ops/host/40-santi-deploy.sh`, not to either executable.

## Package files

| Source                                  | Installs to            | Purpose                                      |
| --------------------------------------- | ---------------------- | -------------------------------------------- |
| `root/lib/systemd/system/santi.service` | `/lib/systemd/system/` | Runtime service using `santi-api serve`      |
| `root/etc/santi/santi.env.example`      | `/etc/santi/`          | Seed for operator-managed environment values |
| `control`                               | `DEBIAN/`              | Package identity and version                 |
| `postinst`, `prerm`, `postrm`           | `DEBIAN/`              | User, directory, and systemd lifecycle       |

Both binaries install under `/usr/bin`. Maintainer scripts never delete `/home/santi/.santi`.

## Deployment boundary

`runseal :deploy` streams a root-side transaction over the repository-local SSH boundary. It
verifies the durable source package, downloads and verifies the selected beta, gracefully stops
`santi.service`, snapshots `runtime/`, installs the candidate, retains its package, and starts the
new `santi-api` service. Doctor, HTTP readiness, and soul-memory continuity must all pass.

After the destructive boundary, any failed check restores the source runtime and package and keeps
the candidate runtime for diagnosis. A successful candidate is not complete until
`30-santi-recovery.sh` validates and arms the post-deploy recovery capsule.

The service has `TimeoutStopSec=50`, greater than the default `SANTI_SERVER_GRACE=30`, so systemd
allows an in-flight turn to drain.
