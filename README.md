# MVP AI Screen Capture

This Tauri-based MVP targets Linux (Fedora) first and captures display, system audio, and optionally microphone input for downstream AI processing.

## Fedora prerequisites

Install the multimedia stack and portal packages before running the app:

```bash
sudo dnf install -y \
	gstreamer1 gstreamer1-plugins-base gstreamer1-plugins-good \
	gstreamer1-plugins-bad-free gstreamer1-plugins-ugly \
	gst-plugins-bad-freeworld gst-plugin-pipewire \
	pipewire pipewire-alsa pipewire-pulseaudio \
	xdg-desktop-portal xdg-desktop-portal-gtk
```

The `gst-launch-1.0` tool can verify screen and audio capture availability before testing inside the app.

## Debugging capture output

Set `DEBUG_SAVE=1` when launching the Tauri dev server to persist chunk samples under `debug_output/` for manual inspection. Leave the flag unset in normal runs to avoid writing user data to disk.

## Development workflow

1. Install frontend deps with `pnpm install` (if not already).
2. Run `cargo check --manifest-path src-tauri/Cargo.toml` to validate the Rust backend.
3. Use `pnpm tauri dev` (or your preferred runner) to start the desktop shell.

Refer to `story.md` for the full implementation plan and acceptance criteria checklist.
