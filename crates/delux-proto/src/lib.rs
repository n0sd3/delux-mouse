//! Pure encoder/decoder for the Delux M900 PRO 8K vendor HID protocol
//! (receiver 1D57:FA65). No I/O. See docs/protocol/delux-protocol.md.

pub mod buttons;
pub mod event;
pub mod frame;
pub mod polling;
pub mod power_light;
pub mod reset;
pub mod sensor;

pub use buttons::{encode_buttons, ButtonAction, ButtonMap, SLOT_COUNT};
pub use event::{parse_event, BatteryState, Event, EventKind, MODEL_M900PRO_8K_MS4};
pub use frame::{checksum16, frames, Frame, REPORT_LEN};
pub use polling::{encode_polling, PollingRate};
pub use power_light::{encode_power_light, PowerLightConfig};
pub use reset::encode_factory_reset;
pub use sensor::{encode_sensor, DpiStage, Lod, Rgb, SensorConfig};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("empty packet")]
    EmptyPacket,
    #[error("packet too long: {0} bytes")]
    PacketTooLong(usize),
    #[error("{field} out of range: {value}")]
    OutOfRange { field: &'static str, value: i64 },
}

/// Deserialize a fixed-size array, rejecting any other length. (serde's
/// built-in array support silently ignores extra elements.)
pub(crate) fn exact_array<'de, D, T, const N: usize>(d: D) -> Result<[T; N], D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    let v = <Vec<T> as serde::Deserialize>::deserialize(d)?;
    let len = v.len();
    v.try_into()
        .map_err(|_| serde::de::Error::custom(format!("expected {N} elements, found {len}")))
}
