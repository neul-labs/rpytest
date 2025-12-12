Short answer: if you’re still **running the same Python tests**, a “fast pytest in Rust” is more like a **1.3–2× speedup overall** for real-world suites, with **5–10×** only in very specific “tiny test / heavy framework overhead” scenarios.

Let me unpack that with some numbers.

---

## 1. Where the time actually goes

For a typical `pytest` run, total time ≈

* **Interpreter + env startup**
* **Import & collection**
* **Fixture graph / plugin logic**
* **Actual test bodies**
* **Reporting / teardown**

Only some of this is “pytest overhead” you can beat with Rust. The **test bodies** themselves are still Python, so you can’t accelerate those without changing user code.

Typical split in medium/large projects:

* 10–40%: framework overhead (collection, fixtures, plugins, selection, reporting)
* 60–90%: actual test execution (user Python code, DB hits, HTTP calls, etc.)

Best case, you make the 10–40% part “basically free”. The math then:

* If overhead is 30% of runtime and you shrink it to ~0:

  * Speedup = 1 / (1 − 0.3) ≈ **1.43×**
* If overhead is 50% (small/fast tests dominated by framework stuff):

  * Speedup = 1 / (1 − 0.5) = **2×**
* If overhead is 80% (toy repo, trivial tests):

  * Speedup = 1 / (1 − 0.8) = **5×**

That’s the realistic range.

---

## 2. What a “faster drop-in pytest” *can* actually accelerate

Assuming you want **pytest CLI + test semantics + plugin compatibility** (or close):

### You *can* make faster

1. **Startup & collection**

   * A Rust binary starts much faster than `python -m pytest` for cold runs.
   * You can:

     * cache discovery results,
     * avoid repeated import work across runs (daemon mode),
     * do smarter test selection before spinning up Python.

2. **Test selection / filtering**

   * Marker/keyword filtering in Rust on cached inventory vs re-collection each time.
   * `-k`, `-m`, `--maxfail`, etc. can be implemented in a tight loop.

3. **Parallelism overhead**

   * `pytest-xdist` has a non-trivial Python overhead for coordinating workers.
   * You can:

     * do scheduling in Rust,
     * keep workers hot,
     * use a better protocol & IPC.

4. **Reporting & result aggregation**

   * Summaries, JUnit XML, coverage merging, etc. can be streamed & processed in Rust more cheaply.

All of that is “same tests, same assertions, but less framework tax”.

---

## 3. What you *can’t* magically speed up (without changing the story)

You **don’t** get big wins on:

* DB/HTTP-bound tests.
* Heavy CPU Python code in test bodies or app code.
* Big imports (Django, large frameworks) — a lot of that import time is Python doing work.

So on a backend repo where each test hits Postgres and Redis, your shiny Rust runner might shave 20–30% on orchestration, but the wall clock is still dominated by I/O and Python.

---

## 4. Ballpark by scenario

If you build a Rust-based, pytest-compatible runner that:

* keeps a **daemon** around (no re-importing everything each run),
* maintains a **cached test inventory**,
* uses **Rust-based scheduling & aggregation**,
* spawns Python workers only to actually *execute* tests,

then realistic expectations:

| Scenario                                 | Expected speedup vs plain `pytest` |
| ---------------------------------------- | ---------------------------------- |
| Tiny project, “fast unit tests” only     | **3–5×** (overhead-dominated)      |
| Medium project, mixed unit/integration   | **1.3–2×**                         |
| Large suite dominated by DB/HTTP         | **1.1–1.5×**                       |
| Single test debug run (`pytest path::t`) | Mostly **startup win**, maybe 1.5× |

Anything above ~2× on a serious, IO-heavy CI suite starts to require **semantic changes** (e.g. skipping unaffected tests, synthetic parallelism, test flakiness heuristics) — at which point you’re no longer a “pure drop-in”, you’re adding new behavior.

---

## 5. Where this positions you vs uv

Given that:

* **uv**: already optimises env creation & interpreter management, but just shells out to `pytest`.
* **You**: optimise **pytest’s internal overhead** (discovery, selection, parallelism, reporting) itself.

You’re complementary:

* You can even **use uv under the hood** to provision Python + deps fast,
* then show “our runner vs `uv run pytest`” benchmarks and aim for:

> “Same tests, same plugins, same CLI, **1.5× faster on real suites**, 3–5× on small/unit-test-heavy ones.”

If you’d like, I can sketch a minimal architecture that *now* stays within “drop-in pytest” (no fancy dependency graph), focused solely on achieving that 1.3–2× factor.

