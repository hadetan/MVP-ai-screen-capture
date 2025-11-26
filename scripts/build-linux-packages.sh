#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

# Build frontend
echo "Building frontend..."
npm install
npm run build

echo "Building Tauri Linux packages (deb and rpm)..."
npx tauri build -b deb rpm

echo "Packages created in: src-tauri/target/release/bundle/"

# If rpmrebuild is available, fix RPM package requires (Zen: addresses 'webkit2gtk-4.1' mismatches on Fedora)
if command -v rpmrebuild >/dev/null 2>&1; then
	RPM_PATH="src-tauri/target/release/bundle/rpm/tauri-app-0.1.0-1.x86_64.rpm"
	if [ -f "$RPM_PATH" ]; then
		echo "Fixing RPM requires to match Fedora package names..."
		./scripts/fix-rpm-requires.sh "$RPM_PATH"
	fi
fi
