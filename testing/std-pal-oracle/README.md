# std PAL oracle + ecosystem-proof harness

Boots std binaries built against the `morpheus` Rust std PAL under QEMU and
checks serial tokens. Replaces the ad-hoc boot*.sh scripts that lived only in a
session scratchpad (and were lost once). Images are rebuilt, never committed.

Prereqs: toolchain `nightly-2026-02-22` with the morpheus std patch applied
(`../../../morpheus-rust-pal/std/morpheus-std.patch` + the `build.rs` one-liner),
`qemu-system-x86_64`, mtools, OVMF 4M split images in `/usr/share/OVMF`, nasm,
`nc`, python3. The rung crates and `net/hostclient.py` live in
`morpheusx-std-pal-baseline/scratchpad-workspace` (sibling of this repo).

```bash
K=$(git rev-parse --show-toplevel)
WS=$K/../morpheusx-std-pal-baseline/scratchpad-workspace
cd $K && scripts/ci-build-uefi.sh --release                     # kernel EFI
cargo build --release --manifest-path helix/host_tool/Cargo.toml  # inject tool
cd testing/std-pal-oracle
ln -sfn $WS ws                                                  # rung crates + hostclient
./build_rungs.sh                                                # 11 rung ELFs -> bin/
B=$PWD/bin
for r in rung2 rung3 rung4 envargs fsdir sampler; do ./mk_esp.sh $r $B/$r; done
./mk_esp.sh proc $B/proc-parent $B/proc-child:/bin/child
./mk_esp.sh net $B/netrung; ./mk_esp.sh epoll $B/epoll-echo
./mk_esp.sh mio2 $B/miopoll; ./mk_esp.sh tokionet $B/tokio-mt-gate
./run_matrix.sh                                                 # 11/11 expected; logs/*.serial.log
```

`mk_esp.sh <name> <init-elf> [elf:/dest ...]` builds a plain FAT ESP with the
kernel at `/EFI/BOOT/BOOTX64.EFI` and a 32 MiB HelixFS v3 RAM image at
`/morpheus/helix.img` (`/bin/init` = the first ELF; extra files injected at
their `/dest`). `run_rung.sh <name> <ok-regex> <fail-regex> [net]` boots it,
releases the userspace keypress gate over the QEMU monitor, waits for the token;
`net` adds an e1000 with hostfwd and drives `hostclient.py`. Net rungs flake
under host load (slirp accepts before the TCG guest does): run them alone.

## Unmodified ecosystem apps

Apps with a normal `fn main` need an `_start`: `app/crt0/crt0.asm` installs
variant-II TLS and calls the C `main` rustc emits. It is linked by the target
spec, so the app source is untouched and plain cargo works:

```bash
T=$(./app/make-target.sh)
BS="-Z build-std=std,panic_abort -Z build-std-features=compiler-builtins-mem -Z json-target-spec"
cargo +nightly-2026-02-22 install --root /tmp/pc --target $T $BS pulldown-cmark
cargo +nightly-2026-02-22 install --root /tmp/gx --target $T $BS grex
(cd app/applaunch && cargo +nightly-2026-02-22 build --release --target $T $BS)
./mk_esp.sh app app/applaunch/target/x86_64-morpheus-app.abs/release/applaunch \
  /tmp/pc/bin/pulldown-cmark:/bin/pulldown-cmark /tmp/gx/bin/grex:/bin/grex \
  $(for f in app/corpus/* ../../README.md; do echo $f:/corpus/$(basename $f); done)
./run_rung.sh app 'MORPH_APP_DONE ok=8/8' 'EXCEPTION'
```

`applaunch` is `/bin/init`: the kernel spawns init with no argv, so it plays the
shell and runs each line of `/corpus/cmds.txt`, stdout inherited to serial.

Spawn caps to know: executables ≤ 4 MiB (strip release binaries), ≤ 16 argv
entries, ≤ 127 bytes each, 256 bytes total (extra args are silently dropped).
