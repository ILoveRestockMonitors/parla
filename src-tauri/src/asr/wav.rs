/// Encode the only audio contract accepted by local ASR backends.
pub fn pcm16_mono_16k(pcm: &[i16]) -> Vec<u8> {
    let data_len = pcm.len().saturating_mul(2);
    let mut b = Vec::with_capacity(44 + data_len);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&16_000u32.to_le_bytes());
    b.extend_from_slice(&32_000u32.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(data_len as u32).to_le_bytes());
    for sample in pcm {
        b.extend_from_slice(&sample.to_le_bytes());
    }
    b
}

#[cfg(test)]
mod tests {
    #[test]
    fn header_is_canonical() {
        let w = super::pcm16_mono_16k(&[1, -2]);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(&w[20..24], &[1, 0, 1, 0]);
        assert_eq!(&w[24..28], &[0x80, 0x3e, 0, 0]);
    }
}
