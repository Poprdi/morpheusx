; xHCI register definitions (eXtensible Host Controller Interface Spec 1.2)

%ifndef XHCI_REGS_INCLUDED
%define XHCI_REGS_INCLUDED

; ─── Capability Registers (offset from BAR0) ─────────────────────────────
XHCI_CAP_CAPLENGTH      equ 0x00
XHCI_CAP_HCIVERSION     equ 0x02
XHCI_CAP_HCSPARAMS1     equ 0x04
XHCI_CAP_HCSPARAMS2     equ 0x08
XHCI_CAP_HCCPARAMS1     equ 0x10
XHCI_CAP_DBOFF           equ 0x14
XHCI_CAP_RTSOFF          equ 0x18

; ─── Operational Registers (offset from op_base = BAR0 + CAPLENGTH) ──────
XHCI_OP_USBCMD          equ 0x00
XHCI_OP_USBSTS          equ 0x04
XHCI_OP_PAGESIZE        equ 0x08
XHCI_OP_CRCR_LO         equ 0x18
XHCI_OP_CRCR_HI         equ 0x1C
XHCI_OP_DCBAAP_LO       equ 0x30
XHCI_OP_DCBAAP_HI       equ 0x34
XHCI_OP_CONFIG          equ 0x38

; ─── USBCMD bits ─────────────────────────────────────────────────────────
XHCI_CMD_RS              equ (1 << 0)
XHCI_CMD_HCRST           equ (1 << 1)
XHCI_CMD_INTE            equ (1 << 2)

; ─── USBSTS bits ─────────────────────────────────────────────────────────
XHCI_STS_HCH             equ (1 << 0)
XHCI_STS_CNR             equ (1 << 11)

; ─── PORTSC (at op_base + 0x400 + port*0x10) ─────────────────────────────
XHCI_PORTSC_CCS          equ (1 << 0)
XHCI_PORTSC_PED          equ (1 << 1)
XHCI_PORTSC_PR           equ (1 << 4)
XHCI_PORTSC_PP           equ (1 << 9)
XHCI_PORTSC_PRC          equ (1 << 21)

; ─── xHCI Extended Capability IDs ────────────────────────────────────────
XHCI_EXT_CAP_LEGACY      equ 1
XHCI_EXT_CAP_PROTOCOL    equ 2

; ─── USBLEGSUP register (legacy support handoff) ─────────────────────────
XHCI_LEGSUP_BIOS_OWNED   equ (1 << 16)
XHCI_LEGSUP_OS_OWNED     equ (1 << 24)

%endif
