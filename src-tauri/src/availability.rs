//! Full successful runs alone may change availability; every other outcome preserves state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    Active,
    PossiblyClosed,
    Closed,
    Archived,
}
pub fn reconcile(
    current: Availability,
    missing_full_runs: u32,
    seen: bool,
    run_completed: bool,
    complete: bool,
) -> (Availability, u32) {
    if !run_completed || !complete || current == Availability::Archived {
        return (current, missing_full_runs);
    }
    if seen {
        return (Availability::Active, 0);
    }
    let n = missing_full_runs + 1;
    if n >= 2 {
        (Availability::Closed, n)
    } else {
        (Availability::PossiblyClosed, n)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_never_closes() {
        assert_eq!(
            reconcile(Availability::Active, 0, false, true, false),
            (Availability::Active, 0)
        );
        assert_eq!(
            reconcile(Availability::Active, 0, false, false, false),
            (Availability::Active, 0)
        );
    }
    #[test]
    fn two_complete_absences_close() {
        let once = reconcile(Availability::Active, 0, false, true, true);
        assert_eq!(once, (Availability::PossiblyClosed, 1));
        assert_eq!(
            reconcile(once.0, once.1, false, true, true),
            (Availability::Closed, 2)
        );
    }
}
