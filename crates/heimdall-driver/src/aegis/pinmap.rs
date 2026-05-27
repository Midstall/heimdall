//! Aegis IO pinmap: which FPGA pad index is wired to which test-host GPIO
//! line, in which direction. Used by AegisFpgaDriver::run / ::observe.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadDirection {
    In,
    Out,
}

#[derive(Debug, Clone, Copy)]
pub struct PadEntry {
    pub direction: PadDirection,
    pub fpga_pad: u32,
    pub gpio_line: u32,
}

#[derive(Debug, Clone, Default)]
pub struct IoPinmap {
    pub entries: Vec<PadEntry>,
}

impl IoPinmap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, e: PadEntry) -> Self {
        self.entries.push(e);
        self
    }

    pub fn inputs(&self) -> impl Iterator<Item = &PadEntry> {
        self.entries
            .iter()
            .filter(|e| matches!(e.direction, PadDirection::In))
    }

    pub fn outputs(&self) -> impl Iterator<Item = &PadEntry> {
        self.entries
            .iter()
            .filter(|e| matches!(e.direction, PadDirection::Out))
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
