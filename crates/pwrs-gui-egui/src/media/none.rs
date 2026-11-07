//! The backend of a platform whose system player this renderer does not
//! drive: every clip fails to open, saying so.

use super::{Facts, Failure, Frame, News};
use proto::media::{Cause, Settings};
use proto::renderer::WakeLoop;
use pwrs_gui_proto as proto;
use std::sync::Arc;

/// The name the renderer's hello gives this backend: none.
pub(super) const NAME: &str = "";

/// Whether the backend measures the sound it hands the system.
pub(super) const MEASURES: bool = false;

/// A clip this platform's renderer cannot open.
pub(super) struct Engine;

impl Engine {
    pub(super) fn open(_settings: &Settings, _news: Arc<crossbeam_queue::SegQueue<News>>, _wake: Option<WakeLoop>) -> Result<Engine, Failure> {
        Err(Failure::new(Cause::FormatNotSupported, String::from("the egui renderer on this platform plays no media yet")))
    }

    pub(super) fn play(&self) -> Result<(), String> {
        Ok(())
    }

    pub(super) fn pause(&self) -> Result<(), String> {
        Ok(())
    }

    pub(super) fn seek(&self, _seconds: f64) -> Result<(), String> {
        Ok(())
    }

    pub(super) fn seek_near(&self, _seconds: f64) -> Result<(), String> {
        Ok(())
    }

    pub(super) fn set_rate(&self, _rate: f64) -> Result<(), String> {
        Ok(())
    }

    pub(super) fn set_volume(&self, _volume: f64) -> Result<(), String> {
        Ok(())
    }

    pub(super) fn set_muted(&self, _muted: bool) -> Result<(), String> {
        Ok(())
    }

    pub(super) fn set_loop(&self, _looped: bool) -> Result<(), String> {
        Ok(())
    }

    pub(super) fn tend(&self) -> Result<(), String> {
        Ok(())
    }

    pub(super) fn position(&self) -> f64 {
        0.0
    }

    pub(super) fn captions(&self, _position: f64) -> Result<Vec<String>, String> {
        Ok(Vec::new())
    }

    pub(super) fn facts(&self) -> Facts {
        Facts { duration: None, video: None, audio: false }
    }

    pub(super) fn frame(&mut self) -> Result<Option<Frame>, String> {
        Ok(None)
    }
}
