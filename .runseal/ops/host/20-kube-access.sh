#!/usr/bin/env bash
# Give the santi user kubeconfig access to the local k3s — its work surface.
set -euo pipefail
test "$(id -u)" = 0
id santi >/dev/null

src=/etc/rancher/k3s/k3s.yaml
test -f "$src"
install -d -m 0700 -o santi -g santi /home/santi/.kube
install -o santi -g santi -m 0600 "$src" /home/santi/.kube/config
# k3s.yaml targets https://127.0.0.1:6443, which is correct on this host.

# santi's shell tool runs `bash -lc`, so export KUBECONFIG from the login profile.
if ! grep -q 'KUBECONFIG' /home/santi/.profile 2>/dev/null; then
  cat >>/home/santi/.profile <<'EOF'

# santi manages the local k3s cluster as its work surface.
export KUBECONFIG="$HOME/.kube/config"
EOF
  chown santi:santi /home/santi/.profile
fi

if sudo -u santi -H bash -lc 'kubectl get nodes -o name >/dev/null 2>&1 || k3s kubectl get nodes -o name >/dev/null'; then
  echo "[kube] santi can reach the cluster"
else
  echo "[kube] santi cannot reach the cluster" >&2
  exit 1
fi
