//! The eight link estimators through the C ABI, one handle kind each. Each
//! is fed measurements a transport already has and answers what it has
//! worked out from them, as Python's `LossKind`, `LossBursts`, `Timing`,
//! `RoundTripShape`, `Periodicity`, `Capacity`, `Forecast` and
//! `PathChanges` and PowerShell's sensors of those names do. They hold no
//! shared memory and touch no network: each lives in this process's memory
//! behind one claim every call takes, and runs no background work, so strict
//! and managed modes are the same.
//!
//! Each is read in one call that fills a reading struct under that claim, so
//! every value in it comes from the same moment. A value that is not there
//! until enough has been seen comes with a `has_` flag, false until then,
//! and reads 0.0 while its flag is false; Python answers None and
//! PowerShell `$null` for the same values. A measurement in microseconds is
//! refused with `SUBETHA_E_INVALID_ARGUMENT` when it is negative or not
//! finite, as the other two refuse it.

use subetha_cxc::burst_model_sensor::BurstModel;
use subetha_cxc::claim::Claim;
use subetha_cxc::forecast_sensor::ArrivalForecast;
use subetha_cxc::loss_class_sensor::{LossClass, LossClassSensor};
use subetha_cxc::path_sensor::PathSensor;
use subetha_cxc::periodicity_sensor::PeriodicitySensor;
use subetha_cxc::rtt_shape_sensor::RttShape;
use subetha_cxc::temporal_sensor::TemporalSensor;
use subetha_cxc::wbest_sensor::WBestEstimator;

use crate::error::{fail, SUBETHA_E_INVALID_ARGUMENT, SUBETHA_E_WRONG_KIND, SUBETHA_OK};
use crate::handle::{
    subetha_handle, Object, SUBETHA_KIND_LINK_CAPACITY, SUBETHA_KIND_LINK_FORECAST, SUBETHA_KIND_LINK_LOSS_BURSTS,
    SUBETHA_KIND_LINK_LOSS_KIND, SUBETHA_KIND_LINK_PATH_CHANGES, SUBETHA_KIND_LINK_PERIODICITY,
    SUBETHA_KIND_LINK_ROUND_TRIP_SHAPE, SUBETHA_KIND_LINK_TIMING,
};
use crate::runtime::{entry, issue, require_initialized, resolve_mode, with_kind};

/// From `subetha_link_loss_kind_classify`: the air was noisy, so recover
/// the loss locally and do not slow down.
pub const SUBETHA_LINK_LOSS_WIRELESS: u32 = 0;
/// From `subetha_link_loss_kind_classify`: a queue overflowed, so raise
/// redundancy and slow the sender.
pub const SUBETHA_LINK_LOSS_CONGESTION: u32 = 1;

/// A loss-kind sensor's reading.
#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Default)]
pub struct subetha_link_loss_kind_reading {
    /// The mode this handle was created in.
    pub mode: u32,
    /// The share of recent losses classified congestion, from zero to one;
    /// zero before any loss has been classified.
    pub congestion_share: f64,
    /// The spread of recent one-way delays, in microseconds; zero before
    /// two delays have been fed.
    pub delay_spread: f64,
}

/// A loss-bursts sensor's reading.
#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Default)]
pub struct subetha_link_loss_bursts_reading {
    /// The mode this handle was created in.
    pub mode: u32,
    /// Items fed.
    pub samples: u64,
    /// Whether the two-state fit stands: false until enough items, losses
    /// among them, have been fed, and while every item or none was lost.
    /// The four values after it read 0.0 while it is false.
    pub has_fit: bool,
    /// How many items a run of losses lasts on average, from 1 to 64.
    pub mean_run_length: f64,
    /// The share lost once the runs are accounted for, from zero to one.
    pub steady_loss: f64,
    /// The chance per item of entering a run of losses.
    pub entering: f64,
    /// The chance per item of leaving one.
    pub leaving: f64,
}

/// A timing sensor's reading.
#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Default)]
pub struct subetha_link_timing_reading {
    /// The mode this handle was created in.
    pub mode: u32,
    /// The items the trend is taken over.
    pub window: u32,
    /// Items in the window now, at most `window`.
    pub samples: u64,
    /// The variation between arrivals, in microseconds.
    pub jitter: f64,
    /// The average spacing between arrivals, in microseconds.
    pub spacing: f64,
    /// The slope of one-way delay over the window, in microseconds of delay
    /// per microsecond: above zero is a queue filling. Zero before two
    /// items.
    pub trend: f64,
    /// `trend` with `clock_skew` taken out, the one to read when the two
    /// clocks are not synchronized.
    pub trend_debiased: f64,
    /// How fast the receiving clock runs against the sending one, in
    /// microseconds gained per microsecond, the units `trend` is in. Zero
    /// before three items.
    pub clock_skew: f64,
}

/// A round-trip-shape sensor's reading.
#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Default)]
pub struct subetha_link_round_trip_shape_reading {
    /// The mode this handle was created in.
    pub mode: u32,
    /// Round trips fed.
    pub samples: u64,
    /// Whether the two-groups coefficient stands: false until enough round
    /// trips with some spread between them have been fed. `two_groups`
    /// reads 0.0 while it is false.
    pub has_two_groups: bool,
    /// How strongly the round trips fall into two groups, from zero to
    /// one: above 5/9 they do.
    pub two_groups: f64,
    /// How much this looks like a wireless link, from zero to one; zero
    /// while the round trips form one group or `has_two_groups` is false.
    pub wireless_confidence: f64,
}

/// A periodicity sensor's reading.
#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Default)]
pub struct subetha_link_periodicity_reading {
    /// The mode this handle was created in.
    pub mode: u32,
    /// Whether a beat was found: false until the delays span two cycles and
    /// a beat stands out strongly enough. The two values after it read 0.0
    /// while it is false.
    pub has_period: bool,
    /// The beat's length, in seconds.
    pub period_seconds: f64,
    /// How strong the beat is, from zero to one.
    pub strength: f64,
    /// Whether the next spike can be placed; `seconds_to_next` reads 0.0
    /// while it is false.
    pub has_seconds_to_next: bool,
    /// Seconds until the next spike is due.
    pub seconds_to_next: f64,
}

/// A capacity sensor's reading.
#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Default)]
pub struct subetha_link_capacity_reading {
    /// The mode this handle was created in.
    pub mode: u32,
    /// The probe size the sensor was made with, in bytes.
    pub probe_bytes: u32,
    /// Pairs that completed with a spread wide enough to measure.
    pub pairs: u64,
    /// Train probes fed.
    pub train_probes: u64,
    /// Whether a pair has completed; `link_capacity` reads 0.0 while it is
    /// false.
    pub has_link_capacity: bool,
    /// The narrowest link's capacity, in bits a second.
    pub link_capacity: f64,
    /// Whether two train probes have arrived with time between them;
    /// `train_rate` reads 0.0 while it is false.
    pub has_train_rate: bool,
    /// The rate the train arrived at, in bits a second.
    pub train_rate: f64,
    /// Whether both of the above stand; `available` reads 0.0 while it is
    /// false.
    pub has_available: bool,
    /// What is left after the traffic already on the path, in bits a
    /// second, at most `link_capacity`.
    pub available: f64,
}

/// A forecast sensor's reading.
#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Default)]
pub struct subetha_link_forecast_reading {
    /// The mode this handle was created in.
    pub mode: u32,
    /// The smoothed rate so far, in bytes a second.
    pub mean_rate: f64,
    /// What the next interval is likely to carry, in bytes a second: a
    /// conservative lower bound, the rate it falls below one interval in
    /// twenty. Zero before the first interval.
    pub next_rate: f64,
}

/// A path-changes sensor's reading.
#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Default)]
pub struct subetha_link_path_changes_reading {
    /// The mode this handle was created in.
    pub mode: u32,
    /// How much the route has been moving, from zero to one: one on a
    /// change in hop count, fading while the route holds.
    pub route_movement: f64,
    /// The share of items marked congestion experienced, from zero to one.
    pub marked_share: f64,
    /// Whether an item has been fed; the three values after it read zero
    /// while it is false.
    pub has_last: bool,
    /// The last item's remaining time to live.
    pub last_ttl: u8,
    /// The last item's congestion marking.
    pub last_congestion_mark: u8,
    /// How many hops the last item took.
    pub last_hops: u8,
}

enum Sensor {
    LossKind(LossClassSensor),
    LossBursts(BurstModel),
    Timing(TemporalSensor),
    RoundTripShape(RttShape),
    Periodicity(PeriodicitySensor),
    Capacity(WBestEstimator),
    Forecast(ArrivalForecast),
    PathChanges(PathSensor),
}

pub(crate) struct LinkSensorObject {
    kind: u32,
    mode: u32,
    /// What the sensor was made with that its reading reports: the timing
    /// window or the probe size, zero for the rest.
    made_with: u32,
    sensor: Claim<Sensor>,
}

impl LinkSensorObject {
    pub(crate) fn kind(&self) -> u32 {
        self.kind
    }

    /// A sensor parks nothing inside a call, so a destroy has nobody to
    /// wake.
    pub(crate) fn interrupt(&self) {}
}

/// Check what every constructor reads and issue the handle: the library is
/// initialized, `out` is not null, the sensor `make` builds from its own
/// arguments, then `mode`.
///
/// # Safety
/// `out` is null or a valid pointer.
unsafe fn issue_sensor(
    kind: u32,
    made_with: u32,
    mode: u32,
    out: *mut subetha_handle,
    make: impl FnOnce() -> Result<Sensor, i32>,
) -> i32 {
    if let Err(code) = require_initialized() {
        return code;
    }
    if out.is_null() {
        return fail(SUBETHA_E_INVALID_ARGUMENT, "out is null");
    }
    let sensor = match make() {
        Ok(s) => s,
        Err(code) => return code,
    };
    let mode = match resolve_mode(mode) {
        Ok(m) => m,
        Err(code) => return code,
    };
    let object = LinkSensorObject { kind, mode, made_with, sensor: Claim::new(sensor) };
    unsafe { issue(Object::LinkSensor(object), out) }
}

/// The accessor for one kind of sensor: run `f` on it under its claim, with
/// the mode and the construction argument it was made with.
macro_rules! sensor_access {
    ($name:ident, $kind:ident, $variant:ident, $ty:ty, $what:literal) => {
        fn $name(handle: subetha_handle, f: impl FnOnce(&mut $ty, u32, u32) -> i32) -> i32 {
            with_kind(handle, $kind, |object| match object {
                Object::LinkSensor(s) => match &mut *s.sensor.claim() {
                    Sensor::$variant(inner) => f(inner, s.mode, s.made_with),
                    _ => fail(SUBETHA_E_WRONG_KIND, concat!("the handle does not name ", $what)),
                },
                _ => fail(SUBETHA_E_WRONG_KIND, concat!("the handle does not name ", $what)),
            })
        }
    };
}

sensor_access!(with_loss_kind, SUBETHA_KIND_LINK_LOSS_KIND, LossKind, LossClassSensor, "a loss-kind sensor");
sensor_access!(with_loss_bursts, SUBETHA_KIND_LINK_LOSS_BURSTS, LossBursts, BurstModel, "a loss-bursts sensor");
sensor_access!(with_timing, SUBETHA_KIND_LINK_TIMING, Timing, TemporalSensor, "a timing sensor");
sensor_access!(with_round_trip_shape, SUBETHA_KIND_LINK_ROUND_TRIP_SHAPE, RoundTripShape, RttShape, "a round-trip-shape sensor");
sensor_access!(with_periodicity, SUBETHA_KIND_LINK_PERIODICITY, Periodicity, PeriodicitySensor, "a periodicity sensor");
sensor_access!(with_capacity, SUBETHA_KIND_LINK_CAPACITY, Capacity, WBestEstimator, "a capacity sensor");
sensor_access!(with_forecast, SUBETHA_KIND_LINK_FORECAST, Forecast, ArrivalForecast, "a forecast sensor");
sensor_access!(with_path_changes, SUBETHA_KIND_LINK_PATH_CHANGES, PathChanges, PathSensor, "a path-changes sensor");

/// A measurement a sensor can use: a number of microseconds, finite and
/// not negative.
fn measurement(value: f64, what: &str) -> Result<f64, i32> {
    if !value.is_finite() || value < 0.0 {
        return Err(fail(
            SUBETHA_E_INVALID_ARGUMENT,
            format!("{what} is a number of microseconds that is not negative, got {value}"),
        ));
    }
    Ok(value)
}

/// Write `reading` into the caller's `out`.
///
/// # Safety
/// `out` is null or a valid pointer.
unsafe fn write_reading<T>(out: *mut T, reading: T) -> i32 {
    if out.is_null() {
        return fail(SUBETHA_E_INVALID_ARGUMENT, "out is null");
    }
    // SAFETY: checked non-null; the caller guarantees it is writable.
    unsafe { out.write(reading) };
    SUBETHA_OK
}

/// A value and whether it is there, as a reading carries it: 0.0 when not.
fn flagged(value: Option<f64>) -> (bool, f64) {
    match value {
        Some(v) => (true, v),
        None => (false, 0.0),
    }
}

// The loss-kind sensor.

/// Make a sensor telling a loss caused by a noisy radio from one caused by
/// a queue that overflowed, with nothing observed yet.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_link_loss_kind_create(mode: u32, out: *mut subetha_handle) -> i32 {
    entry(|| unsafe {
        issue_sensor(SUBETHA_KIND_LINK_LOSS_KIND, 0, mode, out, || Ok(Sensor::LossKind(LossClassSensor::new())))
    })
}

/// Feed the spacing between two arrivals, in microseconds. A spacing of
/// zero is accepted and changes nothing, as for a duplicate arrival.
#[unsafe(no_mangle)]
pub extern "C" fn subetha_link_loss_kind_observe_spacing(sensor: subetha_handle, microseconds: f64) -> i32 {
    with_loss_kind(sensor, |s, _, _| match measurement(microseconds, "a spacing") {
        Ok(spacing) => {
            s.observe_interarrival(spacing);
            SUBETHA_OK
        }
        Err(code) => code,
    })
}

/// Feed a one-way delay, in microseconds.
#[unsafe(no_mangle)]
pub extern "C" fn subetha_link_loss_kind_observe_delay(sensor: subetha_handle, microseconds: f64) -> i32 {
    with_loss_kind(sensor, |s, _, _| match measurement(microseconds, "a delay") {
        Ok(delay) => {
            s.observe_owd(delay);
            SUBETHA_OK
        }
        Err(code) => code,
    })
}

/// What a loss of `gap` items was, given the spacing that held it, into
/// `out_class`: `SUBETHA_LINK_LOSS_WIRELESS` or
/// `SUBETHA_LINK_LOSS_CONGESTION`. Congestion is answered when either the
/// spacing shows a queue or the delays are in a congestion spike, since a
/// congestion loss taken for wireless is the costlier mistake. Each call
/// counts toward the congestion share.
///
/// # Safety
/// `out_class` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_link_loss_kind_classify(
    sensor: subetha_handle,
    gap: u32,
    spacing_microseconds: f64,
    out_class: *mut u32,
) -> i32 {
    with_loss_kind(sensor, |s, _, _| {
        if out_class.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out_class is null");
        }
        let spacing = match measurement(spacing_microseconds, "a spacing") {
            Ok(v) => v,
            Err(code) => return code,
        };
        let class = match s.classify(gap, spacing) {
            LossClass::Wireless => SUBETHA_LINK_LOSS_WIRELESS,
            LossClass::Congestion => SUBETHA_LINK_LOSS_CONGESTION,
        };
        // SAFETY: checked non-null; the caller guarantees it is writable.
        unsafe { *out_class = class };
        SUBETHA_OK
    })
}

/// The loss-kind sensor's reading, into `out`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_link_loss_kind_read(sensor: subetha_handle, out: *mut subetha_link_loss_kind_reading) -> i32 {
    with_loss_kind(sensor, |s, mode, _| {
        let reading = subetha_link_loss_kind_reading {
            mode,
            congestion_share: f64::from(s.congestion_fraction()),
            delay_spread: s.recent_owd_spread_us(),
        };
        unsafe { write_reading(out, reading) }
    })
}

// The loss-bursts sensor.

/// Make a sensor fitting whether losses come alone or in runs, with
/// nothing observed yet.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_link_loss_bursts_create(mode: u32, out: *mut subetha_handle) -> i32 {
    entry(|| unsafe {
        issue_sensor(SUBETHA_KIND_LINK_LOSS_BURSTS, 0, mode, out, || Ok(Sensor::LossBursts(BurstModel::new())))
    })
}

/// Feed one item: `lost` is true when it was lost.
#[unsafe(no_mangle)]
pub extern "C" fn subetha_link_loss_bursts_observe(sensor: subetha_handle, lost: bool) -> i32 {
    with_loss_bursts(sensor, |s, _, _| {
        s.observe(lost);
        SUBETHA_OK
    })
}

/// Feed `count` items in order under one handle lookup and one claim,
/// `losses[i]` true when item `i` was lost.
///
/// # Safety
/// `losses` addresses `count` readable flags.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_link_loss_bursts_observe_many(sensor: subetha_handle, losses: *const bool, count: usize) -> i32 {
    with_loss_bursts(sensor, |s, _, _| {
        if count == 0 {
            return SUBETHA_OK;
        }
        if losses.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "losses is null with a non-zero count");
        }
        // SAFETY: the caller guarantees `count` readable flags at `losses`.
        let items = unsafe { std::slice::from_raw_parts(losses, count) };
        for &lost in items {
            s.observe(lost);
        }
        SUBETHA_OK
    })
}

/// The loss-bursts sensor's reading, into `out`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_link_loss_bursts_read(sensor: subetha_handle, out: *mut subetha_link_loss_bursts_reading) -> i32 {
    with_loss_bursts(sensor, |s, mode, _| {
        let mut reading = subetha_link_loss_bursts_reading { mode, samples: s.samples(), ..Default::default() };
        if let (Some((entering, leaving)), Some(run), Some(steady)) = (s.fit(), s.mean_burst_len(), s.steady_loss()) {
            reading.has_fit = true;
            reading.mean_run_length = run;
            reading.steady_loss = steady;
            reading.entering = entering;
            reading.leaving = leaving;
        }
        unsafe { write_reading(out, reading) }
    })
}

// The timing sensor.

/// Make a sensor reading jitter, spacing and whether one-way delay is
/// climbing, over the last `window` items, at least 2.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_link_timing_create(window: u32, mode: u32, out: *mut subetha_handle) -> i32 {
    entry(|| unsafe {
        issue_sensor(SUBETHA_KIND_LINK_TIMING, window, mode, out, || {
            if window < 2 {
                return Err(fail(SUBETHA_E_INVALID_ARGUMENT, "a window covers at least two items"));
            }
            Ok(Sensor::Timing(TemporalSensor::new(window as usize)))
        })
    })
}

/// Feed one item's send time, on the sender's clock, and receive time, on
/// the receiver's, both in microseconds. The two clocks need not agree,
/// and a send time may go back for an item that arrived out of order. A
/// receive time earlier than the previous one is refused with
/// `SUBETHA_E_INVALID_ARGUMENT` naming both and not counted: it is the
/// receiving clock stepping back, not the link.
#[unsafe(no_mangle)]
pub extern "C" fn subetha_link_timing_observe(sensor: subetha_handle, sent: u64, received: u64) -> i32 {
    with_timing(sensor, |s, _, _| match s.checked_observe(sent, received) {
        Ok(()) => SUBETHA_OK,
        Err(back) => fail(SUBETHA_E_INVALID_ARGUMENT, back.to_string()),
    })
}

/// The timing sensor's reading, into `out`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_link_timing_read(sensor: subetha_handle, out: *mut subetha_link_timing_reading) -> i32 {
    with_timing(sensor, |s, mode, window| {
        let reading = subetha_link_timing_reading {
            mode,
            window,
            samples: s.samples() as u64,
            jitter: s.jitter_micros(),
            spacing: s.interarrival_micros(),
            trend: s.owd_trend(),
            trend_debiased: s.owd_trend_debiased(),
            clock_skew: s.skew(),
        };
        unsafe { write_reading(out, reading) }
    })
}

// The round-trip-shape sensor.

/// Make a sensor looking for round trips that fall into two groups, which
/// is what a wireless hop's retries look like, with nothing observed yet.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_link_round_trip_shape_create(mode: u32, out: *mut subetha_handle) -> i32 {
    entry(|| unsafe {
        issue_sensor(SUBETHA_KIND_LINK_ROUND_TRIP_SHAPE, 0, mode, out, || Ok(Sensor::RoundTripShape(RttShape::new())))
    })
}

/// Feed one round trip, in microseconds.
#[unsafe(no_mangle)]
pub extern "C" fn subetha_link_round_trip_shape_observe(sensor: subetha_handle, microseconds: f64) -> i32 {
    with_round_trip_shape(sensor, |s, _, _| match measurement(microseconds, "a round trip") {
        Ok(rtt) => {
            s.observe(rtt);
            SUBETHA_OK
        }
        Err(code) => code,
    })
}

/// Feed `count` round trips under one handle lookup and one claim. Every
/// one is checked before any is fed, so a refusal, naming the first bad
/// one, feeds none.
///
/// # Safety
/// `microseconds` addresses `count` readable doubles.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_link_round_trip_shape_observe_many(
    sensor: subetha_handle,
    microseconds: *const f64,
    count: usize,
) -> i32 {
    with_round_trip_shape(sensor, |s, _, _| {
        if count == 0 {
            return SUBETHA_OK;
        }
        if microseconds.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "microseconds is null with a non-zero count");
        }
        // SAFETY: the caller guarantees `count` readable doubles.
        let rtts = unsafe { std::slice::from_raw_parts(microseconds, count) };
        for (i, &rtt) in rtts.iter().enumerate() {
            if let Err(code) = measurement(rtt, &format!("round trip {i}")) {
                return code;
            }
        }
        for &rtt in rtts {
            s.observe(rtt);
        }
        SUBETHA_OK
    })
}

/// The round-trip-shape sensor's reading, into `out`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_link_round_trip_shape_read(
    sensor: subetha_handle,
    out: *mut subetha_link_round_trip_shape_reading,
) -> i32 {
    with_round_trip_shape(sensor, |s, mode, _| {
        let (has_two_groups, two_groups) = flagged(s.bimodality());
        let reading = subetha_link_round_trip_shape_reading {
            mode,
            samples: s.samples(),
            has_two_groups,
            two_groups,
            wireless_confidence: f64::from(s.wifi_confidence()),
        };
        unsafe { write_reading(out, reading) }
    })
}

// The periodicity sensor.

/// Make a sensor looking for a regular beat in the delays and when the
/// next spike is due, with nothing observed yet.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_link_periodicity_create(mode: u32, out: *mut subetha_handle) -> i32 {
    entry(|| unsafe {
        issue_sensor(SUBETHA_KIND_LINK_PERIODICITY, 0, mode, out, || Ok(Sensor::Periodicity(PeriodicitySensor::new())))
    })
}

/// Feed one delay, in microseconds, and when it was taken, in microseconds
/// on a clock that does not step back; a beat can only be found against
/// time.
#[unsafe(no_mangle)]
pub extern "C" fn subetha_link_periodicity_observe(sensor: subetha_handle, delay_microseconds: f64, at_microseconds: u64) -> i32 {
    with_periodicity(sensor, |s, _, _| match measurement(delay_microseconds, "a delay") {
        Ok(delay) => {
            s.observe(delay, at_microseconds);
            SUBETHA_OK
        }
        Err(code) => code,
    })
}

/// The periodicity sensor's reading, into `out`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_link_periodicity_read(sensor: subetha_handle, out: *mut subetha_link_periodicity_reading) -> i32 {
    with_periodicity(sensor, |s, mode, _| {
        let mut reading = subetha_link_periodicity_reading { mode, ..Default::default() };
        if let Some((period_seconds, strength)) = s.detected_period() {
            reading.has_period = true;
            reading.period_seconds = period_seconds;
            reading.strength = strength;
        }
        (reading.has_seconds_to_next, reading.seconds_to_next) = flagged(s.secs_to_next_spike());
        unsafe { write_reading(out, reading) }
    })
}

// The capacity sensor.

/// Make a sensor estimating the path's capacity from probes of
/// `probe_bytes` bytes each, at least 1, sent in pairs and in trains.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_link_capacity_create(probe_bytes: u32, mode: u32, out: *mut subetha_handle) -> i32 {
    entry(|| unsafe {
        issue_sensor(SUBETHA_KIND_LINK_CAPACITY, probe_bytes, mode, out, || {
            if probe_bytes == 0 {
                return Err(fail(SUBETHA_E_INVALID_ARGUMENT, "a probe is at least one byte"));
            }
            Ok(Sensor::Capacity(WBestEstimator::new(probe_bytes as usize)))
        })
    })
}

/// Feed the arrival of one probe of a pair, in microseconds on a clock that
/// does not step back: `index` 0 is the pair's first probe and any other
/// value its second. A pair whose two probes arrived together carries no
/// rate and is not counted.
#[unsafe(no_mangle)]
pub extern "C" fn subetha_link_capacity_observe_pair(sensor: subetha_handle, index: u8, arrived_microseconds: f64) -> i32 {
    with_capacity(sensor, |s, _, _| match measurement(arrived_microseconds, "an arrival") {
        Ok(arrived) => {
            s.on_pair_probe(index, arrived);
            SUBETHA_OK
        }
        Err(code) => code,
    })
}

/// Feed the arrival of one probe of a train, in microseconds on a clock
/// that does not step back.
#[unsafe(no_mangle)]
pub extern "C" fn subetha_link_capacity_observe_train(sensor: subetha_handle, arrived_microseconds: f64) -> i32 {
    with_capacity(sensor, |s, _, _| match measurement(arrived_microseconds, "an arrival") {
        Ok(arrived) => {
            s.on_train_probe(arrived);
            SUBETHA_OK
        }
        Err(code) => code,
    })
}

/// Forget every probe and start again, which is what a caller does when
/// the path may have changed.
#[unsafe(no_mangle)]
pub extern "C" fn subetha_link_capacity_reset(sensor: subetha_handle) -> i32 {
    with_capacity(sensor, |s, _, _| {
        s.reset();
        SUBETHA_OK
    })
}

/// The capacity sensor's reading, into `out`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_link_capacity_read(sensor: subetha_handle, out: *mut subetha_link_capacity_reading) -> i32 {
    with_capacity(sensor, |s, mode, probe_bytes| {
        let (pairs, train_probes) = s.samples();
        let (has_link_capacity, link_capacity) = flagged(s.effective_capacity_bps());
        let (has_train_rate, train_rate) = flagged(s.train_rate_bps());
        let (has_available, available) = flagged(s.available_bps());
        let reading = subetha_link_capacity_reading {
            mode,
            probe_bytes,
            pairs: pairs as u64,
            train_probes: u64::from(train_probes),
            has_link_capacity,
            link_capacity,
            has_train_rate,
            train_rate,
            has_available,
            available,
        };
        unsafe { write_reading(out, reading) }
    })
}

// The forecast sensor.

/// Make a sensor forecasting what the next interval's traffic is likely to
/// be from what the last ones were, with nothing observed yet.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_link_forecast_create(mode: u32, out: *mut subetha_handle) -> i32 {
    entry(|| unsafe {
        issue_sensor(SUBETHA_KIND_LINK_FORECAST, 0, mode, out, || Ok(Sensor::Forecast(ArrivalForecast::new())))
    })
}

/// Feed one interval: `bytes` arrived over `seconds`, which must be finite
/// and above zero.
#[unsafe(no_mangle)]
pub extern "C" fn subetha_link_forecast_observe(sensor: subetha_handle, bytes: u64, seconds: f64) -> i32 {
    with_forecast(sensor, |s, _, _| {
        if !seconds.is_finite() || seconds <= 0.0 {
            return fail(SUBETHA_E_INVALID_ARGUMENT, format!("an interval is a number of seconds above zero, got {seconds}"));
        }
        s.observe(bytes, seconds);
        SUBETHA_OK
    })
}

/// The forecast sensor's reading, into `out`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_link_forecast_read(sensor: subetha_handle, out: *mut subetha_link_forecast_reading) -> i32 {
    with_forecast(sensor, |s, mode, _| {
        let reading = subetha_link_forecast_reading { mode, mean_rate: s.mean_bps(), next_rate: s.forecast_bps() };
        unsafe { write_reading(out, reading) }
    })
}

// The path-changes sensor.

/// Make a sensor watching for the route changing under the traffic, with
/// nothing observed yet.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_link_path_changes_create(mode: u32, out: *mut subetha_handle) -> i32 {
    entry(|| unsafe {
        issue_sensor(SUBETHA_KIND_LINK_PATH_CHANGES, 0, mode, out, || Ok(Sensor::PathChanges(PathSensor::new())))
    })
}

/// Feed one item received: its remaining time to live, its two-bit
/// congestion marking (3 is congestion experienced), and how many hops it
/// took.
#[unsafe(no_mangle)]
pub extern "C" fn subetha_link_path_changes_observe(sensor: subetha_handle, ttl: u8, congestion_mark: u8, hops: u8) -> i32 {
    with_path_changes(sensor, |s, _, _| {
        s.observe(ttl, congestion_mark, hops);
        SUBETHA_OK
    })
}

/// The path-changes sensor's reading, into `out`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_link_path_changes_read(sensor: subetha_handle, out: *mut subetha_link_path_changes_reading) -> i32 {
    with_path_changes(sensor, |s, mode, _| {
        let mut reading = subetha_link_path_changes_reading {
            mode,
            route_movement: f64::from(s.path_shift()),
            marked_share: f64::from(s.ecn_ce()),
            ..Default::default()
        };
        if let Some((ttl, mark, hops)) = s.last() {
            reading.has_last = true;
            reading.last_ttl = ttl;
            reading.last_congestion_mark = mark;
            reading.last_hops = hops;
        }
        unsafe { write_reading(out, reading) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{subetha_handle_destroy, subetha_init, SUBETHA_MODE_STRICT};

    /// The loss sensors: nothing to say until fed enough, then what the
    /// Rust sensors say from the same items; a measurement Python refuses
    /// is refused; a handle of one sensor is refused by another's calls.
    #[test]
    fn the_loss_sensors_read_what_rust_reads_from_the_same_items() {
        assert_eq!(subetha_init(SUBETHA_MODE_STRICT), SUBETHA_OK);
        let (mut kind, mut bursts) = (0, 0);
        let mut class = 9;
        // SAFETY: valid out pointers and arrays throughout.
        unsafe {
            assert_eq!(subetha_link_loss_kind_create(SUBETHA_MODE_STRICT, &mut kind), SUBETHA_OK);
            let mut rust_kind = LossClassSensor::new();
            for spacing in [1_000.0, 1_200.0, 1_000.0] {
                assert_eq!(subetha_link_loss_kind_observe_spacing(kind, spacing), SUBETHA_OK);
                rust_kind.observe_interarrival(spacing);
            }
            for delay in [5_000.0, 5_100.0] {
                assert_eq!(subetha_link_loss_kind_observe_delay(kind, delay), SUBETHA_OK);
                rust_kind.observe_owd(delay);
            }
            assert_eq!(subetha_link_loss_kind_classify(kind, 1, 2_000.0, &mut class), SUBETHA_OK);
            assert_eq!(class, SUBETHA_LINK_LOSS_WIRELESS, "spacing inside the window and no spike");
            assert_eq!(rust_kind.classify(1, 2_000.0), LossClass::Wireless);
            assert_eq!(subetha_link_loss_kind_classify(kind, 1, 5_000.0, &mut class), SUBETHA_OK);
            assert_eq!(class, SUBETHA_LINK_LOSS_CONGESTION, "spacing past the window is a queue");
            assert_eq!(rust_kind.classify(1, 5_000.0), LossClass::Congestion);
            let mut k = subetha_link_loss_kind_reading::default();
            assert_eq!(subetha_link_loss_kind_read(kind, &mut k), SUBETHA_OK);
            assert_eq!(k.congestion_share, f64::from(rust_kind.congestion_fraction()));
            assert_eq!(k.delay_spread, rust_kind.recent_owd_spread_us());
            assert_eq!(subetha_link_loss_kind_observe_spacing(kind, -1.0), SUBETHA_E_INVALID_ARGUMENT);
            assert_eq!(subetha_link_loss_kind_observe_delay(kind, f64::NAN), SUBETHA_E_INVALID_ARGUMENT);

            assert_eq!(subetha_link_loss_bursts_create(SUBETHA_MODE_STRICT, &mut bursts), SUBETHA_OK);
            let mut b = subetha_link_loss_bursts_reading::default();
            assert_eq!(subetha_link_loss_bursts_read(bursts, &mut b), SUBETHA_OK);
            assert!(!b.has_fit);
            assert_eq!((b.samples, b.mean_run_length, b.entering), (0, 0.0, 0.0));
            let trace: Vec<bool> = (0..300).map(|i| i % 10 < 3).collect();
            assert_eq!(subetha_link_loss_bursts_observe_many(bursts, trace.as_ptr(), trace.len()), SUBETHA_OK);
            assert_eq!(subetha_link_loss_bursts_observe(bursts, false), SUBETHA_OK);
            let mut rust_bursts = BurstModel::new();
            for &lost in &trace {
                rust_bursts.observe(lost);
            }
            rust_bursts.observe(false);
            assert_eq!(subetha_link_loss_bursts_read(bursts, &mut b), SUBETHA_OK);
            assert!(b.has_fit);
            assert_eq!(b.samples, 301);
            assert_eq!(Some(b.mean_run_length), rust_bursts.mean_burst_len());
            assert_eq!(Some(b.steady_loss), rust_bursts.steady_loss());
            assert_eq!(Some((b.entering, b.leaving)), rust_bursts.fit());

            assert_eq!(subetha_link_timing_observe(bursts, 0, 1), SUBETHA_E_WRONG_KIND);
            assert_eq!(subetha_link_loss_bursts_observe(kind, true), SUBETHA_E_WRONG_KIND);
            assert_eq!(subetha_handle_destroy(kind), SUBETHA_OK);
            assert_eq!(subetha_handle_destroy(bursts), SUBETHA_OK);
        }
    }

    /// The timing, round-trip and periodicity sensors read what the Rust
    /// sensors read; a receive time that goes back and a batch holding a
    /// bad round trip are refused with nothing counted.
    #[test]
    fn the_delay_sensors_read_what_rust_reads_and_refuse_what_python_refuses() {
        assert_eq!(subetha_init(SUBETHA_MODE_STRICT), SUBETHA_OK);
        let (mut timing, mut shape, mut beat, mut other) = (0, 0, 0, 0);
        // SAFETY: valid out pointers and arrays throughout.
        unsafe {
            assert_eq!(subetha_link_timing_create(1, SUBETHA_MODE_STRICT, &mut other), SUBETHA_E_INVALID_ARGUMENT);
            assert_eq!(subetha_link_timing_create(8, SUBETHA_MODE_STRICT, &mut timing), SUBETHA_OK);
            let mut rust_timing = TemporalSensor::new(8);
            for (sent, received) in [(0u64, 1_000u64), (1_000, 2_050), (2_000, 3_000), (3_000, 4_200)] {
                assert_eq!(subetha_link_timing_observe(timing, sent, received), SUBETHA_OK);
                rust_timing.observe(sent, received);
            }
            assert_eq!(subetha_link_timing_observe(timing, 4_000, 4_100), SUBETHA_E_INVALID_ARGUMENT);
            let mut t = subetha_link_timing_reading::default();
            assert_eq!(subetha_link_timing_read(timing, &mut t), SUBETHA_OK);
            assert_eq!((t.window, t.samples), (8, 4), "the refused item is not counted");
            assert_eq!(t.jitter, rust_timing.jitter_micros());
            assert_eq!(t.spacing, rust_timing.interarrival_micros());
            assert_eq!(t.trend, rust_timing.owd_trend());
            assert_eq!(t.trend_debiased, rust_timing.owd_trend_debiased());
            assert_eq!(t.clock_skew, rust_timing.skew());

            assert_eq!(subetha_link_round_trip_shape_create(SUBETHA_MODE_STRICT, &mut shape), SUBETHA_OK);
            let mut r = subetha_link_round_trip_shape_reading::default();
            assert_eq!(subetha_link_round_trip_shape_read(shape, &mut r), SUBETHA_OK);
            assert!(!r.has_two_groups);
            let rtts: Vec<f64> = (0..100).map(|i| if i % 2 == 0 { 1_000.0 } else { 9_000.0 }).collect();
            let bad = [1_000.0, -5.0];
            assert_eq!(subetha_link_round_trip_shape_observe_many(shape, bad.as_ptr(), bad.len()), SUBETHA_E_INVALID_ARGUMENT);
            assert_eq!(subetha_link_round_trip_shape_observe_many(shape, rtts.as_ptr(), rtts.len()), SUBETHA_OK);
            assert_eq!(subetha_link_round_trip_shape_observe(shape, 1_000.0), SUBETHA_OK);
            let mut rust_shape = RttShape::new();
            for &rtt in &rtts {
                rust_shape.observe(rtt);
            }
            rust_shape.observe(1_000.0);
            assert_eq!(subetha_link_round_trip_shape_read(shape, &mut r), SUBETHA_OK);
            assert_eq!(r.samples, 101, "the refused batch fed nothing");
            assert!(r.has_two_groups);
            assert_eq!(Some(r.two_groups), rust_shape.bimodality());
            assert_eq!(r.wireless_confidence, f64::from(rust_shape.wifi_confidence()));

            assert_eq!(subetha_link_periodicity_create(SUBETHA_MODE_STRICT, &mut beat), SUBETHA_OK);
            let mut p = subetha_link_periodicity_reading::default();
            assert_eq!(subetha_link_periodicity_read(beat, &mut p), SUBETHA_OK);
            assert!(!p.has_period && !p.has_seconds_to_next);
            let mut rust_beat = PeriodicitySensor::new();
            for i in 0..500u64 {
                let at = i * 100_000;
                let phase = (at as f64 / 1e6) / 10.0 * std::f64::consts::TAU;
                let delay = 40_000.0 + 60_000.0 * phase.sin().max(0.0).powi(4);
                assert_eq!(subetha_link_periodicity_observe(beat, delay, at), SUBETHA_OK);
                rust_beat.observe(delay, at);
            }
            assert_eq!(subetha_link_periodicity_read(beat, &mut p), SUBETHA_OK);
            assert!(p.has_period && p.has_seconds_to_next);
            assert_eq!(Some((p.period_seconds, p.strength)), rust_beat.detected_period());
            assert_eq!(Some(p.seconds_to_next), rust_beat.secs_to_next_spike());
            assert_eq!(subetha_link_periodicity_observe(beat, f64::INFINITY, 0), SUBETHA_E_INVALID_ARGUMENT);

            for h in [timing, shape, beat] {
                assert_eq!(subetha_handle_destroy(h), SUBETHA_OK);
            }
        }
    }

    /// The capacity, forecast and path sensors read what the Rust sensors
    /// read, and a reset forgets every probe.
    #[test]
    fn the_rate_and_path_sensors_read_what_rust_reads() {
        assert_eq!(subetha_init(SUBETHA_MODE_STRICT), SUBETHA_OK);
        let (mut capacity, mut forecast, mut path, mut other) = (0, 0, 0, 0);
        // SAFETY: valid out pointers throughout.
        unsafe {
            assert_eq!(subetha_link_capacity_create(0, SUBETHA_MODE_STRICT, &mut other), SUBETHA_E_INVALID_ARGUMENT);
            assert_eq!(subetha_link_capacity_create(1_400, SUBETHA_MODE_STRICT, &mut capacity), SUBETHA_OK);
            let mut c = subetha_link_capacity_reading::default();
            assert_eq!(subetha_link_capacity_read(capacity, &mut c), SUBETHA_OK);
            assert!(!c.has_link_capacity && !c.has_train_rate && !c.has_available);
            assert_eq!(c.probe_bytes, 1_400);
            let mut rust_capacity = WBestEstimator::new(1_400);
            assert_eq!(subetha_link_capacity_observe_pair(capacity, 0, 1_000.0), SUBETHA_OK);
            assert_eq!(subetha_link_capacity_observe_pair(capacity, 1, 1_100.0), SUBETHA_OK);
            rust_capacity.on_pair_probe(0, 1_000.0);
            rust_capacity.on_pair_probe(1, 1_100.0);
            assert_eq!(subetha_link_capacity_read(capacity, &mut c), SUBETHA_OK);
            assert!(c.has_link_capacity && !c.has_train_rate && !c.has_available, "a pair without a train");
            for i in 0..20u32 {
                let at = 10_000.0 + 400.0 * f64::from(i);
                assert_eq!(subetha_link_capacity_observe_train(capacity, at), SUBETHA_OK);
                rust_capacity.on_train_probe(at);
            }
            assert_eq!(subetha_link_capacity_read(capacity, &mut c), SUBETHA_OK);
            assert!(c.has_link_capacity && c.has_train_rate && c.has_available);
            assert_eq!((c.pairs, c.train_probes), (1, 20));
            assert_eq!(Some(c.link_capacity), rust_capacity.effective_capacity_bps());
            assert_eq!(Some(c.train_rate), rust_capacity.train_rate_bps());
            assert_eq!(Some(c.available), rust_capacity.available_bps());
            assert_eq!(subetha_link_capacity_observe_train(capacity, -1.0), SUBETHA_E_INVALID_ARGUMENT);
            assert_eq!(subetha_link_capacity_reset(capacity), SUBETHA_OK);
            assert_eq!(subetha_link_capacity_read(capacity, &mut c), SUBETHA_OK);
            assert!(!c.has_link_capacity && !c.has_available);
            assert_eq!((c.pairs, c.train_probes), (0, 0));

            assert_eq!(subetha_link_forecast_create(SUBETHA_MODE_STRICT, &mut forecast), SUBETHA_OK);
            for _ in 0..10 {
                assert_eq!(subetha_link_forecast_observe(forecast, 125_000, 1.0), SUBETHA_OK);
            }
            assert_eq!(subetha_link_forecast_observe(forecast, 1, 0.0), SUBETHA_E_INVALID_ARGUMENT);
            let mut f = subetha_link_forecast_reading::default();
            assert_eq!(subetha_link_forecast_read(forecast, &mut f), SUBETHA_OK);
            assert_eq!(f.mean_rate, 125_000.0, "bytes a second");
            assert!(f.next_rate > 0.0 && f.next_rate < f.mean_rate);

            assert_eq!(subetha_link_path_changes_create(SUBETHA_MODE_STRICT, &mut path), SUBETHA_OK);
            let mut g = subetha_link_path_changes_reading::default();
            assert_eq!(subetha_link_path_changes_read(path, &mut g), SUBETHA_OK);
            assert!(!g.has_last);
            assert_eq!(subetha_link_path_changes_observe(path, 54, 0, 10), SUBETHA_OK);
            assert_eq!(subetha_link_path_changes_observe(path, 53, 3, 11), SUBETHA_OK);
            let mut rust_path = PathSensor::new();
            rust_path.observe(54, 0, 10);
            rust_path.observe(53, 3, 11);
            assert_eq!(subetha_link_path_changes_read(path, &mut g), SUBETHA_OK);
            assert!(g.has_last);
            assert_eq!((g.last_ttl, g.last_congestion_mark, g.last_hops), (53, 3, 11));
            assert_eq!(g.route_movement, f64::from(rust_path.path_shift()));
            assert_eq!(g.marked_share, f64::from(rust_path.ecn_ce()));
            assert_eq!(subetha_link_path_changes_read(path, std::ptr::null_mut()), SUBETHA_E_INVALID_ARGUMENT);

            for h in [capacity, forecast, path] {
                assert_eq!(subetha_handle_destroy(h), SUBETHA_OK);
            }
        }
    }
}
