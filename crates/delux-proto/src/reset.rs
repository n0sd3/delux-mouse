//! Cmd 0x0C — factory reset.

pub fn encode_factory_reset() -> Vec<u8> {
    vec![0x0C, 0x0A, 0x01, 0xFE, 0x01, 0xFE, 0, 0, 0, 0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_vector() {
        assert_eq!(
            encode_factory_reset(),
            vec![0x0C, 0x0A, 0x01, 0xFE, 0x01, 0xFE, 0, 0, 0, 0]
        );
    }
}
