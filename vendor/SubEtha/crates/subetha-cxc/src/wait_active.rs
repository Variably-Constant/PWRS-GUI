//! The plan every blocking wait in this process takes its lengths from.
//!
//! [`active_plan`] answers, for a park kind, the plan in force now: the
//! calibrated plan for the calling thread's core class and the current load
//! band once [`wait_calibration`](crate::wait_calibration) has published it,
//! and 0.5.1's fixed ladder ([`fallback`]) until then. [`plan_in_force`]
//! answers the same plan without starting the calibration, with where it
//! came from. The environment's settings are read once per process:
//! - `SUBETHA_WAIT_CALIBRATION=off`: no calibration; every wait keeps the
//!   fixed ladder. `on`, the default, calibrates.
//! - `SUBETHA_NO_MONITOR_WAIT=1`: no monitor phase.
//! - `SUBETHA_MONITOR_WAIT_CYCLES=<cycles>`: the monitor phase's length.
//! - `SUBETHA_PRE_PARK_SPIN=<rounds>`: the spin phase's rounds; 0 skips the
//!   spin.
//!
//! A setting present with a value this process cannot use is ignored and
//! named, with the reason, by [`ignored_settings`].

use std::fmt::Display;
use std::str::FromStr;
use std::sync::OnceLock;

use crate::monitor_wait::{
    monitor_wait_budget_cycles, monitor_wait_kind, tpause_holds, umwait_holds, MonitorWaitKind,
};
use crate::wait_instr::{MwaitxHint, UserWait};
use crate::wait_plan::{fallback, MonitorFamily, Overrides, ParkKind, Plan};

struct Settings {
    overrides: Overrides,
    calibration: bool,
    ignored: Vec<(&'static str, String)>,
}

fn settings() -> &'static Settings {
    static READ: OnceLock<Settings> = OnceLock::new();
    READ.get_or_init(|| {
        let mut ignored = Vec::new();
        let monitor_cycles = setting::<u64>("SUBETHA_MONITOR_WAIT_CYCLES", &mut ignored);
        let spin_rounds = setting::<u32>("SUBETHA_PRE_PARK_SPIN", &mut ignored);
        let calibration = calibration_from(std::env::var("SUBETHA_WAIT_CALIBRATION"), &mut ignored);
        Settings {
            overrides: Overrides {
                no_monitor: std::env::var_os("SUBETHA_NO_MONITOR_WAIT").is_some_and(|v| v == "1"),
                monitor_cycles,
                spin_rounds,
            },
            calibration,
            ignored,
        }
    })
}

/// Whether `SUBETHA_WAIT_CALIBRATION`, read as `value`, leaves calibration
/// on: `off` turns it off; `on`, or no value, leaves it on. Any other value
/// leaves it on and is added to `ignored` with the reason.
fn calibration_from(
    value: Result<String, std::env::VarError>,
    ignored: &mut Vec<(&'static str, String)>,
) -> bool {
    const NAME: &str = "SUBETHA_WAIT_CALIBRATION";
    match value {
        Ok(text) => match text.trim() {
            "off" => false,
            "on" => true,
            _ => {
                ignored.push((NAME, format!("{text:?} is neither on nor off")));
                true
            }
        },
        Err(std::env::VarError::NotPresent) => true,
        Err(err @ std::env::VarError::NotUnicode(_)) => {
            ignored.push((NAME, err.to_string()));
            true
        }
    }
}

/// A number from the environment variable `name`. `None` when it is not
/// set; when it is set to something that is not such a number, `None` with
/// the variable and the reason added to `ignored`.
fn setting<T>(name: &'static str, ignored: &mut Vec<(&'static str, String)>) -> Option<T>
where
    T: FromStr,
    T::Err: Display,
{
    match std::env::var(name) {
        Ok(text) => match text.trim().parse() {
            Ok(value) => Some(value),
            Err(err) => {
                ignored.push((name, format!("{text:?}: {err}")));
                None
            }
        },
        Err(std::env::VarError::NotPresent) => None,
        Err(err @ std::env::VarError::NotUnicode(_)) => {
            ignored.push((name, err.to_string()));
            None
        }
    }
}

/// The environment's settings, read on first use.
pub fn overrides() -> Overrides {
    settings().overrides
}

/// Each setting the environment gave a value this process could not use,
/// with the reason. Those settings are ignored.
pub fn ignored_settings() -> &'static [(&'static str, String)] {
    &settings().ignored
}

/// The monitor family this host has, as the fallback plan runs it: C1 for
/// `MWAITX`, C0.1 for `UMWAIT`, or C0.1 `TPAUSE` slices where `UMWAIT`
/// does not hold and `TPAUSE` does. `None` when the host has none, when
/// neither WAITPKG wait holds, or when the environment switched it off.
fn detected_family() -> Option<MonitorFamily> {
    monitor_wait_kind().and_then(|kind| match kind {
        MonitorWaitKind::Mwaitx => Some(MonitorFamily::Mwaitx(MwaitxHint::C1)),
        MonitorWaitKind::Waitpkg if umwait_holds() => Some(MonitorFamily::Waitpkg(UserWait::C01)),
        MonitorWaitKind::Waitpkg if tpause_holds() => Some(MonitorFamily::Tpause(UserWait::C01)),
        MonitorWaitKind::Waitpkg => None,
        MonitorWaitKind::ArmWfe => Some(MonitorFamily::ArmWfe),
    })
}

/// Whether this process calibrates its waits.
pub(crate) fn calibration_enabled() -> bool {
    settings().calibration
}

/// Where a plan in force came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanSource {
    /// Calibration published it for the calling thread's core class in the
    /// current load band.
    Calibrated,
    /// 0.5.1's fixed ladder ([`fallback`]), since calibration has published
    /// no plan for this class and band, has not started, or is off.
    Fallback,
}

/// A plan a wait would use now, with where it came from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlanInForce {
    pub plan: Plan,
    pub source: PlanSource,
}

/// The plan a wait of `park` kind would use now on the calling thread, and
/// whether calibration made it. Never starts the calibration, so it is safe
/// where starting a thread is not, such as a thread-local destructor.
pub fn plan_in_force(park: ParkKind) -> PlanInForce {
    match crate::wait_calibration::plan_for(park) {
        Some(plan) => PlanInForce { plan, source: PlanSource::Calibrated },
        None => PlanInForce {
            plan: fallback(detected_family(), monitor_wait_budget_cycles(), &overrides()),
            source: PlanSource::Fallback,
        },
    }
}

/// The plan a wait of `park` kind uses now: the calibrated plan for the
/// calling thread's core class in the current load band where calibration
/// has published one, and the fallback plan otherwise. Outside this crate's
/// unit tests, the first call starts the calibration.
pub fn active_plan(park: ParkKind) -> Plan {
    #[cfg(not(test))]
    crate::wait_calibration::start();
    plan_in_force(park).plan
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Without calibration the plan is 0.5.1's ladder: the spin the
    /// environment sets or 32 rounds, then the detected family for the
    /// active budget.
    #[test]
    fn without_calibration_the_plan_is_the_fixed_ladder() {
        for park in [ParkKind::Local, ParkKind::CrossProcess] {
            let plan = active_plan(park);
            assert_eq!(plan.spin_rounds, overrides().spin_rounds.unwrap_or(32));
            match (plan.monitor, detected_family()) {
                (Some(phase), Some(family)) => {
                    assert_eq!(phase.family, family);
                    assert_eq!(phase.budget_cycles, monitor_wait_budget_cycles());
                    assert_eq!(phase.guard_floor_cycles, None);
                }
                (None, None) => {}
                (phase, family) => panic!("plan monitor {phase:?} against detected {family:?}"),
            }
        }
    }

    /// Until calibration publishes, the plan in force is the fixed ladder,
    /// says so, and is the plan `active_plan` answers.
    #[test]
    fn the_plan_in_force_names_the_fixed_ladder_until_calibration_publishes() {
        for park in [ParkKind::Local, ParkKind::CrossProcess] {
            let in_force = plan_in_force(park);
            assert_eq!(in_force.source, PlanSource::Fallback);
            assert_eq!(in_force.plan, fallback(detected_family(), monitor_wait_budget_cycles(), &overrides()));
            assert_eq!(in_force.plan, active_plan(park));
        }
    }

    #[test]
    fn calibration_is_on_unless_the_setting_says_off() {
        let mut ignored = Vec::new();
        assert!(calibration_from(Err(std::env::VarError::NotPresent), &mut ignored));
        assert!(calibration_from(Ok("on".to_string()), &mut ignored));
        assert!(!calibration_from(Ok(" off ".to_string()), &mut ignored));
        assert!(ignored.is_empty(), "{ignored:?}");
        assert!(calibration_from(Ok("sometimes".to_string()), &mut ignored));
        assert_eq!(ignored.len(), 1);
        assert_eq!(ignored[0].0, "SUBETHA_WAIT_CALIBRATION");
        assert!(ignored[0].1.contains("sometimes"), "the reason names the value: {:?}", ignored[0].1);
    }

    #[test]
    fn a_setting_that_is_not_a_number_is_ignored_and_named() {
        let mut ignored = Vec::new();
        // SAFETY: the variable is unique to this test and read only here.
        unsafe { std::env::set_var("SUBETHA_TEST_WAIT_SETTING", "many") };
        let value = setting::<u32>("SUBETHA_TEST_WAIT_SETTING", &mut ignored);
        // SAFETY: as above.
        unsafe { std::env::remove_var("SUBETHA_TEST_WAIT_SETTING") };
        assert_eq!(value, None);
        assert_eq!(ignored.len(), 1);
        assert_eq!(ignored[0].0, "SUBETHA_TEST_WAIT_SETTING");
        assert!(ignored[0].1.contains("many"), "the reason names the value: {:?}", ignored[0].1);
    }
}
