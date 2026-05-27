//! In-memory `MpsseBackend` for unit tests. Records all writes and serves
//! reads from a pre-queued FIFO of bytes.

use std::collections::VecDeque;

use crate::error::TransportError;
use crate::ftdi::mpsse::MpsseBackend;
use crate::traits::Result;

#[derive(Default)]
pub struct MockMpsse {
    written: Vec<Vec<u8>>,
    pending_reads: VecDeque<u8>,
}

impl MockMpsse {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append bytes to the queue that will be returned by future `read_exact`.
    pub fn queue_read<I: IntoIterator<Item = u8>>(&mut self, bytes: I) {
        for b in bytes {
            self.pending_reads.push_back(b);
        }
    }

    /// Inspect everything written, as a single flat byte vector.
    pub fn writes_concatenated(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for chunk in &self.written {
            out.extend_from_slice(chunk);
        }
        out
    }

    /// Inspect the individual write chunks in order.
    pub fn writes(&self) -> &[Vec<u8>] {
        &self.written
    }

    pub fn clear_writes(&mut self) {
        self.written.clear();
    }

    pub fn pending_read_len(&self) -> usize {
        self.pending_reads.len()
    }
}

impl MpsseBackend for MockMpsse {
    fn write_all(&mut self, data: &[u8]) -> Result<()> {
        self.written.push(data.to_vec());
        Ok(())
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        for slot in buf.iter_mut() {
            *slot = self.pending_reads.pop_front().ok_or_else(|| {
                TransportError::Protocol("mock mpsse: read_exact ran out of queued bytes".into())
            })?;
        }
        Ok(())
    }
}
