//! Read RISC-V architectural state (GPRs and PC) into a heimdall_core::State.

use heimdall_core::{State, ValueRepr};
use heimdall_transport::openocd::OpenocdRpc;

use super::debug_module::{CSR_REGNO_DPC, DebugModule};

/// Read x1..x31 and PC into a State snapshot. Call only when the CPU is
/// halted. GPRs are stored under HARDWARE names (`x1`..`x31`), the
/// canonical Heimdall convention. Frontends translate to ABI names
/// (`ra`, `sp`, `fp`, `a0`, ...) at render time via the `gpr_abi_name`
/// helper on `DebugModule`. Every register is recorded, including
/// zero-valued ones, so a "missing" key in the diff genuinely means
/// the observe never completed rather than "register happened to be
/// zero".
///
/// **DMI-bypass reads.** The observe path used to go through OpenOCD's
/// `reg <name>` command, which is mediated by OpenOCD's register cache.
/// Some OpenOCD builds (verified against nixpkgs openocd-0.12.0,
/// 2026-06-02) don't invalidate that cache on a target's self-halt via
/// `ebreak`: `reg <name>` returns the value the cache last sampled and
/// never issues an access-register command on the wire. To stay
/// independent of that quirk, every read here goes through
/// [`DebugModule::read_register_via_abstract`], which writes the DM's
/// `command` register, polls `abstractcs.busy`, checks `cmderr`, and
/// reads `data0`/`data1` directly. The bytes are whatever the DM
/// holds *now*, regardless of OpenOCD's cache state. PC comes from the
/// `dpc` CSR (0x7b1) because the architectural PC has no abstract-
/// command regno of its own. It's only well-defined to read while the
/// core is halted in Debug Mode, which `run()` already guarantees.
/// Failures from the bypass propagate (not swallowed) since observe
/// builds the verdict input. Silently masking a transport-level error
/// here would taint the diff and downstream coverage/divergence work.
pub async fn snapshot_xregs_pc<T: OpenocdRpc>(
    jtag: &mut T,
) -> Result<State, heimdall_transport::TransportError> {
    let mut dm = DebugModule::new(jtag);
    let mut state = State::new();
    for i in 1..32u8 {
        let v = dm.read_gpr_via_abstract(i).await?;
        state = state.with(format!("x{i}"), ValueRepr::U64(v));
    }
    let pc = dm.read_register_via_abstract(CSR_REGNO_DPC).await?;
    state = state.with("pc", ValueRepr::U64(pc));
    Ok(state)
}
