# Native Collector Improvement Plan

## Goal
Improve the native test collector to match pytest's test discovery capabilities, reducing the collection gap from ~89 tests to near-zero.

## Current Issues

| Issue | Missing Tests | Cause |
|-------|---------------|-------|
| Async functions | ~10 | Only handles `FunctionDef`, not `AsyncFunctionDef` |
| Parametrized expansion | ~70 | Adds marker but doesn't create individual test nodes |
| Skip/xfail at collection | ~12 | Decorators not detected during collection |

## Implementation Plan

### Phase 1: Handle Async Functions

**File**: `crates/rpytest-daemon/src/collector.rs`

**Changes**:
1. Add `ast::Stmt::AsyncFunctionDef` case in `extract_stmt_items()`
2. Reuse existing test extraction logic (async tests are collected the same way)

```rust
ast::Stmt::AsyncFunctionDef(func) => {
    // Same logic as FunctionDef - async doesn't affect collection
    let fn_name = &func.name;
    if fn_name.starts_with("test_") || fn_name.starts_with("Test") {
        // Extract test as normal
    }
}
```

---

### Phase 2: Expand Parametrized Tests

**Changes**:
1. Parse `@pytest.mark.parametrize` decorator arguments
2. Generate test IDs for each parameter combination
3. Handle stacked parametrize (cartesian product)
4. Handle `pytest.param()` with custom IDs

**Key Challenge**: Parsing Python literal values from AST

**Approach**:
1. Extract parametrize decorator args:
   - First arg: parameter names string (e.g., `"x,y"`)
   - Second arg: list of values or `pytest.param()` calls

2. For each parameter combination:
   - Generate unique node ID: `test_file.py::test_func[param-id]`
   - Store parameters for later test generation

**Example**:
```python
@pytest.mark.parametrize("x", [1, 2, 3])
def test_single_param(x):
```

Should generate:
- `test_file.py::test_single_param[1]`
- `test_file.py::test_single_param[2]`
- `test_file.py::test_single_param[3]`

---

### Phase 3: Detect Skip/Xfail at Collection

**Changes**:
1. Add `skip` and `xfail` to extracted markers
2. Detect both decorator forms:
   - `@pytest.mark.skip`
   - `@pytest.mark.skip(reason="...")`
   - `@pytest.mark.xfail`
   - `@pytest.mark.xfail(reason="...", condition=...)`

**Implementation**:
```rust
fn extract_skip_xfail_markers(&self, decorators: &[ast::Expr]) -> (bool, bool, Option<String>) {
    // Returns (is_skip, is_xfail, reason)
}
```

---

### Phase 4: Extract Line Numbers

**Changes**:
1. Get line number from AST node's location info
2. Update `NativeTestNode::line_number`

**Note**: `rustpython-parser` provides location info in the AST nodes

---

### Phase 5: Test and Verify

**Test files to verify**:
- `example_tests/test_fuzzy.py` - Basic tests (should work)
- `example_tests/test_fuzzy_parametrize.py` - Parametrized tests
- `example_tests/test_fuzzy_collection.py` - Collection edge cases
- `example_tests/test_fuzzy_markers.py` - Skip/xfail markers

**Verification**:
```bash
python3 -m pytest example_tests/ --collect-only 2>&1 | grep "collected"
./target/debug/rpytest run example_tests/ --collect-only 2>&1 | grep "Collected"
```

---

## Files to Modify

| File | Changes |
|------|---------|
| `crates/rpytest-daemon/src/collector.rs` | Main collector logic |
| `crates/rpytest-daemon/src/models.rs` | Possibly add `ParametrizedTestNode` or extend `NativeTestNode` |
| `crates/rpytest/src/main.rs` | Update verify-dropin tests if needed |

---

## Success Criteria

| Metric | Before | Target |
|--------|--------|--------|
| Test collection match | 201/290 (69%) | 285/290 (98%) |
| Async tests | 0 | All |
| Parametrized tests | 1 marker | Expanded nodes |
| Skip/xfail detected | No | Yes |

---

## Complexity Assessment

| Phase | Complexity | Risk |
|-------|------------|------|
| Async functions | Low | Minimal - just add another AST node type |
| Parametrized expansion | Medium | Medium - requires parsing Python literals |
| Skip/xfail detection | Low | Minimal - just marker detection |
| Line numbers | Low | Minimal - use AST location info |

---

## Backward Compatibility

All changes are additive:
- Existing test collection continues to work
- New fields in `NativeTestNode` are optional/default
- No breaking changes to IPC protocol
