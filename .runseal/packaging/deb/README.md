# santi `.deb` packaging (PHASE-07 STEP 5)

Static packaging artifacts for the minimal server `.deb`. The `.deb` is the low-friction entry that
lets Liberte take over upgrades (`santi upgrade`); it replaces the hand-rolled `manage.sh`
version-slots on the server. The release pipeline currently supports Linux x86_64 only.

## Files here

| File                            | Installs to            | Purpose                                                                               |
| ------------------------------- | ---------------------- | ------------------------------------------------------------------------------------- |
| `santi.service`                 | `/lib/systemd/system/` | the runtime unit (User=santi, drain-aware `TimeoutStopSec`)                           |
| `santi-upgrade.service`         | `/lib/systemd/system/` | the detached oneshot unit `santi upgrade` kicks (runs OUTSIDE santi.service's cgroup) |
| `santi.env.example`             | `/etc/santi/`          | example EnvironmentFile; the real `santi.env` is operator-managed and never clobbered |
| `control`                       | `DEBIAN/control`       | package metadata; `__VERSION__` substituted at build                                  |
| `postinst` / `prerm` / `postrm` | `DEBIAN/`              | user + dirs + systemd enable; stop-on-remove; never touch runtime data                |

The binary is installed to `/usr/bin/santi`.

## Design notes

- **Enable, don't auto-start.** `postinst` installs + enables but does NOT start: starting needs
  operator secrets in `/etc/santi/santi.env`, and during a self-upgrade the `santi upgrade`
  orchestrator owns the stop/start around dpkg.
- **`TimeoutStopSec` (620) > `SANTI_SHUTDOWN_GRACE_SECS` (600)** so systemd never SIGKILLs santi
  mid-drain (STEP 3). `santi-upgrade.service` `TimeoutStartSec` (900) likewise exceeds
  `SANTI_UPGRADE_TIMEOUT_SECS` (600).
- **Runtime data is sacred.** No maintainer script ever deletes `/home/santi/.santi` — the soul's
  memory is the one thing that must survive.
- **Rollback artifacts are runtime-owned.** Upgrade launch copies packages into a content-addressed
  store, checks Debian package/version identity plus SHA-256, and atomically records the package
  currently installed. `--previous-deb` is required once to bootstrap that manifest; later upgrades
  resolve the durable artifact and reject a conflicting caller file. A successful trial promotes its
  retained candidate before finalization, while rollback restores both runtime data and the previous
  installed-package manifest.
- **Upgrade unit runs as santi + sudo**, so every file it writes stays santi-owned; the privileged
  dpkg/systemctl calls use santi's passwordless sudo (a SystemHost detail tuned on-box in STEP 6).

## Build assembly

Building and publishing the `.deb` lives in the Forgejo release pipeline
(`.runseal/wrappers/release-ci.ts` + `.forgejo/workflows/release-beta.yml`) and shares the
outward-facing R2 path with the Linux tarball:

1. In the `x86_64-unknown-linux-gnu` build, after producing the binary, stage a tree:
   `usr/bin/santi`, `lib/systemd/system/{santi,santi-upgrade}.service`,
   `etc/santi/santi.env.example`, `DEBIAN/{control,postinst,prerm,postrm}` (control's `__VERSION__`
   ← the release version; scripts `chmod 0755`).
2. `dpkg-deb --build` produces `santi-x86_64-unknown-linux-gnu.deb`.
3. `publish` puts it on R2 next to the tarballs and records it in `metadata.json`.
4. Server upgrade path becomes `curl <r2>/…/santi_<v>_amd64.deb -o … && santi upgrade …`.

The Forgejo release job validates the package member, publishes it, then performs a public install
and live service-health smoke on Linux.
