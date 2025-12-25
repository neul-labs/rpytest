"""Generate synthetic test files for benchmarking."""
import os

# Configuration
NUM_FILES = 20
TESTS_PER_FILE = 25  # Total: 500 tests
OUTPUT_DIR = os.path.dirname(os.path.abspath(__file__))

def generate_test_file(file_num: int, tests_per_file: int) -> str:
    """Generate content for a test file."""
    lines = [
        '"""Auto-generated test file for benchmarking."""',
        'import pytest',
        'import time',
        'import math',
        '',
    ]

    # Add a test class
    lines.append(f'class TestSuite{file_num}:')
    lines.append(f'    """Test suite {file_num}."""')
    lines.append('')

    for i in range(tests_per_file // 2):
        # Simple assertion tests
        lines.append(f'    def test_simple_{i}(self):')
        lines.append(f'        """Simple test {i}."""')
        lines.append(f'        assert {i} + 1 == {i + 1}')
        lines.append(f'        assert "hello" == "hello"')
        lines.append('')

    lines.append('')

    # Add standalone test functions
    for i in range(tests_per_file // 2):
        test_type = i % 5

        if test_type == 0:
            # Simple math test
            lines.append(f'def test_math_{file_num}_{i}():')
            lines.append(f'    """Math test {i}."""')
            lines.append(f'    result = sum(range({i + 10}))')
            lines.append(f'    assert result == {sum(range(i + 10))}')
            lines.append('')

        elif test_type == 1:
            # String test
            lines.append(f'def test_string_{file_num}_{i}():')
            lines.append(f'    """String test {i}."""')
            lines.append(f'    s = "test" * {i + 1}')
            lines.append(f'    assert len(s) == {4 * (i + 1)}')
            lines.append('')

        elif test_type == 2:
            # List test
            lines.append(f'def test_list_{file_num}_{i}():')
            lines.append(f'    """List test {i}."""')
            lines.append(f'    lst = list(range({i + 20}))')
            lines.append(f'    assert len(lst) == {i + 20}')
            lines.append(f'    assert lst[-1] == {i + 19}')
            lines.append('')

        elif test_type == 3:
            # Dict test
            lines.append(f'def test_dict_{file_num}_{i}():')
            lines.append(f'    """Dict test {i}."""')
            lines.append(f'    d = {{k: k*2 for k in range({i + 5})}}')
            lines.append(f'    assert len(d) == {i + 5}')
            lines.append(f'    assert d[0] == 0')
            lines.append('')

        else:
            # Test with fixture
            lines.append(f'def test_with_fixture_{file_num}_{i}(simple_data):')
            lines.append(f'    """Fixture test {i}."""')
            lines.append(f'    assert "key" in simple_data')
            lines.append(f'    assert len(simple_data["numbers"]) == 100')
            lines.append('')

    return '\n'.join(lines)


def main():
    print(f"Generating {NUM_FILES} test files with {TESTS_PER_FILE} tests each...")
    print(f"Total tests: {NUM_FILES * TESTS_PER_FILE}")

    for i in range(NUM_FILES):
        filename = os.path.join(OUTPUT_DIR, f'test_suite_{i:02d}.py')
        content = generate_test_file(i, TESTS_PER_FILE)
        with open(filename, 'w') as f:
            f.write(content)
        print(f"  Created {filename}")

    print("Done!")


if __name__ == '__main__':
    main()
