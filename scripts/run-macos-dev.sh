#!/bin/sh
set -eu

executable=$(node "$(dirname "$0")/prepare-macos-dev.mjs" "$1")
shift
export LOMI_DEV_BUNDLE=1
exec "$executable" "$@"
