#!/usr/bin/env bash
# Leak check for OpenG7250. Fails when a tracked (or staged) file breaks
# SECURITY.md: vendor binaries outside the release package, 64-hex strings
# outside the allow list, or local home-folder paths.
#
# Usage: tools/leak-check.sh            check all tracked files
#        tools/leak-check.sh --staged   check staged files only (pre-commit)
set -euo pipefail

root=$(git rev-parse --show-toplevel)
cd "$root"
allow="tools/leak-allow.txt"

files=()
if [[ "${1:-}" == "--staged" ]]; then
  list_cmd=(git diff --cached --name-only --diff-filter=ACMR)
else
  list_cmd=(git ls-files)
fi
while IFS= read -r line; do files+=("$line"); done < <("${list_cmd[@]}")

allowed() {  # $1 = path; true when listed in the allow list
  [[ -f "$allow" ]] && grep -v '^#' "$allow" | grep -qxF -- "$1"
}

fail=0
for f in ${files[@]+"${files[@]}"}; do
  [[ -f "$f" ]] || continue
  case "$f" in
    re/analysis/firmware/*) continue ;;   # public, encrypted release package
  esac
  case "$f" in
    *.bin|*.bbf|*.ctl|*.elf|*.hex|*.dfu|*.img)
      echo "leak-check: vendor or firmware binary: $f"; fail=1; continue ;;
  esac
  grep -Iq . "$f" 2>/dev/null || continue  # skip other binary files
  if grep -nE '[0-9a-fA-F]{64}' "$f" >/dev/null && ! allowed "$f"; then
    echo "leak-check: 64-hex string (hash or key?) in $f"; fail=1
  fi
  if grep -nE '/(Users|home)/[A-Za-z0-9._-]+/' "$f" >/dev/null && ! allowed "$f"; then
    echo "leak-check: local home path in $f"; fail=1
  fi
done

if [[ $fail -ne 0 ]]; then
  echo "leak-check: FAILED (see SECURITY.md; exceptions go in $allow with a reason)"
  exit 1
fi
echo "leak-check: OK"
