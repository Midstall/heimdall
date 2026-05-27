//! Linux GPIO character-device backend via the `gpiocdev` crate. Talks to
//! `/dev/gpiochip0` (or any explicit chip path). Implements `Transport` (no-op
//! open/close/reset for `gpiochip`) and `GpioOps`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use gpiocdev::line::Value;
use gpiocdev::request::Request;
use tracing::debug;

use crate::caps::GpioOps;
use crate::error::TransportError;
use crate::traits::{ResetTarget, Result, Transport, TransportKind};

pub struct GpioCdevTransport {
    chip: PathBuf,
    open: bool,
    /// One `Request` per line offset; lines are acquired lazily on first use.
    lines: HashMap<u32, Request>,
    /// Input-only line requests, acquired lazily on first read.
    input_lines: HashMap<u32, Request>,
}

impl GpioCdevTransport {
    pub fn new(chip: impl Into<PathBuf>) -> Self {
        Self {
            chip: chip.into(),
            open: false,
            lines: HashMap::new(),
            input_lines: HashMap::new(),
        }
    }

    fn ensure_line(&mut self, line: u32) -> Result<&Request> {
        if !self.open {
            return Err(TransportError::NotOpen);
        }
        if !self.lines.contains_key(&line) {
            let req = Request::builder()
                .on_chip(&self.chip)
                .with_line(line)
                .as_output(Value::Inactive)
                .request()
                .map_err(|e| TransportError::Io(std::io::Error::other(e)))?;
            self.lines.insert(line, req);
            debug!(chip = %self.chip.display(), line, "gpio line acquired");
        }
        Ok(self.lines.get(&line).expect("just inserted"))
    }
}

#[async_trait]
impl Transport for GpioCdevTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::Gpio
    }

    async fn open(&mut self) -> Result<()> {
        // Lines are acquired lazily in `ensure_line` on first use. Just record
        // that we are open.
        self.open = true;
        debug!(chip = %self.chip.display(), "gpio-cdev open");
        Ok(())
    }

    async fn close(&mut self) -> Result<()> {
        self.lines.clear();
        self.input_lines.clear();
        self.open = false;
        Ok(())
    }

    async fn reset(&mut self, _target: ResetTarget) -> Result<()> {
        // GPIO transport itself cannot trigger a meaningful reset target; the
        // caller composes higher-level transports (e.g. bitbang-jtag) to do
        // that. Return Ok so reset(System) is a no-op when only GPIO is used.
        Ok(())
    }
}

impl GpioOps for GpioCdevTransport {
    fn set(&mut self, line: u32, high: bool) -> Result<()> {
        let v = if high { Value::Active } else { Value::Inactive };
        let req = self.ensure_line(line)?;
        req.set_value(line, v)
            .map_err(|e| TransportError::Io(std::io::Error::other(e)))?;
        Ok(())
    }

    fn pulse(&mut self, line: u32, duration: Duration) -> Result<()> {
        self.set(line, true)?;
        std::thread::sleep(duration);
        self.set(line, false)?;
        Ok(())
    }

    fn read(&mut self, line: u32) -> Result<bool> {
        if !self.open {
            return Err(TransportError::NotOpen);
        }
        if !self.input_lines.contains_key(&line) {
            let req = Request::builder()
                .on_chip(&self.chip)
                .with_line(line)
                .as_input()
                .request()
                .map_err(|e| TransportError::Io(std::io::Error::other(e)))?;
            self.input_lines.insert(line, req);
            debug!(chip = %self.chip.display(), line, "gpio input line acquired");
        }
        let req = self.input_lines.get(&line).expect("just inserted");
        let v = req
            .value(line)
            .map_err(|e| TransportError::Io(std::io::Error::other(e)))?;
        Ok(matches!(v, Value::Active))
    }
}
