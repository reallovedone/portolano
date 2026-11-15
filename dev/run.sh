#!/usr/bin/env bash
# Runs a root-level pnpm binary with the Rust toolchain on PATH.
# Usage: bash dev/run.sh tauri dev
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=/dev/null
source "$ROOT/dev/env.sh"
cd "$ROOT"
exec pnpm exec "$@"
