#!/usr/bin/env bash
# Create a product-side work tree that cannot see re/.
#
# Usage: tools/product-clone.sh <target-dir> <cleared-his-commit> [repo-url] [branch]
#
# Result:
#   <target-dir>        sparse, blobless clone with product/ and testbench/ only
#   <target-dir>-his    read-only work tree with his/ only, at the cleared commit
#
# Product agents work only in these two trees and never inspect re/ through
# git history (git show, git log -p and similar on re/ paths).
set -euo pipefail

target=${1:?target dir}
his_commit=${2:?cleared his commit (see the latest gate review)}
url=${3:-https://github.com/Guru-RF/OpenG7250.git}
branch=${4:-main}

git clone --filter=blob:none --sparse --branch "$branch" "$url" "$target"
git -C "$target" sparse-checkout set --cone product testbench

git -C "$target" fetch origin "$his_commit" || true
git -C "$target" worktree add --no-checkout --detach "$target-his" "$his_commit"
git -C "$target-his" sparse-checkout set --cone his
git -C "$target-his" checkout --detach "$his_commit"
chmod -R a-w "$target-his/his"

echo "product tree: $target"
echo "cleared HIS:  $target-his (read-only, at $his_commit)"
