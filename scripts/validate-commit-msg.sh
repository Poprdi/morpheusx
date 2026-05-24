#!/bin/bash
# Validate commit message format for MorpheusX
# Follows conventional commits (feat, fix, docs, refactor, test, chore)

set -e

COMMIT_MSG=$(git log -1 --format=%B "$1" 2>/dev/null || echo "")
FIRST_LINE=$(echo "$COMMIT_MSG" | head -n1)

# Check for empty commit
if [ -z "$FIRST_LINE" ]; then
    echo "ERROR: Empty commit message"
    exit 1
fi

# Check line length (max 100 chars for first line)
if [ ${#FIRST_LINE} -gt 100 ]; then
    echo "ERROR: First line exceeds 100 characters (${#FIRST_LINE})"
    exit 1
fi

# Check format: type(scope): description
# Valid types: feat, fix, docs, style, refactor, test, chore, perf, ci, build
if echo "$FIRST_LINE" | grep -qE '^(feat|fix|docs|style|refactor|test|chore|perf|ci|build)(\([^)]+\))?: '; then
    echo "OK: Commit message follows conventional commits format"
    exit 0
else
    echo "WARNING: Commit message doesn't follow conventional commits format"
    echo "Expected: type(scope): description"
    echo "Found: $FIRST_LINE"
    exit 0  # Warning only, not fatal
fi
