#!/usr/bin/env bash
# Host-independent deploy transaction vectors. Filesystem/tar/hash behavior is
# real; dpkg, systemd, curl, and the final binary are controlled fakes.

set -euo pipefail

REPO_ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
PROGRAM="$REPO_ROOT/.runseal/ops/host/40-santi-deploy.sh"
TEST_ROOT=$(mktemp -d)
trap 'rm -rf -- "$TEST_ROOT"' EXIT
FAKE_BIN="$TEST_ROOT/bin"
STATE="$TEST_ROOT/state"
CANDIDATE="$TEST_ROOT/candidate.deb"
mkdir -p "$FAKE_BIN" "$STATE"

make_package() {
  local file=$1 version=$2
  mkdir -p "$(dirname "$file")"
  printf 'Package: santi\nVersion: %s\n' "$version" >"$file"
}

make_package "$CANDIDATE" "0.1.0-beta.55"

cat >"$FAKE_BIN/dpkg-query" <<'EOF'
#!/usr/bin/env bash
cat "$SANTI_DEPLOY_TEST_STATE/installed"
EOF

cat >"$FAKE_BIN/dpkg-deb" <<'EOF'
#!/usr/bin/env bash
sed -n "s/^$3: //p" "$2"
EOF

cat >"$FAKE_BIN/dpkg" <<'EOF'
#!/usr/bin/env bash
version=$(sed -n 's/^Version: //p' "$2")
printf '%s\n' "$version" >"$SANTI_DEPLOY_TEST_STATE/installed"
if [[ $version == 0.1.0-beta.55 ]]; then
  printf 'candidate-db\n' >"$SANTI_DEPLOY_TEST_HOME/runtime/db"
  printf '33\n' >"$SANTI_DEPLOY_TEST_HOME/runtime/db.schema"
else
  printf 'source-db\n' >"$SANTI_DEPLOY_TEST_HOME/runtime/db"
  printf '28\n' >"$SANTI_DEPLOY_TEST_HOME/runtime/db.schema"
fi
EOF

cat >"$FAKE_BIN/systemctl" <<'EOF'
#!/usr/bin/env bash
case "$1" in
  is-active)
    state=$(cat "$SANTI_DEPLOY_TEST_STATE/service")
    printf '%s\n' "$state"
    [[ $state == active ]]
    ;;
  stop)
    printf 'inactive\n' >"$SANTI_DEPLOY_TEST_STATE/service"
    ;;
  start)
    printf 'active\n' >"$SANTI_DEPLOY_TEST_STATE/service"
    ;;
  *)
    exit 2
    ;;
esac
EOF

cat >"$FAKE_BIN/curl" <<'EOF'
#!/usr/bin/env bash
for argument in "$@"; do
  if [[ $argument == test://candidate ]]; then
    destination=""
    previous=""
    for item in "$@"; do
      if [[ $previous == -o ]]; then destination=$item; fi
      previous=$item
    done
    cp "$SANTI_DEPLOY_TEST_CANDIDATE" "$destination"
    exit 0
  fi
done
[[ $(cat "$SANTI_DEPLOY_TEST_STATE/service") == active ]] || exit 1
installed=$(cat "$SANTI_DEPLOY_TEST_STATE/installed")
[[ ${SANTI_DEPLOY_TEST_FAIL_CANDIDATE:-0} != 1 || $installed != 0.1.0-beta.55 ]]
EOF

cat >"$FAKE_BIN/santi-api" <<'EOF'
#!/usr/bin/env bash
printf '{"estate_ready":true,"estate_error":null,"ok":true}\n'
EOF

chmod 0755 "$FAKE_BIN"/*

prepare_home() {
  local scenario=$1 home source package hash bytes
  home="$TEST_ROOT/$scenario/home"
  source="$TEST_ROOT/$scenario/source.deb"
  make_package "$source" "0.1.0-beta.54"
  hash=$(sha256sum "$source" | awk '{print $1}')
  bytes=$(stat -c '%s' "$source")
  package="$home/runtime/upgrade/packages/$hash/santi.deb"
  mkdir -p "$(dirname "$package")" "$home/runtime/souls/soul_default/memory"
  cp "$source" "$package"
  printf 'source-db\n' >"$home/runtime/db"
  printf '28\n' >"$home/runtime/db.schema"
  printf 'durable-memory\n' >"$home/runtime/souls/soul_default/memory/MEMORY.md"
  printf '{"protocol_version":1,"artifact":{"package":"santi","version":"0.1.0-beta.54","sha256":"%s","bytes":%s}}\n' \
    "$hash" "$bytes" >"$home/runtime/upgrade/installed-package.json"
  printf '%s\n' "$home"
}

invoke() {
  local home=$1
  shift
  env \
    PATH="$FAKE_BIN:/usr/bin:/bin" \
    SANTI_DEPLOY_TEST=1 \
    SANTI_DEPLOY_TEST_STATE="$STATE" \
    SANTI_DEPLOY_TEST_HOME="$home" \
    SANTI_DEPLOY_TEST_CANDIDATE="$CANDIDATE" \
    SANTI_DEPLOY_HOME="$home" \
    SANTI_DEPLOY_API="$FAKE_BIN/santi-api" \
    SANTI_DEPLOY_DEB_URL=test://candidate \
    SANTI_DEPLOY_HEALTH_ATTEMPTS=1 \
    SANTI_DEPLOY_HEALTH_DELAY=0 \
    bash "$PROGRAM" v0.1.0-beta.55 "$@"
}

assert_text() {
  local file=$1 expected=$2
  [[ $(cat "$file") == "$expected" ]] || {
    echo "unexpected $file" >&2
    exit 1
  }
}

SUCCESS_HOME=$(prepare_home success)
printf '0.1.0-beta.54\n' >"$STATE/installed"
printf 'active\n' >"$STATE/service"
invoke "$SUCCESS_HOME" >/dev/null
assert_text "$STATE/installed" "0.1.0-beta.55"
assert_text "$STATE/service" "active"
assert_text "$SUCCESS_HOME/runtime/db.schema" "33"
assert_text "$SUCCESS_HOME/runtime/souls/soul_default/memory/MEMORY.md" "durable-memory"
grep -q '"version":"0.1.0-beta.55"' "$SUCCESS_HOME/runtime/upgrade/installed-package.json"
tar -xOzf "$SUCCESS_HOME/santi-runtime-backup.tar.gz" \
  runtime/upgrade/installed-package.json | grep -q '"version":"0.1.0-beta.54"'
[[ ! -e $SUCCESS_HOME/recovery/.deploy-lock ]]

FAILURE_HOME=$(prepare_home failure)
printf '0.1.0-beta.54\n' >"$STATE/installed"
printf 'active\n' >"$STATE/service"
if SANTI_DEPLOY_TEST_FAIL_CANDIDATE=1 invoke "$FAILURE_HOME" >/dev/null 2>&1; then
  echo "unhealthy candidate unexpectedly deployed" >&2
  exit 1
fi
assert_text "$STATE/installed" "0.1.0-beta.54"
assert_text "$STATE/service" "active"
assert_text "$FAILURE_HOME/runtime/db.schema" "28"
assert_text "$FAILURE_HOME/runtime/souls/soul_default/memory/MEMORY.md" "durable-memory"
failed=$(find "$FAILURE_HOME/recovery" -mindepth 1 -maxdepth 1 -type d \
  -name 'failed-*' -print -quit)
[[ -n $failed && -f $failed/candidate-runtime/db ]]
assert_text "$failed/candidate-runtime/db.schema" "33"
[[ ! -e $FAILURE_HOME/recovery/.deploy-lock ]]

echo "deploy host vectors: ok"
