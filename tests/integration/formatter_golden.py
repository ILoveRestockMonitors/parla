"""Single attempt production formatter quality gate."""
import json, os, subprocess, tempfile, time

CASES = [
    ("numbers", "Send Claude the 3 reports by 5 pm.", lambda x: "3" in x and "5" in x),
    ("negation", "Do not delete the API token.", lambda x: "not" in x.lower() and "api" in x.lower()),
    ("identifier", "Call iPhone APIClient_v2.", lambda x: "iPhone" in x and "APIClient_v2" in x),
    ("cleanup", "Please send the report to Claude.", lambda x: "Claude" in x and "report" in x.lower()),
]

def call(binary, envelope, timeout=60):
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False, encoding="utf-8") as f:
        json.dump(envelope, f)
        path = f.name
    try:
        start = time.perf_counter()
        proc = subprocess.run([binary, "--format-json", path], capture_output=True,
                               text=True, timeout=timeout)
        elapsed = time.perf_counter() - start
        try:
            payload = json.loads(proc.stdout) if proc.stdout.strip() else {}
        except json.JSONDecodeError:
            payload = {}
        return proc.returncode, elapsed, payload, proc.stderr.strip()
    finally:
        os.unlink(path)

def main():
    binary = os.environ.get("PARLA_BIN", "parla")
    results = []
    for name, transcript, check in CASES:
        envelope = {"mode": "dictation", "raw_transcript": transcript,
                    "context": {"app_category": "personal_chat"}, "language": "en-US",
                    "options": {"max_tokens": 256},
                    "vocabulary": ["Claude", "API", "iPhone", "APIClient_v2"]}
        code, elapsed, payload, stderr = call(binary, envelope)
        output = payload.get("text", "") if isinstance(payload, dict) else ""
        passed = code == 0 and isinstance(output, str) and bool(output) and check(output)
        results.append({"id": name, "passed": passed, "elapsed_s": round(elapsed, 3),
                        "output": output, "exit_code": code, "stderr": stderr})
        print(f"{name:>10} | {'PASS' if passed else 'FAIL'} | {elapsed:5.2f}s | {output[:80]!r}")
    out_path = os.environ.get("PARLA_GOLDEN_RESULTS")
    if out_path:
        with open(out_path, "w", encoding="utf-8") as f:
            json.dump(results, f, indent=2)
    score = sum(r["passed"] for r in results)
    print(f"SCORE: {score}/{len(results)}")
    return 0 if score == len(results) else 1

if __name__ == "__main__":
    raise SystemExit(main())
