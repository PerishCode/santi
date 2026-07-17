# Santi service operations

This directory is the canonical source for Santi-specific cold operations. The infra repo owns the
generic host, DNS, k3s installation, and shared Authentik deployment; everything that names or
configures the Santi service lives here.

All commands run from the Santi repo root and use its gitignored `.local/ssh/config`.

## Cold host wiring

The Debian package creates the service account and runtime home. These idempotent scripts add the
estate-specific privileges and connect that account to its local k3s work surface:

```sh
for script in 00-host-preflight 10-santi-user 20-kube-access; do
  runseal :scp ".runseal/ops/host/${script}.sh" "hk-03.zxiyun:/tmp/${script}.sh"
  runseal :ssh hk-03.zxiyun -- bash "/tmp/${script}.sh"
done
```

## Authentik registration

The registration is idempotent. It creates or reuses the forward-domain provider/application, binds
it to the embedded outpost, and creates the CLI service account. Its printed `SANTI_AUTH_*` values
belong in this repo's local client config, never in git:

```sh
runseal :scp .runseal/ops/edge/register-authentik.sh \
  hk-03.zxiyun:/tmp/register-santi-authentik.sh
runseal :ssh hk-03.zxiyun -- sh /tmp/register-santi-authentik.sh
```

Pass `--rotate` to delete and recreate the app-password token. Capture the output directly into a
mode-600 local file, update `.local/secrets/santi.toml`, and delete the capture after the client
health check; do not print credentials into terminal or task logs.

## Edge apply

The namespace, ingress routes, window proxy, and current compatibility panel are applied from this
repo. ConfigMaps are regenerated idempotently and the Deployment is restarted explicitly because a
ConfigMap update does not roll pods by itself.

```sh
for file in ingress.yaml window.yaml; do
  runseal :scp ".runseal/ops/edge/${file}" "hk-03.zxiyun:/tmp/${file}"
done
for file in index.html nginx.conf; do
  runseal :scp ".runseal/ops/edge/resources/${file}" "hk-03.zxiyun:/tmp/${file}"
done

runseal :ssh hk-03.zxiyun -- 'k3s kubectl apply -f /tmp/ingress.yaml
k3s kubectl create configmap window-html -n santi --from-file=index.html=/tmp/index.html \
  --dry-run=client -o yaml | k3s kubectl apply -f -
k3s kubectl create configmap window-nginx -n santi --from-file=default.conf=/tmp/nginx.conf \
  --dry-run=client -o yaml | k3s kubectl apply -f -
k3s kubectl apply -f /tmp/window.yaml
k3s kubectl rollout restart deployment/window -n santi'
```

Runtime release and upgrade remain `runseal :release` and `runseal :deploy` respectively.
