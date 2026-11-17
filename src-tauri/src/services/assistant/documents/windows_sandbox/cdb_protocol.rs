//! Test-only startup fence. No debugger observation grants runtime admission.
#[derive(Clone, Copy)]
pub(super) struct Configured {
    pub pid: u32,
    pub tid: u32,
    pub before: u32,
    pub after: u32,
    pub confirmed: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Waiting,
    InitialHeld,
    Configured,
    Continuing,
    Held,
    Resumed,
    Aborted,
}

pub(super) struct Fence {
    pid: u32,
    tid: u32,
    state: State,
}
impl Fence {
    pub fn new(pid: u32, tid: u32) -> Self {
        Self {
            pid,
            tid,
            state: State::Waiting,
        }
    }
    pub fn configure(&mut self, value: Configured) -> Result<(), ()> {
        if self.state != State::InitialHeld
            || value.pid != self.pid
            || value.tid != self.tid
            || !value.confirmed
            || value.after != (value.before | 2)
        {
            self.abort();
            return Err(());
        }
        self.state = State::Configured;
        Ok(())
    }
    pub fn initial_hold(
        &mut self,
        previous: u32,
        paired_resume: impl FnOnce() -> u32,
    ) -> Result<(), ()> {
        if self.state != State::Waiting || previous != 1 {
            self.abort();
            return Err(());
        }
        if paired_resume() != 2 {
            self.abort();
            return Err(());
        }
        self.state = State::InitialHeld;
        Ok(())
    }
    pub fn may_continue(&self) -> bool {
        self.state == State::Configured
    }
    pub fn continuing(&mut self) -> Result<(), ()> {
        if !self.may_continue() {
            self.abort();
            return Err(());
        }
        self.state = State::Continuing;
        Ok(())
    }
    pub fn hold(&mut self, previous: u32, paired_resume: impl FnOnce() -> u32) -> Result<bool, ()> {
        if self.state != State::Continuing || !matches!(previous, 1 | 2) {
            self.abort();
            return Err(());
        }
        let paired = paired_resume();
        match (previous, paired) {
            (1, 2) => {
                self.state = State::Held;
                Ok(true)
            }
            // Undo only this probe's increment. The debugger may release its
            // extra hold before this paired call, changing 3 to 2. Neither
            // observation grants resume; a fresh exact 1/2 proof is required.
            (2, 2 | 3) => Ok(false),
            _ => {
                self.abort();
                Err(())
            }
        }
    }
    pub fn may_resume(&self) -> bool {
        self.state == State::Held
    }
    pub fn resumed(&mut self, previous: u32) -> Result<(), ()> {
        if !self.may_resume() || previous != 1 {
            self.abort();
            return Err(());
        }
        self.state = State::Resumed;
        Ok(())
    }
    pub fn may_go(&self) -> bool {
        self.state == State::Resumed
    }
    pub fn abort(&mut self) {
        self.state = State::Aborted;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn valid() -> Configured {
        Configured {
            pid: 10,
            tid: 20,
            before: 0x70,
            after: 0x72,
            confirmed: true,
        }
    }
    fn configured() -> Fence {
        let mut fence = Fence::new(10, 20);
        fence.initial_hold(1, || 2).unwrap();
        fence.configure(valid()).unwrap();
        fence
    }
    #[test]
    fn configuration_requires_proven_original_hold_before_attach() {
        let mut fence = Fence::new(10, 20);
        assert!(fence.configure(valid()).is_err());
        assert!(!fence.may_resume());
        assert!(!fence.may_go());
    }
    #[test]
    fn wrong_or_incomplete_debugger_configuration_never_allows_resume() {
        for value in [
            Configured { pid: 11, ..valid() },
            Configured { tid: 21, ..valid() },
            Configured {
                after: 0x70,
                ..valid()
            },
            Configured {
                after: 0x76,
                ..valid()
            },
            Configured {
                confirmed: false,
                ..valid()
            },
        ] {
            let mut fence = Fence::new(10, 20);
            fence.initial_hold(1, || 2).unwrap();
            assert!(fence.configure(value).is_err());
            assert!(!fence.may_resume());
            assert!(!fence.may_go());
        }
    }
    #[test]
    fn unexpected_suspend_count_never_pairs_resume_or_sends_go() {
        for count in [0, 3, u32::MAX] {
            let mut fence = configured();
            fence.continuing().unwrap();
            assert!(fence
                .hold(count, || panic!(
                    "must retain unexpected suspension until Job cleanup"
                ))
                .is_err());
            assert!(!fence.may_resume());
            assert!(!fence.may_go());
        }
    }
    #[test]
    fn only_restored_hold_then_original_resume_one_allows_go() {
        let mut fence = configured();
        fence.continuing().unwrap();
        assert!(!fence.hold(2, || 3).unwrap());
        assert!(!fence.may_resume());
        assert!(!fence.hold(2, || 2).unwrap());
        assert!(!fence.may_resume());
        assert!(fence.hold(1, || 2).unwrap());
        assert!(fence.may_resume());
        assert!(!fence.may_go());
        fence.resumed(1).unwrap();
        assert!(fence.may_go());
        for count in [0, 2, u32::MAX] {
            let mut fence = configured();
            fence.continuing().unwrap();
            assert!(fence.hold(1, || 2).unwrap());
            assert!(fence.resumed(count).is_err());
            assert!(!fence.may_go());
        }
    }
    #[test]
    fn cancel_timeout_exit_or_missing_tail_are_terminal() {
        for state in [
            State::Waiting,
            State::InitialHeld,
            State::Configured,
            State::Continuing,
            State::Held,
            State::Resumed,
        ] {
            let mut fence = Fence::new(10, 20);
            fence.state = state;
            fence.abort();
            assert!(!fence.may_resume());
            assert!(!fence.may_go());
            assert!(!fence.may_continue());
            assert!(fence.configure(valid()).is_err());
        }
    }
    #[test]
    fn original_hold_and_continue_cannot_be_assumed_or_repeated() {
        for count in [0, 2, u32::MAX] {
            let mut fence = Fence::new(10, 20);
            assert!(fence
                .initial_hold(count, || panic!("unexpected hold must not resume"))
                .is_err());
            assert!(!fence.may_continue());
        }
        for paired in [0, 1, 3, u32::MAX] {
            let mut fence = Fence::new(10, 20);
            assert!(fence.initial_hold(1, || paired).is_err());
            assert!(!fence.may_continue());
        }
        let mut fence = Fence::new(10, 20);
        fence.initial_hold(1, || 2).unwrap();
        assert!(fence.continuing().is_err());
        let mut fence = configured();
        fence.continuing().unwrap();
        assert!(fence.continuing().is_err());
        assert!(!fence.may_resume());
    }
    #[test]
    fn invalid_release_pair_cannot_become_a_final_hold() {
        for count in [1, 2] {
            for paired in [0, 1, 4, u32::MAX] {
                let mut fence = configured();
                fence.continuing().unwrap();
                assert!(fence.hold(count, || paired).is_err());
                assert!(!fence.may_resume());
                assert!(!fence.may_go());
            }
        }
    }
}
