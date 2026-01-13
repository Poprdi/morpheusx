; ═══════════════════════════════════════════════════════════════════════════
; AHCI Register Definitions
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Intel Wildcat Point-LP SATA Controller [AHCI Mode]
; PCI Vendor: 0x8086, Device: 0x9C83, Class: 0x0106
;
; AHCI is memory-mapped - all registers accessed via MMIO from BAR5
;
; Reference: AHCI 1.3.1 Specification, Intel PCH Datasheet
; ═══════════════════════════════════════════════════════════════════════════

section .data

; ═══════════════════════════════════════════════════════════════════════════
; AHCI Generic Host Control (HBA) Registers - Offset from ABAR
; ═══════════════════════════════════════════════════════════════════════════

; Host Capability Register
AHCI_HBA_CAP            equ 0x00    ; RO - Host Capabilities
AHCI_HBA_GHC            equ 0x04    ; RW - Global Host Control
AHCI_HBA_IS             equ 0x08    ; RWC - Interrupt Status
AHCI_HBA_PI             equ 0x0C    ; RO - Ports Implemented
AHCI_HBA_VS             equ 0x10    ; RO - AHCI Version
AHCI_HBA_CCC_CTL        equ 0x14    ; RW - Command Completion Coalescing Control
AHCI_HBA_CCC_PORTS      equ 0x18    ; RW - CCC Ports
AHCI_HBA_EM_LOC         equ 0x1C    ; RO - Enclosure Management Location
AHCI_HBA_EM_CTL         equ 0x20    ; RW - Enclosure Management Control
AHCI_HBA_CAP2           equ 0x24    ; RO - Extended Capabilities
AHCI_HBA_BOHC           equ 0x28    ; RW - BIOS/OS Handoff Control & Status

; ═══════════════════════════════════════════════════════════════════════════
; Per-Port Registers - Offset from ABAR + 0x100 + (port * 0x80)
; ═══════════════════════════════════════════════════════════════════════════

AHCI_PORT_BASE          equ 0x100   ; Base offset for port 0
AHCI_PORT_SIZE          equ 0x80    ; Size of each port register block

; Port registers (add to port base)
AHCI_PxCLB              equ 0x00    ; RW - Command List Base Address (low)
AHCI_PxCLBU             equ 0x04    ; RW - Command List Base Address (high)
AHCI_PxFB               equ 0x08    ; RW - FIS Base Address (low)
AHCI_PxFBU              equ 0x0C    ; RW - FIS Base Address (high)
AHCI_PxIS               equ 0x10    ; RWC - Interrupt Status
AHCI_PxIE               equ 0x14    ; RW - Interrupt Enable
AHCI_PxCMD              equ 0x18    ; RW - Command and Status
AHCI_PxRSV              equ 0x1C    ; Reserved
AHCI_PxTFD              equ 0x20    ; RO - Task File Data
AHCI_PxSIG              equ 0x24    ; RO - Signature
AHCI_PxSSTS             equ 0x28    ; RO - SATA Status (SCR0: SStatus)
AHCI_PxSCTL             equ 0x2C    ; RW - SATA Control (SCR2: SControl)
AHCI_PxSERR             equ 0x30    ; RWC - SATA Error (SCR1: SError)
AHCI_PxSACT             equ 0x34    ; RW - SATA Active (NCQ)
AHCI_PxCI               equ 0x38    ; RW - Command Issue
AHCI_PxSNTF             equ 0x3C    ; RWC - SATA Notification
AHCI_PxFBS              equ 0x40    ; RW - FIS-based Switching Control
AHCI_PxDEVSLP           equ 0x44    ; RW - Device Sleep
AHCI_PxVS               equ 0x70    ; RW - Vendor Specific (4 DWORDs)

; ═══════════════════════════════════════════════════════════════════════════
; HBA Capability Register (CAP) Bits
; ═══════════════════════════════════════════════════════════════════════════

AHCI_CAP_NP_MASK        equ 0x1F        ; Number of Ports - 1
AHCI_CAP_SXS            equ (1 << 5)    ; Supports External SATA
AHCI_CAP_EMS            equ (1 << 6)    ; Enclosure Management
AHCI_CAP_CCCS           equ (1 << 7)    ; Command Completion Coalescing
AHCI_CAP_NCS_MASK       equ (0x1F << 8) ; Number of Command Slots - 1
AHCI_CAP_NCS_SHIFT      equ 8
AHCI_CAP_PSC            equ (1 << 13)   ; Partial State Capable
AHCI_CAP_SSC            equ (1 << 14)   ; Slumber State Capable
AHCI_CAP_PMD            equ (1 << 15)   ; PIO Multiple DRQ Block
AHCI_CAP_FBSS           equ (1 << 16)   ; FIS-based Switching
AHCI_CAP_SPM            equ (1 << 17)   ; Port Multiplier
AHCI_CAP_SAM            equ (1 << 18)   ; AHCI Mode Only
AHCI_CAP_ISS_MASK       equ (0xF << 20) ; Interface Speed Support
AHCI_CAP_ISS_SHIFT      equ 20
AHCI_CAP_SCLO           equ (1 << 24)   ; Command List Override
AHCI_CAP_SAL            equ (1 << 25)   ; Activity LED
AHCI_CAP_SALP           equ (1 << 26)   ; Aggressive Link Power
AHCI_CAP_SSS            equ (1 << 27)   ; Staggered Spin-up
AHCI_CAP_SMPS           equ (1 << 28)   ; Mechanical Presence Switch
AHCI_CAP_SSNTF          equ (1 << 29)   ; SNotification
AHCI_CAP_SNCQ           equ (1 << 30)   ; Native Command Queuing
AHCI_CAP_S64A           equ (1 << 31)   ; 64-bit Addressing

; ═══════════════════════════════════════════════════════════════════════════
; Global Host Control (GHC) Bits
; ═══════════════════════════════════════════════════════════════════════════

AHCI_GHC_HR             equ (1 << 0)    ; HBA Reset
AHCI_GHC_IE             equ (1 << 1)    ; Interrupt Enable
AHCI_GHC_MRSM           equ (1 << 2)    ; MSI Revert to Single Message
AHCI_GHC_AE             equ (1 << 31)   ; AHCI Enable

; ═══════════════════════════════════════════════════════════════════════════
; Port Command (PxCMD) Bits
; ═══════════════════════════════════════════════════════════════════════════

AHCI_PXCMD_ST           equ (1 << 0)    ; Start (DMA engine)
AHCI_PXCMD_SUD          equ (1 << 1)    ; Spin-Up Device
AHCI_PXCMD_POD          equ (1 << 2)    ; Power On Device
AHCI_PXCMD_CLO          equ (1 << 3)    ; Command List Override
AHCI_PXCMD_FRE          equ (1 << 4)    ; FIS Receive Enable
AHCI_PXCMD_CCS_MASK     equ (0x1F << 8) ; Current Command Slot
AHCI_PXCMD_CCS_SHIFT    equ 8
AHCI_PXCMD_MPSS         equ (1 << 13)   ; Mechanical Presence Switch State
AHCI_PXCMD_FR           equ (1 << 14)   ; FIS Receive Running
AHCI_PXCMD_CR           equ (1 << 15)   ; Command List Running
AHCI_PXCMD_CPS          equ (1 << 16)   ; Cold Presence State
AHCI_PXCMD_PMA          equ (1 << 17)   ; Port Multiplier Attached
AHCI_PXCMD_HPCP         equ (1 << 18)   ; Hot Plug Capable Port
AHCI_PXCMD_MPSP         equ (1 << 19)   ; Mechanical Presence Switch
AHCI_PXCMD_CPD          equ (1 << 20)   ; Cold Presence Detection
AHCI_PXCMD_ESP          equ (1 << 21)   ; External SATA Port
AHCI_PXCMD_FBSCP        equ (1 << 22)   ; FIS-based Switching Capable
AHCI_PXCMD_APSTE        equ (1 << 23)   ; Automatic Partial to Slumber
AHCI_PXCMD_ATAPI        equ (1 << 24)   ; Device is ATAPI
AHCI_PXCMD_DLAE         equ (1 << 25)   ; Drive LED on ATAPI Enable
AHCI_PXCMD_ALPE         equ (1 << 26)   ; Aggressive Link Power Enable
AHCI_PXCMD_ASP          equ (1 << 27)   ; Aggressive Slumber/Partial
AHCI_PXCMD_ICC_MASK     equ (0xF << 28) ; Interface Communication Control
AHCI_PXCMD_ICC_SHIFT    equ 28

; ICC values
AHCI_ICC_IDLE           equ 0
AHCI_ICC_ACTIVE         equ 1
AHCI_ICC_PARTIAL        equ 2
AHCI_ICC_SLUMBER        equ 6
AHCI_ICC_DEVSLEEP       equ 8

; ═══════════════════════════════════════════════════════════════════════════
; Port SATA Status (PxSSTS) - SStatus
; ═══════════════════════════════════════════════════════════════════════════

AHCI_SSTS_DET_MASK      equ 0x0F        ; Device Detection (bits 3:0)
AHCI_SSTS_SPD_MASK      equ 0xF0        ; Current Interface Speed (bits 7:4)
AHCI_SSTS_SPD_SHIFT     equ 4
AHCI_SSTS_IPM_MASK      equ 0xF00       ; Interface Power Management (bits 11:8)
AHCI_SSTS_IPM_SHIFT     equ 8

; DET values (Device Detection)
AHCI_DET_NONE           equ 0           ; No device detected
AHCI_DET_PRESENT        equ 1           ; Device present, no comm
AHCI_DET_PHY_COMM       equ 3           ; Device present, PHY communication established
AHCI_DET_PHY_OFFLINE    equ 4           ; PHY in offline mode

; SPD values (Speed)
AHCI_SPD_NONE           equ 0           ; No speed negotiated
AHCI_SPD_GEN1           equ 1           ; 1.5 Gbps
AHCI_SPD_GEN2           equ 2           ; 3.0 Gbps
AHCI_SPD_GEN3           equ 3           ; 6.0 Gbps

; IPM values (Power State)
AHCI_IPM_NONE           equ 0           ; No device / not in power state
AHCI_IPM_ACTIVE         equ 1           ; Interface in active state
AHCI_IPM_PARTIAL        equ 2           ; Interface in partial state
AHCI_IPM_SLUMBER        equ 6           ; Interface in slumber state
AHCI_IPM_DEVSLEEP       equ 8           ; Interface in DevSleep state

; ═══════════════════════════════════════════════════════════════════════════
; Port Task File Data (PxTFD)
; ═══════════════════════════════════════════════════════════════════════════

AHCI_TFD_STS_MASK       equ 0xFF        ; Status byte
AHCI_TFD_ERR_MASK       equ 0xFF00      ; Error byte (shifted)
AHCI_TFD_ERR_SHIFT      equ 8

; Status bits
AHCI_TFD_STS_BSY        equ (1 << 7)    ; Busy
AHCI_TFD_STS_DRQ        equ (1 << 3)    ; Data Request
AHCI_TFD_STS_ERR        equ (1 << 0)    ; Error

; ═══════════════════════════════════════════════════════════════════════════
; Port Interrupt Status (PxIS) - Used for completion detection
; ═══════════════════════════════════════════════════════════════════════════

AHCI_PXIS_DHRS          equ (1 << 0)    ; Device to Host Register FIS
AHCI_PXIS_PSS           equ (1 << 1)    ; PIO Setup FIS
AHCI_PXIS_DSS           equ (1 << 2)    ; DMA Setup FIS
AHCI_PXIS_SDBS          equ (1 << 3)    ; Set Device Bits FIS
AHCI_PXIS_UFS           equ (1 << 4)    ; Unknown FIS
AHCI_PXIS_DPS           equ (1 << 5)    ; Descriptor Processed
AHCI_PXIS_PCS           equ (1 << 6)    ; Port Connect Change
AHCI_PXIS_DMPS          equ (1 << 7)    ; Device Mechanical Presence
AHCI_PXIS_PRCS          equ (1 << 22)   ; PhyRdy Change
AHCI_PXIS_IPMS          equ (1 << 23)   ; Incorrect Port Multiplier
AHCI_PXIS_OFS           equ (1 << 24)   ; Overflow
AHCI_PXIS_INFS          equ (1 << 26)   ; Interface Non-fatal Error
AHCI_PXIS_IFS           equ (1 << 27)   ; Interface Fatal Error
AHCI_PXIS_HBDS          equ (1 << 28)   ; Host Bus Data Error
AHCI_PXIS_HBFS          equ (1 << 29)   ; Host Bus Fatal Error
AHCI_PXIS_TFES          equ (1 << 30)   ; Task File Error

; Error mask - any of these indicates failure
AHCI_PXIS_ERR_MASK      equ (AHCI_PXIS_TFES | AHCI_PXIS_HBFS | AHCI_PXIS_HBDS | AHCI_PXIS_IFS | AHCI_PXIS_INFS | AHCI_PXIS_OFS)

; ═══════════════════════════════════════════════════════════════════════════
; Device Signature Values (PxSIG)
; ═══════════════════════════════════════════════════════════════════════════

AHCI_SIG_ATA            equ 0x00000101  ; ATA device
AHCI_SIG_ATAPI          equ 0xEB140101  ; ATAPI device
AHCI_SIG_SEMB           equ 0xC33C0101  ; Enclosure Management Bridge
AHCI_SIG_PM             equ 0x96690101  ; Port Multiplier

; ═══════════════════════════════════════════════════════════════════════════
; ATA Commands
; ═══════════════════════════════════════════════════════════════════════════

ATA_CMD_READ_DMA_EXT    equ 0x25        ; READ DMA EXT (48-bit LBA)
ATA_CMD_WRITE_DMA_EXT   equ 0x35        ; WRITE DMA EXT (48-bit LBA)
ATA_CMD_IDENTIFY        equ 0xEC        ; IDENTIFY DEVICE
ATA_CMD_FLUSH_CACHE_EXT equ 0xEA        ; FLUSH CACHE EXT
ATA_CMD_READ_SECTORS    equ 0x20        ; READ SECTORS (28-bit LBA)
ATA_CMD_WRITE_SECTORS   equ 0x30        ; WRITE SECTORS (28-bit LBA)

; ═══════════════════════════════════════════════════════════════════════════
; FIS Types
; ═══════════════════════════════════════════════════════════════════════════

FIS_TYPE_REG_H2D        equ 0x27        ; Register FIS - Host to Device
FIS_TYPE_REG_D2H        equ 0x34        ; Register FIS - Device to Host
FIS_TYPE_DMA_ACTIVATE   equ 0x39        ; DMA Activate FIS
FIS_TYPE_DMA_SETUP      equ 0x41        ; DMA Setup FIS (for First-Party DMA)
FIS_TYPE_DATA           equ 0x46        ; Data FIS
FIS_TYPE_BIST           equ 0x58        ; BIST Activate FIS
FIS_TYPE_PIO_SETUP      equ 0x5F        ; PIO Setup FIS
FIS_TYPE_DEV_BITS       equ 0xA1        ; Set Device Bits FIS

; ═══════════════════════════════════════════════════════════════════════════
; Command Header Flags (dword 0 bits 15:0)
; ═══════════════════════════════════════════════════════════════════════════

AHCI_CMD_FIS_LEN_MASK   equ 0x1F        ; FIS length in DWORDs (bits 4:0)
AHCI_CMD_ATAPI          equ (1 << 5)    ; ATAPI command
AHCI_CMD_WRITE          equ (1 << 6)    ; Write (device reads data)
AHCI_CMD_PREFETCH       equ (1 << 7)    ; Prefetchable
AHCI_CMD_RESET          equ (1 << 8)    ; Reset
AHCI_CMD_BIST           equ (1 << 9)    ; BIST
AHCI_CMD_CLR_BUSY       equ (1 << 10)   ; Clear Busy upon R_OK
AHCI_CMD_PMP_MASK       equ (0xF << 12) ; Port Multiplier Port

; ═══════════════════════════════════════════════════════════════════════════
; Structure Sizes and Alignment
; ═══════════════════════════════════════════════════════════════════════════

; Command List: 32 entries × 32 bytes = 1024 bytes, 1K aligned
AHCI_CMD_LIST_SIZE      equ 1024
AHCI_CMD_LIST_ALIGN     equ 1024
AHCI_CMD_HEADER_SIZE    equ 32
AHCI_CMD_SLOTS          equ 32

; FIS Receive area: 256 bytes, 256-byte aligned
AHCI_FIS_SIZE           equ 256
AHCI_FIS_ALIGN          equ 256

; Command Table: 128 bytes (for header/ACMD) + PRDTs, 128-byte aligned
; Each PRDT entry is 16 bytes
AHCI_CMD_TABLE_SIZE     equ 256         ; Minimum for header + some PRDTs
AHCI_CMD_TABLE_ALIGN    equ 128
AHCI_PRDT_ENTRY_SIZE    equ 16
AHCI_MAX_PRDT_ENTRIES   equ 8           ; Limit for simplicity

; ═══════════════════════════════════════════════════════════════════════════
; PCI Configuration
; ═══════════════════════════════════════════════════════════════════════════

; Intel Wildcat Point-LP SATA Controller
AHCI_VENDOR_INTEL       equ 0x8086
AHCI_DEVICE_WPT_LP      equ 0x9C83      ; ThinkPad T450s

; PCI Class Code for AHCI
PCI_CLASS_SATA_AHCI     equ 0x010601    ; Mass Storage, SATA, AHCI

; AHCI uses BAR5 (ABAR - AHCI Base Address Register)
AHCI_BAR_INDEX          equ 5
