#!/usr/bin/env bash
# Build every oracle rung ELF against the morpheus std PAL (toolchain nightly-2026-02-22).
# One shared target dir so std is built once. Run from anywhere.
set -u
S=$(cd "$(dirname "$0")" && pwd)
WS=${WS:-$S/ws}
export CARGO_TARGET_DIR=$S/target
JSON=$WS/x86_64-morpheus-std-bin.json
ARGS=(--release --target "$JSON" -Z build-std=std,panic_abort -Z build-std-features=compiler-builtins-mem -Z json-target-spec)
mkdir -p "$S/bin"
fail=0
for c in rung2 rung3 rung4 envargs fsdir sampler proc/parent proc/child net epoll-echo miopoll tokio-net-mt; do
  echo "=== $c"
  ( cd "$WS/$c" && cargo +nightly-2026-02-22 build "${ARGS[@]}" ) || { echo "BUILD FAILED: $c"; fail=1; }
done
cp -f "$CARGO_TARGET_DIR"/x86_64-morpheus-std-bin/release/{rung2,rung3,rung4,envargs,fsdir,sampler,proc-parent,proc-child,netrung,epoll-echo,miopoll,tokio-mt-gate} "$S/bin/" 2>/dev/null
ls -l "$S/bin"
echo "RUNGS_BUILD_EXIT=$fail"
