#!/usr/bin/env bash
# Read-only preflight for santi's host (zxiyun/hk-03).
set -euo pipefail

echo "[preflight] host=$(hostname)"
test "$(id -u)" = 0

. /etc/os-release
echo "[preflight] os=$ID-$VERSION_ID codename=${VERSION_CODENAME:-}"
test "$ID" = debian

command -v systemctl >/dev/null
command -v curl >/dev/null || { echo "  ! curl missing (apt-get install -y curl)" >&2; exit 1; }

k3s_state="$(systemctl is-active k3s 2>/dev/null || echo inactive)"
echo "[preflight] k3s: $k3s_state (santi's managed work surface — must stay)"
test "$k3s_state" = active

echo "[preflight] mem MB=$(free -m | awk '/Mem/{print $2}')  swap MB=$(free -m | awk '/Swap/{print $2}')"
echo "[preflight] clusterissuers:"
k3s kubectl get clusterissuer 2>/dev/null | sed 's/^/  /' || echo "  (none)"
echo "[preflight] :43307 listeners=$(ss -tlnH 'sport = :43307' 2>/dev/null | wc -l)"
echo "[preflight] santi user: $(id santi >/dev/null 2>&1 && echo present || echo absent)"
echo "[preflight] OK"
