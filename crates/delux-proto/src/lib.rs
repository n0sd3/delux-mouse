//! Pure encoder/decoder for the Delux M900 PRO 8K vendor HID protocol
//! (receiver 1D57:FA65). No I/O. See docs/protocol/delux-protocol.md.

pub mod frame;

pub use frame::{checksum16, frames, Frame, REPORT_LEN};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("empty packet")]
    EmptyPacket,
    #[error("packet too long: {0} bytes")]
    PacketTooLong(usize),
    #[error("{field} out of range: {value}")]
    OutOfRange { field: &'static str, value: i64 },
}
