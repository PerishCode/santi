#!/usr/bin/env bash
# Deploy one published beta with a source snapshot and automatic pre-arm
# rollback. Streamed over SSH; the live runtime never owns this program.

set -euo pipefail
IFS=$'\n\t'

VERSION=${1:-}
SANTI_DEPLOY_HOME=${SANTI_DEPLOY_HOME:-/home/santi/.santi}
SANTI_DEPLOY_SERVICE=${SANTI_DEPLOY_SERVICE:-santi.service}
SANTI_DEPLOY_HEALTH=${SANTI_DEPLOY_HEALTH:-http://127.0.0.1:43307/api/v1/health}
SANTI_DEPLOY_PUBLIC_URL=${SANTI_DEPLOY_PUBLIC_URL:-https://releases.santi.perish.uk}
SANTI_DEPLOY_API=${SANTI_DEPLOY_API:-/usr/bin/santi-api}
SANTI_DEPLOY_HEALTH_ATTEMPTS=${SANTI_DEPLOY_HEALTH_ATTEMPTS:-30}
SANTI_DEPLOY_HEALTH_DELAY=${SANTI_DEPLOY_HEALTH_DELAY:-2}
RUNTIME_ROOT="$SANTI_DEPLOY_HOME/runtime"
RECOVERY_ROOT="$SANTI_DEPLOY_HOME/recovery"
RAW_BACKUP="$SANTI_DEPLOY_HOME/santi-runtime-backup.tar.gz"
INSTALLED_MANIFEST="$RUNTIME_ROOT/upgrade/installed-package.json"
MEMORY_RELATIVE="souls/soul_default/memory/MEMORY.md"
LOCK_DIRECTORY="$RECOVERY_ROOT/.deploy-lock"
WORK=""
SOURCE_DEB=""
SOURCE_VERSION=""
CANDIDATE_DEB=""
CANDIDATE_VERSION=${VERSION#v}
CANDIDATE_SHA=""
CANDIDATE_BYTES=""
DEPLOY_TOUCHED=0
DEPLOY_DESTRUCTIVE=0
DEPLOY_SUCCEEDED=0

fail() {
  echo "deploy: $*" >&2
  exit 1
}

require_root() {
  if [[ ${EUID:-$(id -u)} -ne 0 && ${SANTI_DEPLOY_TEST:-0} != 1 ]]; then
    fail "must run as root"
  fi
}

require_commands() {
  local command_name
  for command_name in awk chmod chown cp curl date dirname dpkg dpkg-deb dpkg-query env find \
    grep install mkdir mktemp mv rm rmdir sed seq sha256sum sleep stat sync systemctl tar tr; do
    command -v "$command_name" >/dev/null 2>&1 ||
      fail "required command unavailable: $command_name"
  done
}

sha256_file() {
  sha256sum "$1" | awk '{print $1}'
}

bytes_file() {
  stat -c '%s' "$1"
}

installed_version() {
  dpkg-query -W -f='${Version}' santi 2>/dev/null || true
}

deb_field() {
  dpkg-deb -f "$1" "$2" 2>/dev/null || true
}

require_deb_identity() {
  local file=$1 expected=$2 package actual
  [[ -f $file && ! -L $file ]] || fail "package is not a regular file: $file"
  package=$(deb_field "$file" Package)
  actual=$(deb_field "$file" Version)
  [[ $package == santi ]] || fail "package identity mismatch for $file: $package"
  [[ $actual == "$expected" ]] ||
    fail "package version mismatch for $file: expected=$expected actual=$actual"
}

json_string() {
  local file=$1 field=$2 value
  value=$(sed -n "s/.*\"$field\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p" "$file" |
    head -n 1)
  [[ -n $value ]] || fail "JSON field unavailable: $field in $file"
  printf '%s\n' "$value"
}

json_number() {
  local file=$1 field=$2 value
  value=$(sed -n "s/.*\"$field\"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p" "$file" |
    head -n 1)
  [[ $value =~ ^[0-9]+$ ]] || fail "JSON number unavailable: $field in $file"
  printf '%s\n' "$value"
}

wait_health() {
  local attempt
  for attempt in $(seq 1 "$SANTI_DEPLOY_HEALTH_ATTEMPTS"); do
    if curl -fsS -o /dev/null "$SANTI_DEPLOY_HEALTH"; then
      return 0
    fi
    sleep "$SANTI_DEPLOY_HEALTH_DELAY"
  done
  return 1
}

load_source() {
  local package hash bytes retained current
  [[ -f $INSTALLED_MANIFEST && ! -L $INSTALLED_MANIFEST ]] ||
    fail "durable installed-package manifest is unavailable"
  package=$(json_string "$INSTALLED_MANIFEST" package)
  SOURCE_VERSION=$(json_string "$INSTALLED_MANIFEST" version)
  hash=$(json_string "$INSTALLED_MANIFEST" sha256)
  bytes=$(json_number "$INSTALLED_MANIFEST" bytes)
  [[ $package == santi ]] || fail "installed manifest package is not santi"
  [[ $SOURCE_VERSION =~ ^[A-Za-z0-9._:+~-]+$ ]] ||
    fail "installed manifest version is unsafe"
  [[ $hash =~ ^[0-9a-f]{64}$ ]] || fail "installed manifest sha256 is invalid"
  current=$(installed_version)
  [[ $current == "$SOURCE_VERSION" ]] ||
    fail "installed manifest differs from dpkg: manifest=$SOURCE_VERSION dpkg=$current"
  retained="$RUNTIME_ROOT/upgrade/packages/$hash/santi.deb"
  require_deb_identity "$retained" "$SOURCE_VERSION"
  [[ $(sha256_file "$retained") == "$hash" ]] || fail "retained source hash mismatch"
  [[ $(bytes_file "$retained") == "$bytes" ]] || fail "retained source size mismatch"
  SOURCE_DEB="$WORK/source.deb"
  cp -- "$retained" "$SOURCE_DEB"
}

fetch_candidate() {
  local artifact="$WORK/artifact.json" expected_bytes expected_sha package seal="$WORK/seal.json" url
  CANDIDATE_DEB="$WORK/candidate.deb"
  curl -fsSL \
    "$SANTI_DEPLOY_PUBLIC_URL/v1/releases/beta/$VERSION/seal.json" \
    -o "$seal"
  [[ $(json_string "$seal" product) == santi ]] || fail "release seal product mismatch"
  [[ $(json_string "$seal" channel) == beta ]] || fail "release seal channel mismatch"
  [[ $(json_string "$seal" version) == "$VERSION" ]] || fail "release seal version mismatch"
  awk '
      /^[[:space:]]*"linux-x64-deb":[[:space:]]*\{/ { held = 1 }
      held { print }
      held && /^[[:space:]]*\}[,]?[[:space:]]*$/ { exit }
    ' "$seal" >"$artifact"
  url=$(json_string "$artifact" url)
  expected_sha=$(json_string "$artifact" sha256)
  expected_bytes=$(json_number "$artifact" size)
  [[ $(json_string "$artifact" name) == santi-x86_64-unknown-linux-gnu.deb ]] ||
    fail "release seal Debian asset mismatch"
  [[ $expected_sha =~ ^[0-9a-f]{64}$ ]] || fail "release seal Debian hash is invalid"
  curl -fsSL "$url" -o "$CANDIDATE_DEB"
  [[ $(sha256_file "$CANDIDATE_DEB") == "$expected_sha" ]] ||
    fail "candidate package hash disagrees with release seal"
  [[ $(bytes_file "$CANDIDATE_DEB") == "$expected_bytes" ]] ||
    fail "candidate package size disagrees with release seal"
  require_deb_identity "$CANDIDATE_DEB" "$CANDIDATE_VERSION"
  package=$(deb_field "$CANDIDATE_DEB" Package)
  [[ $package == santi ]] || fail "candidate package is not santi"
  CANDIDATE_SHA=$(sha256_file "$CANDIDATE_DEB")
  CANDIDATE_BYTES=$(bytes_file "$CANDIDATE_DEB")
  [[ $CANDIDATE_VERSION != "$SOURCE_VERSION" ]] || fail "candidate equals installed source"
}

snapshot_source() {
  local temporary="$SANTI_DEPLOY_HOME/.santi-runtime-backup.$$"
  [[ -d $RUNTIME_ROOT && ! -L $RUNTIME_ROOT ]] || fail "runtime root is unavailable"
  if find "$RUNTIME_ROOT" -type l -print -quit | grep -q .; then
    fail "runtime contains a symbolic link"
  fi
  tar -czf "$temporary" -C "$SANTI_DEPLOY_HOME" runtime
  chmod 0600 "$temporary"
  if [[ ${SANTI_DEPLOY_TEST:-0} != 1 ]]; then
    chown santi:santi "$temporary"
  fi
  sync -f "$temporary"
  mv "$temporary" "$RAW_BACKUP"
  sync -f "$SANTI_DEPLOY_HOME"
}

retain_candidate() {
  local directory="$RUNTIME_ROOT/upgrade/packages/$CANDIDATE_SHA"
  local manifest_tmp="$RUNTIME_ROOT/upgrade/.installed-package.$$"
  mkdir -p "$directory"
  install -m 0644 "$CANDIDATE_DEB" "$directory/santi.deb"
  printf '{"protocol_version":1,"artifact":{"package":"santi","version":"%s","sha256":"%s","bytes":%s}}\n' \
    "$CANDIDATE_VERSION" "$CANDIDATE_SHA" "$CANDIDATE_BYTES" >"$manifest_tmp"
  chmod 0644 "$manifest_tmp"
  if [[ ${SANTI_DEPLOY_TEST:-0} != 1 ]]; then
    chown -R santi:santi "$RUNTIME_ROOT/upgrade"
  fi
  sync -f "$directory/santi.deb"
  mv "$manifest_tmp" "$INSTALLED_MANIFEST"
  sync -f "$RUNTIME_ROOT/upgrade"
}

doctor_candidate() {
  (
    set -a
    [[ ! -r /etc/santi/santi.env ]] || . /etc/santi/santi.env
    set +a
    env SANTI_HOME="$SANTI_DEPLOY_HOME" "$SANTI_DEPLOY_API" doctor
  )
}

rollback_source() {
  local timestamp failed stage
  timestamp=$(date -u +%Y%m%dT%H%M%SZ)
  failed="$RECOVERY_ROOT/failed-$SOURCE_VERSION--$CANDIDATE_VERSION--$timestamp"
  stage="$WORK/restore"
  echo "deploy: restoring source $SOURCE_VERSION" >&2
  systemctl stop "$SANTI_DEPLOY_SERVICE" >/dev/null 2>&1 || true
  mkdir -p -m 0700 "$failed"
  mv "$RUNTIME_ROOT" "$failed/candidate-runtime"
  mkdir -m 0700 "$stage"
  tar -xzf "$RAW_BACKUP" -C "$stage" --no-same-owner --no-same-permissions
  [[ -d $stage/runtime && ! -L $stage/runtime ]] || return 1
  mv "$stage/runtime" "$RUNTIME_ROOT"
  rmdir "$stage"
  if [[ ${SANTI_DEPLOY_TEST:-0} != 1 ]]; then
    chown -R santi:santi "$RUNTIME_ROOT"
  fi
  dpkg -i "$SOURCE_DEB"
  if [[ ${SANTI_DEPLOY_TEST:-0} != 1 ]]; then
    chown -R root:root "$RECOVERY_ROOT"
  fi
  [[ $(installed_version) == "$SOURCE_VERSION" ]] || return 1
  systemctl start "$SANTI_DEPLOY_SERVICE"
  wait_health || return 1
  echo "deploy: source restored; candidate runtime retained at $failed/candidate-runtime" >&2
}

cleanup() {
  local code=$?
  trap - EXIT
  set +e
  if [[ $DEPLOY_SUCCEEDED != 1 ]]; then
    if [[ $DEPLOY_DESTRUCTIVE == 1 ]]; then
      rollback_source || {
        systemctl stop "$SANTI_DEPLOY_SERVICE" >/dev/null 2>&1 || true
        echo "deploy: rollback failed; service remains stopped" >&2
      }
    elif [[ $DEPLOY_TOUCHED == 1 ]]; then
      systemctl start "$SANTI_DEPLOY_SERVICE" >/dev/null 2>&1 || true
    fi
  fi
  [[ -z $WORK || $WORK != /tmp/santi-deploy.* ]] || rm -rf -- "$WORK"
  rmdir "$LOCK_DIRECTORY" 2>/dev/null || true
  exit "$code"
}

[[ $VERSION =~ ^v[0-9]+\.[0-9]+\.[0-9]+-beta\.[1-9][0-9]*$ ]] ||
  fail "version must look like vX.Y.Z-beta.N"
require_root
require_commands
mkdir -p -m 0700 "$RECOVERY_ROOT"
[[ ! -e $RECOVERY_ROOT/armed ]] || fail "an armed recovery capsule blocks deploy"
[[ ! -e $RECOVERY_ROOT/arming ]] || fail "an incomplete recovery arm blocks deploy"
mkdir "$LOCK_DIRECTORY" 2>/dev/null || fail "another deploy is active"
trap cleanup EXIT
WORK=$(mktemp -d /tmp/santi-deploy.XXXXXX)

load_source
fetch_candidate
BEFORE=$(sha256_file "$RUNTIME_ROOT/$MEMORY_RELATIVE")
echo "deploy: $SOURCE_VERSION -> $CANDIDATE_VERSION"
DEPLOY_TOUCHED=1
systemctl stop "$SANTI_DEPLOY_SERVICE"
state=$(systemctl is-active "$SANTI_DEPLOY_SERVICE" 2>/dev/null || true)
[[ $state != active && $state != activating ]] || fail "service did not stop"
snapshot_source
DEPLOY_DESTRUCTIVE=1
dpkg -i "$CANDIDATE_DEB"
[[ $(installed_version) == "$CANDIDATE_VERSION" ]] || fail "candidate did not install"
retain_candidate
systemctl start "$SANTI_DEPLOY_SERVICE"
wait_health || fail "candidate service did not become healthy"
doctor_candidate
AFTER=$(sha256_file "$RUNTIME_ROOT/$MEMORY_RELATIVE")
[[ $BEFORE == "$AFTER" ]] || fail "soul memory changed: before=$BEFORE after=$AFTER"
DEPLOY_SUCCEEDED=1
echo "deploy: ready version=$CANDIDATE_VERSION memory=$AFTER"
