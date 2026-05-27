//! Aegis JTAG instruction codes.
//!
//! Source of truth: `~/Midstall/aegis/ip/lib/src/components/digital/jtag_tap.dart`.
//! These values must stay in sync with the silicon RTL.

/// JTAG instruction-register width in bits, per Aegis silicon.
pub const IR_WIDTH: u8 = 4;

/// EXTEST (mapped to BYPASS in the Aegis TAP).
pub const IR_EXTEST: u32 = 0x0;

/// IDCODE: TDO returns the 32-bit IDCODE during DR shift.
pub const IR_IDCODE: u32 = 0x1;

/// CONFIG: TDI shifts directly into the fabric config chain during DR shift.
/// On Update-DR the silicon latches the bitstream into the per-tile config
/// registers (cfgLoad pulses internally).
pub const IR_CONFIG: u32 = 0x2;

/// USER: reserved for user-defined boundary scan (cfgIn / cfgLoad / etc.).
pub const IR_USER: u32 = 0x3;

/// BYPASS: TDI passes straight to TDO with one cycle of delay.
pub const IR_BYPASS: u32 = 0xF;

/// Default IDCODE Aegis silicon ships with. Customizable per-device; check
/// the per-device descriptor or silicon RTL for the actual value if the
/// default has been overridden.
pub const DEFAULT_IDCODE: u32 = 0x0000_0001;
