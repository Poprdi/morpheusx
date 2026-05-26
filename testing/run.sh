#!/usr/bin/env bash
# Thin shim — actual logic lives in ../setup-dev.sh
set -euo pipefail
cd "$(dirname "$0")/.."
exec ./setup-dev.sh run "$@"
