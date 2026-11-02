//! The audio effect the Media Foundation backend gives each media engine:
//! a transform that hands each block of the decoded sound on unchanged but
//! for its time, after measuring its highest sample for the player's
//! Sound, and moves its time by the shift the clip's edit lists ask for.
//! It takes 32-bit float and 16-bit integer PCM, the formats the system's
//! decoders give; the engine leaves out an effect it cannot connect.

use super::News;
use crossbeam_queue::SegQueue;
use pwrs_gui_proto as proto;
use std::ffi::c_void;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use windows::Win32::Foundation::{E_INVALIDARG, E_NOTIMPL, E_POINTER, E_UNEXPECTED};
use windows::Win32::Media::MediaFoundation::{
    IMFAttributes, IMFMediaEvent, IMFMediaType, IMFSample, IMFTransform, IMFTransform_Impl, MF_E_INVALIDMEDIATYPE, MF_E_INVALIDSTREAMNUMBER, MF_E_NO_MORE_TYPES, MF_E_NOTACCEPTING,
    MF_E_TRANSFORM_NEED_MORE_INPUT, MF_E_TRANSFORM_TYPE_NOT_SET, MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE,
    MFAudioFormat_Float, MFAudioFormat_PCM, MFMediaType_Audio, MFT_INPUT_STATUS_ACCEPT_DATA, MFT_INPUT_STREAM_INFO, MFT_INPUT_STREAM_WHOLE_SAMPLES, MFT_MESSAGE_COMMAND_FLUSH, MFT_MESSAGE_TYPE,
    MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STATUS_SAMPLE_READY, MFT_OUTPUT_STREAM_INFO, MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFT_OUTPUT_STREAM_WHOLE_SAMPLES, MFT_SET_TYPE_TEST_ONLY,
};
use windows::core::{Error, HRESULT, Interface, Ref, Result, implement};

/// How a block's samples are written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Samples {
    Float32,
    Int16,
}

/// The sound of a media type the tap takes: how its samples are written,
/// how many of each channel make a second, and how many channels there are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Format {
    samples: Samples,
    rate: u32,
    channels: u32,
}

impl Format {
    /// How many seconds a block of `bytes` lasts.
    fn seconds(self, bytes: usize) -> f64 {
        let width = match self.samples {
            Samples::Float32 => 4,
            Samples::Int16 => 2,
        };
        (bytes / (width * self.channels as usize)) as f64 / f64::from(self.rate)
    }
}

/// The sound a media type gives, where the tap takes it: 32-bit float or
/// 16-bit integer PCM, with a rate and a number of channels.
fn format_of(kind: &IMFMediaType) -> Result<Format> {
    // SAFETY: the type the pipeline gives, read alone.
    unsafe {
        if kind.GetGUID(&MF_MT_MAJOR_TYPE)? != MFMediaType_Audio {
            return Err(MF_E_INVALIDMEDIATYPE.into());
        }
        let subtype = kind.GetGUID(&MF_MT_SUBTYPE)?;
        let samples = if subtype == MFAudioFormat_Float {
            Samples::Float32
        } else if subtype == MFAudioFormat_PCM && kind.GetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE)? == 16 {
            Samples::Int16
        } else {
            return Err(MF_E_INVALIDMEDIATYPE.into());
        };
        let (rate, channels) = (kind.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)?, kind.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)?);
        if rate == 0 || channels == 0 {
            return Err(MF_E_INVALIDMEDIATYPE.into());
        }
        Ok(Format { samples, rate, channels })
    }
}

/// A media type's sound as the trace names it: its subtype, bits, rate and
/// channels, each as the type gives it.
fn describe(kind: &IMFMediaType) -> String {
    let unset = |e: Error| format!("unset ({:#010x})", e.code().0);
    let number = |r: Result<u32>| r.map_or_else(unset, |n| n.to_string());
    // SAFETY: the type the pipeline gives, read alone.
    let (subtype, bits, rate, channels) =
        unsafe { (kind.GetGUID(&MF_MT_SUBTYPE), kind.GetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE), kind.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND), kind.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)) };
    let subtype = match subtype {
        Ok(s) if s == MFAudioFormat_Float => String::from("float"),
        Ok(s) if s == MFAudioFormat_PCM => String::from("PCM"),
        Ok(s) => format!("{s:?}"),
        Err(e) => unset(e),
    };
    format!("{subtype}, {} bits, {} Hz, {} channels", number(bits), number(rate), number(channels))
}

/// The highest sample of `bytes`, from 0 to 1, as `samples` writes them.
fn peak(bytes: &[u8], samples: Samples) -> f32 {
    match samples {
        Samples::Float32 => bytes.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b).abs()).fold(0.0, f32::max),
        Samples::Int16 => bytes.as_chunks::<2>().0.iter().map(|b| f32::from(i16::from_le_bytes(*b)).abs() / 32768.0).fold(0.0, f32::max),
    }
}

/// What the tap holds between the pipeline's calls.
#[derive(Default)]
struct Held {
    input: Option<IMFMediaType>,
    output: Option<IMFMediaType>,
    /// The sound of the input's type.
    format: Option<Format>,
    /// The block taken in and not yet handed on.
    pending: Option<IMFSample>,
    /// Where in the clip the last block taken starts, in seconds, and
    /// whether it held sound; neither since the input's type was set or the
    /// pipeline flushed. The trace names where the sound starts and stops.
    last: Option<(f64, bool)>,
}

/// `MFCreateMediaType`, which makes a media type.
pub(super) type CreateMediaType = unsafe extern "system" fn(*mut *mut c_void) -> HRESULT;

/// The tap: its media types and the block it holds, which the pipeline's
/// threads reach one at a time, and where it tells what it heard.
#[implement(IMFTransform)]
pub(super) struct Tap {
    held: Mutex<Held>,
    news: Arc<SegQueue<News>>,
    /// How far each block's time moves, in 100-nanosecond units.
    shift: Arc<AtomicI64>,
    /// How far the engine's clock runs ahead of the clip's, in
    /// 100-nanosecond units, which places each block in the clip.
    clock: Arc<AtomicI64>,
    create_type: CreateMediaType,
}

impl Tap {
    /// A tap telling `news` what it hears and where in the clip, moving
    /// each block's time by `shift`, the engine's clock `clock` ahead of the
    /// clip's, as the transform the media engine takes; `create_type` makes
    /// the types it offers.
    pub(super) fn transform(news: Arc<SegQueue<News>>, shift: Arc<AtomicI64>, clock: Arc<AtomicI64>, create_type: CreateMediaType) -> IMFTransform {
        Tap { held: Mutex::new(Held::default()), news, shift, clock, create_type }.into()
    }

    fn held(&self) -> Result<MutexGuard<'_, Held>> {
        self.held.lock().map_err(|e| Error::new(E_UNEXPECTED, format!("the audio tap broke off: {e}")))
    }

    /// Measures `sample`, its sound `format`, and tells its highest sample
    /// as the player's news, with the span of the clip it holds: from its
    /// time as the engine plays it, less how far the engine's clock runs
    /// ahead of the clip's, for as long as its samples last. Gives where it
    /// starts, in seconds, and its highest sample.
    fn measure(&self, sample: &IMFSample, format: Format) -> Result<(f64, f32)> {
        // SAFETY: the pipeline's sample, its time read and its buffer
        // locked, read and unlocked here.
        unsafe {
            let played = sample.GetSampleTime()? + self.shift.load(Ordering::Acquire) - self.clock.load(Ordering::Acquire);
            let buffer = sample.ConvertToContiguousBuffer()?;
            let (mut data, mut length) = (std::ptr::null_mut(), 0u32);
            buffer.Lock(&mut data, None, Some(&mut length))?;
            let loudest = if data.is_null() { 0.0 } else { peak(std::slice::from_raw_parts(data, length as usize), format.samples) };
            buffer.Unlock()?;
            let at = played as f64 / 1e7;
            self.news.push(News::Heard { at, until: at + format.seconds(length as usize), peak: loudest });
            Ok((at, loudest))
        }
    }
}

/// Traces where the sound of a block starting `at` seconds into the clip,
/// its highest sample `peak`, stands against the last block's: where the
/// sound starts and stops, and a block that does not follow the last.
fn follow(last: Option<(f64, bool)>, at: f64, peak: f32) {
    let sounding = peak > 0.0;
    match last {
        Some((before, _)) if at <= before => proto::trace::line(|| format!("media: the audio tap's block at {at:.3} s of the clip does not follow the last, at {before:.3} s")),
        Some((_, was)) if was == sounding => {}
        _ => proto::trace::line(|| format!("media: the audio tap hears {} from {at:.3} s of the clip, the block's peak {peak:.3}", if sounding { "sound" } else { "silence" })),
    }
}

/// Refuses a stream other than the tap's one, numbered 0.
fn one(stream: u32) -> Result<()> {
    if stream == 0 { Ok(()) } else { Err(MF_E_INVALIDSTREAMNUMBER.into()) }
}

/// Writes `value` where the pipeline asks, refusing no place to write it.
fn give<T>(at: *mut T, value: T) -> Result<()> {
    if at.is_null() {
        return Err(E_POINTER.into());
    }
    // SAFETY: the caller's place for the value, not null.
    unsafe { at.write(value) };
    Ok(())
}

impl IMFTransform_Impl for Tap_Impl {
    fn GetStreamLimits(&self, pdwinputminimum: *mut u32, pdwinputmaximum: *mut u32, pdwoutputminimum: *mut u32, pdwoutputmaximum: *mut u32) -> Result<()> {
        give(pdwinputminimum, 1)?;
        give(pdwinputmaximum, 1)?;
        give(pdwoutputminimum, 1)?;
        give(pdwoutputmaximum, 1)
    }

    fn GetStreamCount(&self, pcinputstreams: *mut u32, pcoutputstreams: *mut u32) -> Result<()> {
        give(pcinputstreams, 1)?;
        give(pcoutputstreams, 1)
    }

    /// Its one stream each way is numbered 0, which it need not list.
    fn GetStreamIDs(&self, _dwinputidarraysize: u32, _pdwinputids: *mut u32, _dwoutputidarraysize: u32, _pdwoutputids: *mut u32) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn GetInputStreamInfo(&self, dwinputstreamid: u32, pstreaminfo: *mut MFT_INPUT_STREAM_INFO) -> Result<()> {
        one(dwinputstreamid)?;
        give(pstreaminfo, MFT_INPUT_STREAM_INFO { hnsMaxLatency: 0, dwFlags: MFT_INPUT_STREAM_WHOLE_SAMPLES.0 as u32, cbSize: 0, cbMaxLookahead: 0, cbAlignment: 0 })
    }

    fn GetOutputStreamInfo(&self, dwoutputstreamid: u32) -> Result<MFT_OUTPUT_STREAM_INFO> {
        one(dwoutputstreamid)?;
        Ok(MFT_OUTPUT_STREAM_INFO { dwFlags: (MFT_OUTPUT_STREAM_WHOLE_SAMPLES.0 | MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0) as u32, cbSize: 0, cbAlignment: 0 })
    }

    /// It keeps no attributes, which a transform need not.
    fn GetAttributes(&self) -> Result<IMFAttributes> {
        Err(E_NOTIMPL.into())
    }

    fn GetInputStreamAttributes(&self, _dwinputstreamid: u32) -> Result<IMFAttributes> {
        Err(E_NOTIMPL.into())
    }

    fn GetOutputStreamAttributes(&self, _dwoutputstreamid: u32) -> Result<IMFAttributes> {
        Err(E_NOTIMPL.into())
    }

    /// Its streams are fixed.
    fn DeleteInputStream(&self, _dwstreamid: u32) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn AddInputStreams(&self, _cstreams: u32, _adwstreamids: *const u32) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    /// The types it takes: the type it hands on, once that is set; else
    /// 32-bit float PCM and then 16-bit integer PCM, of any rate and number
    /// of channels.
    fn GetInputAvailableType(&self, dwinputstreamid: u32, dwtypeindex: u32) -> Result<IMFMediaType> {
        one(dwinputstreamid)?;
        if let Some(output) = &self.held()?.output {
            return if dwtypeindex == 0 { Ok(output.clone()) } else { Err(MF_E_NO_MORE_TYPES.into()) };
        }
        let (subtype, bits) = match dwtypeindex {
            0 => (MFAudioFormat_Float, 32),
            1 => (MFAudioFormat_PCM, 16),
            _ => return Err(MF_E_NO_MORE_TYPES.into()),
        };
        let mut raw = std::ptr::null_mut();
        // SAFETY: Media Foundation's own export, which gives a new type;
        // its attributes are set before it is handed out.
        unsafe {
            let made = (self.create_type)(&mut raw);
            if made.is_err() {
                return Err(made.into());
            }
            let kind = IMFMediaType::from_raw(raw);
            kind.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            kind.SetGUID(&MF_MT_SUBTYPE, &subtype)?;
            kind.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, bits)?;
            Ok(kind)
        }
    }

    /// The type it hands on is the type it takes.
    fn GetOutputAvailableType(&self, dwoutputstreamid: u32, dwtypeindex: u32) -> Result<IMFMediaType> {
        one(dwoutputstreamid)?;
        if dwtypeindex > 0 {
            return Err(MF_E_NO_MORE_TYPES.into());
        }
        self.held()?.input.clone().ok_or_else(|| MF_E_TRANSFORM_TYPE_NOT_SET.into())
    }

    fn SetInputType(&self, dwinputstreamid: u32, ptype: Ref<IMFMediaType>, dwflags: u32) -> Result<()> {
        one(dwinputstreamid)?;
        let given: &Option<IMFMediaType> = &ptype;
        let test = dwflags & MFT_SET_TYPE_TEST_ONLY.0 as u32 != 0;
        let format = match given {
            Some(kind) => {
                let format = format_of(kind);
                let asked = if test { "asked whether it takes" } else { "given" };
                proto::trace::line(|| format!("media: the audio tap is {asked} the input {}, which it {}", describe(kind), if format.is_ok() { "takes" } else { "refuses" }));
                Some(format?)
            }
            None => {
                proto::trace::line(|| String::from("media: the audio tap's input type is cleared"));
                None
            }
        };
        if test {
            return Ok(());
        }
        let mut held = self.held()?;
        held.input = given.clone();
        held.format = format;
        held.last = None;
        Ok(())
    }

    fn SetOutputType(&self, dwoutputstreamid: u32, ptype: Ref<IMFMediaType>, dwflags: u32) -> Result<()> {
        one(dwoutputstreamid)?;
        let given: &Option<IMFMediaType> = &ptype;
        let mut held = self.held()?;
        if let Some(kind) = given {
            let format = format_of(kind)?;
            if held.format.is_some_and(|f| f != format) {
                return Err(MF_E_INVALIDMEDIATYPE.into());
            }
        }
        if dwflags & MFT_SET_TYPE_TEST_ONLY.0 as u32 == 0 {
            held.output = given.clone();
        }
        Ok(())
    }

    fn GetInputCurrentType(&self, dwinputstreamid: u32) -> Result<IMFMediaType> {
        one(dwinputstreamid)?;
        self.held()?.input.clone().ok_or_else(|| MF_E_TRANSFORM_TYPE_NOT_SET.into())
    }

    fn GetOutputCurrentType(&self, dwoutputstreamid: u32) -> Result<IMFMediaType> {
        one(dwoutputstreamid)?;
        self.held()?.output.clone().ok_or_else(|| MF_E_TRANSFORM_TYPE_NOT_SET.into())
    }

    fn GetInputStatus(&self, dwinputstreamid: u32) -> Result<u32> {
        one(dwinputstreamid)?;
        Ok(if self.held()?.pending.is_none() { MFT_INPUT_STATUS_ACCEPT_DATA.0 as u32 } else { 0 })
    }

    fn GetOutputStatus(&self) -> Result<u32> {
        Ok(if self.held()?.pending.is_some() { MFT_OUTPUT_STATUS_SAMPLE_READY.0 as u32 } else { 0 })
    }

    /// It hands on every block it takes.
    fn SetOutputBounds(&self, _hnslowerbound: i64, _hnsupperbound: i64) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    /// It takes no events of a stream.
    fn ProcessEvent(&self, _dwinputstreamid: u32, _pevent: Ref<IMFMediaEvent>) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    /// A flush drops the block it holds; every other message needs nothing.
    fn ProcessMessage(&self, emessage: MFT_MESSAGE_TYPE, _ulparam: usize) -> Result<()> {
        if emessage == MFT_MESSAGE_COMMAND_FLUSH {
            let mut held = self.held()?;
            held.pending = None;
            held.last = None;
        }
        Ok(())
    }

    /// Takes a block, measures it and moves its time, and holds it until it
    /// is asked for.
    fn ProcessInput(&self, dwinputstreamid: u32, psample: Ref<IMFSample>, _dwflags: u32) -> Result<()> {
        one(dwinputstreamid)?;
        let given: &Option<IMFSample> = &psample;
        let Some(sample) = given else {
            return Err(E_INVALIDARG.into());
        };
        let mut held = self.held()?;
        if held.pending.is_some() {
            proto::trace::line(|| String::from("media: the audio tap is given a block while it holds one, which it refuses"));
            return Err(MF_E_NOTACCEPTING.into());
        }
        let Some(format) = held.format else {
            proto::trace::line(|| String::from("media: the audio tap is given a block before its input type, which it refuses"));
            return Err(MF_E_TRANSFORM_TYPE_NOT_SET.into());
        };
        let (at, loudest) = self.measure(sample, format).inspect_err(|e| proto::trace::line(|| format!("media: the audio tap cannot measure a block: {}", e.message())))?;
        follow(held.last, at, loudest);
        held.last = Some((at, loudest > 0.0));
        let shift = self.shift.load(Ordering::Acquire);
        if shift != 0 {
            // SAFETY: the pipeline's sample, its time moved alone.
            unsafe { sample.SetSampleTime(sample.GetSampleTime()? + shift)? };
        }
        held.pending = Some(sample.clone());
        Ok(())
    }

    /// Hands on the block it holds, or asks for another.
    fn ProcessOutput(&self, _dwflags: u32, coutputbuffercount: u32, poutputsamples: *mut MFT_OUTPUT_DATA_BUFFER, pdwstatus: *mut u32) -> Result<()> {
        if coutputbuffercount != 1 || poutputsamples.is_null() {
            return Err(E_INVALIDARG.into());
        }
        let Some(sample) = self.held()?.pending.take() else {
            return Err(MF_E_TRANSFORM_NEED_MORE_INPUT.into());
        };
        // SAFETY: the caller's one output buffer, given the sample it holds
        // from now, its status cleared.
        unsafe {
            let out = &mut *poutputsamples;
            out.pSample = std::mem::ManuallyDrop::new(Some(sample));
            out.dwStatus = 0;
        }
        if !pdwstatus.is_null() {
            // SAFETY: the caller's status, not null.
            unsafe { pdwstatus.write(0) };
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_lasts_as_long_as_its_samples_at_the_rate_the_tap_took() {
        let mono = Format { samples: Samples::Float32, rate: 48000, channels: 1 };
        assert_eq!(mono.seconds(192_000), 1.0, "48000 floats of one channel");
        assert_eq!(mono.seconds(4096), 1024.0 / 48000.0, "a block of 1024 floats");
        let stereo = Format { samples: Samples::Int16, rate: 44100, channels: 2 };
        assert_eq!(stereo.seconds(176_400), 1.0, "44100 pairs of 16-bit samples");
        assert_eq!(stereo.seconds(3), 0.0, "bytes short of one sample of each channel last no time");
    }
}
