//! MPSSE (Multi-Protocol Synchronous Serial Engine) JTAG driver.
//!
//! Generic over any byte-stream backend (`MpsseBackend`) so the JTAG state
//! machine and command encoder can be unit-tested with a recording mock,
//! independent of the real USB transport.
//!
//! Reference: FTDI Application Note AN_108, "Command Processor for MPSSE and
//! MCU Host Bus Emulation Modes".
//!
//! JTAG conventions (matches OpenOCD `ftdi` driver):
//!   ADBUS0 = TCK (out)
//!   ADBUS1 = TDI (out)
//!   ADBUS2 = TDO (in)
//!   ADBUS3 = TMS (out)
//!   Data shifted LSB-first; TDI updated on falling TCK; TDO sampled on rising TCK.

use crate::error::TransportError;
use crate::traits::Result;

/// MPSSE command opcodes used by this driver.
pub mod cmd {
    /// Clock data bytes OUT on TDI, -ve TCK edge, LSB first. No TDO read.
    pub const SHIFT_BYTES_OUT: u8 = 0x19;
    /// Clock data bytes OUT on TDI + IN on TDO, -ve/+ve edges, LSB first.
    pub const SHIFT_BYTES_INOUT: u8 = 0x39;
    /// Clock data BITS OUT on TDI, -ve TCK edge, LSB first.
    pub const SHIFT_BITS_OUT: u8 = 0x1B;
    /// Clock data BITS OUT on TDI + IN on TDO, LSB first.
    pub const SHIFT_BITS_INOUT: u8 = 0x3B;
    /// Clock TMS BITS OUT, TDI held constant (bit 7 of payload), no read.
    pub const TMS_OUT: u8 = 0x4B;
    /// Clock TMS BITS OUT, TDI held constant, with TDO read.
    pub const TMS_INOUT: u8 = 0x6B;

    /// Disconnect TDI->TDO internal loopback.
    pub const LOOPBACK_OFF: u8 = 0x85;
    /// Set TCK divisor (1 + arg). F_TCK = base / (2 * (1 + divisor)).
    pub const SET_TCK_DIVISOR: u8 = 0x86;
    /// Send buffered TDO bytes to USB immediately.
    pub const SEND_IMMEDIATE: u8 = 0x87;
    /// Enable clock divide by 5 (default for FT2232H, gives 12 MHz base).
    pub const CLOCK_DIVIDE_5_ENABLE: u8 = 0x8B;
    /// Disable clock divide by 5 (FT232H/FT4232H, gives 60 MHz base).
    pub const CLOCK_DIVIDE_5_DISABLE: u8 = 0x8A;
    /// Set low byte data + direction. Args: value, direction.
    pub const SET_BITS_LOW: u8 = 0x80;
    /// Bad-command marker echoed back by the engine when it sees garbage.
    pub const BAD_COMMAND: u8 = 0xAA;
}

/// Sink/source for raw MPSSE wire bytes. Implementations carry the actual
/// USB I/O. The JTAG state machine layers on top of this trait so it can
/// be driven against a mock for unit tests.
pub trait MpsseBackend: Send + Sync {
    fn write_all(&mut self, data: &[u8]) -> Result<()>;
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<()>;
}

/// JTAG TAP states we explicitly traverse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapState {
    TestLogicReset,
    RunTestIdle,
}

/// MPSSE-driven JTAG engine. Owns the TAP state, IR width, and an arbitrary
/// `MpsseBackend`. All shifts batch into the smallest possible USB round-trip.
pub struct MpsseJtag<B: MpsseBackend> {
    backend: B,
    tap: TapState,
    ir_width: u8,
}

impl<B: MpsseBackend> MpsseJtag<B> {
    pub fn new(backend: B, ir_width: u8) -> Self {
        assert!(
            (1..=32).contains(&ir_width),
            "ir_width must be in 1..=32 (got {ir_width})"
        );
        Self {
            backend,
            tap: TapState::TestLogicReset,
            ir_width,
        }
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    pub fn into_backend(self) -> B {
        self.backend
    }

    pub fn ir_width(&self) -> u8 {
        self.ir_width
    }

    pub fn set_ir_width(&mut self, ir_width: u8) {
        assert!((1..=32).contains(&ir_width));
        self.ir_width = ir_width;
    }

    pub fn tap_state(&self) -> TapState {
        self.tap
    }

    /// Send the standard MPSSE init sequence: disable loopback, set TCK
    /// divisor for ~1 MHz from the 12 MHz base, drive ADBUS pins for JTAG.
    /// Also flushes any pending bad-command echoes from a prior session.
    pub fn init(&mut self, divisor: u16) -> Result<()> {
        // Step 1: deliberately send a known-bad opcode and consume the
        // expected `0xFA, 0xAA` echo. This synchronizes the engine and
        // verifies the channel is in MPSSE mode.
        self.backend.write_all(&[cmd::BAD_COMMAND])?;
        let mut sync = [0u8; 2];
        self.backend.read_exact(&mut sync)?;
        if sync != [0xFA, cmd::BAD_COMMAND] {
            return Err(TransportError::Protocol(format!(
                "ftdi mpsse sync failed: expected [0xFA, 0xAA], got {sync:02x?}"
            )));
        }

        // Step 2: configure the engine for JTAG.
        let init = [
            cmd::LOOPBACK_OFF,
            cmd::CLOCK_DIVIDE_5_ENABLE,
            cmd::SET_TCK_DIVISOR,
            (divisor & 0xFF) as u8,
            ((divisor >> 8) & 0xFF) as u8,
            // ADBUS: TCK=0(out), TDI=1(out), TDO=2(in), TMS=3(out).
            // Direction = 0b0000_1011 = 0x0B. Initial value = TMS high (0x08).
            cmd::SET_BITS_LOW,
            0x08,
            0x0B,
        ];
        self.backend.write_all(&init)?;
        Ok(())
    }

    /// Walk to Test-Logic-Reset by clocking 6x TMS=1, then drop to Idle.
    pub fn reset_tap(&mut self) -> Result<()> {
        // 6 TMS=1 clocks + 1 TMS=0 (Idle), all in one TMS_OUT command.
        // Payload bit 0..6 = TMS sequence LSB first: 0b0_111_1111? No:
        // we want: 1,1,1,1,1,1,0 → 0b0_111_111 = 0x3F (with length=7).
        // Bits are sent LSB first. With length=7, byte = 0011_1111 = 0x3F.
        self.backend.write_all(&[cmd::TMS_OUT, 6, 0b0011_1111])?;
        self.tap = TapState::RunTestIdle;
        Ok(())
    }

    /// From RunTestIdle, navigate to Shift-DR via TMS=1,0,0.
    fn nav_idle_to_shift_dr(&mut self) -> Result<()> {
        debug_assert!(matches!(self.tap, TapState::RunTestIdle));
        // 3 TMS clocks: 1,0,0 → byte = 0b00_000_001 = 0x01.
        self.backend.write_all(&[cmd::TMS_OUT, 2, 0x01])?;
        Ok(())
    }

    /// From RunTestIdle, navigate to Shift-IR via TMS=1,1,0,0.
    fn nav_idle_to_shift_ir(&mut self) -> Result<()> {
        debug_assert!(matches!(self.tap, TapState::RunTestIdle));
        // 4 TMS clocks: 1,1,0,0 → byte = 0b0000_0011 = 0x03.
        self.backend.write_all(&[cmd::TMS_OUT, 3, 0x03])?;
        Ok(())
    }

    /// From Exit1-IR/DR, return to RunTestIdle via TMS=1,0.
    fn nav_exit1_to_idle(&mut self) -> Result<()> {
        // 2 TMS clocks: 1,0 → byte = 0b0000_0001 = 0x01.
        self.backend.write_all(&[cmd::TMS_OUT, 1, 0x01])?;
        self.tap = TapState::RunTestIdle;
        Ok(())
    }

    /// Shift exactly 32 bits out of DR with TDI=0, reading TDO. Used for IDCODE.
    /// Caller must be in RunTestIdle.
    fn shift_dr_idcode(&mut self) -> Result<u32> {
        self.nav_idle_to_shift_dr()?;
        // Shift 31 bits of TDI=0 with TDO read, then a 32nd bit via TMS_INOUT
        // that simultaneously exits to Exit1-DR.
        //
        // We use 3 full bytes (24 bits) of SHIFT_BYTES_INOUT, then 7 bits via
        // SHIFT_BITS_INOUT, then 1 bit via TMS_INOUT to exit.
        let mut prog = Vec::new();
        // 3 bytes (24 bits) TDI=0 with read.
        prog.extend_from_slice(&[cmd::SHIFT_BYTES_INOUT, 2, 0, 0, 0, 0]);
        // 7 bits (bits 24..31) TDI=0 with read.
        prog.extend_from_slice(&[cmd::SHIFT_BITS_INOUT, 6, 0]);
        // 1 bit via TMS=1 to exit Shift-DR. TDI=0 (bit 7 of payload).
        prog.extend_from_slice(&[cmd::TMS_INOUT, 0, 0x01]);
        prog.push(cmd::SEND_IMMEDIATE);
        self.backend.write_all(&prog)?;

        // Read back: 3 bytes + 1 byte (bits-mode response, LSB-aligned) + 1
        // byte (TMS_INOUT response, single bit captured in bit 7).
        let mut buf = [0u8; 5];
        self.backend.read_exact(&mut buf)?;
        let bytes24 = u32::from(buf[0]) | (u32::from(buf[1]) << 8) | (u32::from(buf[2]) << 16);
        // 7-bit response: bits captured at top of the byte (LSB first into
        // shift register means the FIRST bit clocked is the LSB of the
        // ACCUMULATED 7-bit value, then shifted right (length-1)-i positions
        // by the engine). Per AN_108, the bits land MSB-justified, so to
        // recover them you shift right by (8 - length).
        let bits7 = (buf[3] >> (8 - 7)) as u32;
        // 1-bit TMS response: bit 7 of the response byte.
        let bit_last = ((buf[4] >> 7) & 1) as u32;
        let idcode = bytes24 | (bits7 << 24) | (bit_last << 31);

        self.tap = TapState::RunTestIdle;
        self.nav_exit1_to_idle()?;
        Ok(idcode)
    }

    /// Scan for the IDCODE on the only TAP on the chain. Returns the IDCODE,
    /// or an empty vec if the bus is floating (0x00000000 or 0xFFFFFFFF).
    pub fn scan_idcode(&mut self) -> Result<Vec<u32>> {
        self.reset_tap()?;
        let idcode = self.shift_dr_idcode()?;
        if idcode == 0 || idcode == u32::MAX {
            Ok(Vec::new())
        } else {
            Ok(vec![idcode])
        }
    }

    /// Shift IR (`self.ir_width` bits) with `ir` as TDI, then shift DR with
    /// `data` and capture `bits` TDO bits back. Returns Vec<u8>, LSB-aligned.
    pub fn shift_ir_dr(&mut self, ir: u32, bits: usize, data: &[u8]) -> Result<Vec<u8>> {
        self.reset_tap()?;
        self.shift_ir(ir)?;
        self.shift_dr(bits, data)
    }

    /// Shift the IR (assumes we're at RunTestIdle).
    fn shift_ir(&mut self, ir: u32) -> Result<()> {
        self.nav_idle_to_shift_ir()?;
        self.shift_payload(self.ir_width as usize, &ir.to_le_bytes(), false)?;
        self.nav_exit1_to_idle()
    }

    /// Shift the DR (assumes we're at RunTestIdle).
    fn shift_dr(&mut self, bits: usize, data: &[u8]) -> Result<Vec<u8>> {
        self.nav_idle_to_shift_dr()?;
        let out = self.shift_payload(bits, data, true)?;
        self.nav_exit1_to_idle()?;
        Ok(out)
    }

    /// Core shift primitive. Sends `bits` bits of `data` over TDI, captures
    /// `bits` TDO bits if `read` is true, and uses TMS=1 on the final bit to
    /// exit the Shift state into Exit1.
    ///
    /// Splits naturally into full-byte chunks for `bits >= 16`, then a
    /// sub-byte BITS_OUT/BITS_INOUT for any leftover, then a TMS clock for the
    /// last bit (which also carries TDI).
    fn shift_payload(&mut self, bits: usize, data: &[u8], read: bool) -> Result<Vec<u8>> {
        if bits == 0 {
            return Ok(Vec::new());
        }
        // Reserve one bit for the TMS-exit clock.
        let mut remaining = bits - 1;
        let mut consumed = 0usize;

        let mut prog: Vec<u8> = Vec::new();

        // Full bytes (each = 8 bits).
        let full_bytes = remaining / 8;
        if full_bytes > 0 {
            let opcode = if read {
                cmd::SHIFT_BYTES_INOUT
            } else {
                cmd::SHIFT_BYTES_OUT
            };
            let len_minus_one = (full_bytes - 1) as u16;
            prog.push(opcode);
            prog.push((len_minus_one & 0xFF) as u8);
            prog.push(((len_minus_one >> 8) & 0xFF) as u8);
            for i in 0..full_bytes {
                prog.push(data.get(i).copied().unwrap_or(0));
            }
            consumed += full_bytes * 8;
            remaining -= full_bytes * 8;
        }

        // Sub-byte BITS (1..=7) excluding the final TMS-exit bit.
        if remaining > 0 {
            let opcode = if read {
                cmd::SHIFT_BITS_INOUT
            } else {
                cmd::SHIFT_BITS_OUT
            };
            prog.push(opcode);
            prog.push((remaining - 1) as u8);
            let byte = sub_byte(data, consumed, remaining);
            prog.push(byte);
            consumed += remaining;
        }

        // Final bit via TMS=1 to exit Shift state, with TDI carried in bit 7.
        let _ = consumed;
        let last_tdi = bit_at(data, bits - 1);
        let opcode = if read { cmd::TMS_INOUT } else { cmd::TMS_OUT };
        prog.push(opcode);
        prog.push(0); // length-1 = 0 means 1 clock
        prog.push(0x01 | ((last_tdi as u8) << 7));

        if read {
            prog.push(cmd::SEND_IMMEDIATE);
        }
        self.backend.write_all(&prog)?;

        if !read {
            return Ok(Vec::new());
        }

        // Decode response.
        let mut resp_len = 0usize;
        if full_bytes > 0 {
            resp_len += full_bytes;
        }
        let leftover_bits = bits - 1 - full_bytes * 8;
        if leftover_bits > 0 {
            resp_len += 1; // sub-byte response
        }
        resp_len += 1; // TMS_INOUT byte
        let mut resp = vec![0u8; resp_len];
        self.backend.read_exact(&mut resp)?;

        let mut out = vec![0u8; bits.div_ceil(8)];
        let mut cursor = 0usize;
        let mut bit_idx = 0usize;

        // Full-byte response: each byte is direct LSB-first capture.
        for i in 0..full_bytes {
            let byte = resp[cursor + i];
            for bit_in_byte in 0..8 {
                let v = (byte >> bit_in_byte) & 1 == 1;
                if v {
                    out[bit_idx / 8] |= 1 << (bit_idx % 8);
                }
                bit_idx += 1;
            }
        }
        cursor += full_bytes;

        // Sub-byte response: LSB-justified at bit position (8 - leftover_bits).
        if leftover_bits > 0 {
            let byte = resp[cursor];
            let raw = byte >> (8 - leftover_bits);
            for bit_in_byte in 0..leftover_bits {
                let v = (raw >> bit_in_byte) & 1 == 1;
                if v {
                    out[bit_idx / 8] |= 1 << (bit_idx % 8);
                }
                bit_idx += 1;
            }
            cursor += 1;
        }

        // TMS_INOUT response: single bit at bit 7.
        let last_byte = resp[cursor];
        let last_v = (last_byte >> 7) & 1 == 1;
        if last_v {
            out[bit_idx / 8] |= 1 << (bit_idx % 8);
        }

        Ok(out)
    }
}

fn bit_at(data: &[u8], idx: usize) -> bool {
    data.get(idx / 8)
        .map(|b| (b >> (idx % 8)) & 1 == 1)
        .unwrap_or(false)
}

/// Pack `n_bits` (<= 8) starting at bit `start` from `data` into a single
/// LSB-first byte. Bits beyond `data` are 0.
fn sub_byte(data: &[u8], start: usize, n_bits: usize) -> u8 {
    debug_assert!(n_bits <= 8);
    let mut byte = 0u8;
    for i in 0..n_bits {
        if bit_at(data, start + i) {
            byte |= 1 << i;
        }
    }
    byte
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ftdi::mock::MockMpsse;

    /// Helper: open an MpsseJtag wrapping a MockMpsse pre-loaded with the
    /// init-sync echo.
    fn jtag_with_sync(read_responses: Vec<u8>) -> MpsseJtag<MockMpsse> {
        let mut mock = MockMpsse::new();
        // init() will write [0xAA] and read 2 bytes back, then write init seq.
        mock.queue_read([0xFA, 0xAA]);
        for b in read_responses {
            mock.queue_read([b]);
        }
        let mut j = MpsseJtag::new(mock, 5);
        j.init(0x05).expect("init");
        j
    }

    #[test]
    fn init_sequence_writes_expected_setup_bytes() {
        let mut mock = MockMpsse::new();
        mock.queue_read([0xFA, 0xAA]);
        let mut j = MpsseJtag::new(mock, 5);
        j.init(0x05).expect("init");

        let writes = j.backend().writes_concatenated();
        // First write: sync test (0xAA).
        // Then init: LOOPBACK_OFF, CLOCK_DIVIDE_5_ENABLE, SET_TCK_DIVISOR, 0x05, 0x00,
        //            SET_BITS_LOW, 0x08, 0x0B
        assert_eq!(
            writes,
            vec![
                0xAA, // sync
                cmd::LOOPBACK_OFF,
                cmd::CLOCK_DIVIDE_5_ENABLE,
                cmd::SET_TCK_DIVISOR,
                0x05,
                0x00,
                cmd::SET_BITS_LOW,
                0x08,
                0x0B,
            ]
        );
    }

    #[test]
    fn init_rejects_sync_mismatch() {
        let mut mock = MockMpsse::new();
        mock.queue_read([0x00, 0x00]);
        let mut j = MpsseJtag::new(mock, 5);
        let err = j.init(0x05).unwrap_err();
        assert!(matches!(err, TransportError::Protocol(_)));
    }

    #[test]
    fn reset_tap_emits_six_tms_high() {
        let mut j = jtag_with_sync(vec![]);
        j.backend_mut().clear_writes();
        j.reset_tap().expect("reset");
        let writes = j.backend().writes_concatenated();
        assert_eq!(writes, vec![cmd::TMS_OUT, 6, 0b0011_1111]);
        assert_eq!(j.tap_state(), TapState::RunTestIdle);
    }

    #[test]
    fn scan_idcode_pass_through_0xdeadbeef() {
        // 0xDEADBEEF in little-endian byte order: EF BE AD DE
        // Response sequence we'll feed:
        //   sync: FA AA
        //   shift_dr_idcode reads 5 bytes:
        //     bytes 0..2 = 0xEF, 0xBE, 0xAD (first 24 bits LSB-first)
        //     7-bit response: 0xDE >> 1 = 0x6F shifted up by 1 = MSB-justified
        //       0xDE = 1101_1110, bits 0..6 are 0,1,1,1,1,0,1 (LSB first) = 0b0101_1110 raw
        //       MSB-justified to bit 7: shift left by 1 -> 0xBC? Let me recompute.
        //
        //       Bits of 0xDE (bit0..bit7) = 0,1,1,1,1,0,1,1
        //       We need bits 24..30 of IDCODE which are bits 0..6 of 0xDE:
        //         0,1,1,1,1,0,1
        //       These need to be encoded such that (response >> (8-7)) recovers them.
        //       If response = 0bXXXX_XXXY where Y is unused and bits 1..7 are bits
        //       0..6 in LSB-first, then:
        //         response bits 1..7 (LSB->MSB) = 0,1,1,1,1,0,1
        //         response = 0b1011_1100 = 0xBC
        //       Verify: 0xBC >> 1 = 0x5E = 0b0101_1110, bits 0..6 = 0,1,1,1,1,0,1. Match.
        //     1-bit response: bit 7 of 0xDE = 1, encoded as bit 7 of the byte = 0x80.
        let mut mock = MockMpsse::new();
        mock.queue_read([0xFA, 0xAA]);
        mock.queue_read([0xEF, 0xBE, 0xAD, 0xBC, 0x80]);

        let mut j = MpsseJtag::new(mock, 5);
        j.init(0x05).expect("init");
        let codes = j.scan_idcode().expect("idcode");
        assert_eq!(codes, vec![0xDEADBEEFu32]);
    }

    #[test]
    fn scan_idcode_floating_returns_empty() {
        let mut mock = MockMpsse::new();
        mock.queue_read([0xFA, 0xAA]);
        mock.queue_read([0x00, 0x00, 0x00, 0x00, 0x00]);

        let mut j = MpsseJtag::new(mock, 5);
        j.init(0x05).expect("init");
        let codes = j.scan_idcode().expect("idcode");
        assert!(codes.is_empty());
    }

    #[test]
    fn shift_ir_dr_round_trip_one_byte() {
        // Shift IR=0x05 (BYPASS in many designs, doesn't matter for the test),
        // then DR with 8 bits of 0xA5. The mock will reply with TDO=0x5A.
        // We need to size the response queue to match what shift_payload reads.
        //
        // For 8-bit DR shift: full_bytes=0 (because bits-1=7), leftover_bits=7, +1 TMS bit.
        // So shift_dr response = 1 byte (BITS_INOUT) + 1 byte (TMS_INOUT) = 2 bytes.
        //
        // Need TDO LSB-first = 0x5A = 0b0101_1010. Bits 0..7 = 0,1,0,1,1,0,1,0.
        // For BITS_INOUT(7 bits = bits 0..6 of TDO): bits = 0,1,0,1,1,0,1.
        //   Encoded MSB-justified: bit at position 7-i, so:
        //     bit0=0 → position 1
        //     bit1=1 → position 2
        //     bit2=0 → position 3
        //     bit3=1 → position 4
        //     bit4=1 → position 5
        //     bit5=0 → position 6
        //     bit6=1 → position 7
        //   Building from bit positions: 0b1011_0100 then verify (>> 1) = 0b0101_1010 yes? Let me recompute.
        //   Actually engine spec: "data is shifted into the register and the register is then shifted right by (8-length) bits"
        //   So if bits clocked in (LSB first) are b0,b1,b2,b3,b4,b5,b6, they end up at positions (7,6,5,4,3,2,1) of the register
        //   after right-shift by 1: positions (6,5,4,3,2,1,0).
        //   Reading the response byte: bit 6=b0=0, bit 5=b1=1, bit 4=b2=0, bit 3=b3=1, bit 2=b4=1, bit 1=b5=0, bit 0=b6=1
        //   = 0b0_0101101 = 0x2D? Let me just verify: bits (7..0) = (0,0,1,0,1,1,0,1) = 0x2D
        //   Then (0x2D >> (8 - 7)) = (0x2D >> 1) = 0x16 = 0b0001_0110
        //   Bits 0..6 of 0x16 = 0,1,1,0,1,0,0 → that's NOT 0,1,0,1,1,0,1. So my MSB-justification logic is wrong.
        //
        // Let me try a different encoding: just have the response be b0..b6 LSB-first WITHIN the LOW 7 bits of the byte.
        //   So response = 0x__ where bit i = b_i for i=0..6.
        //   We want b0..b6 = 0,1,0,1,1,0,1.
        //   response = 0b_0101_1010 & 0x7F = 0x5A & 0x7F = 0x5A.
        //   Now my decoder does (resp_byte >> (8 - n_bits)) = (0x5A >> 1) = 0x2D. Then unpacks bits 0..6 of 0x2D = 1,0,1,1,0,1,0. WRONG.
        //
        // Hmm. The engine apparently shifts MSB-first into LSB-first within a byte. Let me consult AN_108 more carefully...
        // Actually the FTDI engine for BITS_INOUT with LSB-first sends each bit into bit 7 then shifts the register RIGHT by 1.
        // After N bits, the bits are at positions (7-N+1)..(7) with bit 0 of input at position 7-N+1 and bit N-1 at position 7.
        // So for N=7 and inputs b0..b6 (LSB first clocked):
        //   register after right-shift by (8-7) = 1:
        //     position 7 = b6 (last clocked, at top after shift)
        //     position 6 = b5
        //     ...
        //     position 1 = b0
        //     position 0 = whatever was there (random/0)
        //   So encoded byte = (b6 << 7) | (b5 << 6) | ... | (b0 << 1) | 0
        //
        // My decoder does (byte >> (8 - n_bits)) = byte >> 1. For the bits above:
        //   shifted = (b6 << 6) | (b5 << 5) | ... | (b0 << 0)
        //   Then I extract bits 0..6 LSB-first: bit0=b0, bit1=b1, ..., bit6=b6. CORRECT.
        //
        // So for TDO bits 0..6 = 0,1,0,1,1,0,1:
        //   encoded_byte = (1<<7) | (0<<6) | (1<<5) | (1<<4) | (0<<3) | (1<<2) | (0<<1) | 0
        //                = 1011_0100 = 0xB4
        //   Verify: (0xB4 >> 1) = 0x5A, bits 0..6 of 0x5A = 0,1,0,1,1,0,1. YES.

        let mut mock = MockMpsse::new();
        mock.queue_read([0xFA, 0xAA]);
        // shift_ir reads: ir_width=5, bits-1=4. full_bytes=0, leftover_bits=4, TMS=1.
        //   Response: 1 sub-byte (4 bits) + 1 TMS byte = 2 bytes.
        //   But shift_ir uses read=false → no response.
        // Actually shift_ir uses read=false. Let me check... yes:
        //   shift_payload(self.ir_width as usize, &ir.to_le_bytes(), false)
        //   read=false → no response queued.
        //
        // shift_dr (bits=8, read=true):
        //   bits-1 = 7. full_bytes = 0. leftover_bits = 7.
        //   Response: 1 byte (BITS_INOUT) + 1 byte (TMS_INOUT) = 2 bytes.
        //   For TDO=0x5A: bits 0..6 = 0,1,0,1,1,0,1, bit 7 = 0.
        //   BITS_INOUT byte = 0xB4 (computed above).
        //   TMS_INOUT byte = 0x00 (bit 7 = bit 7 of TDO = 0).
        mock.queue_read([0xB4, 0x00]);

        let mut j = MpsseJtag::new(mock, 5);
        j.init(0x05).expect("init");
        let tdo = j.shift_ir_dr(0x05, 8, &[0xA5]).expect("shift");
        assert_eq!(tdo, vec![0x5A]);
    }
}
