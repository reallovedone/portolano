#!/usr/bin/env bash
# Runs every check of the project: Rust format/lint/tests/build, UI check/tests/build, and
# the compose file validation. Stops at the first failure.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=/dev/null
source "$ROOT/dev/env.sh"
cd "$ROOT"

step() { printf '\n== %s\n' "$*"; }

step "cargo fmt --check";    cargo fmt --check
step "cargo clippy";         cargo clippy --workspace --all-targets -- -D warnings
step "cargo test";           cargo test --workspace
step "ui: pnpm check";       pnpm --dir ui check
step "ui: pnpm test";        pnpm --dir ui test
step "ui: pnpm build";       pnpm --dir ui build
step "cargo build";          cargo build --workspace
step "docker compose config"; docker compose -f dev/docker-compose.yml config --quiet
printf '\nAll checks passed.\n'
