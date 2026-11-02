//! How a request's wait for its reply runs ([`Pace`]): with no time limit,
//! as most requests wait ([`Unpaced`]), or as a test's input waits
//! ([`InputPace`]), each step having its time from when the renderer
//! begins it.

use crate::GuiError;
use pwrs_gui_proto as proto;
use proto::FromHelper;
use proto::test_input::Step;
use std::time::{Duration, Instant};

/// How the wait for a request's reply runs: the moment it gives up at, and
/// what each message with the request's number does to it.
pub(crate) trait Pace {
    /// The moment the wait gives up at; `None` waits with no limit.
    fn due(&self) -> Option<Instant>;
    /// Takes `msg`, a message with the request's number, at `now`: true
    /// when it is the reply, which ends the wait; false when the wait goes
    /// on, its due moment moved as `msg` says.
    fn ends(&mut self, msg: &FromHelper, now: Instant) -> bool;
    /// Why the wait failed at its due moment.
    fn late(&self) -> GuiError;
}

/// The wait for a request's one reply, with no time limit.
pub(crate) struct Unpaced;

impl Pace for Unpaced {
    fn due(&self) -> Option<Instant> {
        None
    }

    fn ends(&mut self, _msg: &FromHelper, _now: Instant) -> bool {
        true
    }

    /// Never asked for, since this wait has no due moment.
    fn late(&self) -> GuiError {
        GuiError::Transport(String::from("a wait with no time limit was taken as late"))
    }
}

/// The wait for a test's input ([`crate::Session::test_input`]). Each step
/// has its time to be taken and seen ([`Step::allowed`]) from when the
/// renderer says it begins ([`FromHelper::InputStep`]), the first from when
/// the request is sent. The wait fails naming the step whose time ran out.
pub(crate) struct InputPace {
    /// Each step's time and description, in the order given.
    steps: Vec<(Duration, String)>,
    /// The time each step has, beside a wait's own seconds.
    time: Duration,
    /// The step under way, from 0.
    at: usize,
    /// When the step under way runs out of its time; `None` for a moment
    /// past the end of this clock, which is no limit.
    due: Option<Instant>,
}

impl InputPace {
    /// The wait for `steps`, each given `time`, its request sent at `now`;
    /// refused, naming the step from 1, for a step whose time no clock
    /// counts. An input of no steps has `time` for its reply.
    pub(crate) fn new(steps: &[Step], time: Duration, now: Instant) -> Result<InputPace, GuiError> {
        let steps = steps
            .iter()
            .enumerate()
            .map(|(i, step)| match step.allowed(time) {
                Ok(allowed) => Ok((allowed, step.description())),
                Err(why) => Err(GuiError::Refused(format!("step {}: {why}", i + 1))),
            })
            .collect::<Result<Vec<_>, GuiError>>()?;
        let first = match steps.first() {
            Some((allowed, _)) => *allowed,
            None => time,
        };
        Ok(InputPace { steps, time, at: 0, due: now.checked_add(first) })
    }
}

impl Pace for InputPace {
    fn due(&self) -> Option<Instant> {
        self.due
    }

    fn ends(&mut self, msg: &FromHelper, now: Instant) -> bool {
        let FromHelper::InputStep { step, .. } = msg else {
            return true;
        };
        let total = self.steps.len();
        match usize::try_from(*step) {
            Ok(number) if (1..=total).contains(&number) => {
                proto::trace::line(|| format!("client: input step {number} of {total} begins: {}", self.steps[number - 1].1));
                self.at = number - 1;
                self.due = now.checked_add(self.steps[number - 1].0);
            }
            Ok(_) | Err(_) => eprintln!("PWRS-GUI: the renderer began step {step} of an input of {total} steps; the wait goes on as it was"),
        }
        false
    }

    fn late(&self) -> GuiError {
        let total = self.steps.len();
        match self.steps.get(self.at) {
            Some((allowed, described)) => GuiError::Late(format!("step {} of {total}, {described}, was not taken and seen within {} seconds", self.at + 1, allowed.as_secs_f64())),
            None => GuiError::Late(format!("the renderer did not answer an input of no steps within {} seconds", self.time.as_secs_f64())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::test_input::{Button, Keys};

    const NONE: Keys = Keys { shift: false, ctrl: false, alt: false };

    fn steps() -> Vec<Step> {
        vec![
            Step::Press { button: Button::Right, at: [40.0, 12.0], keys: NONE },
            Step::Wait { seconds: 0.5 },
            Step::Release { button: Button::Right, at: [43.0, 12.0], keys: NONE },
        ]
    }

    fn begun(step: u32) -> FromHelper {
        FromHelper::InputStep { request: 4, step }
    }

    #[test]
    fn each_step_is_due_its_time_after_it_begins_and_a_wait_its_seconds_besides() {
        let start = Instant::now();
        let time = Duration::from_secs(10);
        let mut pace = InputPace::new(&steps(), time, start).expect("the steps have times");
        assert_eq!(pace.due(), start.checked_add(time), "the first step from the request's sending");
        let later = start + Duration::from_secs(3);
        assert!(!pace.ends(&begun(2), later), "a step begun is no reply");
        assert_eq!(pace.due(), later.checked_add(Duration::from_millis(10_500)), "a wait has its own seconds besides");
        assert!(!pace.ends(&begun(3), later), "a step begun is no reply");
        assert_eq!(pace.due(), later.checked_add(time));
        assert!(pace.ends(&FromHelper::TestEventDone { request: 4, outcome: Ok(proto::Value::Null) }, later), "the reply ends the wait");
    }

    #[test]
    fn a_late_wait_names_the_step_under_way_and_its_time() {
        let start = Instant::now();
        let mut pace = InputPace::new(&steps(), Duration::from_secs(10), start).expect("the steps have times");
        let named = |pace: &InputPace| match pace.late() {
            GuiError::Late(why) => why,
            other => panic!("a late wait failed as {other:?}"),
        };
        assert_eq!(named(&pace), "step 1 of 3, a press of the Right button at 40, 12, was not taken and seen within 10 seconds");
        assert!(!pace.ends(&begun(2), start), "a step begun is no reply");
        assert_eq!(named(&pace), "step 2 of 3, a wait of 0.5 seconds, was not taken and seen within 10.5 seconds");
    }

    #[test]
    fn a_step_the_input_does_not_have_leaves_the_wait_as_it_was() {
        let start = Instant::now();
        let mut pace = InputPace::new(&steps(), Duration::from_secs(10), start).expect("the steps have times");
        let due = pace.due();
        assert!(!pace.ends(&begun(0), start + Duration::from_secs(5)));
        assert!(!pace.ends(&begun(4), start + Duration::from_secs(5)));
        assert_eq!(pace.due(), due);
    }

    #[test]
    fn a_step_whose_time_no_clock_counts_refuses_the_input_naming_it() {
        match InputPace::new(&[Step::Move { at: [1.0, 1.0], keys: NONE }, Step::Wait { seconds: 1e300 }], Duration::from_secs(10), Instant::now()) {
            Err(GuiError::Refused(why)) => assert!(why.starts_with("step 2: "), "{why}"),
            Err(other) => panic!("refused as {other:?}"),
            Ok(_) => panic!("a wait no clock counts was taken"),
        }
    }
}
