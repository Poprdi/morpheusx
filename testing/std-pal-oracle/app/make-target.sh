#!/usr/bin/env bash
# Emit an absolute-path target spec (rustc json specs can't be relative) and
# assemble the crt0 it links. Prints the spec path; pass it to --target.
set -euo pipefail
D=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$D/../../.." && pwd)
nasm -f elf64 -o "$D/crt0/crt0.o" "$D/crt0/crt0.asm"
sed "s|@REPO@|$REPO|g" "$D/x86_64-morpheus-app.json" > "$D/x86_64-morpheus-app.abs.json"
echo "$D/x86_64-morpheus-app.abs.json"
