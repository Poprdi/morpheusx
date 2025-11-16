#!/bin/bash
# Quick UEFI debugging helper

echo "=== UEFI Bootloader Debug Helper ==="
echo ""
echo "IMPORTANT: Start QEMU FIRST in another terminal!"
echo "  Run: ./testing/run.sh"
echo ""
echo "Then this script will connect GDB to QEMU's debug server."
echo ""
echo "GDB Commands for UEFI debugging:"
echo "  c or continue    - Let it run until hang/breakpoint"
echo "  Ctrl+C           - Break when it hangs"
echo "  x/10i \$rip       - See current instructions"
echo "  si or stepi      - Step one instruction"
echo "  info registers   - Show all registers"
echo ""
read -p "Press Enter when QEMU is running..."

cargo build --release --target x86_64-unknown-uefi
gdb -ex "target remote localhost:1234" \
    -ex "set architecture i386:x86-64" \
    -ex "info registers rip" \
    -ex "x/10i \$rip"
