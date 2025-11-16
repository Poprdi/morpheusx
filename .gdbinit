# GDB init for debugging UEFI bootloader
# Connect to QEMU's gdbserver
target remote localhost:1234

# Set architecture to x86-64
set architecture i386:x86-64

# UEFI PE binaries don't have ELF symbols directly
# We need to find where UEFI loaded the bootloader in memory
# Typically around 0x6000000 - 0x8000000

# To find the load address:
# 1. Look at serial output for any address hints
# 2. Or disassemble around common UEFI load areas
# 3. Or use: info proc mappings (if supported)

# Once you find the base address (e.g., 0x6838000), load symbols:
# add-symbol-file target/x86_64-unknown-uefi/release/deps/morpheus_bootloader-8acf3ef690878a2e.efi 0x6838000

# Useful commands:
# info registers - show all registers
# x/10i $rip - disassemble at current location
# x/10i 0xADDRESS - disassemble at specific address  
# bt - backtrace (won't work without symbols)
# break *0xADDRESS - breakpoint at specific address
# continue - continue execution
# stepi - step one instruction
# nexti - step over one instruction

# For now, just show where we are
info registers rip rsp rbp
x/10i $rip

# Uncomment after finding load address:
# add-symbol-file target/x86_64-unknown-uefi/release/deps/morpheus_bootloader-8acf3ef690878a2e.efi 0xYOUR_BASE_ADDRESS
