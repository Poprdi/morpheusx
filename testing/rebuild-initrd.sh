#!/bin/bash
# Rebuild initramfs with proper libraries for bash
set -e

cd "$(dirname "$0")"
BASE_DIR="$(pwd)"

WORK_DIR="/tmp/morpheus-initrd-build"
INITRD_TARGET="$BASE_DIR/esp/initrds/initramfs-arch.img"
ROOTFS_DIR="$BASE_DIR/esp/rootfs"

echo "=================================="
echo "  Rebuilding Arch Linux Initramfs"
echo "=================================="
echo ""

# Check if rootfs exists
if [ ! -d "$ROOTFS_DIR" ]; then
    echo "Error: Rootfs not found at $ROOTFS_DIR"
    echo "Run ./install-arch.sh first"
    exit 1
fi

# Clean and create work directory
rm -rf "$WORK_DIR"
mkdir -p "$WORK_DIR"/{bin,sbin,lib,lib64,usr/bin,usr/sbin,usr/lib,etc,proc,sys,dev,newroot}

echo "Trying to use busybox (statically linked, no dependency issues)..."

# Check if we have a downloaded static busybox
if [ -f "/tmp/busybox-static" ]; then
    echo "✓ Found static busybox in /tmp - using it"
    cp "/tmp/busybox-static" "$WORK_DIR/bin/busybox"
    
    # Create symlinks for common commands
    for cmd in sh bash ash ls cat echo mount umount grep sed awk ps cp mv rm mkdir clear; do
        ln -sf busybox "$WORK_DIR/bin/$cmd"
    done
    
    echo "✓ Using busybox (statically linked - no libraries needed!)"
# Check if busybox is available on the system
elif command -v busybox &> /dev/null; then
    echo "✓ Found system busybox - using static binary"
    cp $(which busybox) "$WORK_DIR/bin/"
    
    # Create symlinks for common commands
    for cmd in sh bash ash ls cat echo mount umount grep sed awk ps cp mv rm mkdir clear; do
        ln -sf busybox "$WORK_DIR/bin/$cmd"
    done
    
    echo "✓ Using busybox (statically linked - no libraries needed!)"
elif [ -f "$ROOTFS_DIR/usr/bin/busybox" ]; then
    echo "✓ Found busybox in rootfs"
    cp "$ROOTFS_DIR/usr/bin/busybox" "$WORK_DIR/bin/"
    
    for cmd in sh bash ash ls cat echo mount umount grep sed awk ps cp mv rm mkdir clear; do
        ln -sf busybox "$WORK_DIR/bin/$cmd"
    done
    
    echo "✓ Using busybox from rootfs"
else
    echo "⚠ Busybox not found - falling back to bash with libraries"
    
    # Copy bash
    cp "$ROOTFS_DIR/usr/bin/bash" "$WORK_DIR/bin/" || cp "$ROOTFS_DIR/bin/bash" "$WORK_DIR/bin/"
    ln -s bash "$WORK_DIR/bin/sh"
    
    # Copy the dynamic linker from rootfs
    cp -L "$ROOTFS_DIR/lib64/ld-linux-x86-64.so.2" "$WORK_DIR/lib64/" || \
        cp -L "$ROOTFS_DIR/usr/lib64/ld-linux-x86-64.so.2" "$WORK_DIR/lib64/"
    
    # Find and copy all required libraries for bash - ALWAYS from rootfs, not host
    for lib in $(ldd "$WORK_DIR/bin/bash" 2>/dev/null | grep -oP '(/lib64|/usr/lib64)/\S+\.so[^ ]*'); do
        libname=$(basename "$lib")
        # Try various locations in rootfs
        if [ -f "$ROOTFS_DIR/lib64/$libname" ]; then
            cp -L "$ROOTFS_DIR/lib64/$libname" "$WORK_DIR/lib64/"
        elif [ -f "$ROOTFS_DIR/usr/lib64/$libname" ]; then
            cp -L "$ROOTFS_DIR/usr/lib64/$libname" "$WORK_DIR/lib64/"
        elif [ -f "$ROOTFS_DIR$lib" ]; then
            cp -L "$ROOTFS_DIR$lib" "$WORK_DIR/lib64/"
        fi
    done
    
    # Also copy some useful utilities if available
    for util in ls cat mount umount ps; do
        if [ -f "$ROOTFS_DIR/usr/bin/$util" ]; then
            cp "$ROOTFS_DIR/usr/bin/$util" "$WORK_DIR/bin/" 2>/dev/null || true
            # Copy their libs too - from rootfs only
            if [ -f "$WORK_DIR/bin/$util" ]; then
                for lib in $(ldd "$WORK_DIR/bin/$util" 2>/dev/null | grep -oP '(/lib64|/usr/lib64)/\S+\.so[^ ]*' || true); do
                    libname=$(basename "$lib")
                    if [ -f "$ROOTFS_DIR/lib64/$libname" ]; then
                        cp -L "$ROOTFS_DIR/lib64/$libname" "$WORK_DIR/lib64/" 2>/dev/null || true
                    elif [ -f "$ROOTFS_DIR/usr/lib64/$libname" ]; then
                        cp -L "$ROOTFS_DIR/usr/lib64/$libname" "$WORK_DIR/lib64/" 2>/dev/null || true
                    elif [ -f "$ROOTFS_DIR$lib" ]; then
                        cp -L "$ROOTFS_DIR$lib" "$WORK_DIR/lib64/" 2>/dev/null || true
                    fi
                done
            fi
        fi
    done
fi

# If we have busybox, create a simple init
if [ -f "$WORK_DIR/bin/busybox" ]; then
    cat > "$WORK_DIR/init" << 'EOF'
#!/bin/sh

# Mount filesystems
mount -t proc none /proc
mount -t sysfs none /sys
mount -t devtmpfs none /dev

clear

echo ""
echo "==========================================" 
echo "  Morpheus Bootloader - SUCCESS!"
echo "=========================================="
echo ""
echo "Kernel booted successfully!"
echo ""
echo "Bootloader: Morpheus v1.0.1"
echo "Protocol:   Linux EFI Boot Stub"
echo ""
echo "Kernel cmdline:"
cat /proc/cmdline
echo ""
echo "=========================================="
echo ""
echo "Busybox shell ready. Type 'exit' to halt."
echo ""

# Start shell (redirect stderr to suppress tty warning)
exec /bin/sh 2>/dev/null
EOF
    chmod +x "$WORK_DIR/init"
else
    # No busybox - use the complex workaround
    # Create library cache config
    mkdir -p "$WORK_DIR/etc"
    cat > "$WORK_DIR/etc/ld.so.conf" << 'EOF'
/lib64
/usr/lib64
EOF

    # Create a simple static init wrapper in C
    cat > "$WORK_DIR/init.c" << 'EOF'
#include <unistd.h>
#include <stdlib.h>

int main() {
    char *args[] = {"/bin/sh", "/init.sh", NULL};
    char *env[] = {"LD_LIBRARY_PATH=/lib64:/usr/lib64", "PATH=/bin:/sbin:/usr/bin:/usr/sbin", NULL};
    execve("/bin/sh", args, env);
    return 1;
}
EOF

    # Try to compile statically
    if command -v gcc &> /dev/null || command -v musl-gcc &> /dev/null; then
        echo "Compiling static init wrapper..."
        if command -v musl-gcc &> /dev/null; then
            musl-gcc -static -o "$WORK_DIR/init" "$WORK_DIR/init.c" 2>/dev/null || \
                gcc -static -o "$WORK_DIR/init" "$WORK_DIR/init.c" 2>/dev/null || \
                echo "⚠ Static compilation failed, using script workaround"
        else
            gcc -static -o "$WORK_DIR/init" "$WORK_DIR/init.c" 2>/dev/null || \
                echo "⚠ Static compilation failed, using script workaround"
        fi
        rm -f "$WORK_DIR/init.c"
    fi

    # If static init wasn't created, fall back to script with manual invocation
    if [ ! -f "$WORK_DIR/init" ] || [ ! -x "$WORK_DIR/init" ]; then
        echo "Using shell script init (may have library issues)"
        cat > "$WORK_DIR/init" << 'EOF'
#!/bin/sh
export LD_LIBRARY_PATH=/lib64:/usr/lib64
exec /init.sh
EOF
        chmod +x "$WORK_DIR/init"
    fi

    # Create the actual init logic script
    cat > "$WORK_DIR/init.sh" << 'EOF'
#!/bin/sh

# Mount essential filesystems
mount -t proc none /proc 2>/dev/null
mount -t sysfs none /sys 2>/dev/null
mount -t devtmpfs none /dev 2>/dev/null

# Clear screen
echo -e "\033c"

echo ""
echo "=========================================="
echo "  Morpheus Bootloader - SUCCESS!"
echo "=========================================="
echo ""
echo "The kernel has booted successfully!"
echo ""
echo "Bootloader: Morpheus v1.0.1"
echo "Protocol:   Linux EFI Boot Stub"
echo ""
echo "Kernel command line:"
cat /proc/cmdline 2>/dev/null || echo "(unable to read)"
echo ""
echo "=========================================="
echo ""
echo "Dropping to bash shell..."
echo ""

# Start bash
exec /bin/bash
EOF

    chmod +x "$WORK_DIR/init.sh"
fi

# Create the initramfs
echo "Packing initramfs..."
cd "$WORK_DIR"
find . -print0 | cpio --null -o --format=newc 2>/dev/null | gzip -9 > "$INITRD_TARGET"

echo ""
echo "✓ Initramfs created: $INITRD_TARGET"
echo "  Size: $(du -h "$INITRD_TARGET" | cut -f1)"
echo ""

# List what we included
if [ -L "$WORK_DIR/bin/busybox" ] || [ -f "$WORK_DIR/bin/busybox" ]; then
    echo "Contents:"
    echo "  - busybox (statically linked - no library issues!)"
    echo "  - init script with success message"
    echo "  - Essential mount points"
else
    echo "Contents:"
    echo "  - bash (with libraries)"
    echo "  - init script"
    echo "  - Essential mount points"
fi
echo ""

# Cleanup
cd "$BASE_DIR"
rm -rf "$WORK_DIR"

echo "Done! Now rebuild the bootloader:"
echo "  cd testing && ./build.sh"
echo ""
