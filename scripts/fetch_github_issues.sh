#!/bin/bash
# fetch_github_issues.sh - Fetch and display GitHub issue status for restic-manager
# Usage: ./scripts/fetch_github_issues.sh
#
# This script fetches open and recently closed issues from GitHub to help
# maintainers manually update PROGRESS.md. It does NOT automatically update
# PROGRESS.md - that must be done manually.

set -euo pipefail

# Verify gh CLI is available and authenticated
if ! command -v gh &>/dev/null; then
    echo "Error: gh CLI not found. Please install GitHub CLI from https://cli.github.com/" >&2
    exit 1
fi

if ! gh auth status &>/dev/null; then
    echo "Error: Not authenticated with GitHub CLI. Please run 'gh auth login'" >&2
    exit 1
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "Fetching current issues from GitHub..."

# Get open issues (no truncation - show all)
OPEN_ISSUES=$(gh issue list --state open --json number,title,labels,url 2>&1 || echo "[]")

# Get recently closed issues
CLOSED_ISSUES=$(gh issue list --state closed --json number,title,labels --limit 50 2>&1 || echo "[]")

echo ""
echo "=== Open Issues ==="
echo "$OPEN_ISSUES" | jq -r '.[] | "#\(.number): \(.title) [\(.labels | map(.name) | join(", "))]"' 2>/dev/null || echo "$OPEN_ISSUES"

echo ""
echo "=== Recently Closed Issues ==="
echo "$CLOSED_ISSUES" | jq -r '.[] | "#\(.number): \(.title) [\(.labels | map(.name) | join(", "))]"' 2>/dev/null || echo "$CLOSED_ISSUES"

echo ""
echo "To update PROGRESS.md manually:"
echo "1. Edit the Status column for each issue in the tables"
echo "2. Move completed issues from Backlog to Done"
echo "3. Update the 'Last updated' date at the top of the Current Focus section"
echo "4. Add PR links to the PR column as work progresses"
