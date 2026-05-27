//! JTAG TAP state machine.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapState {
    TestLogicReset,
    RunTestIdle,
    SelectDR,
    CaptureDR,
    ShiftDR,
    Exit1DR,
    PauseDR,
    Exit2DR,
    UpdateDR,
    SelectIR,
    CaptureIR,
    ShiftIR,
    Exit1IR,
    PauseIR,
    Exit2IR,
    UpdateIR,
}

/// Standard IEEE 1149.1 TAP state transitions.
pub fn next(state: TapState, tms: bool) -> TapState {
    use TapState::*;
    match (state, tms) {
        (TestLogicReset, false) => RunTestIdle,
        (TestLogicReset, true) => TestLogicReset,
        (RunTestIdle, false) => RunTestIdle,
        (RunTestIdle, true) => SelectDR,
        (SelectDR, false) => CaptureDR,
        (SelectDR, true) => SelectIR,
        (CaptureDR, false) => ShiftDR,
        (CaptureDR, true) => Exit1DR,
        (ShiftDR, false) => ShiftDR,
        (ShiftDR, true) => Exit1DR,
        (Exit1DR, false) => PauseDR,
        (Exit1DR, true) => UpdateDR,
        (PauseDR, false) => PauseDR,
        (PauseDR, true) => Exit2DR,
        (Exit2DR, false) => ShiftDR,
        (Exit2DR, true) => UpdateDR,
        (UpdateDR, false) => RunTestIdle,
        (UpdateDR, true) => SelectDR,
        (SelectIR, false) => CaptureIR,
        (SelectIR, true) => TestLogicReset,
        (CaptureIR, false) => ShiftIR,
        (CaptureIR, true) => Exit1IR,
        (ShiftIR, false) => ShiftIR,
        (ShiftIR, true) => Exit1IR,
        (Exit1IR, false) => PauseIR,
        (Exit1IR, true) => UpdateIR,
        (PauseIR, false) => PauseIR,
        (PauseIR, true) => Exit2IR,
        (Exit2IR, false) => ShiftIR,
        (Exit2IR, true) => UpdateIR,
        (UpdateIR, false) => RunTestIdle,
        (UpdateIR, true) => SelectDR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_via_five_tms_high() {
        let mut s = TapState::ShiftDR;
        for _ in 0..5 {
            s = next(s, true);
        }
        assert_eq!(s, TapState::TestLogicReset);
    }

    #[test]
    fn idle_to_shift_dr() {
        let mut s = TapState::RunTestIdle;
        s = next(s, true);
        s = next(s, false);
        s = next(s, false);
        assert_eq!(s, TapState::ShiftDR);
    }

    #[test]
    fn idle_to_shift_ir() {
        let mut s = TapState::RunTestIdle;
        s = next(s, true);
        s = next(s, true);
        s = next(s, false);
        s = next(s, false);
        assert_eq!(s, TapState::ShiftIR);
    }
}
