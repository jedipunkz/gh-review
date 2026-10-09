#!/usr/bin/env bash
# Usage: next-version.sh <latest-tag|""> <major|minor|patch>
# Prints the next vMAJOR.MINOR.PATCH tag. An empty latest tag means v0.0.0.
set -euo pipefail

latest="${1:-}"
bump="${2:-patch}"

if [ -z "$latest" ]; then
  latest="v0.0.0"
fi
if ! [[ "$latest" =~ ^v([0-9]+)\.([0-9]+)\.([0-9]+)$ ]]; then
  echo "latest tag is not vMAJOR.MINOR.PATCH: $latest" >&2
  exit 1
fi
major="${BASH_REMATCH[1]}"
minor="${BASH_REMATCH[2]}"
patch="${BASH_REMATCH[3]}"

case "$bump" in
  major) major=$((major + 1)); minor=0; patch=0 ;;
  minor) minor=$((minor + 1)); patch=0 ;;
  patch) patch=$((patch + 1)) ;;
  *)
    echo "bump must be major, minor, or patch: $bump" >&2
    exit 1
    ;;
esac

echo "v${major}.${minor}.${patch}"
