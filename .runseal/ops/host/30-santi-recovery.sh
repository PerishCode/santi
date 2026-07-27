#!/usr/bin/env bash
# Build and operate the post-deploy recovery capsule. This program is streamed
# over SSH; it deliberately depends only on host primitives, never on recovery
# endpoints exposed by the candidate service.

set -euo pipefail
IFS=$'\n\t'

ACTION=${1:-}
SANTI_RECOVERY_HOME=${SANTI_RECOVERY_HOME:-/home/santi/.santi}
SANTI_RECOVERY_BIN=${SANTI_RECOVERY_BIN:-}
SANTI_RECOVERY_SERVICE=${SANTI_RECOVERY_SERVICE:-santi.service}
SANTI_RECOVERY_HEALTH=${SANTI_RECOVERY_HEALTH:-http://127.0.0.1:43307/api/v1/health}
RUNTIME_ROOT="$SANTI_RECOVERY_HOME/runtime"
RECOVERY_ROOT="$SANTI_RECOVERY_HOME/recovery"
RAW_BACKUP="$SANTI_RECOVERY_HOME/santi-runtime-backup.tar.gz"
ARMED_POINTER="$RECOVERY_ROOT/armed"
ARMING_MARKER="$RECOVERY_ROOT/arming"
LOCK_DIRECTORY="$RECOVERY_ROOT/.lock"
MEMORY_RELATIVE="souls/soul_default/memory/MEMORY.md"
MANIFEST_PROTOCOL="santi.recovery.v1"

fail() {
  echo "recovery: $*" >&2
  exit 1
}

require_root() {
  if [[ ${EUID:-$(id -u)} -ne 0 && ${SANTI_RECOVERY_TEST:-0} != 1 ]]; then
    fail "must run as root"
  fi
}

require_commands() {
  local command
  for command in awk chmod chown cp curl date dirname dpkg dpkg-deb dpkg-query env find grep head \
    mkdir mktemp mv rm rmdir sed seq sha256sum sleep stat sync systemctl tar tr; do
    command -v "$command" >/dev/null 2>&1 || fail "required command unavailable: $command"
  done
}

safe_token() {
  [[ $1 =~ ^[A-Za-z0-9._:+~-]+$ ]] || fail "unsafe token: $1"
}

safe_capsule_id() {
  safe_token "$1"
  [[ $1 != .* && $1 == *--*--* ]] || fail "invalid capsule id: $1"
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
  local file=$1 expected_version=$2 package version
  [[ -f $file && ! -L $file ]] || fail "package is not a regular file: $file"
  package=$(deb_field "$file" Package)
  version=$(deb_field "$file" Version)
  [[ $package == santi ]] || fail "package identity mismatch for $file: $package"
  [[ $version == "$expected_version" ]] ||
    fail "package version mismatch for $file: expected=$expected_version actual=$version"
}

json_string_field() {
  local file=$1 field=$2 value
  value=$(tr -d '\n' <"$file" |
    sed -n "s/.*\"$field\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p")
  [[ -n $value ]] || fail "JSON field unavailable: $field in $file"
  printf '%s\n' "$value"
}

json_number_field_from_text() {
  local field=$1 text=$2 value
  value=$(printf '%s' "$text" |
    tr -d '\n' |
    sed -n "s/.*\"$field\"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p")
  [[ $value =~ ^[0-9]+$ ]] || fail "JSON number unavailable: $field"
  printf '%s\n' "$value"
}

archive_entries_are_safe() {
  local archive=$1 entry listing kind
  [[ -f $archive && ! -L $archive ]] || fail "archive is not a regular file: $archive"
  while IFS= read -r entry; do
    [[ -n $entry ]] || continue
    [[ $entry == runtime || $entry == runtime/ || $entry == runtime/* ]] ||
      fail "archive entry is outside runtime/: $entry"
    [[ $entry != /* && $entry != *\\* && ! $entry =~ (^|/)\.\.(/|$) ]] ||
      fail "unsafe archive entry: $entry"
  done < <(tar -tzf "$archive")
  listing=$(tar -tvzf "$archive")
  while IFS= read -r entry; do
    [[ -n $entry ]] || continue
    kind=${entry:0:1}
    [[ $kind == d || $kind == - ]] || fail "archive contains a link or special entry"
  done <<<"$listing"
}

extract_archive() {
  local archive=$1 destination=$2
  archive_entries_are_safe "$archive"
  mkdir -m 0700 "$destination"
  tar --extract --gzip --file "$archive" --directory "$destination" \
    --no-same-owner --no-same-permissions
  [[ -d $destination/runtime && ! -L $destination/runtime ]] ||
    fail "archive has no regular runtime directory"
  if find "$destination/runtime" -type l -print -quit | grep -q .; then
    fail "extracted runtime contains a symbolic link"
  fi
}

find_single_deb() {
  local root=$1 expected_version=$2 match="" count=0 candidate package version
  while IFS= read -r -d '' candidate; do
    package=$(deb_field "$candidate" Package)
    version=$(deb_field "$candidate" Version)
    if [[ $package == santi && $version == "$expected_version" ]]; then
      match=$candidate
      count=$((count + 1))
    fi
  done < <(find "$root" -type f -name santi.deb -print0)
  [[ $count -eq 1 ]] ||
    fail "expected exactly one retained santi $expected_version package under $root; found $count"
  printf '%s\n' "$match"
}

schema_for_runtime() {
  local runtime=$1 output binary=$SANTI_RECOVERY_BIN
  if [[ -z $binary ]]; then
    if [[ -x /usr/bin/santi-api ]]; then
      binary=/usr/bin/santi-api
    else
      binary=/usr/bin/santi
    fi
  fi
  output=$(env \
    SANTI_HOME="$(dirname "$runtime")" \
    SANTI_PATHS_DATABASE="$runtime/db" \
    SANTI_PATHS_RUNTIME_ROOT="$runtime" \
    "$binary" doctor --storage-only 2>/dev/null || true)
  json_number_field_from_text schema_version "$output"
}

manifest_value() {
  local manifest=$1 key=$2
  awk -F '\t' -v key="$key" '
    $1 == key {
      count += 1
      value = $2
      if (NF != 2) malformed = 1
    }
    END {
      if (count != 1 || malformed || value == "") exit 1
      print value
    }
  ' "$manifest" || fail "manifest key invalid or missing: $key"
}

validate_manifest_shape() {
  local manifest=$1
  awk -F '\t' '
    BEGIN {
      keys["protocol"] = 1
      keys["capsule_id"] = 1
      keys["source_version"] = 1
      keys["candidate_version"] = 1
      keys["source_schema"] = 1
      keys["candidate_schema"] = 1
      keys["backup_sha256"] = 1
      keys["backup_bytes"] = 1
      keys["source_deb_sha256"] = 1
      keys["source_deb_bytes"] = 1
      keys["candidate_deb_sha256"] = 1
      keys["candidate_deb_bytes"] = 1
      keys["memory_sha256"] = 1
      keys["created_utc"] = 1
    }
    NF != 2 || !($1 in keys) || seen[$1]++ || $2 == "" { invalid = 1 }
    END { if (invalid || NR != 14) exit 1 }
  ' "$manifest" || fail "capsule manifest shape is invalid"
}

load_manifest() {
  local capsule=$1 manifest="$capsule/manifest"
  [[ -f $manifest && ! -L $manifest ]] || fail "capsule manifest is not a regular file"
  validate_manifest_shape "$manifest"
  M_PROTOCOL=$(manifest_value "$manifest" protocol)
  M_CAPSULE_ID=$(manifest_value "$manifest" capsule_id)
  M_SOURCE_VERSION=$(manifest_value "$manifest" source_version)
  M_CANDIDATE_VERSION=$(manifest_value "$manifest" candidate_version)
  M_SOURCE_SCHEMA=$(manifest_value "$manifest" source_schema)
  M_CANDIDATE_SCHEMA=$(manifest_value "$manifest" candidate_schema)
  M_BACKUP_SHA256=$(manifest_value "$manifest" backup_sha256)
  M_BACKUP_BYTES=$(manifest_value "$manifest" backup_bytes)
  M_SOURCE_DEB_SHA256=$(manifest_value "$manifest" source_deb_sha256)
  M_SOURCE_DEB_BYTES=$(manifest_value "$manifest" source_deb_bytes)
  M_CANDIDATE_DEB_SHA256=$(manifest_value "$manifest" candidate_deb_sha256)
  M_CANDIDATE_DEB_BYTES=$(manifest_value "$manifest" candidate_deb_bytes)
  M_MEMORY_SHA256=$(manifest_value "$manifest" memory_sha256)
  M_CREATED_UTC=$(manifest_value "$manifest" created_utc)
}

require_hash() {
  [[ $1 =~ ^[0-9a-f]{64}$ ]] || fail "invalid sha256 in manifest: $1"
}

require_number() {
  [[ $1 =~ ^[0-9]+$ ]] || fail "invalid number in manifest: $1"
}

source_manifest_version() {
  local archive=$1 temporary manifest package version
  temporary=$(mktemp "$RECOVERY_ROOT/.installed-package.XXXXXX")
  tar -xOzf "$archive" runtime/upgrade/installed-package.json >"$temporary" || {
    rm -f "$temporary"
    fail "raw backup has no installed-package manifest"
  }
  package=$(json_string_field "$temporary" package)
  version=$(json_string_field "$temporary" version)
  rm -f "$temporary"
  [[ $package == santi ]] || fail "raw backup installed package is not santi: $package"
  printf '%s\n' "$version"
}

load_archive_facts() {
  local archive=$1 scratch runtime package_manifest retained
  scratch=$(mktemp -d "$RECOVERY_ROOT/.archive.XXXXXX")
  extract_archive "$archive" "$scratch/unpacked"
  runtime="$scratch/unpacked/runtime"
  package_manifest="$runtime/upgrade/installed-package.json"
  [[ $(json_string_field "$package_manifest" package) == santi ]] ||
    fail "archive installed package is not santi"
  A_SOURCE_VERSION=$(json_string_field "$package_manifest" version)
  retained=$(find_single_deb "$runtime/upgrade/packages" "$A_SOURCE_VERSION")
  require_deb_identity "$retained" "$A_SOURCE_VERSION"
  A_SOURCE_DEB_SHA256=$(sha256_file "$retained")
  A_SOURCE_DEB_BYTES=$(bytes_file "$retained")
  A_SOURCE_SCHEMA=$(schema_for_runtime "$runtime")
  A_MEMORY_SHA256=$(sha256_file "$runtime/$MEMORY_RELATIVE")
  rm -rf -- "$scratch"
}

validate_capsule() {
  local expected_id=$1 capsule="$RECOVERY_ROOT/$1"
  safe_capsule_id "$expected_id"
  [[ -d $capsule && ! -L $capsule ]] || fail "capsule not found: $expected_id"
  load_manifest "$capsule"
  [[ $M_PROTOCOL == "$MANIFEST_PROTOCOL" ]] || fail "unsupported capsule protocol: $M_PROTOCOL"
  [[ $M_CAPSULE_ID == "$expected_id" ]] || fail "capsule id disagrees with manifest"
  safe_token "$M_SOURCE_VERSION"
  safe_token "$M_CANDIDATE_VERSION"
  require_number "$M_SOURCE_SCHEMA"
  require_number "$M_CANDIDATE_SCHEMA"
  require_number "$M_BACKUP_BYTES"
  require_number "$M_SOURCE_DEB_BYTES"
  require_number "$M_CANDIDATE_DEB_BYTES"
  require_hash "$M_BACKUP_SHA256"
  require_hash "$M_SOURCE_DEB_SHA256"
  require_hash "$M_CANDIDATE_DEB_SHA256"
  require_hash "$M_MEMORY_SHA256"
  [[ $M_CREATED_UTC =~ ^[0-9]{8}T[0-9]{6}Z$ ]] || fail "invalid creation time in manifest"

  archive_entries_are_safe "$capsule/runtime.tar.gz"
  require_deb_identity "$capsule/source.deb" "$M_SOURCE_VERSION"
  require_deb_identity "$capsule/candidate.deb" "$M_CANDIDATE_VERSION"
  [[ $(sha256_file "$capsule/runtime.tar.gz") == "$M_BACKUP_SHA256" ]] ||
    fail "capsule runtime archive hash mismatch"
  [[ $(bytes_file "$capsule/runtime.tar.gz") == "$M_BACKUP_BYTES" ]] ||
    fail "capsule runtime archive size mismatch"
  [[ $(sha256_file "$capsule/source.deb") == "$M_SOURCE_DEB_SHA256" ]] ||
    fail "capsule source package hash mismatch"
  [[ $(bytes_file "$capsule/source.deb") == "$M_SOURCE_DEB_BYTES" ]] ||
    fail "capsule source package size mismatch"
  [[ $(sha256_file "$capsule/candidate.deb") == "$M_CANDIDATE_DEB_SHA256" ]] ||
    fail "capsule candidate package hash mismatch"
  [[ $(bytes_file "$capsule/candidate.deb") == "$M_CANDIDATE_DEB_BYTES" ]] ||
    fail "capsule candidate package size mismatch"
  load_archive_facts "$capsule/runtime.tar.gz"
  [[ $A_SOURCE_VERSION == "$M_SOURCE_VERSION" ]] ||
    fail "capsule archive source version mismatch"
  [[ $A_SOURCE_DEB_SHA256 == "$M_SOURCE_DEB_SHA256" ]] ||
    fail "capsule archive source package hash mismatch"
  [[ $A_SOURCE_DEB_BYTES == "$M_SOURCE_DEB_BYTES" ]] ||
    fail "capsule archive source package size mismatch"
  [[ $A_MEMORY_SHA256 == "$M_MEMORY_SHA256" ]] || fail "capsule archive memory hash mismatch"
  [[ $A_SOURCE_SCHEMA == "$M_SOURCE_SCHEMA" ]] ||
    fail "capsule archive schema mismatch: manifest=$M_SOURCE_SCHEMA actual=$A_SOURCE_SCHEMA"
}

read_armed_id() {
  local id
  [[ -f $ARMED_POINTER && ! -L $ARMED_POINTER ]] || fail "no recovery capsule is armed"
  IFS= read -r id <"$ARMED_POINTER"
  safe_capsule_id "$id"
  printf '%s\n' "$id"
}

require_exact_armed() {
  local expected=$1 armed
  armed=$(read_armed_id)
  [[ $armed == "$expected" ]] || fail "armed capsule is $armed, not $expected"
}

require_current_candidate() {
  local current
  current=$(installed_version)
  [[ $current == "$M_CANDIDATE_VERSION" ]] ||
    fail "installed version is not the capsule candidate: expected=$M_CANDIDATE_VERSION actual=$current"
  [[ $(schema_for_runtime "$RUNTIME_ROOT") == "$M_CANDIDATE_SCHEMA" ]] ||
    fail "current schema differs from capsule"
}

require_candidate_health() {
  [[ $(systemctl is-active "$SANTI_RECOVERY_SERVICE" 2>/dev/null || true) == active ]] ||
    fail "candidate service is not active"
  curl -fsS -o /dev/null "$SANTI_RECOVERY_HEALTH" || fail "candidate service is not healthy"
}

acquire_lock() {
  mkdir -p -m 0700 "$RECOVERY_ROOT"
  [[ -d $RECOVERY_ROOT && ! -L $RECOVERY_ROOT ]] || fail "recovery root is not a directory"
  chmod 0700 "$RECOVERY_ROOT"
  if [[ ${SANTI_RECOVERY_TEST:-0} != 1 ]]; then
    chown -R root:root "$RECOVERY_ROOT"
  fi
  mkdir "$LOCK_DIRECTORY" 2>/dev/null || fail "another recovery operation is active"
}

release_lock() {
  rmdir "$LOCK_DIRECTORY" 2>/dev/null || true
}

write_arming_marker() {
  local value=$1 temporary="$RECOVERY_ROOT/.arming.$$"
  printf '%s\n' "$value" >"$temporary"
  chmod 0600 "$temporary"
  mv "$temporary" "$ARMING_MARKER"
  sync -f "$RECOVERY_ROOT"
}

action_guard_deploy() {
  if [[ -e $ARMED_POINTER ]]; then
    fail "an armed recovery capsule blocks deploy; run runseal :rollback status"
  fi
  if [[ -e $ARMING_MARKER ]]; then
    fail "an incomplete capsule arm blocks deploy; inspect status, then run runseal :rollback repair"
  fi
  echo "recovery gate: clear"
}

action_arm() {
  local candidate_version source_version source_stage source_deb candidate_deb
  local source_schema candidate_schema source_memory timestamp
  local source_component candidate_component capsule_id temporary final manifest pointer_tmp
  require_root
  require_commands
  acquire_lock
  trap release_lock EXIT
  [[ ! -e $ARMED_POINTER ]] || fail "a recovery capsule is already armed"
  timestamp=$(date -u +%Y%m%dT%H%M%SZ)
  # From this point onward any failure must block the next deploy. `repair`
  # retries construction from the raw snapshot and retained packages.
  write_arming_marker "pending--pending--$timestamp"
  [[ -f $RAW_BACKUP && ! -L $RAW_BACKUP ]] || fail "raw upgrader backup unavailable: $RAW_BACKUP"
  [[ -d $RUNTIME_ROOT && ! -L $RUNTIME_ROOT ]] || fail "current runtime unavailable"

  candidate_version=$(installed_version)
  [[ -n $candidate_version ]] || fail "installed candidate version unavailable"
  safe_token "$candidate_version"
  source_version=$(source_manifest_version "$RAW_BACKUP")
  safe_token "$source_version"
  [[ $source_version != "$candidate_version" ]] || fail "source and candidate versions are identical"

  source_component=${source_version//:/_}
  candidate_component=${candidate_version//:/_}
  capsule_id="$source_component--$candidate_component--$timestamp"
  safe_capsule_id "$capsule_id"
  temporary="$RECOVERY_ROOT/.${capsule_id}.tmp.$$"
  final="$RECOVERY_ROOT/$capsule_id"
  [[ ! -e $temporary && ! -e $final ]] || fail "capsule target already exists: $capsule_id"

  mkdir -p -m 0700 "$RECOVERY_ROOT"
  write_arming_marker "$capsule_id"

  source_stage="$temporary/source-stage"
  mkdir -m 0700 "$temporary"
  extract_archive "$RAW_BACKUP" "$source_stage"
  source_deb=$(find_single_deb "$source_stage/runtime/upgrade/packages" "$source_version")
  candidate_deb=$(find_single_deb "$RUNTIME_ROOT/upgrade/packages" "$candidate_version")
  require_deb_identity "$source_deb" "$source_version"
  require_deb_identity "$candidate_deb" "$candidate_version"
  source_schema=$(schema_for_runtime "$source_stage/runtime")
  candidate_schema=$(schema_for_runtime "$RUNTIME_ROOT")
  source_memory=$(sha256_file "$source_stage/runtime/$MEMORY_RELATIVE")

  cp -- "$RAW_BACKUP" "$temporary/runtime.tar.gz"
  cp -- "$source_deb" "$temporary/source.deb"
  cp -- "$candidate_deb" "$temporary/candidate.deb"
  chmod 0600 "$temporary/runtime.tar.gz" "$temporary/source.deb" "$temporary/candidate.deb"
  manifest="$temporary/manifest"
  {
    printf 'protocol\t%s\n' "$MANIFEST_PROTOCOL"
    printf 'capsule_id\t%s\n' "$capsule_id"
    printf 'source_version\t%s\n' "$source_version"
    printf 'candidate_version\t%s\n' "$candidate_version"
    printf 'source_schema\t%s\n' "$source_schema"
    printf 'candidate_schema\t%s\n' "$candidate_schema"
    printf 'backup_sha256\t%s\n' "$(sha256_file "$temporary/runtime.tar.gz")"
    printf 'backup_bytes\t%s\n' "$(bytes_file "$temporary/runtime.tar.gz")"
    printf 'source_deb_sha256\t%s\n' "$(sha256_file "$temporary/source.deb")"
    printf 'source_deb_bytes\t%s\n' "$(bytes_file "$temporary/source.deb")"
    printf 'candidate_deb_sha256\t%s\n' "$(sha256_file "$temporary/candidate.deb")"
    printf 'candidate_deb_bytes\t%s\n' "$(bytes_file "$temporary/candidate.deb")"
    printf 'memory_sha256\t%s\n' "$source_memory"
    printf 'created_utc\t%s\n' "$timestamp"
  } >"$manifest"
  chmod 0600 "$manifest"
  rm -rf -- "$source_stage"
  sync -f "$temporary/runtime.tar.gz"
  sync -f "$temporary/source.deb"
  sync -f "$temporary/candidate.deb"
  sync -f "$manifest"
  sync -f "$temporary"
  mv "$temporary" "$final"
  validate_capsule "$capsule_id"
  printf 'armed_utc\t%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >"$final/ARMED"
  chmod 0600 "$final/ARMED"
  pointer_tmp="$RECOVERY_ROOT/.armed.$$"
  printf '%s\n' "$capsule_id" >"$pointer_tmp"
  chmod 0600 "$pointer_tmp"
  mv "$pointer_tmp" "$ARMED_POINTER"
  mv "$ARMING_MARKER" "$final/arming-completed"
  sync -f "$RECOVERY_ROOT"
  echo "recovery capsule armed: $capsule_id"
  echo "source=$source_version schema=$source_schema candidate=$candidate_version schema=$candidate_schema"
}

action_status() {
  local capsule_id service_state health_state="unhealthy"
  require_commands
  if [[ ! -e $RECOVERY_ROOT ]]; then
    echo "recovery: no capsule armed"
    return 0
  fi
  [[ -d $RECOVERY_ROOT && ! -L $RECOVERY_ROOT ]] || fail "recovery root is not a directory"
  if [[ -e $ARMING_MARKER && ! -e $ARMED_POINTER ]]; then
    echo "recovery: capsule arm incomplete: $(head -n 1 "$ARMING_MARKER" 2>/dev/null || true)"
    return 1
  fi
  if [[ ! -e $ARMED_POINTER ]]; then
    echo "recovery: no capsule armed"
    return 0
  fi
  capsule_id=$(read_armed_id)
  validate_capsule "$capsule_id"
  require_current_candidate
  service_state=$(systemctl is-active "$SANTI_RECOVERY_SERVICE" 2>/dev/null || true)
  if [[ $service_state == active ]] && curl -fsS -o /dev/null "$SANTI_RECOVERY_HEALTH"; then
    health_state="healthy"
  fi
  echo "recovery capsule: $capsule_id"
  echo "state: armed"
  echo "source: $M_SOURCE_VERSION (schema $M_SOURCE_SCHEMA)"
  echo "candidate: $M_CANDIDATE_VERSION (schema $M_CANDIDATE_SCHEMA)"
  echo "service: $service_state"
  echo "health: $health_state"
  echo "validation: OK"
}

action_accept() {
  local capsule_id=${1:-} capsule accepted_tmp
  [[ -n $capsule_id ]] || fail "accept requires a capsule id"
  require_root
  require_commands
  acquire_lock
  trap release_lock EXIT
  require_exact_armed "$capsule_id"
  capsule="$RECOVERY_ROOT/$capsule_id"
  validate_capsule "$capsule_id"
  require_current_candidate
  require_candidate_health
  accepted_tmp="$capsule/.ACCEPTED.$$"
  printf 'accepted_utc\t%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >"$accepted_tmp"
  chmod 0600 "$accepted_tmp"
  mv "$accepted_tmp" "$capsule/ACCEPTED"
  mv "$ARMED_POINTER" "$capsule/accepted-pointer"
  mv "$capsule/ARMED" "$capsule/armed-accepted"
  echo "recovery capsule accepted: $capsule_id"
}

ROLLBACK_DESTRUCTIVE=0
ROLLBACK_SUCCEEDED=0
rollback_cleanup() {
  local code=$?
  if [[ $ROLLBACK_DESTRUCTIVE == 1 && $ROLLBACK_SUCCEEDED != 1 ]]; then
    systemctl stop "$SANTI_RECOVERY_SERVICE" >/dev/null 2>&1 || true
    echo "recovery: rollback stopped after the destructive boundary; service remains stopped" >&2
    echo "recovery: candidate runtime and both packages remain in the capsule" >&2
  fi
  release_lock
  return "$code"
}

action_execute() {
  local capsule_id=${1:-} confirm_flag=${2:-} confirmed_version=${3:-}
  local capsule stage current_runtime candidate_runtime restored_memory current health_ok rolled_tmp
  local runtime_owner recovery_owner
  [[ -n $capsule_id && $confirm_flag == --confirm && -n $confirmed_version ]] ||
    fail "execute requires <capsule-id> --confirm <candidate-version>"
  require_root
  require_commands
  acquire_lock
  trap rollback_cleanup EXIT
  require_exact_armed "$capsule_id"
  capsule="$RECOVERY_ROOT/$capsule_id"
  validate_capsule "$capsule_id"
  [[ $confirmed_version == "$M_CANDIDATE_VERSION" ]] ||
    fail "confirmation mismatch: expected=$M_CANDIDATE_VERSION actual=$confirmed_version"
  require_current_candidate
  [[ -f $capsule/ARMED && ! -L $capsule/ARMED ]] || fail "capsule is not in ARMED state"
  candidate_runtime="$capsule/candidate-runtime"
  [[ ! -e $candidate_runtime ]] || fail "candidate runtime quarantine already exists"
  stage="$RECOVERY_ROOT/.rollback-${capsule_id}.$$"
  extract_archive "$capsule/runtime.tar.gz" "$stage"
  [[ $(schema_for_runtime "$stage/runtime") == "$M_SOURCE_SCHEMA" ]] ||
    fail "staged source schema differs from capsule"
  restored_memory=$(sha256_file "$stage/runtime/$MEMORY_RELATIVE")
  [[ $restored_memory == "$M_MEMORY_SHA256" ]] || fail "staged source memory differs from capsule"
  runtime_owner=$(stat -c '%u:%g' "$RUNTIME_ROOT")
  recovery_owner=$(stat -c '%u:%g' "$RECOVERY_ROOT")

  echo "recovery: stopping $SANTI_RECOVERY_SERVICE"
  systemctl stop "$SANTI_RECOVERY_SERVICE"
  current=$(systemctl is-active "$SANTI_RECOVERY_SERVICE" 2>/dev/null || true)
  [[ $current != active && $current != activating ]] || fail "service did not stop"
  ROLLBACK_DESTRUCTIVE=1
  current_runtime="$RUNTIME_ROOT"
  mv "$current_runtime" "$candidate_runtime"
  mv "$stage/runtime" "$current_runtime"
  rmdir "$stage"
  chown -R "$runtime_owner" "$current_runtime"
  dpkg -i "$capsule/source.deb"
  # Santi packages intentionally chown the whole runtime home. Restore the
  # root-owned recovery boundary before bringing the source service back.
  chown -R "$recovery_owner" "$RECOVERY_ROOT"
  current=$(installed_version)
  [[ $current == "$M_SOURCE_VERSION" ]] ||
    fail "source package did not install: expected=$M_SOURCE_VERSION actual=$current"
  [[ $(schema_for_runtime "$current_runtime") == "$M_SOURCE_SCHEMA" ]] ||
    fail "restored source runtime failed storage verification"
  [[ $(sha256_file "$current_runtime/$MEMORY_RELATIVE") == "$M_MEMORY_SHA256" ]] ||
    fail "restored source memory failed verification"

  systemctl start "$SANTI_RECOVERY_SERVICE"
  health_ok=0
  for _ in $(seq 1 30); do
    if curl -fsS -o /dev/null "$SANTI_RECOVERY_HEALTH"; then
      health_ok=1
      break
    fi
    sleep 2
  done
  [[ $health_ok == 1 ]] || fail "restored source service did not become healthy"
  [[ $(systemctl is-active "$SANTI_RECOVERY_SERVICE" 2>/dev/null || true) == active ]] ||
    fail "restored source service is not active"

  rolled_tmp="$capsule/.ROLLED_BACK.$$"
  {
    printf 'rolled_back_utc\t%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf 'from_version\t%s\n' "$M_CANDIDATE_VERSION"
    printf 'to_version\t%s\n' "$M_SOURCE_VERSION"
  } >"$rolled_tmp"
  chmod 0600 "$rolled_tmp"
  mv "$rolled_tmp" "$capsule/ROLLED_BACK"
  mv "$ARMED_POINTER" "$capsule/rolled-back-pointer"
  mv "$capsule/ARMED" "$capsule/armed-rolled-back"
  ROLLBACK_SUCCEEDED=1
  echo "recovery rollback complete: $M_CANDIDATE_VERSION -> $M_SOURCE_VERSION"
  echo "candidate runtime preserved: $candidate_runtime"
}

case "$ACTION" in
  guard-deploy)
    action_guard_deploy
    ;;
  arm)
    action_arm
    ;;
  status)
    action_status
    ;;
  accept)
    shift
    action_accept "$@"
    ;;
  execute)
    shift
    action_execute "$@"
    ;;
  *)
    fail "usage: $0 guard-deploy|arm|status|accept <id>|execute <id> --confirm <version>"
    ;;
esac
