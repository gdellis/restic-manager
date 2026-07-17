#!/bin/bash
# update_progress.sh - Update PROGRESS.md with current issue status
# Usage: ./scripts/update_progress.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROGRESS_FILE="$REPO_ROOT/PROGRESS.md"

# Fetch current issues from GitHub using gh CLI
echo "Fetching current issues from GitHub..."

# Get open issues
OPEN_ISSUES=$(gh issue list --state open --json number,title,labels 2>/dev/null || echo "[]")

# Get closed issues from last 30 days
CLOSED_ISSUES=$(gh issue list --state closed --json number,title,labels --limit 50 2>/dev/null || echo "[]")

echo "Open issues:"
echo "$OPEN_ISSUES" | head -20
echo ""
echo "Closed issues:"
echo "$CLOSED_ISSUES" | head -20

echo ""
echo "To update PROGRESS.md manually:"
echo "1. Edit the Status column for each issue"
echo "2. Move completed issues from Backlog to Done"
echo "3. Update the Last updated date"
echo ""
echo "Status legend:"
echo "  🟡 Backlog - Not started"
echo "  🟠 In Progress - Actively being worked on"
echo "  ✅ Done - Completed and merged"
echo "  🟢 Icebox - Future consideration"
