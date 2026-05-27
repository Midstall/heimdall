//! Read RISC-V architectural state (GPRs and PC) into a heimdall_core::State.

use heimdall_core::{State, ValueRepr};
use heimdall_transport::openocd::OpenocdRpc;

use super::debug_module::DebugModule;

/// Read x1..x31 and PC into a State snapshot. Call only when the CPU is
/// halted.
pub async fn snapshot_xregs_pc<T: OpenocdRpc>(
    jtag: &mut T,
) -> Result<State, heimdall_transport::TransportError> {
    let mut dm = DebugModule::new(jtag);
    let mut state = State::new();
    for i in 1..32u8 {
        let v = dm.read_gpr(i).await?;
        if v != 0 {
            state = state.with(format!("x{i}"), ValueRepr::U64(v));
        }
    }
    let pc = dm.read_csr("pc").await?;
    state = state.with("pc", ValueRepr::U64(pc));
    Ok(state)
}
