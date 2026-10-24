//! Deterministic state model for the Harness MVP.

use std::fmt;

/// States included in the current MVP scope.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RunState {
    New,
    Understanding,
    Contracting,
    AcceptanceAuthoring,
    Planning,
    Executing,
    Verifying,
    AwaitingHuman,
    AwaitingMerge,
    Done,
    Failed,
    Cancelled,
}

impl RunState {
    pub const ALL: [Self; 12] = [
        Self::New,
        Self::Understanding,
        Self::Contracting,
        Self::AcceptanceAuthoring,
        Self::Planning,
        Self::Executing,
        Self::Verifying,
        Self::AwaitingHuman,
        Self::AwaitingMerge,
        Self::Done,
        Self::Failed,
        Self::Cancelled,
    ];

    /// Returns whether the engine permits a direct transition to `next`.
    ///
    /// Gate-specific return states are handled by the run coordinator, which
    /// records the state to resume before entering `AwaitingHuman`.
    pub const fn can_transition_to(self, next: Self) -> bool {
        use RunState::*;

        matches!(
            (self, next),
            (New, Understanding)
                | (Understanding, Contracting)
                | (Contracting, AcceptanceAuthoring)
                | (AcceptanceAuthoring, Planning)
                | (Planning, Executing)
                | (Planning, Failed)
                | (Executing, Planning)
                | (Executing, Verifying)
                | (Executing, AwaitingHuman)
                | (Executing, Failed)
                | (Verifying, Executing)
                | (Verifying, AwaitingMerge)
                | (AwaitingMerge, Done)
                | (AwaitingMerge, Cancelled)
        ) || (!self.is_terminal() && matches!(next, Cancelled))
    }

    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Understanding => "understanding",
            Self::Contracting => "contracting",
            Self::AcceptanceAuthoring => "acceptance_authoring",
            Self::Planning => "planning",
            Self::Executing => "executing",
            Self::Verifying => "verifying",
            Self::AwaitingHuman => "awaiting_human",
            Self::AwaitingMerge => "awaiting_merge",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl fmt::Display for RunState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::RunState;

    #[test]
    fn mvp_happy_path_is_allowed() {
        let path = [
            RunState::New,
            RunState::Understanding,
            RunState::Contracting,
            RunState::AcceptanceAuthoring,
            RunState::Planning,
            RunState::Executing,
            RunState::Verifying,
            RunState::AwaitingMerge,
            RunState::Done,
        ];

        for pair in path.windows(2) {
            assert!(pair[0].can_transition_to(pair[1]));
        }
    }

    #[test]
    fn verification_rejection_returns_to_execution() {
        assert!(RunState::Verifying.can_transition_to(RunState::Executing));
    }

    #[test]
    fn terminal_runs_cannot_transition() {
        for terminal in [RunState::Done, RunState::Failed, RunState::Cancelled] {
            assert!(terminal.is_terminal());
            assert!(!terminal.can_transition_to(RunState::Executing));
            assert!(!terminal.can_transition_to(RunState::Cancelled));
        }
    }

    #[test]
    fn unrelated_direct_transitions_are_rejected() {
        assert!(!RunState::New.can_transition_to(RunState::Executing));
        assert!(!RunState::Planning.can_transition_to(RunState::Verifying));
        assert!(!RunState::AwaitingMerge.can_transition_to(RunState::Executing));
    }
}
