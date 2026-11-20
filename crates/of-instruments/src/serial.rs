//! UI-free serial monitor byte log and queued host transmit buffer.

use of_core::SimTime;

/// Direction of a serial log entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    /// Bytes received from the simulated device (UART RX).
    DeviceToHost,
    /// Bytes sent from the host toward the device (UART TX).
    HostToDevice,
}

/// One logged serial payload at a simulation time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Simulation time stamped on this chunk (typically the access instant).
    pub at: SimTime,
    /// Whether this chunk is device-to-host or host-to-device.
    pub dir: Dir,
    /// Raw payload bytes.
    pub data: Vec<u8>,
}

/// How host transmits reach the device in v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// Bytes are queued for the integration layer to drain; not injected in v1.
    QueuedOnly,
}

/// Host send rejected because the payload was empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("cannot send an empty payload")]
pub enum SendError {
    /// The host attempted to send a zero-length buffer.
    Empty,
}

/// Serial byte log with a separate pending TX queue.
#[derive(Debug, Default)]
pub struct SerialMonitor {
    entries: Vec<Entry>,
    pending_tx: Vec<u8>,
    baud: Option<f64>,
}

impl SerialMonitor {
    /// Maximum total payload bytes retained in the log before dropping oldest entries.
    pub const MAX_PAYLOAD_BYTES: usize = 64 * 1024;

    /// Creates an empty monitor with no baud rate configured.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records bytes received from the device at `at`.
    ///
    /// Empty slices are ignored.
    pub fn push_rx(&mut self, at: SimTime, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.entries.push(Entry {
            at,
            dir: Dir::DeviceToHost,
            data: bytes.to_vec(),
        });
        self.trim_to_cap();
    }

    /// Queues host bytes for transmit and logs a host-to-device entry.
    pub fn try_send(&mut self, at: SimTime, bytes: &[u8]) -> Result<(), SendError> {
        if bytes.is_empty() {
            return Err(SendError::Empty);
        }
        self.entries.push(Entry {
            at,
            dir: Dir::HostToDevice,
            data: bytes.to_vec(),
        });
        self.pending_tx.extend_from_slice(bytes);
        self.trim_to_cap();
        Ok(())
    }

    /// Takes all pending host transmit bytes, leaving the queue empty.
    pub fn drain_tx_queue(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.pending_tx)
    }

    /// Clears the log and pending TX queue; baud and delivery mode are unchanged.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.pending_tx.clear();
    }

    /// Sets the displayed baud rate (informational only in v1).
    pub fn set_baud(&mut self, baud: Option<f64>) {
        self.baud = baud;
    }

    /// Returns the configured baud rate, if any.
    pub fn baud(&self) -> Option<f64> {
        self.baud
    }

    /// Returns how host sends are delivered in this version.
    pub fn delivery(&self) -> Delivery {
        Delivery::QueuedOnly
    }

    /// Returns the chronological byte log.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Returns the pending host transmit queue without consuming it.
    pub fn pending_tx(&self) -> &[u8] {
        &self.pending_tx
    }

    /// Builds display lines by splitting on byte `0x0A` (`\n`), then lossy UTF-8 per line.
    ///
    /// Raw bytes are accumulated across log entries so multi-byte UTF-8 is not split at
    /// chunk boundaries. Each line's timestamp is the simulation time of the first byte
    /// in that line. Device-to-host and host-to-device streams are not merged: a direction
    /// change flushes any partial line for the previous direction before continuing.
    pub fn display_lines(&self) -> Vec<(SimTime, Dir, String)> {
        let mut out = Vec::new();
        let mut buf: Vec<u8> = Vec::new();
        let mut start = SimTime::ZERO;
        let mut dir: Option<Dir> = None;

        for e in &self.entries {
            if dir != Some(e.dir) {
                if let Some(d) = dir
                    && !buf.is_empty()
                {
                    out.push((start, d, String::from_utf8_lossy(&buf).into_owned()));
                    buf.clear();
                }
                dir = Some(e.dir);
                start = e.at;
            }
            for &b in &e.data {
                if buf.is_empty() {
                    start = e.at;
                }
                if b == b'\n' {
                    out.push((start, e.dir, String::from_utf8_lossy(&buf).into_owned()));
                    buf.clear();
                } else {
                    buf.push(b);
                }
            }
        }
        if let Some(d) = dir
            && !buf.is_empty()
        {
            out.push((start, d, String::from_utf8_lossy(&buf).into_owned()));
        }
        out
    }

    /// Concatenates all device-to-host payload bytes, then decodes once as lossy UTF-8.
    pub fn text_device_to_host(&self) -> String {
        let mut bytes = Vec::new();
        for e in &self.entries {
            if e.dir == Dir::DeviceToHost {
                bytes.extend_from_slice(&e.data);
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn trim_to_cap(&mut self) {
        while self.entries.iter().map(|e| e.data.len()).sum::<usize>() > Self::MAX_PAYLOAD_BYTES {
            self.entries.remove(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use of_core::SimTime;

    #[test]
    fn push_rx_records_device_to_host() {
        let mut m = SerialMonitor::new();
        m.push_rx(SimTime::from_us(100), b"hi");
        assert_eq!(m.entries().len(), 1);
        assert_eq!(m.entries()[0].dir, Dir::DeviceToHost);
        assert_eq!(m.entries()[0].data, b"hi");
        assert_eq!(m.text_device_to_host(), "hi");
    }

    #[test]
    fn try_send_empty_errors_and_leaves_state() {
        let mut m = SerialMonitor::new();
        assert!(matches!(m.try_send(SimTime::ZERO, b""), Err(SendError::Empty)));
        assert!(m.entries().is_empty());
        assert!(m.pending_tx().is_empty());
    }

    #[test]
    fn try_send_queues_and_logs_host_to_device() {
        let mut m = SerialMonitor::new();
        m.try_send(SimTime::from_us(1), b"help\n").unwrap();
        assert_eq!(m.pending_tx(), b"help\n");
        assert_eq!(m.entries()[0].dir, Dir::HostToDevice);
        assert_eq!(m.delivery(), Delivery::QueuedOnly);
        assert_eq!(m.drain_tx_queue(), b"help\n");
        assert!(m.pending_tx().is_empty());
    }

    #[test]
    fn clear_resets_log_and_queue_keeps_baud() {
        let mut m = SerialMonitor::new();
        m.set_baud(Some(115_200.0));
        m.push_rx(SimTime::ZERO, b"x");
        m.try_send(SimTime::ZERO, b"y").unwrap();
        m.clear();
        assert!(m.entries().is_empty());
        assert!(m.pending_tx().is_empty());
        assert_eq!(m.baud(), Some(115_200.0));
    }

    #[test]
    fn history_cap_drops_oldest_payload() {
        let mut m = SerialMonitor::new();
        // Two chunks that together exceed the cap when a third is added.
        let big = vec![b'a'; SerialMonitor::MAX_PAYLOAD_BYTES / 2 + 1];
        m.push_rx(SimTime::from_us(1), &big);
        m.push_rx(SimTime::from_us(2), &big);
        m.push_rx(SimTime::from_us(3), b"z");
        let total: usize = m.entries().iter().map(|e| e.data.len()).sum();
        assert!(total <= SerialMonitor::MAX_PAYLOAD_BYTES);
        assert_eq!(m.entries().last().unwrap().data, b"z");
    }

    #[test]
    fn display_lines_split_on_newline_with_first_byte_timestamp() {
        let mut m = SerialMonitor::new();
        m.push_rx(SimTime::from_us(10), b"oh");
        m.push_rx(SimTime::from_us(11), b"m\n");
        m.push_rx(SimTime::from_us(20), b"forge");
        let lines = m.display_lines();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].0, SimTime::from_us(10));
        assert_eq!(lines[0].1, Dir::DeviceToHost);
        assert_eq!(lines[0].2, "ohm");
        assert_eq!(lines[1].0, SimTime::from_us(20));
        assert_eq!(lines[1].2, "forge");
    }

    #[test]
    fn utf8_spanning_push_rx_chunks() {
        let mut m = SerialMonitor::new();
        m.push_rx(SimTime::from_us(1), &[0xC3]);
        m.push_rx(SimTime::from_us(2), &[0xA9, b'\n']);
        let text = m.text_device_to_host();
        assert!(text.contains('é'), "expected é in {text:?}");
        assert!(!text.contains('\u{FFFD}'), "unexpected replacement char in {text:?}");
        let lines = m.display_lines();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].2, "é");
    }
}
