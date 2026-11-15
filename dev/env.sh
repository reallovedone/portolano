# Source this file to get the project toolchain on PATH:
#   source dev/env.sh
# Homebrew's rustup is keg-only, so its proxies (cargo, rustc, ...) are not
# linked into /opt/homebrew/bin. This avoids editing ~/.zshrc.
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
