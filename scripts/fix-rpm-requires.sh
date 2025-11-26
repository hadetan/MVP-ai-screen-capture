#!/usr/bin/env bash
set -euo pipefail

if [ $# -lt 1 ]; then
  echo "Usage: $0 <path-to-rpm>"
  exit 1
fi

RPM_INPUT="$1"

if ! command -v rpmrebuild >/dev/null 2>&1; then
  echo "Please install rpmrebuild: sudo dnf install -y rpmrebuild"
  exit 2
fi

echo "Fixing rpm textual requires for: $RPM_INPUT"

# Replace the textual require `webkit2gtk-4.1` with Fedora's `webkit2gtk4.1` package name
OUTPUT=$(rpmrebuild -p -f "sed -e 's/Requires:\s*webkit2gtk-4.1/Requires: webkit2gtk4.1/'" "$RPM_INPUT")

echo "Rebuilt RPM: $OUTPUT"
echo "You can now install with: sudo dnf install $OUTPUT"
