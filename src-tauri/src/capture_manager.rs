use std::sync::Mutex;
use std::time::Duration;

use anyhow::{anyhow, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};

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
    #[allow(dead_code)]
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
    system_audio_pipeline: Option<gst::Pipeline>,
    mic_pipeline: Option<gst::Pipeline>,
}

impl Default for ManagerState {
    fn default() -> Self {
        Self {
            status: CaptureState::Idle,
            options: CaptureOptions::default(),
            video_pipeline: None,
            system_audio_pipeline: None,
            mic_pipeline: None,
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

        self.configure_pipelines(&options)?;

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
        let video_pipeline = gst::Pipeline::new();
        let system_audio_pipeline = gst::Pipeline::new();
        let mic_pipeline = if options.capture_mic {
            Some(gst::Pipeline::new())
        } else {
            None
        };

        let mut inner = self.inner.lock().expect("manager mutex poisoned");
        inner.video_pipeline = Some(video_pipeline);
        inner.system_audio_pipeline = Some(system_audio_pipeline);
        inner.mic_pipeline = mic_pipeline;
        Ok(())
    }

    fn teardown_pipeline(pipeline: Option<gst::Pipeline>) {
        if let Some(p) = pipeline {
            let _ = p.set_state(gst::State::Null);
        }
    }
}
