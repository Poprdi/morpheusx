#!/bin/bash
# Install Arch Linux rootfs for Morpheus bootloader testing
# Downloads compressed Arch rootfs and configures bootloader

set -e

FORCE_REBUILD=0
if [[ ${1:-} == "--force" || ${1:-} == "-f" ]]; then
    FORCE_REBUILD=1
    shift || true
fi

cd "$(dirname "$0")"
BASE_DIR="$(pwd)"

ESP_DIR="$BASE_DIR/esp"
KERNELS_DIR="$ESP_DIR/kernels"
INITRD_DIR="$ESP_DIR/initrds"
ROOTFS_DIR="$ESP_DIR/rootfs"
WORK_DIR="/tmp/morpheus-arch-setup"
ROOTFS_MARKER="$ROOTFS_DIR/.arch_rootfs_ready"
BOOTSTRAP_URL="https://geo.mirror.pkgbuild.com/iso/latest/archlinux-bootstrap-x86_64.tar.zst"
BOOTSTRAP_URL_FALLBACK="https://america.mirror.pkgbuild.com/iso/latest/archlinux-bootstrap-x86_64.tar.zst"
BOOTSTRAP_FILE="$WORK_DIR/archlinux-bootstrap.tar.zst"

echo "========================================"
echo "  Morpheus Arch Linux Rootfs Setup"
echo "========================================"
echo ""
echo "This script will:"
echo "  1. Download Arch Linux bootstrap rootfs"
echo "  2. Download kernel and initramfs"
echo "  3. Configure bootloader for Arch Linux"
echo ""
read -p "Continue? [Y/n]: " -n 1 -r
echo
if [[ $REPLY =~ ^[Nn]$ ]]; then
    echo "Aborted."
    exit 0
fi

# Create directories
echo ""
echo "Creating directory structure..."
mkdir -p "$KERNELS_DIR"
mkdir -p "$INITRD_DIR"
mkdir -p "$ROOTFS_DIR"
mkdir -p "$WORK_DIR"
echo "✓ Directories created"

# Check for required tools
echo ""
echo "Checking dependencies..."
if ! command -v curl &> /dev/null; then
    echo "Installing curl..."
    sudo apt-get update -qq
    sudo apt-get install -y curl
fi
if ! command -v zstd &> /dev/null; then
    echo "Installing zstd..."
    sudo apt-get install -y zstd
fi
echo "✓ Dependencies ready"

# Download Arch Linux bootstrap
echo ""
echo "Downloading Arch Linux bootstrap rootfs..."
if [ -f "$BOOTSTRAP_FILE" ]; then
    echo "Using cached bootstrap: $BOOTSTRAP_FILE"
else
    echo "Downloading from: $BOOTSTRAP_URL"
    echo "Size: ~150 MB (compressed)"
    echo ""
    curl -fL -o "$BOOTSTRAP_FILE" "$BOOTSTRAP_URL" --progress-bar || {
        echo "✗ Download failed!"
        echo "Trying alternative mirror..."
        curl -fL -o "$BOOTSTRAP_FILE" "$BOOTSTRAP_URL_FALLBACK" --progress-bar || {
            echo "✗ All download attempts failed!"
            exit 1
        }
    }
    echo ""
    echo "✓ Downloaded bootstrap: $(du -h "$BOOTSTRAP_FILE" | cut -f1)"
fi

if ! zstd -t "$BOOTSTRAP_FILE" >/dev/null 2>&1; then
    echo "Cached bootstrap is invalid, re-downloading..."
    rm -f "$BOOTSTRAP_FILE"
    curl -fL -o "$BOOTSTRAP_FILE" "$BOOTSTRAP_URL" --progress-bar || {
        echo "✗ Download failed!"
        echo "Trying alternative mirror..."
        curl -fL -o "$BOOTSTRAP_FILE" "$BOOTSTRAP_URL_FALLBACK" --progress-bar || {
            echo "✗ All download attempts failed!"
            exit 1
        }
    }
    echo ""
    echo "✓ Downloaded bootstrap: $(du -h "$BOOTSTRAP_FILE" | cut -f1)"
    zstd -t "$BOOTSTRAP_FILE" >/dev/null 2>&1 || {
        echo "✗ Bootstrap archive still invalid"
        exit 1
    }
fi

KERNEL_TARGET="$KERNELS_DIR/vmlinuz-arch"
INITRD_TARGET="$INITRD_DIR/initramfs-arch.img"

# Extract kernel and initramfs from bootstrap if needed
echo ""
echo "Preparing kernel and initramfs..."
cd "$WORK_DIR"

need_kernel=true
need_initrd=true

if [ -f "$KERNEL_TARGET" ]; then
    echo "✓ Kernel already present: $KERNEL_TARGET ($(du -h "$KERNEL_TARGET" | cut -f1))"
    need_kernel=false
fi

if [ -f "$INITRD_TARGET" ]; then
    echo "✓ Initramfs already present: $INITRD_TARGET ($(du -h "$INITRD_TARGET" | cut -f1))"
    need_initrd=false
fi

if $need_kernel || $need_initrd; then
    if ! zstd -d "$BOOTSTRAP_FILE" -c | tar -x -f - root.x86_64/boot/vmlinuz-linux root.x86_64/boot/initramfs-linux.img 2>/dev/null; then
        echo "Note: Boot files not in bootstrap, will download separately"
    fi
fi

# Copy kernel if extracted
if $need_kernel && [ -f "$WORK_DIR/root.x86_64/boot/vmlinuz-linux" ]; then
    cp "$WORK_DIR/root.x86_64/boot/vmlinuz-linux" "$KERNEL_TARGET"
    chmod 644 "$KERNEL_TARGET"
    echo "✓ Kernel: $KERNEL_TARGET ($(du -h "$KERNEL_TARGET" | cut -f1))"
elif $need_kernel; then
    # Download kernel from Arch package repository
    echo "Downloading kernel package..."
    KERNEL_URL="https://geo.mirror.pkgbuild.com/core/os/x86_64/"
    KERNEL_PKG=$(curl -s "$KERNEL_URL" | grep -oP 'linux-[0-9]+\.[0-9]+\.[0-9]+(?:\.[0-9]+)?(?:\.arch\d+)?-[0-9]+-x86_64\.pkg\.tar\.zst' | head -1)
    
    if [ -z "$KERNEL_PKG" ]; then
        echo "✗ Could not find kernel package"
        exit 1
    fi
    
    curl -fL -o kernel.pkg.tar.zst "$KERNEL_URL$KERNEL_PKG" --progress-bar

    if ! zstd -t kernel.pkg.tar.zst >/dev/null 2>&1; then
        echo "Kernel package corrupted, re-downloading..."
        rm -f kernel.pkg.tar.zst
        curl -fL -o kernel.pkg.tar.zst "$KERNEL_URL$KERNEL_PKG" --progress-bar || {
            echo "✗ Failed to download kernel package"
            exit 1
        }
        zstd -t kernel.pkg.tar.zst >/dev/null 2>&1 || {
            echo "✗ Kernel package still invalid"
            exit 1
        }
    fi
    
    # Extract kernel from package
    KERNEL_STAGE_DIR="$WORK_DIR/kernel_stage"
    rm -rf "$KERNEL_STAGE_DIR"
    mkdir -p "$KERNEL_STAGE_DIR"

    if zstd -d kernel.pkg.tar.zst -c | tar --wildcards -x -f - -C "$KERNEL_STAGE_DIR" 'usr/lib/modules/*/vmlinuz' 'usr/lib/modules/*/initcpio/*' 2>/dev/null; then
        KERNEL_SRC=$(find "$KERNEL_STAGE_DIR" -name vmlinuz -print -quit)
    else
        echo "✗ Failed to extract kernel from package"
        exit 1
    fi

    if [ -z "$KERNEL_SRC" ] || [ ! -f "$KERNEL_SRC" ]; then
        echo "✗ Kernel binary not found in package"
        exit 1
    fi

    cp "$KERNEL_SRC" "$KERNEL_TARGET"
    chmod 644 "$KERNEL_TARGET"
    
    # Also grab initramfs if present in the package
    INITRD_SRC=$(find "$KERNEL_STAGE_DIR" -path "*/initcpio/initramfs-linux.img" -print -quit)
    if [ -n "$INITRD_SRC" ] && [ -f "$INITRD_SRC" ]; then
        cp "$INITRD_SRC" "$INITRD_TARGET"
        chmod 644 "$INITRD_TARGET"
        echo "✓ Kernel + Initramfs extracted from package"
        need_initrd=false
    fi
    
    rm -rf "$KERNEL_STAGE_DIR"
    echo "✓ Kernel: $KERNEL_TARGET ($(du -h "$KERNEL_TARGET" | cut -f1))"
fi

# Copy initramfs if extracted
if $need_initrd && [ -f "$WORK_DIR/root.x86_64/boot/initramfs-linux.img" ]; then
    cp "$WORK_DIR/root.x86_64/boot/initramfs-linux.img" "$INITRD_TARGET"
    chmod 644 "$INITRD_TARGET"
    echo "✓ Initramfs: $INITRD_TARGET ($(du -h "$INITRD_TARGET" | cut -f1))"
elif $need_initrd; then
    # Create a minimal initramfs with just a shell
    echo "Creating minimal initramfs..."
    INITRD_BUILD="$WORK_DIR/initrd_build"
    rm -rf "$INITRD_BUILD"
    mkdir -p "$INITRD_BUILD"/{bin,sbin,etc,proc,sys,newroot,usr/bin,usr/sbin}
    
    # Copy essential binaries from the rootfs
    if [ -f "$ROOTFS_DIR/usr/bin/bash" ]; then
        cp "$ROOTFS_DIR/usr/bin/bash" "$INITRD_BUILD/bin/"
        cp "$ROOTFS_DIR/usr/bin/sh" "$INITRD_BUILD/bin/" 2>/dev/null || ln -s bash "$INITRD_BUILD/bin/sh"
    fi
    
    # Create init script
    cat > "$INITRD_BUILD/init" << 'INIT_EOF'
#!/bin/sh
mount -t proc none /proc
mount -t sysfs none /sys
echo "Minimal initramfs loaded"
echo "Dropping to shell..."
exec /bin/sh
INIT_EOF
    chmod +x "$INITRD_BUILD/init"
    
    # Create initramfs
    cd "$INITRD_BUILD"
    find . | cpio -o -H newc 2>/dev/null | gzip > "$INITRD_TARGET"
    cd "$WORK_DIR"
    rm -rf "$INITRD_BUILD"
    
    echo "✓ Minimal initramfs created: $INITRD_TARGET ($(du -h "$INITRD_TARGET" | cut -f1))"
fi

# Extract rootfs to ESP
echo ""
if [[ $FORCE_REBUILD -eq 0 && -f "$ROOTFS_MARKER" ]]; then
    echo "Rootfs already prepared at $ROOTFS_DIR (use --force to rebuild)."
else
    echo "Extracting Arch Linux rootfs..."
    cd "$ROOTFS_DIR"
    if ! zstd -d "$BOOTSTRAP_FILE" -c | sudo tar -x -f - --strip-components=1; then
        echo "✗ Failed to extract rootfs"
        exit 1
    fi
    sudo chown -R $(id -un):$(id -gn) "$ROOTFS_DIR"
    chmod -R u+rwX,go+rX "$ROOTFS_DIR"

    echo "Pruning optional documentation/locales to keep ESP size reasonable..."
    rm -rf \
        "$ROOTFS_DIR/usr/share/man" \
        "$ROOTFS_DIR/usr/share/info" \
        "$ROOTFS_DIR/usr/share/locale" \
        "$ROOTFS_DIR/usr/share/i18n" \
        "$ROOTFS_DIR/usr/share/doc" \
        "$ROOTFS_DIR/usr/share/licenses" \
        "$ROOTFS_DIR/usr/share/misc" \
        "$ROOTFS_DIR/usr/share/terminfo" \
        "$ROOTFS_DIR/usr/share/bash-completion" \
        "$ROOTFS_DIR/usr/share/zsh" \
        "$ROOTFS_DIR/usr/share/fonts" \
        "$ROOTFS_DIR/usr/share/pixmaps" \
        "$ROOTFS_DIR/usr/share/applications" \
        "$ROOTFS_DIR/usr/share/gtk-doc" \
        "$ROOTFS_DIR/usr/share/gir-1.0" \
        "$ROOTFS_DIR/usr/share/gobject-introspection-1.0" \
        "$ROOTFS_DIR/usr/share/pkgconfig" \
        "$ROOTFS_DIR/usr/share/vala" \
        "$ROOTFS_DIR/usr/include"

    echo "Stripping toolchains and runtimes we don't need..."
    rm -rf \
        "$ROOTFS_DIR/usr/lib/systemd" \
        "$ROOTFS_DIR/usr/lib/udev" \
        "$ROOTFS_DIR/usr/lib/python3.13" \
        "$ROOTFS_DIR/usr/lib/gnupg" \
        "$ROOTFS_DIR/usr/lib/gcc" \
        "$ROOTFS_DIR/usr/libexec/gcc" \
        "$ROOTFS_DIR/usr/lib/security" \
        "$ROOTFS_DIR/usr/lib/xtables" \
        "$ROOTFS_DIR/usr/lib/pkcs11" \
        "$ROOTFS_DIR/usr/lib/p11-kit" \
        "$ROOTFS_DIR/usr/lib/gconv"

    rm -f \
        "$ROOTFS_DIR/usr/bin/go" \
        "$ROOTFS_DIR/usr/bin/gcc" \
        "$ROOTFS_DIR/usr/bin/g++" \
        "$ROOTFS_DIR/usr/bin/gfortran" \
        "$ROOTFS_DIR/usr/bin/ld.gold"

    PRUNE_LIB_PATTERNS=(
        "libasan.so*"
        "libtsan.so*"
        "libubsan.so*"
        "liblsan.so*"
        "libgfortran.so*"
        "libgphobos.so*"
        "libgo.so*"
        "libquadmath.so*"
        "libstdc++.so*"
        "libgomp.so*"
        "libitm.so*"
        "libgdruntime.so*"
        "libicudata.so*"
        "libicuuc.so*"
        "libicui18n.so*"
        "libicuio.so*"
        "libicutu.so*"
        "libicutest.so*"
        "libleancrypto*.so*"
        "libp11-kit.so*"
    )

    for pattern in "${PRUNE_LIB_PATTERNS[@]}"; do
        find "$ROOTFS_DIR/usr/lib" -maxdepth 1 -name "$pattern" -print -exec rm -f {} + 2>/dev/null || true
    done

    find "$ROOTFS_DIR/usr/lib" -type f \( -name '*.a' -o -name '*.o' \) -print -delete
    echo "✓ Rootfs extracted to $ROOTFS_DIR"

    # Create boot configuration
    cd "$(dirname "$0")"
    cat > "$ROOTFS_DIR/boot-config.txt" << 'EOF'
Arch Linux Boot Configuration
==============================

Kernel: /kernels/vmlinuz-arch
Initramfs: /initrds/initramfs-arch.img
Rootfs: /rootfs/ (Arch Linux bootstrap)

Recommended kernel cmdline:
  root=/dev/ram0 rw console=ttyS0,115200 debug init=/usr/bin/bash

Alternative (if rootfs on disk):
  root=/dev/sda2 rw console=ttyS0,115200

For shell access:
  init=/usr/bin/bash (drops to bash shell)
  
For normal boot:
  init=/usr/lib/systemd/systemd (if systemd is installed)
EOF

    # Create a README
    cat > README-ARCH.md << 'EOF'
# Arch Linux Boot Setup

## Files
- **Kernel**: `esp/kernels/vmlinuz-arch`
- **Initramfs**: `esp/initrds/initramfs-arch.img`
- **Rootfs**: `esp/rootfs/` (extracted Arch bootstrap)

## Boot Configuration

The bootloader needs to be updated to reference the Arch kernel and initramfs.

Edit `bootloader/src/tui/distro_launcher.rs` and add:

```rust
KernelEntry {
    name: String::from("Arch Linux"),
    path: String::from("\\kernels\\vmlinuz-arch"),
    cmdline: String::from("root=/dev/ram0 rw console=ttyS0,115200 debug init=/usr/bin/bash"),
    initrd: Some(String::from("\\initrds\\initramfs-arch.img")),
},
```

## Building & Testing

1. Update the bootloader source (see above)
2. Rebuild: `./testing/build.sh`
3. Run in QEMU: `./testing/run.sh`
4. Select "Arch Linux" from the menu

## Expected Behavior

The kernel will boot with the initramfs, then drop to a bash shell.
You'll see kernel messages on the serial console (QEMU output).

## Notes

- The rootfs is currently in `esp/rootfs/` but not automatically mounted
- For a full boot, you'd need to create a disk image and install Arch there
- The initramfs provides a minimal environment
- Use `console=ttyS0,115200` to see output in QEMU's serial console
EOF

    touch "$ROOTFS_MARKER"
fi

echo ""
echo "========================================"
echo "  Arch Linux Setup Complete!"
echo "========================================"
echo ""
echo "Files installed:"
echo "  Kernel:    $(cd "$KERNELS_DIR" && pwd)/vmlinuz-arch"
echo "  Initramfs: $(cd "$INITRD_DIR" && pwd)/initramfs-arch.img"
echo "  Rootfs:    $(cd "$ROOTFS_DIR" && pwd)/"
echo ""
echo "Next steps:"
echo ""
echo "1. Update bootloader configuration:"
echo "   Edit: bootloader/src/tui/distro_launcher.rs"
echo "   Add Arch Linux entry (see README-ARCH.md)"
echo ""
echo "2. Rebuild bootloader:"
echo "   ./testing/build.sh"
echo ""
echo "3. Test in QEMU:"
echo "   ./testing/run.sh"
echo ""
echo "See README-ARCH.md for detailed instructions."
echo ""

