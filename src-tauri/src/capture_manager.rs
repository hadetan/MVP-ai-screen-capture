use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_audio as gst_audio;
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use serde_json::json;

static GSTREAMER: OnceCell<()> = OnceCell::new();

fn ensure_gstreamer_initialized() -> Result<()> {
    GSTREAMER
        .get_or_try_init(|| {
            gst::init()?;
            Ok(())
        })
        .map(|_| ())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CaptureTarget {
    FullDisplay,
    Window { id: String },
}

impl Default for CaptureTarget {
    fn default() -> Self {
        CaptureTarget::FullDisplay
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureOptions {
    #[serde(default = "CaptureOptions::default_chunk_ms")]
    pub chunk_duration_ms: u64,
    #[serde(default)]
    pub capture_mic: bool,
    #[serde(default)]
    pub debug_save: bool,
    #[serde(default)]
    pub target: CaptureTarget,
}

impl Default for CaptureOptions {
    fn default() -> Self {
        Self {
            chunk_duration_ms: Self::default_chunk_ms(),
            capture_mic: false,
            debug_save: false,
            target: CaptureTarget::FullDisplay,
        }
    }
}

impl CaptureOptions {
    pub fn chunk_duration(&self) -> Duration {
        Duration::from_millis(self.chunk_duration_ms.max(1000))
    }

    pub const fn default_chunk_ms() -> u64 {
        5_000
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureState {
    Idle,
    Starting,
    Running,
    Stopping,
}

impl CaptureState {
    fn is_active(self) -> bool {
        matches!(self, CaptureState::Starting | CaptureState::Running | CaptureState::Stopping)
    }
}

struct ManagerState {
    status: CaptureState,
    options: CaptureOptions,
    video_pipeline: Option<gst::Pipeline>,
    video_chunk_buffer: Option<Arc<Mutex<VideoChunkBuffer>>>,
    system_audio_pipeline: Option<gst::Pipeline>,
    system_audio_chunk_buffer: Option<Arc<Mutex<AudioChunkBuffer>>>,
    mic_pipeline: Option<gst::Pipeline>,
    mic_chunk_buffer: Option<Arc<Mutex<AudioChunkBuffer>>>,
    chunk_sender: Option<mpsc::Sender<CapturedChunk>>,
}

impl Default for ManagerState {
    fn default() -> Self {
        Self {
            status: CaptureState::Idle,
            options: CaptureOptions::default(),
            video_pipeline: None,
            video_chunk_buffer: None,
            system_audio_pipeline: None,
            system_audio_chunk_buffer: None,
            mic_pipeline: None,
            mic_chunk_buffer: None,
            chunk_sender: None,
        }
    }
}

#[derive(Default)]
pub struct CaptureManager {
    inner: Mutex<ManagerState>,
}

impl CaptureManager {
    pub fn start_capture(&self, options: CaptureOptions) -> Result<()> {
        ensure_gstreamer_initialized()?;

        {
            let mut inner = self.inner.lock().expect("manager mutex poisoned");
            if inner.status.is_active() {
                return Err(anyhow!("capture already running"));
            }
            inner.status = CaptureState::Starting;
            inner.options = options.clone();
        }

        if let Err(err) = self.configure_pipelines(&options) {
            let mut inner = self.inner.lock().expect("manager mutex poisoned");
            inner.status = CaptureState::Idle;
            return Err(err);
        }

        let mut inner = self.inner.lock().expect("manager mutex poisoned");
        inner.status = CaptureState::Running;
        Ok(())
    }

    pub fn stop_capture(&self) -> Result<()> {
        let mut inner = self.inner.lock().expect("manager mutex poisoned");
        if !inner.status.is_active() {
            return Ok(());
        }
        inner.status = CaptureState::Stopping;
        Self::teardown_pipeline(inner.video_pipeline.take());
        Self::teardown_pipeline(inner.system_audio_pipeline.take());
        Self::teardown_pipeline(inner.mic_pipeline.take());
        inner.video_chunk_buffer = None;
        inner.system_audio_chunk_buffer = None;
        inner.mic_chunk_buffer = None;
        inner.status = CaptureState::Idle;
        Ok(())
    }

    pub fn status(&self) -> CaptureState {
        self.inner.lock().expect("manager mutex poisoned").status
    }

    #[allow(dead_code)]
    pub fn set_options(&self, options: CaptureOptions) -> Result<()> {
        let mut inner = self.inner.lock().expect("manager mutex poisoned");
        if inner.status.is_active() {
            return Err(anyhow!("stop capture before updating options"));
        }
        inner.options = options;
        Ok(())
    }

    fn configure_pipelines(&self, options: &CaptureOptions) -> Result<()> {
        // create chunk channel and consumer
        let (tx, rx) = mpsc::channel::<CapturedChunk>();
        let debug_save = options.debug_save;
        std::thread::Builder::new()
            .name("chunk_consumer".into())
            .spawn(move || {
                if debug_save {
                    let _ = std::fs::create_dir_all("debug_output");
                }
                for chunk in rx {
                    if debug_save {
                        // write raw data and metadata
                        let ts = chunk.start_ts_unix_nanos;
                        let fname = format!("debug_output/chunk-{}-{}-{}.raw", ts, chunk.id, chunk.kind);
                        let _ = std::fs::write(&fname, &chunk.data);
                        let meta_fname = format!("debug_output/chunk-{}-{}-{}.json", ts, chunk.id, chunk.kind);
                        let _ = std::fs::write(&meta_fname, serde_json::to_string_pretty(&chunk.metadata).unwrap_or_default());
                        println!("[capture] debug-saved chunk {} -> {}", chunk.id, fname);
                    } else {
                        println!("[capture] consumed chunk {} kind={} len={}", chunk.id, chunk.kind, chunk.data_len);
                    }
                }
            })?;

        let tx_clone_v = tx.clone();
        let tx_clone_s = tx.clone();
        let tx_clone_m = tx.clone();

        let video_handles = Self::build_video_pipeline(options, Some(tx_clone_v))?;
        let system_audio_handles = Self::build_system_audio_pipeline(options, Some(tx_clone_s))?;
        let mic_handles = if options.capture_mic {
            Some(Self::build_mic_audio_pipeline(options, Some(tx_clone_m))?)
        } else {
            None
        };

        Self::start_pipeline(&video_handles.pipeline, "video").map_err(|err| {
            let _ = video_handles.pipeline.set_state(gst::State::Null);
            err
        })?;

        if let Err(err) = Self::start_pipeline(&system_audio_handles.pipeline, "system_audio") {
            let _ = video_handles.pipeline.set_state(gst::State::Null);
            let _ = system_audio_handles.pipeline.set_state(gst::State::Null);
            return Err(err);
        }

        if let Some(handles) = mic_handles.as_ref() {
            if let Err(err) = Self::start_pipeline(&handles.pipeline, "mic") {
                let _ = video_handles.pipeline.set_state(gst::State::Null);
                let _ = system_audio_handles.pipeline.set_state(gst::State::Null);
                let _ = handles.pipeline.set_state(gst::State::Null);
                return Err(err);
            }
        }

        let VideoPipelineHandles {
            pipeline: video_pipeline,
            chunk_buffer: video_chunk_buffer,
        } = video_handles;
        let AudioPipelineHandles {
            pipeline: system_audio_pipeline,
            chunk_buffer: system_audio_chunk_buffer,
        } = system_audio_handles;
        let (mic_pipeline, mic_chunk_buffer) = if let Some(handles) = mic_handles {
            (Some(handles.pipeline), Some(handles.chunk_buffer))
        } else {
            (None, None)
        };

        let mut inner = self.inner.lock().expect("manager mutex poisoned");
        inner.video_pipeline = Some(video_pipeline);
        inner.video_chunk_buffer = Some(video_chunk_buffer);
        inner.system_audio_pipeline = Some(system_audio_pipeline);
        inner.system_audio_chunk_buffer = Some(system_audio_chunk_buffer);
        inner.mic_pipeline = mic_pipeline;
        inner.mic_chunk_buffer = mic_chunk_buffer;
        inner.chunk_sender = Some(tx);
        Ok(())
    }

    fn start_pipeline(pipeline: &gst::Pipeline, label: &str) -> Result<()> {
        pipeline
            .set_state(gst::State::Playing)
            .map_err(|err| anyhow!("failed to start {label} pipeline: {err:?}"))?;
        Ok(())
    }

    fn teardown_pipeline(pipeline: Option<gst::Pipeline>) {
        if let Some(p) = pipeline {
            let _ = p.set_state(gst::State::Null);
        }
    }
}

struct VideoPipelineHandles {
    pipeline: gst::Pipeline,
    chunk_buffer: Arc<Mutex<VideoChunkBuffer>>,
}

struct AudioPipelineHandles {
    pipeline: gst::Pipeline,
    chunk_buffer: Arc<Mutex<AudioChunkBuffer>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CapturedChunk {
    pub id: u64,
    pub kind: String,
    pub start_ts_unix_nanos: u128,
    pub duration_ms: u64,
    pub metadata: serde_json::Value,
    pub data_len: usize,
    #[serde(skip)]
    pub data: Vec<u8>,
}

fn missing_element(name: &str) -> anyhow::Error {
    anyhow!("missing GStreamer element '{name}' — ensure required plugins are installed")
}

impl CaptureManager {
    fn build_video_pipeline(options: &CaptureOptions, sender: Option<mpsc::Sender<CapturedChunk>>) -> Result<VideoPipelineHandles> {
        let pipeline = gst::Pipeline::new();
        let src = gst::ElementFactory::make("pipewiresrc")
            .name("video_source")
            .build()
            .map_err(|_| missing_element("pipewiresrc"))?;
        src.set_property("do-timestamp", &true);

        match &options.target {
            CaptureTarget::FullDisplay => {
                // Nothing extra — the portal UI will prompt for full display selection.
            }
            CaptureTarget::Window { id } => {
                if let Ok(node_id) = id.parse::<u32>() {
                    if src.find_property("target-node").is_some() {
                        src.set_property("target-node", &node_id);
                    }
                }
            }
        }

        let convert = gst::ElementFactory::make("videoconvert")
            .name("video_convert")
            .build()
            .map_err(|_| missing_element("videoconvert"))?;
        let scale = gst::ElementFactory::make("videoscale")
            .name("video_scale")
            .build()
            .map_err(|_| missing_element("videoscale"))?;

        let caps = gst::Caps::builder("video/x-raw")
            .field("format", &"RGBA")
            .field("framerate", &gst::Fraction::new(30, 1))
            .build();

        let sink = gst::ElementFactory::make("appsink")
            .name("video_sink")
            .build()
            .map_err(|_| missing_element("appsink"))?;
        let appsink = sink
            .clone()
            .dynamic_cast::<gst_app::AppSink>()
            .map_err(|_| anyhow!("failed to downcast appsink"))?;

        appsink.set_caps(Some(&caps));
        appsink.set_property("emit-signals", &true);
        appsink.set_property("sync", &false);
        appsink.set_property("max-buffers", &5u32);
        appsink.set_property("drop", &true);

        pipeline.add_many(&[&src, &convert, &scale, &sink])?;
        gst::Element::link_many(&[&src, &convert, &scale, &sink])?;

        let chunk_buffer = Arc::new(Mutex::new(VideoChunkBuffer::new_with_sender(
            options.chunk_duration(),
            options.debug_save,
            sender,
        )));
        let chunk_buffer_clone = Arc::clone(&chunk_buffer);

        let callbacks = gst_app::AppSinkCallbacks::builder()
            .new_sample(move |appsink| {
                let sample = appsink
                    .pull_sample()
                    .map_err(|_| gst::FlowError::Error)?;
                let mut guard = chunk_buffer_clone
                    .lock()
                    .map_err(|_| gst::FlowError::Error)?;
                guard.handle_sample(&sample);
                Ok(gst::FlowSuccess::Ok)
            })
            .build();

        appsink.set_callbacks(callbacks);

        Ok(VideoPipelineHandles {
            pipeline,
            chunk_buffer,
        })
    }

    fn build_system_audio_pipeline(options: &CaptureOptions, sender: Option<mpsc::Sender<CapturedChunk>>) -> Result<AudioPipelineHandles> {
        let device = std::env::var("SC_SYSTEM_AUDIO_DEVICE")
            .unwrap_or_else(|_| "@DEFAULT_SINK@.monitor".to_string());
        Self::build_pulse_audio_pipeline("system_audio_source", "system_audio", Some(device), options, sender)
    }

    fn build_mic_audio_pipeline(options: &CaptureOptions, sender: Option<mpsc::Sender<CapturedChunk>>) -> Result<AudioPipelineHandles> {
        let device = std::env::var("SC_MIC_AUDIO_DEVICE")
            .unwrap_or_else(|_| "@DEFAULT_SOURCE@".to_string());
        Self::build_pulse_audio_pipeline("mic_audio_source", "mic", Some(device), options, sender)
    }

    fn build_pulse_audio_pipeline(
        source_name: &str,
        label: &'static str,
        device: Option<String>,
        options: &CaptureOptions,
        sender: Option<mpsc::Sender<CapturedChunk>>,
    ) -> Result<AudioPipelineHandles> {
        let pipeline = gst::Pipeline::new();
        let src = gst::ElementFactory::make("pulsesrc")
            .name(source_name)
            .build()
            .map_err(|_| missing_element("pulsesrc"))?;

        if let Some(device_name) = device {
            if src.find_property("device").is_some() {
                src.set_property("device", &device_name);
            }
        }

        let convert = gst::ElementFactory::make("audioconvert")
            .name(format!("{source_name}_convert"))
            .build()
            .map_err(|_| missing_element("audioconvert"))?;
        let resample = gst::ElementFactory::make("audioresample")
            .name(format!("{source_name}_resample"))
            .build()
            .map_err(|_| missing_element("audioresample"))?;

        let caps = gst::Caps::builder("audio/x-raw")
            .field("format", &"F32LE")
            .field("rate", &48_000i32)
            .field("channels", &2i32)
            .build();

        let sink = gst::ElementFactory::make("appsink")
            .name(format!("{source_name}_sink"))
            .build()
            .map_err(|_| missing_element("appsink"))?;
        let appsink = sink
            .clone()
            .dynamic_cast::<gst_app::AppSink>()
            .map_err(|_| anyhow!("failed to downcast audio appsink"))?;

        appsink.set_caps(Some(&caps));
        appsink.set_property("emit-signals", &true);
        appsink.set_property("sync", &false);
        appsink.set_property("max-buffers", &20u32);
        appsink.set_property("drop", &true);

        pipeline.add_many(&[&src, &convert, &resample, &sink])?;
        gst::Element::link_many(&[&src, &convert, &resample, &sink])?;

        let chunk_buffer = Arc::new(Mutex::new(AudioChunkBuffer::new_with_sender(
            label,
            options.chunk_duration(),
            options.debug_save,
            sender,
        )));
        let chunk_buffer_clone = Arc::clone(&chunk_buffer);

        let callbacks = gst_app::AppSinkCallbacks::builder()
            .new_sample(move |appsink| {
                let sample = appsink
                    .pull_sample()
                    .map_err(|_| gst::FlowError::Error)?;
                let mut guard = chunk_buffer_clone
                    .lock()
                    .map_err(|_| gst::FlowError::Error)?;
                guard.handle_sample(&sample);
                Ok(gst::FlowSuccess::Ok)
            })
            .build();

        appsink.set_callbacks(callbacks);

        Ok(AudioPipelineHandles {
            pipeline,
            chunk_buffer,
        })
    }
}

struct VideoChunkBuffer {
    chunk_duration: Duration,
    debug_save: bool,
    chunk_start: Instant,
    frames_in_chunk: u64,
    accum: Vec<u8>,
    start_ts_unix_nanos: u128,
    id_counter: u64,
    sender: Option<mpsc::Sender<CapturedChunk>>,
}

impl VideoChunkBuffer {
    fn new(chunk_duration: Duration, debug_save: bool) -> Self {
        Self::new_with_sender(chunk_duration, debug_save, None)
    }
    fn new_with_sender(
        chunk_duration: Duration,
        debug_save: bool,
        sender: Option<mpsc::Sender<CapturedChunk>>,
    ) -> Self {
        Self {
            chunk_duration,
            debug_save,
            chunk_start: Instant::now(),
            frames_in_chunk: 0,
            accum: Vec::new(),
            start_ts_unix_nanos: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default(),
            id_counter: 0,
            sender,
        }
    }

    fn handle_sample(&mut self, sample: &gst::Sample) {
        // append buffer bytes to accumulator
        if let Some(buffer) = sample.buffer() {
            if let Ok(map) = buffer.map_readable() {
                self.accum.extend_from_slice(map.as_slice());
            }
        }
        self.frames_in_chunk += 1;
        if self.chunk_start.elapsed() >= self.chunk_duration {
            self.flush(sample);
        }
    }

    fn flush(&mut self, sample: &gst::Sample) {
        // gather metadata
        let meta = VideoFrameMetadata::from_sample(sample);
        let id = self.id_counter;
        self.id_counter += 1;
        let duration_ms = self.chunk_duration.as_millis() as u64;
        let metadata = if let Some(m) = meta {
            json!({
                "width": m.width,
                "height": m.height,
                "format": m.format,
                "pts": m.pts.map(|d| d.as_millis())
            })
        } else {
            json!(null)
        };

        let chunk = CapturedChunk {
            id,
            kind: "video".to_string(),
            start_ts_unix_nanos: self.start_ts_unix_nanos,
            duration_ms,
            metadata,
            data_len: self.accum.len(),
            data: std::mem::take(&mut self.accum),
        };

        if let Some(sender) = &self.sender {
            let _ = sender.send(chunk);
        } else {
            println!("[capture] video chunk ready id={} len={}", id, chunk.data_len);
        }

        self.frames_in_chunk = 0;
        self.chunk_start = Instant::now();
        self.start_ts_unix_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
    }
}

#[derive(Debug)]
struct VideoFrameMetadata {
    width: i32,
    height: i32,
    format: Option<String>,
    pts: Option<Duration>,
}

impl VideoFrameMetadata {
    fn from_sample(sample: &gst::Sample) -> Option<Self> {
        let caps = sample.caps()?;
        let structure = caps.structure(0)?;
        let width = structure.get::<i32>("width").ok()?;
        let height = structure.get::<i32>("height").ok()?;
        let format = structure
            .get::<&str>("format")
            .ok()
            .map(|value| value.to_string());
        let pts = sample
            .buffer()
            .and_then(|buffer| buffer.pts())
            .map(|clock_time| Duration::from_nanos(clock_time.nseconds()));

        Some(Self {
            width,
            height,
            format,
            pts,
        })
    }
}

struct AudioChunkBuffer {
    label: &'static str,
    chunk_duration: Duration,
    debug_save: bool,
    chunk_start: Instant,
    frames_accumulated: u64,
    last_metadata: Option<AudioFrameMetadata>,
    accum: Vec<u8>,
    start_ts_unix_nanos: u128,
    id_counter: u64,
    sender: Option<mpsc::Sender<CapturedChunk>>,
}

impl AudioChunkBuffer {
    fn new(label: &'static str, chunk_duration: Duration, debug_save: bool) -> Self {
        Self::new_with_sender(label, chunk_duration, debug_save, None)
    }

    fn new_with_sender(
        label: &'static str,
        chunk_duration: Duration,
        debug_save: bool,
        sender: Option<mpsc::Sender<CapturedChunk>>,
    ) -> Self {
        Self {
            label,
            chunk_duration,
            debug_save,
            chunk_start: Instant::now(),
            frames_accumulated: 0,
            last_metadata: None,
            accum: Vec::new(),
            start_ts_unix_nanos: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default(),
            id_counter: 0,
            sender,
        }
    }

    fn handle_sample(&mut self, sample: &gst::Sample) {
        if let Some(buffer) = sample.buffer() {
            if let Ok(map) = buffer.map_readable() {
                self.accum.extend_from_slice(map.as_slice());
            }
        }
        if let Some(meta) = AudioFrameMetadata::from_sample(sample) {
            self.frames_accumulated += meta.frames as u64;
            self.last_metadata = Some(meta);
        }

        if self.chunk_start.elapsed() >= self.chunk_duration {
            self.flush();
        }
    }

    fn flush(&mut self) {
        let id = self.id_counter;
        self.id_counter += 1;
        let duration_ms = self.chunk_duration.as_millis() as u64;
        let metadata = if let Some(meta) = self.last_metadata.take() {
            json!({
                "rate": meta.rate,
                "channels": meta.channels,
                "format": meta.format,
                "frames": meta.frames,
                "pts_ms": meta.pts.map(|d| d.as_millis())
            })
        } else {
            json!(null)
        };

        let chunk = CapturedChunk {
            id,
            kind: self.label.to_string(),
            start_ts_unix_nanos: self.start_ts_unix_nanos,
            duration_ms,
            metadata,
            data_len: self.accum.len(),
            data: std::mem::take(&mut self.accum),
        };

        if let Some(sender) = &self.sender {
            let _ = sender.send(chunk);
        } else if self.debug_save {
            // handled by global consumer thread
        } else {
            println!("[capture] {} chunk ready id={} len={}", self.label, id, chunk.data_len);
        }

        self.frames_accumulated = 0;
        self.chunk_start = Instant::now();
        self.start_ts_unix_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
    }
}

#[derive(Debug, Clone)]
struct AudioFrameMetadata {
    rate: i32,
    channels: i32,
    format: Option<String>,
    frames: usize,
    pts: Option<Duration>,
}

impl AudioFrameMetadata {
    fn from_sample(sample: &gst::Sample) -> Option<Self> {
        let caps = sample.caps()?;
        let info = gst_audio::AudioInfo::from_caps(caps).ok()?;
        let buffer = sample.buffer()?;
        let map = buffer.map_readable().ok()?;
        let bpf = info.bpf() as usize;
        if bpf == 0 {
            return None;
        }
        let frames = map.size() / bpf;
        let format = Some(info.format().to_string());
        let pts = buffer
            .pts()
            .map(|clock_time| Duration::from_nanos(clock_time.nseconds()));

        Some(Self {
            rate: info.rate() as i32,
            channels: info.channels() as i32,
            format,
            frames,
            pts,
        })
    }
}
