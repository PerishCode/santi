#!/usr/bin/env sh
set -eu

COMMAND=${1:-install}
[ $# -gt 0 ] && shift || true

CHANNEL=${SANTI_CHANNEL:-stable}
VERSION=${SANTI_VERSION:-}
PUBLIC_URL=${SANTI_RELEASES_PUBLIC_URL:-https://releases.santi.perish.uk}
INSTALL_ROOT=${SANTI_INSTALL_ROOT:-"$HOME/.local/share/santi"}
LOCAL_BIN_DIR=${SANTI_LOCAL_BIN_DIR:-"$HOME/.local/bin"}
RETAIN=${SANTI_RETAIN:-}

while [ $# -gt 0 ]; do
  case "$1" in
    --channel)
      CHANNEL=${2:-}
      [ -n "$CHANNEL" ] || { echo "--channel requires a value" >&2; exit 1; }
      shift 2
      ;;
    --channel=*)
      CHANNEL=${1#--channel=}
      shift
      ;;
    --version)
      VERSION=${2:-}
      [ -n "$VERSION" ] || { echo "--version requires a value" >&2; exit 1; }
      shift 2
      ;;
    --version=*)
      VERSION=${1#--version=}
      shift
      ;;
    --public-url)
      PUBLIC_URL=${2:-}
      [ -n "$PUBLIC_URL" ] || { echo "--public-url requires a value" >&2; exit 1; }
      shift 2
      ;;
    --public-url=*)
      PUBLIC_URL=${1#--public-url=}
      shift
      ;;
    --install-root)
      INSTALL_ROOT=${2:-}
      [ -n "$INSTALL_ROOT" ] || { echo "--install-root requires a value" >&2; exit 1; }
      shift 2
      ;;
    --install-root=*)
      INSTALL_ROOT=${1#--install-root=}
      shift
      ;;
    --bin-dir)
      LOCAL_BIN_DIR=${2:-}
      [ -n "$LOCAL_BIN_DIR" ] || { echo "--bin-dir requires a value" >&2; exit 1; }
      shift 2
      ;;
    --bin-dir=*)
      LOCAL_BIN_DIR=${1#--bin-dir=}
      shift
      ;;
    --retain)
      RETAIN=true
      shift
      ;;
    --retain=*)
      RETAIN=${1#--retain=}
      shift
      ;;
    -h|--help|help)
      cat <<'EOF'
santi manager

Usage:
  manage.sh install [--channel stable|beta] [--version vX.Y.Z] [--retain[=true|false]]
  manage.sh uninstall [--version vX.Y.Z]

Environment:
  SANTI_RELEASES_PUBLIC_URL  # default: https://releases.santi.perish.uk
  SANTI_CHANNEL
  SANTI_VERSION
  SANTI_INSTALL_ROOT
  SANTI_LOCAL_BIN_DIR
  SANTI_RETAIN
EOF
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

need_public_url() {
  PUBLIC_URL=${PUBLIC_URL%/}
}

normalize_bool() {
  case "$1" in
    true|1|yes|y|on) printf '%s' true ;;
    false|0|no|n|off) printf '%s' false ;;
    *) echo "invalid --retain value: $1" >&2; exit 1 ;;
  esac
}

normalize_version() {
  printf 'v%s' "$(printf '%s' "$1" | sed 's/^v//')"
}

platform_archive() {
  os=$(uname -s)
  arch=$(uname -m)
  case "$os:$arch" in
    Linux:x86_64|Linux:amd64) echo "santi-x86_64-unknown-linux-gnu.tar.gz" ;;
    Darwin:arm64|Darwin:aarch64) echo "santi-aarch64-apple-darwin.tar.gz" ;;
    Darwin:x86_64|Darwin:amd64) echo "santi-x86_64-apple-darwin.tar.gz" ;;
    *) echo "unsupported platform: $os $arch" >&2; exit 1 ;;
  esac
}

latest_version() {
  metadata="$1"
  sed -n 's/.*"releaseVersion"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$metadata" | head -n 1
}

old_versions() {
  current="$1"
  [ -d "$INSTALL_ROOT" ] || return 0
  for path in "$INSTALL_ROOT"/*; do
    [ -d "$path" ] || continue
    name=$(basename "$path")
    [ "$name" != "$current" ] || continue
    printf '%s\n' "$name"
  done
}

retain_old_versions() {
  old="$1"
  if [ -z "$old" ]; then
    printf '%s' true
    return
  fi
  if [ -n "$RETAIN" ]; then
    normalize_bool "$RETAIN"
    return
  fi
  if [ -t 0 ]; then
    printf 'santi: remove previously installed versions after install? [y/N] ' >&2
    IFS= read -r answer || answer=
    case "$answer" in
      y|Y|yes|YES|Yes) printf '%s' false ;;
      *) printf '%s' true ;;
    esac
    return
  fi
  echo "santi: preserving previous versions; pass --retain=false to prune after install" >&2
  printf '%s' true
}

install_santi() {
  need_public_url
  tmpdir=$(mktemp -d)
  trap 'rm -rf "$tmpdir"' EXIT INT TERM

  if [ -z "$VERSION" ]; then
    curl -fsSL "$PUBLIC_URL/$CHANNEL/latest/metadata.json" -o "$tmpdir/metadata.json"
    VERSION=$(latest_version "$tmpdir/metadata.json")
    [ -n "$VERSION" ] || { echo "failed to resolve latest santi version" >&2; exit 1; }
  fi
  VERSION=$(normalize_version "$VERSION")

  old=$(old_versions "$VERSION")
  retain=$(retain_old_versions "$old")

  archive=$(platform_archive)
  archive_url="$PUBLIC_URL/$CHANNEL/versions/$VERSION/$archive"
  curl -fsSL "$archive_url" -o "$tmpdir/$archive"
  rm -rf "$INSTALL_ROOT/$VERSION"
  mkdir -p "$INSTALL_ROOT/$VERSION" "$LOCAL_BIN_DIR"
  tar -xzf "$tmpdir/$archive" -C "$INSTALL_ROOT/$VERSION"
  chmod +x "$INSTALL_ROOT/$VERSION/santi"

  link="$LOCAL_BIN_DIR/santi"
  rm -f "$link"
  ln -s "$INSTALL_ROOT/$VERSION/santi" "$link"
  "$link" --version

  if [ "$retain" = false ]; then
    printf '%s\n' "$old" | while IFS= read -r old_version; do
      [ -n "$old_version" ] || continue
      rm -rf "$INSTALL_ROOT/$old_version"
      printf 'removed old santi %s from %s\n' "$old_version" "$INSTALL_ROOT"
    done
  fi

  printf 'installed santi to %s\n' "$link"
}

remove_empty_dir() {
  dir="$1"
  if [ -d "$dir" ]; then
    rmdir "$dir" 2>/dev/null || true
  fi
}

uninstall_santi() {
  bin_path="$LOCAL_BIN_DIR/santi"
  if [ -n "$VERSION" ]; then
    VERSION=$(normalize_version "$VERSION")
    target="$INSTALL_ROOT/$VERSION/santi"
    if [ -L "$bin_path" ]; then
      link_target=$(readlink "$bin_path" || true)
      if [ "$link_target" = "$target" ]; then
        rm -f "$bin_path"
        printf 'removed %s\n' "$bin_path"
      fi
    fi
    rm -rf "$INSTALL_ROOT/$VERSION"
    remove_empty_dir "$INSTALL_ROOT"
    printf 'removed santi %s from %s\n' "$VERSION" "$INSTALL_ROOT"
    return
  fi

  rm -f "$bin_path"
  rm -rf "$INSTALL_ROOT"
  remove_empty_dir "$LOCAL_BIN_DIR"
  printf 'removed santi from %s and %s\n' "$INSTALL_ROOT" "$bin_path"
}

case "$COMMAND" in
  install) install_santi ;;
  uninstall) uninstall_santi ;;
  *)
    echo "unknown command: $COMMAND" >&2
    exit 1
    ;;
esac
