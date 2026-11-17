// SPDX-License-Identifier: GPL-3.0-only
//! Settings of an AF_XDP port and its queues, with the defaults that spike S3
//! chose.

/// Bytes that the kernel keeps in front of every received frame
/// (`XDP_PACKET_HEADROOM`). A frame must fit in the rest of its chunk.
pub const XDP_PACKET_HEADROOM: u32 = 256;

/// How a queue binds to the network device.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BindMode {
    /// Zero copy when the driver supports it and copy mode otherwise. The
    /// kernel decides at bind time, and [`crate::AfXdpQueue::bind_mode`] reads
    /// the result from the socket.
    #[default]
    Auto,
    /// Zero copy only. The bind fails when the driver cannot do it.
    ZeroCopy,
    /// Copy mode only. The kernel copies every frame between its own buffers
    /// and the UMEM.
    Copy,
}

/// The mode that a bound socket reports through `XDP_OPTIONS`, which is the
/// mode that the queue really runs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BoundMode {
    /// The driver writes and reads frames in the UMEM directly.
    ZeroCopy,
    /// The kernel copies frames.
    Copy,
}

/// Settings of one queue. The defaults come from spike S3, see
/// [`QueueConfig::default`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueConfig {
    /// Bytes of one UMEM frame, a power of two from 2048 up to the page size.
    pub frame_size: u32,
    /// Entries of each of the four rings, a power of two. The queue owns this
    /// many frames for receiving and this many for sending.
    pub ring_size: u32,
    /// Frames per receive or send call that [`crate::Workers`] asks for.
    pub batch: usize,
    /// The requested bind mode.
    pub mode: BindMode,
}

impl Default for QueueConfig {
    /// Frames of 2048 bytes, rings of 2048 entries, batches of 32, and the
    /// automatic bind mode.
    ///
    /// The values follow the S3 runs (`spikes/s3-afxdp/RESULTS.md`). The
    /// DPDK `testpmd` that S3 ran forwarded 32 packets per burst, and
    /// reached 10 Gbps of 1,500 byte frames with one or two queues, so a
    /// queue is one worker. Default mode did not beat forced copy mode on
    /// virtio-net in a way that exceeded the noise of the host, so zero copy
    /// is a bonus where a driver offers it, and the automatic mode costs
    /// nothing where it does not. A 2048 byte frame leaves 1792 bytes after the
    /// kernel headroom, which holds a 1500 byte MTU. S3 did not vary the ring
    /// size, so 2048 is a choice that TRN-19 tunes.
    fn default() -> Self {
        Self {
            frame_size: 2048,
            ring_size: 2048,
            batch: 32,
            mode: BindMode::Auto,
        }
    }
}

/// A setting that the crate refuses.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    /// The frame size is not a power of two in the range that AF_XDP accepts.
    #[error("frame size {0} is not a power of two from 2048 up to the page size")]
    FrameSize(u32),
    /// The ring size is not a power of two in the range that the crate accepts.
    #[error("ring size {0} is not a power of two from 64 to 65536")]
    RingSize(u32),
    /// The batch size is zero or above the ring size.
    #[error("batch size {0} is not between 1 and the ring size")]
    Batch(usize),
    /// The port has no queue.
    #[error("the port has no queue")]
    NoQueues,
    /// A queue number appears twice.
    #[error("queue {0} appears twice")]
    DuplicateQueue(u32),
    /// The interface name is empty or too long for the kernel.
    #[error("interface name {0:?} is empty or longer than 15 bytes")]
    Interface(String),
}

impl QueueConfig {
    /// Checks the settings against `page_size`.
    ///
    /// # Errors
    ///
    /// Returns the first setting that the kernel or this crate would refuse.
    pub fn validate(&self, page_size: u32) -> Result<(), ConfigError> {
        let max_frame = page_size.max(2048);
        if !self.frame_size.is_power_of_two()
            || self.frame_size < 2048
            || self.frame_size > max_frame
        {
            return Err(ConfigError::FrameSize(self.frame_size));
        }
        if !self.ring_size.is_power_of_two() || !(64..=65536).contains(&self.ring_size) {
            return Err(ConfigError::RingSize(self.ring_size));
        }
        if self.batch == 0 || self.batch > self.ring_size as usize {
            return Err(ConfigError::Batch(self.batch));
        }
        Ok(())
    }

    /// The longest frame that a queue with these settings sends and receives.
    #[must_use]
    pub fn max_frame_len(&self) -> usize {
        (self.frame_size - XDP_PACKET_HEADROOM) as usize
    }
}

/// Settings of a port, which is one network interface with one socket per
/// queue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortConfig {
    /// The interface name, for example `eth1`.
    pub interface: String,
    /// The queue numbers to bind. Each one gets its own socket and UMEM.
    pub queues: Vec<u32>,
    /// The settings that every queue shares.
    pub queue: QueueConfig,
}

impl PortConfig {
    /// A port on `interface` with queue 0 and the default queue settings.
    #[must_use]
    pub fn new(interface: impl Into<String>) -> Self {
        Self {
            interface: interface.into(),
            queues: vec![0],
            queue: QueueConfig::default(),
        }
    }

    /// Checks the settings against `page_size`.
    ///
    /// # Errors
    ///
    /// Returns the first setting that the kernel or this crate would refuse.
    pub fn validate(&self, page_size: u32) -> Result<(), ConfigError> {
        if self.interface.is_empty() || self.interface.len() > 15 || self.interface.contains('\0') {
            return Err(ConfigError::Interface(self.interface.clone()));
        }
        if self.queues.is_empty() {
            return Err(ConfigError::NoQueues);
        }
        for (i, q) in self.queues.iter().enumerate() {
            if self.queues[..i].contains(q) {
                return Err(ConfigError::DuplicateQueue(*q));
            }
        }
        self.queue.validate(page_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_on_4k_pages() {
        assert_eq!(PortConfig::new("eth1").validate(4096), Ok(()));
        assert_eq!(QueueConfig::default().max_frame_len(), 1792);
    }

    #[test]
    fn frame_sizes_must_be_powers_of_two_within_the_page() {
        let cfg = |frame_size| QueueConfig {
            frame_size,
            ..QueueConfig::default()
        };
        assert_eq!(cfg(4096).validate(4096), Ok(()));
        assert_eq!(cfg(1024).validate(4096), Err(ConfigError::FrameSize(1024)));
        assert_eq!(cfg(3000).validate(4096), Err(ConfigError::FrameSize(3000)));
        assert_eq!(cfg(8192).validate(4096), Err(ConfigError::FrameSize(8192)));
        assert_eq!(cfg(8192).validate(16384), Ok(()));
    }

    #[test]
    fn ring_and_batch_sizes_are_bounded() {
        let cfg = |ring_size, batch| QueueConfig {
            ring_size,
            batch,
            ..QueueConfig::default()
        };
        assert_eq!(cfg(64, 64).validate(4096), Ok(()));
        assert_eq!(cfg(32, 8).validate(4096), Err(ConfigError::RingSize(32)));
        assert_eq!(cfg(100, 8).validate(4096), Err(ConfigError::RingSize(100)));
        assert_eq!(
            cfg(131_072, 8).validate(4096),
            Err(ConfigError::RingSize(131_072))
        );
        assert_eq!(cfg(64, 0).validate(4096), Err(ConfigError::Batch(0)));
        assert_eq!(cfg(64, 65).validate(4096), Err(ConfigError::Batch(65)));
    }

    #[test]
    fn ports_need_a_name_and_distinct_queues() {
        let mut p = PortConfig::new("eth1");
        p.queues = vec![];
        assert_eq!(p.validate(4096), Err(ConfigError::NoQueues));
        p.queues = vec![0, 1, 0];
        assert_eq!(p.validate(4096), Err(ConfigError::DuplicateQueue(0)));
        assert!(matches!(
            PortConfig::new("").validate(4096),
            Err(ConfigError::Interface(_))
        ));
        assert!(matches!(
            PortConfig::new("a-sixteen-bytes-x").validate(4096),
            Err(ConfigError::Interface(_))
        ));
    }
}
