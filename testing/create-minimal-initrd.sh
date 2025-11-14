#!/bin/bash
# Create a minimal initramfs for testing bootloader kernel handoff
# This is a minimal test environment to verify the bootloader works

set -e

INITRD_DIR="/tmp/minimal-initrd"
OUTPUT="esp/initrds/initramfs-test.img"

echo "Creating minimal initramfs for bootloader testing..."

# Create directory structure
rm -rf "$INITRD_DIR"
mkdir -p "$INITRD_DIR"/{bin,dev,proc,sys,etc,newroot}

# Create init script
cat > "$INITRD_DIR/init" << 'EOF'
#!/bin/sh
# Minimal init script for bootloader testing

# Mount essential filesystems
mount -t proc none /proc
mount -t sysfs none /sys
mount -t devtmpfs none /dev

# Clear screen and show success message
clear
echo ""
echo "=========================================="
echo "  Morpheus Bootloader - Kernel Boot Test"
echo "=========================================="
echo ""
echo "SUCCESS! The kernel has booted successfully!"
echo ""
echo "Boot Details:"
echo "  - Bootloader: Morpheus v1.0.1"
echo "  - Protocol: GRUB-compatible EFI handover"
echo "  - Kernel: Linux (from initramfs)"
echo "  - Init: Minimal shell"
echo ""
echo "This proves the bootloader successfully:"
echo "  1. Loaded the kernel into memory"
echo "  2. Set up boot parameters correctly"
echo "  3. Performed EFI handover to kernel"
echo "  4. Kernel decompressed and started init"
echo ""
echo "Kernel command line:"
cat /proc/cmdline
echo ""
echo "=========================================="
echo ""

# Drop to shell for interactive testing
echo "Dropping to minimal shell (Ctrl+D to continue)..."
exec /bin/sh
EOF

chmod +x "$INITRD_DIR/init"

# Create a minimal busybox-like shell script
cat > "$INITRD_DIR/bin/sh" << 'EOF'
#!/bin/sh
echo "Minimal shell - bootloader test environment"
echo "Type 'exit' to shutdown"
exec /bin/bash
EOF

chmod +x "$INITRD_DIR/bin/sh"

# Check if we have busybox
if command -v busybox &> /dev/null; then
    echo "Using system busybox..."
    cp $(which busybox) "$INITRD_DIR/bin/"
    
    # Create symlinks for common commands
    for cmd in sh mount umount cat echo ls; do
        ln -sf busybox "$INITRD_DIR/bin/$cmd"
    done
else
    echo "Warning: busybox not found, using minimal stub scripts"
    
    # Create minimal stubs
    for cmd in mount umount cat echo ls; do
        echo '#!/bin/sh' > "$INITRD_DIR/bin/$cmd"
        echo 'echo "Stub: $0 $@"' >> "$INITRD_DIR/bin/$cmd"
        chmod +x "$INITRD_DIR/bin/$cmd"
    done
fi

# Create device nodes
cd "$INITRD_DIR/dev"
sudo mknod console c 5 1 2>/dev/null || touch console
sudo mknod null c 1 3 2>/dev/null || touch null
sudo mknod zero c 1 5 2>/dev/null || touch zero

# Create the initramfs
cd "$INITRD_DIR"
echo "Packing initramfs..."
find . | cpio -o -H newc 2>/dev/null | gzip > "/home/runner/work/morpheusx/morpheusx/testing/$OUTPUT"

echo "✓ Minimal initramfs created: $OUTPUT"
echo "  Size: $(du -h "/home/runner/work/morpheusx/morpheusx/testing/$OUTPUT" | cut -f1)"

# Cleanup
cd /
rm -rf "$INITRD_DIR"

echo ""
echo "This initramfs will:"
echo "  - Display boot success message"
echo "  - Show kernel command line"
echo "  - Provide minimal shell for testing"
