#!/bin/bash
# Test runner for iso9660 crate
set -e

cd "$(dirname "$0")"

echo "=========================================="
echo "  ISO9660 Crate Test Suite"
echo "=========================================="
echo ""

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Track results
TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0

run_test_suite() {
    local name="$1"
    local cmd="$2"
    
    echo -e "${YELLOW}Running: $name${NC}"
    TOTAL_TESTS=$((TOTAL_TESTS + 1))
    
    if eval "$cmd"; then
        echo -e "${GREEN}✓ PASSED: $name${NC}"
        PASSED_TESTS=$((PASSED_TESTS + 1))
    else
        echo -e "${RED}✗ FAILED: $name${NC}"
        FAILED_TESTS=$((FAILED_TESTS + 1))
    fi
    echo ""
}

echo "=== Phase 1: Unit Tests ==="
echo ""

run_test_suite "Block I/O Tests" "cargo test --test block_io_tests"
run_test_suite "Volume Tests" "cargo test --test volume_tests"
run_test_suite "Directory Tests" "cargo test --test directory_tests"
run_test_suite "File Tests" "cargo test --test file_tests"
run_test_suite "Boot Tests" "cargo test --test boot_tests"

echo ""
echo "=== Phase 2: Integration Tests ==="
echo ""

run_test_suite "Integration Tests (Quick)" "cargo test --test integration_tests -- --skip test_real_tails_iso --skip create_test_iso"

echo ""
echo "=== Phase 3: Extended Tests (Optional) ==="
echo ""

if [ -f "../testing/esp/.iso/tails-amd64-7.3.1.iso" ]; then
    echo "Tails ISO found, running real ISO test..."
    run_test_suite "Real Tails ISO Test" "cargo test --test integration_tests test_real_tails_iso -- --ignored --nocapture"
else
    echo "Tails ISO not found, skipping real ISO test"
    echo "(Download with: cd ../testing && ./install-tails.sh)"
fi

# Try to create minimal test ISO
if command -v genisoimage &> /dev/null; then
    echo "genisoimage found, creating test ISO..."
    run_test_suite "Create Test ISO" "cargo test --test integration_tests create_test_iso -- --ignored --nocapture"
else
    echo "genisoimage not found, skipping ISO creation test"
    echo "(Install with: apt-get install genisoimage)"
fi

echo ""
echo "=========================================="
echo "  Test Results"
echo "=========================================="
echo ""
echo "Total test suites: $TOTAL_TESTS"
echo -e "${GREEN}Passed: $PASSED_TESTS${NC}"
if [ $FAILED_TESTS -gt 0 ]; then
    echo -e "${RED}Failed: $FAILED_TESTS${NC}"
    echo ""
    echo "Run individual failed tests with:"
    echo "  cargo test --test <test_name> -- --nocapture"
    exit 1
else
    echo -e "${GREEN}All tests passed!${NC}"
fi

echo ""
echo "To run tests individually:"
echo "  cargo test --test volume_tests"
echo "  cargo test --test integration_tests -- --ignored --nocapture"
echo ""
echo "To run all tests with output:"
echo "  cargo test -- --nocapture"
