#!/usr/bin/env bash
# Create the dedicated santi user with passwordless sudo.
set -euo pipefail
test "$(id -u)" = 0

if ! id santi >/dev/null 2>&1; then
  useradd --create-home --shell /bin/bash --comment "santi soul runtime" santi
  echo "[user] created santi"
else
  echo "[user] santi already exists"
fi

install -d -m 0700 -o santi -g santi /home/santi/.santi

cat >/etc/sudoers.d/santi <<'EOF'
# santi is an agent that manages this host and its k3s work surface.
santi ALL=(ALL) NOPASSWD:ALL
EOF
chmod 0440 /etc/sudoers.d/santi
visudo -cf /etc/sudoers.d/santi
echo "[user] passwordless sudo installed and validated"
