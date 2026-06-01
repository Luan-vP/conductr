#!/usr/bin/env bash
# Install the conductr CLI from source.
#
# Usage:
#   ./install.sh [--no-skills] [--force-skills]    # from a local checkout
#   curl -fsSL https://raw.githubusercontent.com/Luan-vP/conductr/develop/install.sh | bash -s -- [--no-skills] [--force-skills]
#
# Flags:
#   --no-skills     Skip symlinking skills into ~/.claude/skills/
#   --force-skills  Overwrite conflicting symlinks (never overwrites real directories)
#
# Env overrides:
#   CONDUCTR_REF      git ref to install when cloning (default: develop)
#   CONDUCTR_PROFILE  cargo profile (default: release)

set -euo pipefail

REPO_URL="https://github.com/Luan-vP/conductr.git"
REF="${CONDUCTR_REF:-develop}"
PROFILE="${CONDUCTR_PROFILE:-release}"
NO_SKILLS=0
FORCE_SKILLS=0

if [[ -t 1 ]]; then
  BOLD=$'\033[1m'; DIM=$'\033[2m'; RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; RESET=$'\033[0m'
else
  BOLD=''; DIM=''; RED=''; GREEN=''; YELLOW=''; RESET=''
fi

say()  { printf '%s%s%s\n' "$BOLD" "$*" "$RESET"; }
warn() { printf '%s%s%s\n' "$YELLOW" "$*" "$RESET" >&2; }
die()  { printf '%s%s%s\n' "$RED"   "$*" "$RESET" >&2; exit 1; }

require() {
  command -v "$1" >/dev/null 2>&1 || die "missing required tool: $1${2:+ ($2)}"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-skills)    NO_SKILLS=1;    shift ;;
    --force-skills) FORCE_SKILLS=1; shift ;;
    *)              die "unknown argument: $1" ;;
  esac
done

require git   "install via your package manager or https://git-scm.com"
require cargo "install Rust via https://rustup.rs"

if [[ -f "Cargo.toml" && -d "crates/conductr" ]]; then
  SOURCE_DIR="$PWD"
  say "Installing conductr from local checkout: ${DIM}${SOURCE_DIR}${RESET}"
else
  TMPDIR="$(mktemp -d -t conductr-install.XXXXXX)"
  trap 'rm -rf "$TMPDIR"' EXIT
  SOURCE_DIR="$TMPDIR/conductr"
  say "Cloning ${REPO_URL} (${REF}) into ${DIM}${SOURCE_DIR}${RESET}"
  git clone --depth=1 --branch "$REF" --recurse-submodules --shallow-submodules \
    "$REPO_URL" "$SOURCE_DIR"
fi

say "Building and installing (profile: ${PROFILE})"
(
  cd "$SOURCE_DIR"
  cargo install --path crates/conductr --profile "$PROFILE" --locked --force
)

CARGO_BIN="${CARGO_HOME:-$HOME/.cargo}/bin"
case ":$PATH:" in
  *":$CARGO_BIN:"*) ;;
  *) warn "warning: ${CARGO_BIN} is not on your PATH; add it to your shell profile" ;;
esac

if command -v conductr >/dev/null 2>&1; then
  printf '%s%s installed:%s %s\n' "$GREEN" "✓ conductr" "$RESET" "$(command -v conductr)"
  conductr --version 2>/dev/null || true
else
  warn "conductr binary installed to ${CARGO_BIN}/conductr but not visible on PATH yet"
fi

# ── Skill symlinks ─────────────────────────────────────────────────────────────

link_skills() {
  local skills_dir="${SOURCE_DIR}/skills"
  local claude_dir="${HOME}/.claude"
  local claude_skills="${claude_dir}/skills"
  local linked=0 skipped=0 warned=0

  if [[ ! -d "$skills_dir" ]]; then
    warn "skills/ directory not found in ${SOURCE_DIR}; skipping skill symlinks"
    return 0
  fi

  if [[ ! -d "$claude_dir" ]]; then
    warn ""
    warn "warning: ~/.claude does not exist — is Claude Code installed?"
    warn "         Skill symlinks skipped. Re-run install.sh after installing Claude Code."
    return 0
  fi

  mkdir -p "$claude_skills"

  for skill_dir in "$skills_dir"/*/; do
    [[ -f "${skill_dir}SKILL.md" ]] || continue
    local name; name="$(basename "$skill_dir")"
    local abs_skill_dir; abs_skill_dir="$(cd "$skill_dir" && pwd)"
    local link_path="${claude_skills}/${name}"

    if [[ -L "$link_path" ]]; then
      local current_target; current_target="$(readlink "$link_path")"
      local abs_current
      if [[ "${current_target}" == /* ]]; then
        abs_current="$current_target"
      else
        abs_current="$(cd "$(dirname "$link_path")/${current_target}" 2>/dev/null && pwd)" || abs_current=""
      fi
      if [[ "$abs_current" == "$abs_skill_dir" ]]; then
        printf '  · %s — already linked\n' "$name"
        skipped=$(( skipped + 1 ))
      elif [[ "$FORCE_SKILLS" -eq 1 ]]; then
        rm "$link_path"
        ln -s "$abs_skill_dir" "$link_path"
        printf '  %s✓%s %s — re-linked (replaced: %s)\n' "$GREEN" "$RESET" "$name" "$current_target"
        linked=$(( linked + 1 ))
      else
        warn "  warning: ~/.claude/skills/${name} → ${current_target} (conflict; use --force-skills to override)"
        warned=$(( warned + 1 ))
      fi
    elif [[ -e "$link_path" ]]; then
      warn "  warning: ~/.claude/skills/${name} is a real directory; refusing to overwrite"
      warned=$(( warned + 1 ))
    else
      ln -s "$abs_skill_dir" "$link_path"
      printf '  %s✓%s %s — linked\n' "$GREEN" "$RESET" "$name"
      linked=$(( linked + 1 ))
    fi
  done

  printf '\n%sSkill links:%s %d linked, %d skipped (already correct), %d warned (conflict)\n' \
    "$BOLD" "$RESET" "$linked" "$skipped" "$warned"
}

if [[ "$NO_SKILLS" -eq 1 ]]; then
  say "Skipping skill symlinks (--no-skills)"
else
  say "Linking skills into ~/.claude/skills/"
  link_skills
fi

cat <<EOF

Next steps:
  conductr --help
  conductr setup wizard
EOF
