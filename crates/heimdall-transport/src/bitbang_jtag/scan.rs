//! High-level JTAG operations on a BitbangJtagTransport.

use crate::bitbang_jtag::BitbangJtagTransport;
use crate::bitbang_jtag::state::next;
use crate::caps::GpioOps;
use crate::traits::{Result, Transport};

fn clock_once<T>(t: &mut BitbangJtagTransport<T>, tms: bool, tdi: bool) -> Result<bool>
where
    T: Transport + GpioOps + Send + Sync,
{
    t.backend.set(t.pins.tms, tms)?;
    t.backend.set(t.pins.tdi, tdi)?;
    t.backend.set(t.pins.tck, false)?;
    std::thread::sleep(t.clock_delay);
    t.backend.set(t.pins.tck, true)?;
    std::thread::sleep(t.clock_delay);
    let tdo = t.backend.read(t.pins.tdo).unwrap_or(false);
    t.tap = next(t.tap, tms);
    Ok(tdo)
}

pub fn tap_reset<T>(t: &mut BitbangJtagTransport<T>) -> Result<()>
where
    T: Transport + GpioOps + Send + Sync,
{
    // Five TMS=1 clocks unconditionally reach TestLogicReset from any state.
    for _ in 0..5 {
        clock_once(t, true, false)?;
    }
    // Then drop to RunTestIdle.
    clock_once(t, false, false)?;
    Ok(())
}

/// Navigate TAP to a target state via a sequence of TMS bits with TDI=0.
fn navigate<T>(t: &mut BitbangJtagTransport<T>, tms_seq: &[bool]) -> Result<()>
where
    T: Transport + GpioOps + Send + Sync,
{
    for &tms in tms_seq {
        clock_once(t, tms, false)?;
    }
    Ok(())
}

fn goto_shift_dr<T>(t: &mut BitbangJtagTransport<T>) -> Result<()>
where
    T: Transport + GpioOps + Send + Sync,
{
    // From RunTestIdle: 1 -> SelectDR, 0 -> CaptureDR, 0 -> ShiftDR.
    navigate(t, &[true, false, false])
}

fn goto_shift_ir<T>(t: &mut BitbangJtagTransport<T>) -> Result<()>
where
    T: Transport + GpioOps + Send + Sync,
{
    // From RunTestIdle: 1 -> SelectDR, 1 -> SelectIR, 0 -> CaptureIR, 0 -> ShiftIR.
    navigate(t, &[true, true, false, false])
}

fn goto_idle<T>(t: &mut BitbangJtagTransport<T>) -> Result<()>
where
    T: Transport + GpioOps + Send + Sync,
{
    // From Exit1*: 1 -> UpdateDR/IR, 0 -> RunTestIdle.
    navigate(t, &[true, false])
}

pub fn scan_idcode<T>(t: &mut BitbangJtagTransport<T>) -> Result<Vec<u32>>
where
    T: Transport + GpioOps + Send + Sync,
{
    // Reset puts IDCODE in IR by default per JTAG spec.
    tap_reset(t)?;
    goto_shift_dr(t)?;
    // Shift out 32 bits little-endian.
    let mut idcode: u32 = 0;
    for i in 0..32u32 {
        // Last bit raises TMS to exit Shift-DR.
        let tms = i == 31;
        let tdo = clock_once(t, tms, false)?;
        if tdo {
            idcode |= 1 << i;
        }
    }
    goto_idle(t)?;
    if idcode == 0 || idcode == u32::MAX {
        Ok(Vec::new())
    } else {
        Ok(vec![idcode])
    }
}

pub fn shift_ir_dr<T>(
    t: &mut BitbangJtagTransport<T>,
    ir: u32,
    bits: usize,
    data: &[u8],
) -> Result<Vec<u8>>
where
    T: Transport + GpioOps + Send + Sync,
{
    tap_reset(t)?;
    // Shift IR.
    let ir_bits = t.ir_width as usize;
    goto_shift_ir(t)?;
    for i in 0..ir_bits {
        let tms = i == ir_bits - 1;
        let tdi = (ir >> i) & 1 == 1;
        clock_once(t, tms, tdi)?;
    }
    goto_idle(t)?;
    // Shift DR.
    goto_shift_dr(t)?;
    let mut out = vec![0u8; bits.div_ceil(8)];
    for i in 0..bits {
        let tms = i == bits - 1;
        let byte = data.get(i / 8).copied().unwrap_or(0);
        let tdi = (byte >> (i % 8)) & 1 == 1;
        let tdo = clock_once(t, tms, tdi)?;
        if tdo {
            out[i / 8] |= 1 << (i % 8);
        }
    }
    goto_idle(t)?;
    Ok(out)
}
