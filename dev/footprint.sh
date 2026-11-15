#!/usr/bin/env bash
# Records what exists on this Mac OUTSIDE the repository that development may create:
# toolchain caches, app data of Portolano builds, Docker objects, temp files.
#
#   bash dev/footprint.sh snapshot <label>   # write .footprint/<label>.txt
#   bash dev/footprint.sh diff <a> <b>       # show what changed between two snapshots
#   bash dev/footprint.sh list               # list snapshots
#
# Read-only: it never deletes or modifies anything outside .footprint/.
# No `-e`/`pipefail` on purpose: missing folders and empty matches are normal here and
# must be recorded as such instead of aborting the snapshot.
set -u
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/.footprint"
mkdir -p "$OUT"

size_of() {
  if [ -e "$1" ]; then du -sk "$1" 2>/dev/null | cut -f1; else echo "absent"; fi
}

snapshot() {
  local label="$1" file="$OUT/$1.txt"
  {
    echo "# footprint snapshot: $label"
    echo "# taken: $(date '+%Y-%m-%d %H:%M:%S')"

    echo; echo "[sizes_kb] toolchain and package caches"
    for p in "$HOME/.rustup" "$HOME/.cargo" "$HOME/.npm" "$HOME/Library/pnpm" \
             "$HOME/Library/Caches/pnpm" "$HOME/Library/Caches/Homebrew" \
             "$HOME/.cache" "$HOME/.docker"; do
      printf '%s\t%s\n' "$(size_of "$p")" "$p"
    done

    echo; echo "[entries] top-level names in the home folder"
    ls -A "$HOME" | LC_ALL=C sort

    echo; echo "[entries] Portolano app data under ~/Library"
    for base in WebKit Caches "Application Support" HTTPStorages Preferences Logs \
                "Saved Application State" Containers; do
      find "$HOME/Library/$base" -maxdepth 1 -iname '*portolano*' 2>/dev/null \
        | while IFS= read -r d; do printf '%s\t%s\n' "$(size_of "$d")" "$d"; done
    done | LC_ALL=C sort -k2

    echo; echo "[entries] temp folders (names only)"
    find "${TMPDIR:-/tmp}" -maxdepth 1 \( -iname '*portolano*' -o -iname '*spike*' \) 2>/dev/null | LC_ALL=C sort
    find /private/tmp -maxdepth 1 \( -iname '*portolano*' -o -iname '*spike*' \) 2>/dev/null | LC_ALL=C sort
    find /private/tmp/claude-501 -maxdepth 1 -not -name 'claude-501' 2>/dev/null \
      | grep -vE 'claude-sandbox-status|cache-break-state' | LC_ALL=C sort

    echo; echo "[docker] images"
    docker images --format '{{.Repository}}:{{.Tag}}\t{{.Size}}' 2>/dev/null | LC_ALL=C sort || echo "docker unavailable"
    echo; echo "[docker] containers"
    docker ps -a --format '{{.Names}}\t{{.Image}}\t{{.Status}}' 2>/dev/null | sed -E 's/\t(Up|Exited).*$/\t\1/' | LC_ALL=C sort || true
    echo; echo "[docker] volumes"
    docker volume ls -q 2>/dev/null | LC_ALL=C sort || true
    echo; echo "[docker] networks"
    docker network ls --format '{{.Name}}' 2>/dev/null | LC_ALL=C sort || true

    echo; echo "[homebrew] installed formulae"
    brew list --formula -1 2>/dev/null | LC_ALL=C sort || echo "brew unavailable"

    echo; echo "[listening] local TCP ports opened by project processes"
    lsof -nP -iTCP -sTCP:LISTEN 2>/dev/null | awk 'NR>1 {print $1"\t"$9}' \
      | grep -E '1420|12222|12223|13389|15900|1808[12]' | LC_ALL=C sort -u || true

    echo; echo "[sizes_kb] inside the repository (ignored build output)"
    for p in "$ROOT/target" "$ROOT/ui/node_modules" "$ROOT/node_modules" "$ROOT/.tmp"; do
      printf '%s\t%s\n' "$(size_of "$p")" "$p"
    done
  } > "$file"
  echo "written: $file"
}

case "${1:-}" in
  snapshot) snapshot "${2:?label required}" ;;
  diff)     diff -u "$OUT/${2:?first label required}.txt" "$OUT/${3:?second label required}.txt" | grep -vE '^(---|\+\+\+|[ +-]# taken)' || echo "no differences" ;;
  list)     ls -1 "$OUT" 2>/dev/null | sed 's/\.txt$//' ;;
  *)        echo "usage: footprint.sh snapshot <label> | diff <a> <b> | list" >&2; exit 2 ;;
esac
