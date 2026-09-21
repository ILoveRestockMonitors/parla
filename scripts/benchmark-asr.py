"""Measure warm Parakeet inference on a supplied recording, without mic or typing."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import statistics
import time
import wave

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--models", required=True)
    parser.add_argument("--fixture", required=True, type=Path)
    parser.add_argument("--threads", default="1,2,4,8")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    spec = importlib.util.spec_from_file_location("parla_shim", ROOT / "src-tauri/assets/parakeet-shim.py")
    shim = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(shim)
    pcm = shim.wav_bytes_to_f32(args.fixture.read_bytes())
    rows = []
    for count in [int(item) for item in args.threads.split(",")]:
        if not 1 <= count <= 16:
            raise ValueError("threads must be between 1 and 16")
        os.environ["PARLA_PARAKEET_THREADS"] = str(count)
        started = time.perf_counter()
        model = shim.build_recognizer(args.models)
        load_ms = (time.perf_counter() - started) * 1000
        durations = []
        transcripts = []
        for _ in range(5):
            started = time.perf_counter()
            stream = model.create_stream()
            stream.accept_waveform(16000, pcm)
            model.decode_stream(stream)
            durations.append((time.perf_counter() - started) * 1000)
            transcripts.append(stream.result.text.strip())
        if len(set(transcripts)) != 1:
            raise AssertionError("Repeated decodes changed the transcript")
        rows.append({"threads": count, "load_ms": load_ms, "first_decode_ms": durations[0],
                     "warm_median_ms": statistics.median(durations[1:]), "warm_runs_ms": durations[1:],
                     "transcript": transcripts[0]})
        del model
    result = {"scope": "recorded fixture, CPU ASR only; no microphone or automatic typing",
              "logical_cpus": os.cpu_count(), "fixture_seconds": len(pcm) / 16000, "results": rows}
    text = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(text, encoding="utf-8")
    print(text)


if __name__ == "__main__":
    main()
