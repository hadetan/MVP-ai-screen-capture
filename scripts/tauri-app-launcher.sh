#!/usr/bin/env bash
set -euo pipefail
# Small wrapper to force desktop launch using X11 backend and disable WebKit compositing.
export GDK_BACKEND=x11
export WEBKIT_DISABLE_COMPOSITING_MODE=1
exec /usr/bin/tauri-app "$@"
