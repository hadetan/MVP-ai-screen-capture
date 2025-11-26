# MVP Story: Screen + System Audio Capture (Linux Fedora)

Overview
--------
This document describes the plan to implement an MVP desktop app using Tauri + Rust that captures:

- Whole display (or a chosen window) video
- System audio (everything that goes to the speakers/monitor)
- Optional microphone capture (toggleable)

Focus: Linux, Fedora (Wayland, PipeWire). Later we will expand to other OSs.

Important constraints & assumptions
----------------------------------
- Platform: Linux (Fedora) — Wayland (default), PipeWire for audio/video.
- Permissions: Use xdg-desktop-portal / sandboxed portal to request screen access on Wayland (the portal will prompt the user and return a PipeWire stream to the app).
- Capture method: Use GStreamer pipelines with PipeWire elements when available.
- Output: For MVP the app will not persist captured data to disk (unless a debug flag is enabled). Captured chunks will be held in memory and streamed to the next stage (AI pipeline) but AI integration is out of scope for this story.
- The pipeline will produce periodic chunks (configurable chunk duration): e.g., every 5 seconds — captured, optionally encoded, and emitted to an in-memory buffer or streamed.
- Accept a mic toggle to include both system audio and mic (mixed or separate streams as the user chooses).
- The app will present a UI (Tauri/React) with Start/Stop, Display (all displays or selected window), and Mic toggle.

High-level architecture
-----------------------
- UI (Tauri WebView): Provides controls for start/stop, screen vs window capture, chunk duration, mic toggle, and debug toggle.
- Rust Backend (Tauri `src-tauri`): Handles commands from the UI and runs capture pipelines.
- CaptureManager (Rust module): Manages lifecycle of GStreamer pipelines for video and audio, chunking, and debug save.
- Inter-thread channels: CaptureManager produces chunked frames via channels that are consumed by a "publisher" which would eventually send data to AI (not implemented here).
- Debug save (optional): If an environment variable is set (e.g., DEBUG_SAVE=1), the chunk is saved temporarily to a local folder (or as a single ephemeral file) for testing.

Key Components & Responsibilities
----------------------------------
1) UI (Tauri Frontend)
   - Start/Stop capture
   - Select capture target (whole display or a specific window). On Wayland the portal will be used; on X11 we may show a list.
   - Mic toggle: Include microphone capture if requested.
   - Chunk duration: Numeric input (defaults to 5 seconds)
   - Debug toggle: Enable/disable DEBUG_SAVE; when enabled the backend may save to disk for debugging.

2) Tauri Rust Backend
   - Expose `start_capture`, `stop_capture`, `set_capture_options`, `list_windows` commands as `#[tauri::command]` functions
   - Start/stop the CaptureManager on demand
   - Subscribe to GStreamer bus messages and report status to the UI

3) CaptureManager (Rust module, `src-tauri/src/capture_manager.rs`)
   - Initialize GStreamer (safe, once) and manage pipelines
   - Create two pipelines or a combined pipeline:
     - Video: `pipewiresrc`/`ximagesrc` (platform-dependent) -> videoconvert -> videoscale -> appsink
     - Audio System: `pulsesrc` or `pipewiresrc` (monitor node) -> audioconvert -> audioresample -> appsink
     - Audio Mic (optional): `pulsesrc` (mic) -> audioconvert -> audioresample -> appsink
   - Use `appsink` for both audio & video to pull buffers into Rust code for processing and chunking
   - Align audio/video timing information with timestamps if the user wants A/V sync (optional initial MVP: independent chunks, but include timestamps)
   - Chunking logic: accumulate frames/buffers for a configurable interval -> emit chunk (memory buffer) -> if DEBUG_SAVE: write to file
   - Graceful stop handling, pipeline restart on config changes

4) Debug & Testing
   - Debug saving only when `DEBUG_SAVE=1`. The app should create a `debug_output` directory within the project or a temp directory to save sample files.
   - Include simple verification UI to show that frames are arriving (optional overlay or thumbnail)

Detailed Design Considerations
-------------------------------
- GStreamer vs native PipeWire bindings:
  - Use `gstreamer-rs` bindings to GStreamer. Rationale: GStreamer is flexible, provides `appsink` and `pipewiresrc` elements, and is well-supported on Linux. `gstreamer-rs` gives Rust wrappers and reduces FFI boilerplate.
  - Dependencies (system) on Fedora: gstreamer1, gst-plugins-base, gst-plugins-good, gst-plugins-bad, gst-plugins-ugly (optional), gst-plugin-pipewire, pipewire, xdg-desktop-portal, xdg-desktop-portal-gtk (for GNOME) or relevant portal backend.

- Element choices and pipes (examples):
  - Video (Wayland w/ portal):
    - `pipewiresrc` name=source ! videoconvert ! videoscale ! video/x-raw,format=I420,width=X,height=Y,framerate=30/1 ! appsink name=video_appsink sync=false
    - Note: On Wayland, we must request permissions via `xdg-desktop-portal` first (GStreamer `pipewiresrc` will use it automatically when configured correctly).
  - Audio (system monitor):
    - `pulsesrc device=alsa_output.pci-0000_00_1b.0.analog-stereo.monitor` or use `pipewiresrc` with the monitor property set to the sink.
    - Pipeline: `pulsesrc device=<monitor> ! audioconvert ! audioresample ! audio/x-raw,rate=48000,channels=2 ! appsink name=audio_appsink sync=false`
  - Microphone (mic input):
    - `pulsesrc device=<mic> ! audioconvert ! audioresample ! appsink name=mic_appsink sync=false`
  - Options: For a simpler path, the app can rely on default `pulsesrc` without specifying a device, and enumerate devices if the user wants to switch.

- Choosing between `appsink` vs `filesink` or encoding:
  - Use `appsink` for in-memory consumption. We can then write bytes to memory and later optionally encode.
  - For efficiency, optionally add an encoder (e.g., `avenc_h264` or `x264enc`) in a separate branch if encoding is required, or `opusenc` for audio. For MVP, raw frames may be acceptable to test the path.

- Chunking and memory usage:
  - Buffer frames and audio samples into a chunk buffer for the specified duration.
  - For memory safety, set a maximum chunk size and backpressure if the consumer is slow. Alternatively, implement a circular buffer.
  - Each chunk will include metadata (start timestamp, duration, sample rate, resolution, stream ids)

- Sync & timestamps:
  - GStreamer buffers contain timestamps; preserve them to determine A/V alignment if needed.
  - For initial MVP we can produce independent audio and video chunks with timestamps and let the AI handle merging.

- Handling window capture (single window)
  - On Wayland portals, the `xdg-desktop-portal` may support either full-screen or a per-window capture: the portal dialog will allow user selection, and the returned PipeWire stream will be for the chosen window.
  - If implementing a window select list is required on the UI side, we can try to query the portal or use a Wayland helper to list potential targets (but the portal selection is preferable as it respects security).

- Tauri & permissions
  - The app must request portal access for Wayland screens. The portal prompts appear when requesting capture (e.g., when `pipewiresrc` is constructed with a portal/session). The Tauri app must ensure that it does not run as a privileged app requiring additional permissions beyond the user-consented portal.

Implementation plan (detailed, step-by-step)
---------------------------------------------
This plan breaks down the work into tasks so we can implement incrementally.

Phase 0 — Setup & Dependencies
- [x] Add `gstreamer` family crates to `src-tauri/Cargo.toml`: `gstreamer`, `gstreamer-app`, `gstreamer-video`.
- [x] Ensure `Cargo.toml` contains platform certificates or feature flags if needed (e.g., `v1_18` or `v1_20` depending on system versions).
- [x] Document Fedora required system packages in README and `story.md`: gstreamer1, gst-plugins-bad/ugly, gst-plugin-pipewire, pipewire, pipewire-pulse, xdg-desktop-portal, xdg-desktop-portal-gtk.

Phase 1 — CaptureManager module scaffold
- [x] Create `src-tauri/src/capture_manager.rs` (if not present) and implement the skeleton:
  - Public API: `start_capture(options) -> Result<(), Error>`, `stop_capture()`, `set_options()`, `status()`. `start_capture` will spawn threads for audio/video pipeline appsink and return a handle.
  - Internal state for active pipelines, app sink, and channels to push buffers to a publisher.
- [x] Add GStreamer initialization and error handling.

Phase 2 — Video capture path
- [ ] Implement a GStreamer pipeline builder function for Wayland portal capture using `pipewiresrc`. The pipeline outputs to `appsink` to receive buffers.
- [ ] Implement appsink callback logic to push frames into a video chunk buffer.
- [ ] Support selecting full display or a portal window (accepts a portal stream ID or let the portal select via prompt).

Phase 3 — Audio capture path
- [ ] Implement a GStreamer audio pipeline that captures the system monitor (the monitor source for the active sink). Use `pulsesrc` or `pipewiresrc` depending on which is most reliable.
- [ ] Implement appsink callback to push audio buffers into an audio chunk buffer.
- [ ] Implement microphone capture path toggled on/off.

Phase 4 — Chunking & Consumer API
- [ ] Implement chunking logic that accumulates audio and video for a configured chunk duration and emits an in-memory chunk (struct with metadata and raw bytes).
- [ ] If `DEBUG_SAVE=1`, write each chunk to a temporary file with naming including timestamp (e.g., `debug_output/chunk-{timestamp}-video.raw` and `chunk-{timestamp}-audio.pcm`), and write logs to help human inspection.
- [ ] Wire up channels that deliver the chunk to a placeholder consumer (AI publisher stub). The UI should be able to see chunk boundaries via events.

Phase 5 — Tauri Commands & UI
- [ ] Add command functions in `src-tauri/src/lib.rs` and wire them to UI calls (e.g., `start_capture`, `stop_capture`, `select_target`, `toggle_mic`).
- [ ] Hook the pipeline state and errors to events that the UI can receive (Tauri events).
- [ ] Add a small UI to start/stop capture, select target, toggle mic, and set chunk duration. In the UI, show small debug logs and sample thumbnails (optional) from debug file.

Phase 6 — Testing, validation & polishing
- [ ] Create a set of manual tests and scripts to validate the following acceptance criteria (below). Implement minimal automated tests for Rust state changes and pipeline creation, where feasible.

Acceptance Criteria (ACs — Checkboxes)
--------------------------------------
These should be used to verify the correctness of the MVP.

Capture Startup & Permissions
- [ ] When the user clicks "Start", the app requests necessary portal permissions via `xdg-desktop-portal` and the portal dialog appears.
- [ ] If the user denies the portal, the app gracefully reports the denial and does not crash.
- [ ] The app can start capture without errors on Fedora (Wayland + PipeWire), given the proper system packages installed.

Video capture
- [ ] The app can capture full-display content and produce periodic video chunks (size/time as set by the chunk duration) within memory (no persisted files when DEBUG_SAVE is off).
- [ ] The `appsink` receives video buffers and the chunker emits chunk objects at the configured interval.
- [ ] The video chunk contains metadata (width, height, pixel format, timestamp).
- [ ] If `DEBUG_SAVE` is enabled, video bytes are written to `debug_output/` with appropriate naming and can be inspected by the developer.

Window capture
- [ ] The app can capture a single window selected via the portal dialog and produce periodic chunks.
- [ ] When a window is closed, the pipeline exits gracefully with an error reported and the state is revertible to start again.

Audio capture — System sound
- [ ] The app captures the system output (audio going to speaker) as a monitor source and produces audio chunks synchronized with timestamps.
- [ ] The audio chunk contains metadata such as sample rate, channels, sample format, and timestamp.
- [ ] `DEBUG_SAVE` offers a raw PCM dump with sample-rate and channels specified for inspection.

Audio capture — Microphone toggle
- [ ] When `mic` is toggled ON before a `start_capture`, the capture includes mic data as an additional audio stream.
- [ ] The mic stream does not overwrite the system audio stream; both can be captured as separate consumer channels and mixed later if needed.

Chunking & Metadata
- [ ] Chunks are created at the configured interval (~5s default) with proper timestamps and chunk IDs.
- [ ] Each chunk includes: start_ts, duration, stream IDs (video, audio, mic if any), and a simple header for the bytes that follow.
- [ ] System handles backpressure: if the consumer is slow or blocked, the app signals a buffer threshold (e.g., drop frames or slow the pipeline in a configurable manner).

Tauri UI & Commands
- [ ] `start_capture`/`stop_capture` commands are exposed and clickable in the frontend UI.
- [ ] Errors during capture initialization are surfaced to the UI with logs.
- [ ] UI supports target selection (full screen or window), mic toggle, and chunk duration.

Testing & Debug
- [ ] The `DEBUG_SAVE=1` env option causes debug chunk files to be saved for both audio & video to a `debug_output` folder.
- [ ] The repo includes a README section detailing dependency installation on Fedora and a test plan including sample `gst-launch-1.0` commands to verify environment readiness.

Quality & Non-goals
-------------------
- This MVP does NOT implement AI integration — the consumer is a placeholder function or channel. AI will be integrated in a later phase.
- This MVP focuses on Linux (Fedora); Windows/macOS are out of scope for now but will be targeted later.
- The app won't include advanced encoding or bandwidth-optimized streaming in Phase 1; raw or basic encoding is fine to validate the pipeline.

Implementation tasks & split (issues suggestions)
------------------------------------------------
- Issue 1: Add GStreamer dependencies and document Fedora system requirements.
- Issue 2: CaptureManager skeleton & GStreamer initialization.
- Issue 3: Implement Video pipeline and appsink callback + chunker.
- Issue 4: Implement Audio pipeline for system monitor and appsink callback + chunker.
- Issue 5: Add Mic toggling with separate appsink.
- Issue 6: UI wiring (Start/Stop/Select/Mic/Duration) + Tauri commands.
- Issue 7: Debug saving & test scripts + README updates.
- Issue 8: Add unit & integration tests for pipeline creation and command behavior.

Testing & verification steps (manual) — Quick Start on Fedora
-------------------------------------------------------------
1) Install OS dependencies (example):
```bash
sudo dnf install -y gstreamer1 gstreamer1-plugins-base gstreamer1-plugins-good gstreamer1-plugins-bad-free gstreamer1-plugins-ugly pipewire pipewire-alsa pipewire-pulseaudio xdg-desktop-portal xdg-desktop-portal-gtk gst-plugins-bad-freeworld
```
2) Start the app and verify the portal prompt when starting screen capture.
3) Play audio in a browser and verify captured audio when mic toggle is off (system audio only).
4) Toggle mic on and verify both audio streams. Optionally use `gst-launch-1.0` to test audio loopbacks.
5) Enable debug mode and verify creation of `debug_output` files.

Notes and Edge Cases
--------------------
- On non-Wayland systems (X11), `ximagesrc` may need to be used instead of `pipewiresrc`.
- On different desktops, the portal backend may have different names. Fedora default (GNOME) uses GTK portal backend which prompts user for capture.
- Device enumeration for choosing audio sink sources can be implemented via PulseAudio APIs or by enumerating PipeWire nodes (but PulseAudio/`pactl` is simpler for MVP).
- Additional error handling: gracefully handle devices being absent or failing to allocate resources.

References
----------
- GStreamer AppSink: https://gstreamer.freedesktop.org/documentation/plugin-development/element-plugins.html
- GStreamer Rust bindings: https://crates.io/crates/gstreamer
- PipeWire & xdg-desktop-portal: portal docs and pipewire plugin guidance
- GStreamer pipeline examples for capturing screen and audio

----
