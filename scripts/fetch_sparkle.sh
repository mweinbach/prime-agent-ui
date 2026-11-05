#!/bin/bash
# Fetch the official Sparkle distribution, including the native framework and signing tools.
set -euo pipefail
dest=${1:?Usage: fetch_sparkle.sh EMPTY_DIRECTORY}
mkdir -p "$dest"
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
curl --fail --location --silent --show-error --retry 3 \
  https://github.com/sparkle-project/Sparkle/releases/download/2.10.1/Sparkle-2.10.1.tar.xz \
  -o "$scratch/Sparkle.tar.xz"
(cd "$scratch" && echo '1a856d2a9edb8e628de88121d7be04cf0e2167f8641650b78fca00b14244cf27  Sparkle.tar.xz' | shasum -a 256 -c -)
tar -xJf "$scratch/Sparkle.tar.xz" -C "$dest" ./Sparkle.framework ./bin ./LICENSE
