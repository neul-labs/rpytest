#!/usr/bin/env python3
"""Benchmark script comparing pytest vs rpytest performance."""
import subprocess
import time
import statistics
import sys
import os

# Configuration
BENCHMARK_DIR = "benchmark_suite"
WARMUP_RUNS = 2
MEASURED_RUNS = 5
RPYTEST_BIN = "./target/release/rpytest"
PYTHON = sys.executable

def run_command(cmd: list[str], cwd: str = ".") -> tuple[float, int, str]:
    """Run a command and return (duration, exit_code, output)."""
    start = time.perf_counter()
    result = subprocess.run(
        cmd,
        cwd=cwd,
        capture_output=True,
        text=True
    )
    duration = time.perf_counter() - start
    return duration, result.returncode, result.stdout + result.stderr


def benchmark_pytest(warmup: int, measured: int) -> list[float]:
    """Run pytest benchmark."""
    print("\n=== Benchmarking pytest ===")

    # Warmup runs
    print(f"  Warmup ({warmup} runs)...", end=" ", flush=True)
    for _ in range(warmup):
        run_command([PYTHON, "-m", "pytest", BENCHMARK_DIR, "-q", "--tb=no"])
    print("done")

    # Measured runs
    timings = []
    print(f"  Measuring ({measured} runs):")
    for i in range(measured):
        duration, code, output = run_command(
            [PYTHON, "-m", "pytest", BENCHMARK_DIR, "-q", "--tb=no"]
        )
        timings.append(duration)
        status = "PASS" if code == 0 else f"FAIL({code})"
        print(f"    Run {i+1}: {duration:.3f}s [{status}]")

    return timings


def benchmark_rpytest(warmup: int, measured: int) -> list[float]:
    """Run rpytest benchmark."""
    print("\n=== Benchmarking rpytest ===")

    if not os.path.exists(RPYTEST_BIN):
        print(f"  ERROR: {RPYTEST_BIN} not found!")
        return []

    # Warmup runs (also starts daemon)
    print(f"  Warmup ({warmup} runs)...", end=" ", flush=True)
    for _ in range(warmup):
        run_command([RPYTEST_BIN, BENCHMARK_DIR, "-q", "--tb=no"])
    print("done")

    # Measured runs
    timings = []
    print(f"  Measuring ({measured} runs):")
    for i in range(measured):
        duration, code, output = run_command(
            [RPYTEST_BIN, BENCHMARK_DIR, "-q", "--tb=no"]
        )
        timings.append(duration)
        status = "PASS" if code == 0 else f"FAIL({code})"
        print(f"    Run {i+1}: {duration:.3f}s [{status}]")

    return timings


def benchmark_collection_only() -> tuple[list[float], list[float]]:
    """Benchmark collection only (no execution)."""
    print("\n=== Benchmarking collection only ===")

    pytest_timings = []
    rpytest_timings = []

    # pytest collection
    print("  pytest --collect-only:")
    for i in range(3):
        duration, _, _ = run_command(
            [PYTHON, "-m", "pytest", BENCHMARK_DIR, "--collect-only", "-q"]
        )
        pytest_timings.append(duration)
        print(f"    Run {i+1}: {duration:.3f}s")

    # rpytest collection
    print("  rpytest --collect-only:")
    for i in range(3):
        duration, _, _ = run_command(
            [RPYTEST_BIN, BENCHMARK_DIR, "--collect-only", "-q"]
        )
        rpytest_timings.append(duration)
        print(f"    Run {i+1}: {duration:.3f}s")

    return pytest_timings, rpytest_timings


def print_results(pytest_timings: list[float], rpytest_timings: list[float], label: str):
    """Print benchmark results comparison."""
    if not pytest_timings or not rpytest_timings:
        print(f"\n{label}: Insufficient data")
        return

    pytest_mean = statistics.mean(pytest_timings)
    pytest_stdev = statistics.stdev(pytest_timings) if len(pytest_timings) > 1 else 0

    rpytest_mean = statistics.mean(rpytest_timings)
    rpytest_stdev = statistics.stdev(rpytest_timings) if len(rpytest_timings) > 1 else 0

    speedup = pytest_mean / rpytest_mean if rpytest_mean > 0 else 0

    print(f"\n{'='*60}")
    print(f"  {label}")
    print(f"{'='*60}")
    print(f"  pytest:   {pytest_mean:.3f}s (± {pytest_stdev:.3f}s)")
    print(f"  rpytest:  {rpytest_mean:.3f}s (± {rpytest_stdev:.3f}s)")
    print(f"  Speedup:  {speedup:.2f}x")
    if speedup > 1:
        print(f"  Result:   rpytest is {speedup:.2f}x FASTER")
    elif speedup < 1:
        print(f"  Result:   rpytest is {1/speedup:.2f}x SLOWER")
    else:
        print(f"  Result:   Same performance")
    print(f"{'='*60}")


def main():
    print("="*60)
    print("  rpytest vs pytest Benchmark")
    print("="*60)
    print(f"  Test suite: {BENCHMARK_DIR}")
    print(f"  Python: {PYTHON}")
    print(f"  rpytest: {RPYTEST_BIN}")
    print(f"  Warmup runs: {WARMUP_RUNS}")
    print(f"  Measured runs: {MEASURED_RUNS}")

    # Count tests
    result = subprocess.run(
        [PYTHON, "-m", "pytest", BENCHMARK_DIR, "--collect-only", "-q"],
        capture_output=True, text=True
    )
    for line in result.stdout.split('\n'):
        if 'test' in line.lower() and 'collected' in line.lower():
            print(f"  Tests: {line.strip()}")
            break

    # Full test run benchmark
    pytest_full = benchmark_pytest(WARMUP_RUNS, MEASURED_RUNS)
    rpytest_full = benchmark_rpytest(WARMUP_RUNS, MEASURED_RUNS)

    # Collection-only benchmark
    pytest_collect, rpytest_collect = benchmark_collection_only()

    # Print results
    print("\n" + "="*60)
    print("  RESULTS SUMMARY")
    print("="*60)

    print_results(pytest_full, rpytest_full, "Full Test Run (480 tests)")
    print_results(pytest_collect, rpytest_collect, "Collection Only")

    # Calculate time saved
    if pytest_full and rpytest_full:
        pytest_mean = statistics.mean(pytest_full)
        rpytest_mean = statistics.mean(rpytest_full)
        time_saved = pytest_mean - rpytest_mean
        if time_saved > 0:
            print(f"\n  Time saved per run: {time_saved:.3f}s")
            print(f"  Time saved over 100 runs: {time_saved * 100:.1f}s ({time_saved * 100 / 60:.1f} min)")


if __name__ == "__main__":
    main()
