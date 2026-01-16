//! Intel e1000e register definitions.
//!
//! All register offsets and bit definitions for the Intel 82579/I218 family.
//! These match the definitions in asm/drivers/intel/init.s.
//!
//! # Reference
//! Intel 82579 Datasheet, Section 10 (Programming Interface)

// ═══════════════════════════════════════════════════════════════════════════
// DEVICE CONTROL & STATUS
// ═══════════════════════════════════════════════════════════════════════════

/// Device Control Register.
pub const CTRL: u32 = 0x0000;
/// Device Status Register (RO).
pub const STATUS: u32 = 0x0008;
/// EEPROM/Flash Control Register.
pub const EECD: u32 = 0x0010;
/// EEPROM Read Register.
pub const EERD: u32 = 0x0014;
/// Extended Device Control Register.
pub const CTRL_EXT: u32 = 0x0018;
/// MDI Control Register (PHY access).
pub const MDIC: u32 = 0x0020;

// ═══════════════════════════════════════════════════════════════════════════
// INTERRUPT REGISTERS
// ═══════════════════════════════════════════════════════════════════════════

/// Interrupt Cause Read (RO).
pub const ICR: u32 = 0x00C0;
/// Interrupt Cause Set (WO).
pub const ICS: u32 = 0x00C8;
/// Interrupt Mask Set/Read.
pub const IMS: u32 = 0x00D0;
/// Interrupt Mask Clear (WO).
pub const IMC: u32 = 0x00D8;

// ═══════════════════════════════════════════════════════════════════════════
// RECEIVE REGISTERS
// ═══════════════════════════════════════════════════════════════════════════

/// Receive Control Register.
pub const RCTL: u32 = 0x0100;
/// Receive Descriptor Base Address Low.
pub const RDBAL: u32 = 0x2800;
/// Receive Descriptor Base Address High.
pub const RDBAH: u32 = 0x2804;
/// Receive Descriptor Length (bytes).
pub const RDLEN: u32 = 0x2808;
/// Receive Descriptor Head.
pub const RDH: u32 = 0x2810;
/// Receive Descriptor Tail.
pub const RDT: u32 = 0x2818;
/// Receive Descriptor Control.
pub const RXDCTL: u32 = 0x2828;

// ═══════════════════════════════════════════════════════════════════════════
// TRANSMIT REGISTERS
// ═══════════════════════════════════════════════════════════════════════════

/// Transmit Control Register.
pub const TCTL: u32 = 0x0400;
/// Transmit Descriptor Base Address Low.
pub const TDBAL: u32 = 0x3800;
/// Transmit Descriptor Base Address High.
pub const TDBAH: u32 = 0x3804;
/// Transmit Descriptor Length (bytes).
pub const TDLEN: u32 = 0x3808;
/// Transmit Descriptor Head.
pub const TDH: u32 = 0x3810;
/// Transmit Descriptor Tail.
pub const TDT: u32 = 0x3818;
/// Transmit Descriptor Control.
pub const TXDCTL: u32 = 0x3828;

// ═══════════════════════════════════════════════════════════════════════════
// RECEIVE ADDRESS REGISTERS
// ═══════════════════════════════════════════════════════════════════════════

/// Receive Address Low (MAC bytes 0-3).
pub const RAL0: u32 = 0x5400;
/// Receive Address High (MAC bytes 4-5 + flags).
pub const RAH0: u32 = 0x5404;
/// Multicast Table Array (128 entries × 4 bytes).
pub const MTA: u32 = 0x5200;

// ═══════════════════════════════════════════════════════════════════════════
// CTRL REGISTER BITS
// ═══════════════════════════════════════════════════════════════════════════

/// Full Duplex.
pub const CTRL_FD: u32 = 1 << 0;
/// GIO Master Disable.
pub const CTRL_GIO_MASTER_DISABLE: u32 = 1 << 2;
/// Link Reset.
pub const CTRL_LRST: u32 = 1 << 3;
/// Auto-Speed Detection Enable.
pub const CTRL_ASDE: u32 = 1 << 5;
/// Set Link Up.
pub const CTRL_SLU: u32 = 1 << 6;
/// Invert Loss-of-Signal.
pub const CTRL_ILOS: u32 = 1 << 7;
/// Speed selection (bits 8-9).
pub const CTRL_SPEED_MASK: u32 = 3 << 8;
/// Speed 10 Mb/s.
pub const CTRL_SPEED_10: u32 = 0 << 8;
/// Speed 100 Mb/s.
pub const CTRL_SPEED_100: u32 = 1 << 8;
/// Speed 1000 Mb/s.
pub const CTRL_SPEED_1000: u32 = 2 << 8;
/// Force Speed.
pub const CTRL_FRCSPD: u32 = 1 << 11;
/// Force Duplex.
pub const CTRL_FRCDPLX: u32 = 1 << 12;
/// Device Reset.
pub const CTRL_RST: u32 = 1 << 26;
/// PHY Reset.
pub const CTRL_PHY_RST: u32 = 1 << 31;

// ═══════════════════════════════════════════════════════════════════════════
// STATUS REGISTER BITS (Read-Only)
// ═══════════════════════════════════════════════════════════════════════════

/// Full Duplex.
pub const STATUS_FD: u32 = 1 << 0;
/// Link Up.
pub const STATUS_LU: u32 = 1 << 1;
/// Function ID (bits 2-3).
pub const STATUS_FUNC_MASK: u32 = 3 << 2;
/// TX Off.
pub const STATUS_TXOFF: u32 = 1 << 4;
/// Speed (bits 6-7).
pub const STATUS_SPEED_MASK: u32 = 3 << 6;
/// Speed 10 Mb/s.
pub const STATUS_SPEED_10: u32 = 0 << 6;
/// Speed 100 Mb/s.
pub const STATUS_SPEED_100: u32 = 1 << 6;
/// Speed 1000 Mb/s.
pub const STATUS_SPEED_1000: u32 = 2 << 6;
/// GIO Master Enable Status.
pub const STATUS_GIO_MASTER_EN: u32 = 1 << 19;

// ═══════════════════════════════════════════════════════════════════════════
// RCTL REGISTER BITS
// ═══════════════════════════════════════════════════════════════════════════

/// Receiver Enable.
pub const RCTL_EN: u32 = 1 << 1;
/// Store Bad Packets.
pub const RCTL_SBP: u32 = 1 << 2;
/// Unicast Promiscuous Enable.
pub const RCTL_UPE: u32 = 1 << 3;
/// Multicast Promiscuous Enable.
pub const RCTL_MPE: u32 = 1 << 4;
/// Long Packet Enable.
pub const RCTL_LPE: u32 = 1 << 5;
/// Loopback Mode (bits 6-7).
pub const RCTL_LBM_MASK: u32 = 3 << 6;
/// Receive Descriptor Minimum Threshold (bits 8-9).
pub const RCTL_RDMTS_MASK: u32 = 3 << 8;
/// Multicast Offset (bits 12-13).
pub const RCTL_MO_MASK: u32 = 3 << 12;
/// Broadcast Accept Mode.
pub const RCTL_BAM: u32 = 1 << 15;
/// Buffer Size (bits 16-17).
pub const RCTL_BSIZE_MASK: u32 = 3 << 16;
/// Buffer Size 2048 bytes.
pub const RCTL_BSIZE_2048: u32 = 0 << 16;
/// Buffer Size 1024 bytes.
pub const RCTL_BSIZE_1024: u32 = 1 << 16;
/// Buffer Size 512 bytes.
pub const RCTL_BSIZE_512: u32 = 2 << 16;
/// Buffer Size 256 bytes.
pub const RCTL_BSIZE_256: u32 = 3 << 16;
/// VLAN Filter Enable.
pub const RCTL_VFE: u32 = 1 << 18;
/// Canonical Form Indicator Enable.
pub const RCTL_CFIEN: u32 = 1 << 19;
/// Canonical Form Indicator bit.
pub const RCTL_CFI: u32 = 1 << 20;
/// Discard Pause Frames.
pub const RCTL_DPF: u32 = 1 << 22;
/// Pass MAC Control Frames.
pub const RCTL_PMCF: u32 = 1 << 23;
/// Buffer Size Extension.
pub const RCTL_BSEX: u32 = 1 << 25;
/// Strip Ethernet CRC.
pub const RCTL_SECRC: u32 = 1 << 26;

// ═══════════════════════════════════════════════════════════════════════════
// TCTL REGISTER BITS
// ═══════════════════════════════════════════════════════════════════════════

/// Transmitter Enable.
pub const TCTL_EN: u32 = 1 << 1;
/// Pad Short Packets.
pub const TCTL_PSP: u32 = 1 << 3;
/// Collision Threshold (bits 4-11).
pub const TCTL_CT_MASK: u32 = 0xFF << 4;
/// Collision Threshold shift.
pub const TCTL_CT_SHIFT: u32 = 4;
/// Collision Distance (bits 12-21).
pub const TCTL_COLD_MASK: u32 = 0x3FF << 12;
/// Collision Distance shift.
pub const TCTL_COLD_SHIFT: u32 = 12;
/// Re-transmit on Late Collision.
pub const TCTL_RTLC: u32 = 1 << 24;

/// Default Collision Threshold (15).
pub const TCTL_CT_DEFAULT: u32 = 15 << TCTL_CT_SHIFT;
/// Default Collision Distance for Full Duplex (64).
pub const TCTL_COLD_FD: u32 = 64 << TCTL_COLD_SHIFT;
/// Default Collision Distance for Half Duplex (512).
pub const TCTL_COLD_HD: u32 = 512 << TCTL_COLD_SHIFT;

// ═══════════════════════════════════════════════════════════════════════════
// RXDCTL / TXDCTL REGISTER BITS
// ═══════════════════════════════════════════════════════════════════════════

/// Queue Enable (RXDCTL/TXDCTL).
pub const XDCTL_QUEUE_ENABLE: u32 = 1 << 25;

// ═══════════════════════════════════════════════════════════════════════════
// EECD REGISTER BITS (EEPROM Control)
// ═══════════════════════════════════════════════════════════════════════════

/// Auto Read Done.
pub const EECD_AUTO_RD: u32 = 1 << 9;

// ═══════════════════════════════════════════════════════════════════════════
// INTERRUPT MASK VALUES
// ═══════════════════════════════════════════════════════════════════════════

/// All interrupt bits (for masking/clearing).
pub const INT_MASK_ALL: u32 = 0xFFFFFFFF;

// ═══════════════════════════════════════════════════════════════════════════
// RAH REGISTER BITS
// ═══════════════════════════════════════════════════════════════════════════

/// Address Valid.
pub const RAH_AV: u32 = 1 << 31;
/// Address Select (bits 16-17): 00=destination, 01=source.
pub const RAH_ASEL_MASK: u32 = 3 << 16;

// ═══════════════════════════════════════════════════════════════════════════
// MDIC REGISTER BITS
// ═══════════════════════════════════════════════════════════════════════════

/// Data mask (bits 0-15).
pub const MDIC_DATA_MASK: u32 = 0xFFFF;
/// Register address shift (bits 16-20).
pub const MDIC_REG_SHIFT: u32 = 16;
/// PHY address shift (bits 21-25).
pub const MDIC_PHY_SHIFT: u32 = 21;
/// Operation: Write.
pub const MDIC_OP_WRITE: u32 = 1 << 26;
/// Operation: Read.
pub const MDIC_OP_READ: u32 = 2 << 26;
/// Ready bit.
pub const MDIC_READY: u32 = 1 << 28;
/// Interrupt Enable.
pub const MDIC_IE: u32 = 1 << 29;
/// Error bit.
pub const MDIC_ERROR: u32 = 1 << 30;

/// Default PHY address.
pub const PHY_ADDR: u32 = 1;

// ═══════════════════════════════════════════════════════════════════════════
// PHY REGISTERS (MII Standard)
// ═══════════════════════════════════════════════════════════════════════════

/// Basic Mode Control Register.
pub const PHY_BMCR: u32 = 0x00;
/// Basic Mode Status Register.
pub const PHY_BMSR: u32 = 0x01;
/// PHY Identifier 1.
pub const PHY_PHYID1: u32 = 0x02;
/// PHY Identifier 2.
pub const PHY_PHYID2: u32 = 0x03;
/// Auto-Negotiation Advertisement Register.
pub const PHY_ANAR: u32 = 0x04;
/// Auto-Negotiation Link Partner Ability Register.
pub const PHY_ANLPAR: u32 = 0x05;
/// Auto-Negotiation Expansion Register.
pub const PHY_ANER: u32 = 0x06;
/// 1000BASE-T Control Register.
pub const PHY_1000T_CTRL: u32 = 0x09;
/// 1000BASE-T Status Register.
pub const PHY_1000T_STATUS: u32 = 0x0A;

// ═══════════════════════════════════════════════════════════════════════════
// PHY BMCR BITS
// ═══════════════════════════════════════════════════════════════════════════

/// Collision Test.
pub const BMCR_CTST: u16 = 1 << 7;
/// Full Duplex.
pub const BMCR_FULLDPLX: u16 = 1 << 8;
/// Restart Auto-Negotiation.
pub const BMCR_ANRESTART: u16 = 1 << 9;
/// Isolate.
pub const BMCR_ISOLATE: u16 = 1 << 10;
/// Power Down.
pub const BMCR_PDOWN: u16 = 1 << 11;
/// Auto-Negotiation Enable.
pub const BMCR_ANENABLE: u16 = 1 << 12;
/// Speed Selection (MSB).
pub const BMCR_SPEED100: u16 = 1 << 13;
/// Loopback.
pub const BMCR_LOOPBACK: u16 = 1 << 14;
/// Reset.
pub const BMCR_RESET: u16 = 1 << 15;

// ═══════════════════════════════════════════════════════════════════════════
// PHY BMSR BITS
// ═══════════════════════════════════════════════════════════════════════════

/// Extended Capability.
pub const BMSR_ERCAP: u16 = 1 << 0;
/// Jabber Detected.
pub const BMSR_JCD: u16 = 1 << 1;
/// Link Status.
pub const BMSR_LSTATUS: u16 = 1 << 2;
/// Auto-Negotiation Ability.
pub const BMSR_ANEGCAPABLE: u16 = 1 << 3;
/// Remote Fault.
pub const BMSR_RFAULT: u16 = 1 << 4;
/// Auto-Negotiation Complete.
pub const BMSR_ANEGCOMPLETE: u16 = 1 << 5;
/// 10BASE-T Half Duplex.
pub const BMSR_10HALF: u16 = 1 << 11;
/// 10BASE-T Full Duplex.
pub const BMSR_10FULL: u16 = 1 << 12;
/// 100BASE-TX Half Duplex.
pub const BMSR_100HALF: u16 = 1 << 13;
/// 100BASE-TX Full Duplex.
pub const BMSR_100FULL: u16 = 1 << 14;
/// 100BASE-T4.
pub const BMSR_100BASE4: u16 = 1 << 15;

// ═══════════════════════════════════════════════════════════════════════════
// INTERRUPT BITS
// ═══════════════════════════════════════════════════════════════════════════

/// TX Descriptor Written Back.
pub const ICR_TXDW: u32 = 1 << 0;
/// TX Queue Empty.
pub const ICR_TXQE: u32 = 1 << 1;
/// Link Status Change.
pub const ICR_LSC: u32 = 1 << 2;
/// RX Descriptor Minimum Threshold.
pub const ICR_RXDMT0: u32 = 1 << 4;
/// RX Overrun.
pub const ICR_RXO: u32 = 1 << 6;
/// RX Timer Interrupt.
pub const ICR_RXT0: u32 = 1 << 7;
/// All interrupt bits.
pub const ICR_ALL: u32 = 0xFFFFFFFF;

// ═══════════════════════════════════════════════════════════════════════════
// DESCRIPTOR CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

/// Size of one descriptor in bytes.
pub const DESC_SIZE: usize = 16;
/// Default queue size (number of descriptors).
pub const DEFAULT_QUEUE_SIZE: u16 = 32;
/// Default buffer size.
pub const DEFAULT_BUFFER_SIZE: usize = 2048;
/// Maximum frame size (without FCS).
pub const MAX_FRAME_SIZE: usize = 1514;

// ═══════════════════════════════════════════════════════════════════════════
// TX DESCRIPTOR BITS
// ═══════════════════════════════════════════════════════════════════════════

/// End of Packet.
pub const TXD_CMD_EOP: u8 = 1 << 0;
/// Insert FCS.
pub const TXD_CMD_IFCS: u8 = 1 << 1;
/// Insert Checksum.
pub const TXD_CMD_IC: u8 = 1 << 2;
/// Report Status.
pub const TXD_CMD_RS: u8 = 1 << 3;
/// Report Packet Sent.
pub const TXD_CMD_RPS: u8 = 1 << 4;
/// Descriptor Extension.
pub const TXD_CMD_DEXT: u8 = 1 << 5;
/// VLAN Packet Enable.
pub const TXD_CMD_VLE: u8 = 1 << 6;
/// Interrupt Delay Enable.
pub const TXD_CMD_IDE: u8 = 1 << 7;

/// Descriptor Done.
pub const TXD_STA_DD: u8 = 1 << 0;

// ═══════════════════════════════════════════════════════════════════════════
// RX DESCRIPTOR BITS
// ═══════════════════════════════════════════════════════════════════════════

/// Descriptor Done.
pub const RXD_STA_DD: u8 = 1 << 0;
/// End of Packet.
pub const RXD_STA_EOP: u8 = 1 << 1;
/// Ignore Checksum Indication.
pub const RXD_STA_IXSM: u8 = 1 << 2;
/// VLAN Packet.
pub const RXD_STA_VP: u8 = 1 << 3;

/// CRC Error.
pub const RXD_ERR_CE: u8 = 1 << 0;
/// Symbol Error.
pub const RXD_ERR_SE: u8 = 1 << 1;
/// Sequence Error.
pub const RXD_ERR_SEQ: u8 = 1 << 2;
/// Carrier Extension Error.
pub const RXD_ERR_CXE: u8 = 1 << 4;
/// RX Data Error.
pub const RXD_ERR_RXE: u8 = 1 << 5;
/// IP Checksum Error.
pub const RXD_ERR_IPE: u8 = 1 << 6;
/// TCP/UDP Checksum Error.
pub const RXD_ERR_TCPE: u8 = 1 << 7;

/// All fatal RX errors.
pub const RXD_ERR_FATAL: u8 = RXD_ERR_CE | RXD_ERR_SE | RXD_ERR_SEQ | RXD_ERR_RXE;

// ═══════════════════════════════════════════════════════════════════════════
// I218/PCH LPT SPECIFIC REGISTERS
// These are critical for real hardware (ThinkPad T450s, etc.)
// Reference: Linux kernel drivers/net/ethernet/intel/e1000e/ich8lan.c
// ═══════════════════════════════════════════════════════════════════════════

/// Firmware Status Monitor (FWSM) register.
pub const FWSM: u32 = 0x5B54;
/// Host-to-ME register for ULP control.
pub const H2ME: u32 = 0x5B50;
/// Extended Configuration Control register (hardware semaphore).
pub const EXTCNF_CTRL: u32 = 0x0F00;
/// PHY Configuration Count (for LANPHYPC timing).
pub const FEXTNVM3: u32 = 0x003C;
/// Flash Access register.
pub const FEXTNVM4: u32 = 0x0024;
/// Extended Function Control Register 6.
pub const FEXTNVM6: u32 = 0x0010;
/// PHY Control register (PCH specific).
pub const PHPM: u32 = 0x0E14;

// ═══════════════════════════════════════════════════════════════════════════
// CTRL REGISTER BITS - I218 SPECIFIC
// ═══════════════════════════════════════════════════════════════════════════

/// LANPHYPC Override - allows software control of PHY power.
pub const CTRL_LANPHYPC_OVERRIDE: u32 = 1 << 16;
/// LANPHYPC Value - PHY power control value (1=power on).
pub const CTRL_LANPHYPC_VALUE: u32 = 1 << 17;

// ═══════════════════════════════════════════════════════════════════════════
// CTRL_EXT REGISTER BITS - I218 SPECIFIC
// ═══════════════════════════════════════════════════════════════════════════

/// Force SMBus mode for PHY access.
pub const CTRL_EXT_FORCE_SMBUS: u32 = 1 << 11;
/// Link Power Cycle Done - set by HW after LANPHYPC toggle.
pub const CTRL_EXT_LPCD: u32 = 1 << 14;
/// PHY Power Down Enable.
pub const CTRL_EXT_PHYPDEN: u32 = 1 << 20;

// ═══════════════════════════════════════════════════════════════════════════
// FWSM REGISTER BITS
// ═══════════════════════════════════════════════════════════════════════════

/// Firmware Valid - indicates ME firmware is present.
pub const FWSM_FW_VALID: u32 = 1 << 15;
/// ULP Config Done - firmware completed ULP configuration.
pub const FWSM_ULP_CFG_DONE: u32 = 1 << 18;

// ═══════════════════════════════════════════════════════════════════════════
// H2ME REGISTER BITS (Host-to-ME)
// ═══════════════════════════════════════════════════════════════════════════

/// ULP Disable Request to firmware.
pub const H2ME_ULP_DISABLE: u32 = 1 << 1;
/// Start VME handshake.
pub const H2ME_START_VME: u32 = 1 << 0;

// ═══════════════════════════════════════════════════════════════════════════
// EXTCNF_CTRL REGISTER BITS (Hardware Semaphore)
// ═══════════════════════════════════════════════════════════════════════════

/// Software Flag - acquire before PHY/NVM access.
pub const EXTCNF_CTRL_SWFLAG: u32 = 1 << 5;
/// Gate PHY Configuration - prevents PHY config during access.
pub const EXTCNF_CTRL_GATE_PHY_CFG: u32 = 1 << 7;

// ═══════════════════════════════════════════════════════════════════════════
// FEXTNVM3 REGISTER BITS
// ═══════════════════════════════════════════════════════════════════════════

/// PHY Config Counter mask (bits 12-13).
pub const FEXTNVM3_PHY_CFG_COUNTER_MASK: u32 = 0x3 << 12;
/// PHY Config Counter 50ms value.
pub const FEXTNVM3_PHY_CFG_COUNTER_50MS: u32 = 0x1 << 12;

// ═══════════════════════════════════════════════════════════════════════════
// FEXTNVM4 REGISTER BITS
// ═══════════════════════════════════════════════════════════════════════════

/// Beacon Duration mask.
pub const FEXTNVM4_BEACON_DURATION_MASK: u32 = 0x7 << 3;
/// Beacon Duration 16us.
pub const FEXTNVM4_BEACON_DURATION_16US: u32 = 0x3 << 3;

// ═══════════════════════════════════════════════════════════════════════════
// FEXTNVM6 REGISTER BITS
// ═══════════════════════════════════════════════════════════════════════════

/// Request PLL Clock while in K1 state.
pub const FEXTNVM6_REQ_PLL_CLK: u32 = 1 << 6;

// ═══════════════════════════════════════════════════════════════════════════
// PHPM REGISTER BITS
// ═══════════════════════════════════════════════════════════════════════════

/// SPD Enable (Speed).
pub const PHPM_SPD_EN: u32 = 1 << 4;
/// D0A Low Power State Enable.
pub const PHPM_D0A_LPLU: u32 = 1 << 1;

// ═══════════════════════════════════════════════════════════════════════════
// PHY REGISTERS - I218 SPECIFIC (HV/82577/82579 PHY)
// ═══════════════════════════════════════════════════════════════════════════

/// PHY ID Register 1 (expected: 0x0154 for I218).
pub const PHY_ID1: u32 = 0x02;
/// PHY ID Register 2 (expected: 0x15xx for I218).
pub const PHY_ID2: u32 = 0x03;
/// I217/I218 PHY Vendor ID high nibble.
pub const I217_PHY_ID_MASK: u16 = 0x0150;

// ═══════════════════════════════════════════════════════════════════════════
// I218 PHY PAGE/REGISTER ACCESS
// ═══════════════════════════════════════════════════════════════════════════

/// OEM Bits register (page 0).
pub const HV_OEM_BITS: u32 = 0x1F;
/// OEM Bits: Restart Autoneg.
pub const HV_OEM_BITS_RESTART_AN: u16 = 1 << 0;
/// OEM Bits: Low Power Link Up.
pub const HV_OEM_BITS_LPLU: u16 = 1 << 2;

/// KMRN Control register (for cable length).
pub const HV_KMRN_MODE_CTRL: u32 = 0x1EA;

// ═══════════════════════════════════════════════════════════════════════════
// TIMEOUTS (in microseconds)
// ═══════════════════════════════════════════════════════════════════════════

/// MDIC operation timeout (10ms, was 1ms - too short).
pub const MDIC_TIMEOUT_US: u64 = 10_000;
/// Hardware semaphore acquire timeout (1 second).
pub const SWFLAG_TIMEOUT_US: u64 = 1_000_000;
/// ULP disable timeout (2.5 seconds per Linux driver).
pub const ULP_DISABLE_TIMEOUT_US: u64 = 2_500_000;
/// LANPHYPC toggle timeout (50ms).
pub const LANPHYPC_TIMEOUT_US: u64 = 50_000;
/// PHY stabilization after power-on (30ms).
pub const PHY_POWER_ON_DELAY_US: u64 = 30_000;
