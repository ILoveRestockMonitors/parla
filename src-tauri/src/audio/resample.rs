use std::f64::consts::PI;

/// Bounded offline 64 tap windowed-sinc converter. Input is mono f32.
pub fn resample(input: &[f32], source_rate: u32, target_rate: u32) -> Vec<i16> {
    if input.is_empty() || source_rate == 0 || target_rate == 0 || target_rate > 192000 {
        return Vec::new();
    }
    let quantize = |v: f64| (v * 32767.0).round().clamp(-32768.0, 32767.0) as i16;
    if source_rate == target_rate {
        return input.iter().map(|&v| quantize(v as f64)).collect();
    }
    let mut a = source_rate;
    let mut b = target_rate;
    while b != 0 {
        let remainder = a % b;
        a = b;
        b = remainder;
    }
    let phases = (target_rate / a) as usize;
    let cutoff = 0.475 * (1.0f64).min(target_rate as f64 / source_rate as f64);
    // Rational rates repeat a finite set of fractional offsets. Compute the
    // expensive sin/cos coefficients once, never per output sample.
    let coefficients: Vec<[f64; 64]> = (0..phases)
        .map(|phase| {
            let fraction = ((phase as u64 * source_rate as u64) % target_rate as u64) as f64
                / target_rate as f64;
            std::array::from_fn(|index| {
                let x = fraction - (index as f64 - 32.0);
                if x.abs() > 32.0 {
                    return 0.0;
                }
                let z = 2.0 * cutoff * x;
                let sinc = if z.abs() < 1e-12 {
                    1.0
                } else {
                    (PI * z).sin() / (PI * z)
                };
                let window =
                    0.42 + 0.5 * (PI * x / 32.0).cos() + 0.08 * (2.0 * PI * x / 32.0).cos();
                2.0 * cutoff * sinc * window
            })
        })
        .collect();
    // Interior windows all use the complete kernel. Its normalization is a
    // phase constant; don't sum 64 weights and bounds-check 64 indices for
    // every output sample. Keep the original edge calculation unchanged.
    let normalization: Vec<f64> = coefficients.iter().map(|c| c.iter().sum()).collect();
    let len = (input.len() as u64 * target_rate as u64 / source_rate as u64) as usize;
    let mut out = Vec::with_capacity(len);
    for j in 0..len {
        let center = (j as u64 * source_rate as u64 / target_rate as u64) as isize;
        let weights = &coefficients[j % phases];
        if center >= 32 && center + 32 <= input.len() as isize {
            let samples = &input[center as usize - 32..center as usize + 32];
            // Independent lanes avoid a serial 64-add dependency chain and
            // let the compiler vectorize without CPU-specific instructions.
            let mut sums = [0.0f64; 4];
            for (samples, weights) in samples.chunks_exact(4).zip(weights.chunks_exact(4)) {
                for lane in 0..4 {
                    sums[lane] += samples[lane] as f64 * weights[lane];
                }
            }
            let sum: f64 = sums.iter().sum();
            let norm = normalization[j % phases];
            out.push(quantize(if norm.abs() > 1e-9 { sum / norm } else { 0.0 }));
            continue;
        }
        let mut sum = 0.0;
        let mut norm = 0.0;
        for (k, &weight) in weights.iter().enumerate() {
            let index = center + k as isize - 32;
            if index >= 0 && index < input.len() as isize {
                sum += input[index as usize] as f64 * weight;
                norm += weight;
            }
        }
        out.push(quantize(if norm.abs() > 1e-9 { sum / norm } else { 0.0 }));
    }
    out
}

pub fn downmix_interleaved(input: &[f32], channels: usize) -> Vec<f32> {
    if channels == 0 {
        return Vec::new();
    }
    input
        .chunks_exact(channels)
        .map(|f| f.iter().sum::<f32>() / channels as f32)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duration_and_dc() {
        for r in [8000, 16000, 44100, 48000] {
            let x = vec![0.25; r as usize];
            assert_eq!(resample(&x, r, 16000).len(), 16000);
            assert!(resample(&x, r, 16000)
                .iter()
                .all(|v| (*v as f32 / 32767.0 - 0.25).abs() < 0.02));
        }
    }
    #[test]
    fn split_equivalence() {
        let x: Vec<f32> = (0..48000).map(|i| (i as f32 / 1000.0).sin()).collect();
        assert_eq!(
            resample(&x, 48000, 16000),
            resample(&[&x[..17001], &x[17001..]].concat(), 48000, 16000)
        );
    }
    #[test]
    fn stereo_downmix() {
        assert_eq!(downmix_interleaved(&[1., -1., 0.5, 0.5], 2), vec![0.0, 0.5]);
    }
    #[test]
    fn passband_and_alias_rejection() {
        let make = |hz: f64| {
            (0..48000)
                .map(|i| (2.0 * PI * hz * i as f64 / 48000.0).sin() as f32)
                .collect::<Vec<_>>()
        };
        let rms = |x: &[i16]| {
            (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
        };
        let low = resample(&make(1000.0), 48000, 16000);
        let high = resample(&make(12000.0), 48000, 16000);
        assert!(rms(&low) > 5000.0);
        assert!(rms(&high) < rms(&low) * 0.25);
    }
}
