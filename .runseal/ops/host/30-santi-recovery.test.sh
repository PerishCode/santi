#!/usr/bin/env bash
# Host-independent recovery-capsule integration vectors. Real tar/hash/find
# primitives are used; only dpkg, systemd, curl, and the Santi doctor are faked.

set -euo pipefail

REPO_ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
PROGRAM="$REPO_ROOT/.runseal/ops/host/30-santi-recovery.sh"
TEST_ROOT=$(mktemp -d)
trap 'rm -rf -- "$TEST_ROOT"' EXIT
FAKE_BIN="$TEST_ROOT/bin"
STATE="$TEST_ROOT/state"
mkdir -p "$FAKE_BIN" "$STATE"

cat >"$FAKE_BIN/dpkg-query" <<'EOF'
#!/usr/bin/env bash
cat "$SANTI_RECOVERY_TEST_STATE/installed"
EOF

cat >"$FAKE_BIN/dpkg-deb" <<'EOF'
#!/usr/bin/env bash
file=$2
field=$3
sed -n "s/^${field}: //p" "$file"
EOF

cat >"$FAKE_BIN/dpkg" <<'EOF'
#!/usr/bin/env bash
if [[ ${SANTI_RECOVERY_TEST_DPKG_FAIL:-0} == 1 ]]; then
  exit 42
fi
sed -n 's/^Version: //p' "$2" >"$SANTI_RECOVERY_TEST_STATE/installed"
EOF

cat >"$FAKE_BIN/systemctl" <<'EOF'
#!/usr/bin/env bash
case "$1" in
  is-active)
    state=$(cat "$SANTI_RECOVERY_TEST_STATE/service")
    echo "$state"
    [[ $state == active ]]
    ;;
  stop)
    printf 'inactive\n' >"$SANTI_RECOVERY_TEST_STATE/service"
    ;;
  start)
    printf 'active\n' >"$SANTI_RECOVERY_TEST_STATE/service"
    ;;
  *)
    exit 2
    ;;
esac
EOF

cat >"$FAKE_BIN/curl" <<'EOF'
#!/usr/bin/env bash
[[ $(cat "$SANTI_RECOVERY_TEST_STATE/service") == active ]]
EOF

cat >"$FAKE_BIN/santi" <<'EOF'
#!/usr/bin/env bash
schema=$(cat "${SANTI_DB}.schema")
printf '{"schema_version":%s,"expected_schema_version":%s,"ok":true}\n' "$schema" "$schema"
EOF

chmod 0755 "$FAKE_BIN"/*

assert_file_text() {
  local file=$1 expected=$2 actual
  [[ -f $file ]] || {
    echo "missing expected file: $file" >&2
    exit 1
  }
  actual=$(cat "$file")
  [[ $actual == "$expected" ]] || {
    echo "unexpected $file: expected=$expected actual=$actual" >&2
    exit 1
  }
}

make_package() {
  local file=$1 version=$2
  mkdir -p "$(dirname "$file")"
  printf 'Package: santi\nVersion: %s\n' "$version" >"$file"
}

prepare_host() {
  local scenario=$1 home="$TEST_ROOT/$1/home" source_tree="$TEST_ROOT/$1/source"
  mkdir -p "$home" "$source_tree/runtime/souls/soul_default/memory" \
    "$source_tree/runtime/upgrade/packages/source-hash"
  printf 'source-db\n' >"$source_tree/runtime/db"
  printf '28\n' >"$source_tree/runtime/db.schema"
  printf 'durable-memory\n' >"$source_tree/runtime/souls/soul_default/memory/MEMORY.md"
  make_package "$source_tree/runtime/upgrade/packages/source-hash/santi.deb" "0.1.0-beta.53"
  cat >"$source_tree/runtime/upgrade/installed-package.json" <<'EOF'
{"protocol_version":1,"artifact":{"package":"santi","version":"0.1.0-beta.53","sha256":"fixture","bytes":42}}
EOF
  tar -czf "$home/santi-runtime-backup.tar.gz" -C "$source_tree" runtime
  cp -a "$source_tree/runtime" "$home/runtime"
  printf 'candidate-db\n' >"$home/runtime/db"
  printf '33\n' >"$home/runtime/db.schema"
  make_package "$home/runtime/upgrade/packages/candidate-hash/santi.deb" "0.1.0-beta.54"
  cat >"$home/runtime/upgrade/installed-package.json" <<'EOF'
{"protocol_version":1,"artifact":{"package":"santi","version":"0.1.0-beta.54","sha256":"fixture","bytes":42}}
EOF
  printf '%s\n' "$home"
}

invoke() {
  local home=$1
  shift
  env \
    PATH="$FAKE_BIN:$PATH" \
    SANTI_RECOVERY_TEST=1 \
    SANTI_RECOVERY_TEST_STATE="$STATE" \
    SANTI_RECOVERY_HOME="$home" \
    SANTI_RECOVERY_BIN="$FAKE_BIN/santi" \
    bash "$PROGRAM" "$@"
}

printf '0.1.0-beta.54\n' >"$STATE/installed"
printf 'active\n' >"$STATE/service"
ROLLBACK_HOME=$(prepare_host rollback)
invoke "$ROLLBACK_HOME" guard-deploy >/dev/null
invoke "$ROLLBACK_HOME" arm >/dev/null
CAPSULE_ID=$(cat "$ROLLBACK_HOME/recovery/armed")
invoke "$ROLLBACK_HOME" status | grep -q 'validation: OK'
if invoke "$ROLLBACK_HOME" guard-deploy >/dev/null 2>&1; then
  echo "armed capsule did not block deploy" >&2
  exit 1
fi
if invoke "$ROLLBACK_HOME" execute "$CAPSULE_ID" --confirm 0.1.0-beta.999 \
  >/dev/null 2>&1; then
  echo "incorrect candidate confirmation unexpectedly succeeded" >&2
  exit 1
fi
assert_file_text "$STATE/service" "active"
assert_file_text "$ROLLBACK_HOME/runtime/db.schema" "33"
invoke "$ROLLBACK_HOME" execute "$CAPSULE_ID" --confirm 0.1.0-beta.54 >/dev/null
assert_file_text "$STATE/installed" "0.1.0-beta.53"
assert_file_text "$STATE/service" "active"
assert_file_text "$ROLLBACK_HOME/runtime/db.schema" "28"
assert_file_text "$ROLLBACK_HOME/recovery/$CAPSULE_ID/candidate-runtime/db.schema" "33"
[[ -f $ROLLBACK_HOME/recovery/$CAPSULE_ID/ROLLED_BACK ]]
[[ ! -e $ROLLBACK_HOME/recovery/armed ]]
invoke "$ROLLBACK_HOME" guard-deploy >/dev/null

printf '0.1.0-beta.54\n' >"$STATE/installed"
printf 'active\n' >"$STATE/service"
FAILURE_HOME=$(prepare_host failure)
invoke "$FAILURE_HOME" arm >/dev/null
FAILURE_ID=$(cat "$FAILURE_HOME/recovery/armed")
if SANTI_RECOVERY_TEST_DPKG_FAIL=1 invoke "$FAILURE_HOME" execute "$FAILURE_ID" \
  --confirm 0.1.0-beta.54 >/dev/null 2>&1; then
  echo "injected dpkg failure unexpectedly succeeded" >&2
  exit 1
fi
assert_file_text "$STATE/service" "inactive"
assert_file_text "$FAILURE_HOME/recovery/$FAILURE_ID/candidate-runtime/db.schema" "33"
[[ -f $FAILURE_HOME/recovery/armed ]]

printf '0.1.0-beta.54\n' >"$STATE/installed"
printf 'active\n' >"$STATE/service"
ACCEPT_HOME=$(prepare_host accept)
invoke "$ACCEPT_HOME" arm >/dev/null
ACCEPT_ID=$(cat "$ACCEPT_HOME/recovery/armed")
invoke "$ACCEPT_HOME" accept "$ACCEPT_ID" >/dev/null
assert_file_text "$ACCEPT_HOME/runtime/db.schema" "33"
[[ -f $ACCEPT_HOME/recovery/$ACCEPT_ID/ACCEPTED ]]
[[ ! -e $ACCEPT_HOME/recovery/armed ]]

EARLY_FAILURE_HOME="$TEST_ROOT/early-failure/home"
mkdir -p "$EARLY_FAILURE_HOME"
if invoke "$EARLY_FAILURE_HOME" arm >/dev/null 2>&1; then
  echo "arm without a raw backup unexpectedly succeeded" >&2
  exit 1
fi
[[ -f $EARLY_FAILURE_HOME/recovery/arming ]]
if invoke "$EARLY_FAILURE_HOME" guard-deploy >/dev/null 2>&1; then
  echo "incomplete arm did not block deploy" >&2
  exit 1
fi

echo "recovery host vectors: ok"
