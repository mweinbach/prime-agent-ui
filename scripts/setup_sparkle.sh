#!/bin/bash
# One-time enrollment from the maintainer's Mac. The signing identity stays in Keychain.
set -euo pipefail
umask 077
repo=mweinbach/prime-agent-ui
account=${SPARKLE_KEY_ACCOUNT:-ai.primeintellect.prime-agent.desktop}
if [[ $# -gt 1 || (${1:-} != "" && ${1:-} != "--publish") || $(uname -s) != Darwin ]]; then
  echo "Usage (on macOS): scripts/setup_sparkle.sh [--publish]" >&2
  exit 2
fi
cd "$(dirname "$0")/.."
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
if [[ ${1:-} == --publish ]]; then
  commit=$(git rev-parse HEAD)
  remote=$(gh api "repos/$repo/git/ref/heads/main" --jq .object.sha)
  [[ $commit == "$remote" ]] || { echo "Update this checkout to the current GitHub main first." >&2; exit 1; }
  version=$(python3 -B -c 'import sys; from pathlib import Path; sys.path.insert(0,"scripts"); from package_desktop_app import parse_workspace_version; print(parse_workspace_version(Path(".")))')
fi
existing=$(gh variable list --limit 1000 --repo "$repo" --json name,value --jq '.[] | select(.name == "SPARKLE_PUBLIC_ED_KEY") | .value')
secrets=$(gh secret list --limit 1000 --repo "$repo" --json name --jq '.[].name')
scripts/fetch_sparkle.sh "$scratch/sdk"
keytool="$scratch/sdk/bin/generate_keys"
"$keytool" --account "$account"
public=$("$keytool" --account "$account" -p)
if [[ -n $existing && $public != "$existing" ]]; then
  echo "The repository trusts a different key. Use its Keychain account via SPARKLE_KEY_ACCOUNT." >&2
  exit 1
fi
if [[ -z $existing ]] && grep -Fxq SPARKLE_PRIVATE_ED_KEY <<< "$secrets"; then
  echo "A signing secret already exists. Restore its matching public variable before enrolling." >&2
  exit 1
fi
"$keytool" --account "$account" -x "$scratch/key"
gh variable set SPARKLE_PUBLIC_ED_KEY --repo "$repo" --body "$public"
gh secret set SPARKLE_PRIVATE_ED_KEY --repo "$repo" < "$scratch/key"
rm -f "$scratch/key"
echo "Sparkle signing is configured. Keep a secure backup of the $account Keychain entry."
if [[ ${1:-} == --publish ]]; then
  git push "https://github.com/$repo.git" "$commit:refs/tags/desktop-v$version"
fi
