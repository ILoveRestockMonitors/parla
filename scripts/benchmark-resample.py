"""Compare the release resampler against a git baseline without mic or models.

Uses synthetic audio at 44.1/48/96 kHz. This measures conversion, NOT overall
dictation latency. Both implementations use the same compiler and process.
"""
import argparse
import json
import pathlib
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--baseline", default="ef823d3aa4dab0f684012a67ffe24cd2fffe270e")
    parser.add_argument("--output", type=pathlib.Path)
    args = parser.parse_args()
    baseline = subprocess.check_output(
        ["git", "show", f"{args.baseline}:src-tauri/src/audio/resample.rs"], cwd=ROOT, text=True
    )
    current = (ROOT / "src-tauri/src/audio/resample.rs").read_text(encoding="utf-8")
    driver = r'''
use std::{hint::black_box, time::Instant};
fn median(f: impl Fn() -> Vec<i16>) -> f64 {
    let mut times = Vec::new();
    for _ in 0..11 { let start = Instant::now(); black_box(f()); times.push(start.elapsed().as_secs_f64()*1000.0); }
    times.sort_by(f64::total_cmp); times[5]
}
fn main() {
    for rate in [44100u32, 48000, 96000] {
        let x: Vec<f32> = (0..rate*10).map(|i| ((i as f64 * 0.093).sin()*0.4 + (i as f64*0.021).sin()*0.2) as f32).collect();
        let old = before::resample(&x, rate, 16000);
        let new = after::resample(&x, rate, 16000);
        assert_eq!(old.len(), new.len());
        let max_error = old.iter().zip(&new).map(|(&a,&b)| (a as i32-b as i32).abs()).max().unwrap();
        assert!(max_error <= 1, "conversion diverged: {}", max_error);
        let before_ms = median(|| before::resample(black_box(&x), rate, 16000));
        let after_ms = median(|| after::resample(black_box(&x), rate, 16000));
        println!("{{\"rate\":{},\"seconds\":10,\"before_ms\":{:.4},\"after_ms\":{:.4},\"speedup\":{:.3},\"max_pcm_difference\":{}}}", rate, before_ms, after_ms, before_ms/after_ms,max_error);
    }
}
'''
    with tempfile.TemporaryDirectory(prefix="parla-resample-benchmark-") as folder:
        path = pathlib.Path(folder)
        source = path / "bench.rs"
        source.write_text("mod before {\n" + baseline + "\n}\nmod after {\n" + current + "\n}\n" + driver, encoding="utf-8")
        binary = path / ("bench.exe" if __import__("os").name == "nt" else "bench")
        subprocess.run(["rustc", "--edition=2021", "-O", str(source), "-o", str(binary)], check=True)
        records = [json.loads(line) for line in subprocess.check_output([str(binary)], text=True).splitlines()]
    result = {"baseline": args.baseline, "scope": "synthetic resampling only; no ASR, microphone or injection", "results": records}
    text = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(text, encoding="utf-8")
    print(text)


if __name__ == "__main__":
    main()
