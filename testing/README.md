# Morpheus Testing Environment

## Quick Start

```bash
# Build the bootloader
./build.sh

# Run in QEMU
./run.sh
```

## What This Does

1. Builds Morpheus as a UEFI application
2. Copies it to a test ESP (EFI System Partition)
3. Boots QEMU with OVMF firmware
4. OVMF finds and executes Morpheus

## Expected Behavior

Right now: Black screen (bootloader just loops forever)
Next: Will display "Morpheus" text

## Troubleshooting

If QEMU doesn't start:
- Check OVMF path in run.sh
- Ensure qemu-system-x86_64 is installed
- Try with `-nographic` flag for serial console
