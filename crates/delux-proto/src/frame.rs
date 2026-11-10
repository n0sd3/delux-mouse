//! Report-ID-4 framing: `04, len+5, chunk_index, inner…, checksum16 BE, zeros`.

use crate::Error;

/// Report length on the wire: report ID + 63 payload bytes.
pub const REPORT_LEN: usize = 64;
pub type Frame = [u8; REPORT_LEN];

/// Inner-packet bytes carried by one frame (64 - 3 header - 2 checksum).
const CHUNK: usize = 59;

/// 16-bit wrapping sum of `bytes`.
pub fn checksum16(bytes: &[u8]) -> u16 {
    bytes.iter().fold(0u16, |acc, b| acc.wrapping_add(*b as u16))
}

/// Wrap an inner packet into one or more report-4 frames.
pub fn frames(inner: &[u8]) -> Result<Vec<Frame>, Error> {
    if inner.is_empty() {
        return Err(Error::EmptyPacket);
    }
    if inner.len() > CHUNK * 256 {
        return Err(Error::PacketTooLong(inner.len()));
    }
    Ok(inner
        .chunks(CHUNK)
        .enumerate()
        .map(|(i, chunk)| {
            let mut f = [0u8; REPORT_LEN];
            f[0] = 0x04;
            f[1] = chunk.len() as u8 + 5;
            f[2] = i as u8;
            let end = 3 + chunk.len();
            f[3..end].copy_from_slice(chunk);
            let cs = checksum16(&f[..end]).to_be_bytes();
            f[end] = cs[0];
            f[end + 1] = cs[1];
            f
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polling_8k_vector() {
        let f = frames(&[0x06, 0x09, 0x01, 0x74, 0x8B, 0, 0, 0, 0]).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(
            &f[0][..14],
            &[0x04, 0x0E, 0x00, 0x06, 0x09, 0x01, 0x74, 0x8B, 0, 0, 0, 0, 0x01, 0x21]
        );
        assert!(f[0][14..].iter().all(|b| *b == 0));
    }

    #[test]
    fn chunked_111_bytes() {
        let inner: Vec<u8> = (0..111u8).collect();
        let f = frames(&inner).unwrap();
        assert_eq!(f.len(), 2);
        assert_eq!(&f[0][..3], &[0x04, 0x40, 0x00]);
        assert_eq!(&f[0][3..62], &inner[..59]);
        assert_eq!(u16::from_be_bytes([f[0][62], f[0][63]]), checksum16(&f[0][..62]));
        assert_eq!(&f[1][..3], &[0x04, 0x39, 0x01]);
        assert_eq!(&f[1][3..55], &inner[59..]);
        assert_eq!(u16::from_be_bytes([f[1][55], f[1][56]]), checksum16(&f[1][..55]));
    }

    #[test]
    fn empty_is_error() {
        assert!(matches!(frames(&[]), Err(Error::EmptyPacket)));
    }

    #[test]
    fn exactly_59_bytes_is_one_frame() {
        assert_eq!(frames(&[1u8; 59]).unwrap().len(), 1);
        assert_eq!(frames(&[1u8; 60]).unwrap().len(), 2);
    }

    #[test]
    fn too_long_is_error() {
        // chunk index is one byte
        assert!(matches!(frames(&vec![0u8; 59 * 256 + 1]), Err(Error::PacketTooLong(_))));
    }
}
