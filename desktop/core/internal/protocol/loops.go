package protocol

import "time"

// StartLoopRequest asks the daemon to run `/loop`.
//
// Args travels as the person typed it, after `/loop`. The grammar has one home,
// the daemon's: a client recognises the command and never parses its argument,
// so two clients cannot disagree about what `/loop specs/x --protect t/**` means.
type StartLoopRequest struct {
	Workspace   string `json:"workspace"`
	Args        string `json:"args"`
	Model       string `json:"model,omitempty"`
	SandboxMode string `json:"sandbox_mode,omitempty"`
}

// The states of a loop run.
const (
	// LoopSurveying asks the daemon what the spec folders declare.
	LoopSurveying = "surveying"
	// LoopQualifying runs a qualifying session for a folder that declares
	// nothing.
	LoopQualifying = "qualifying"
	// LoopAwaitingReview is a proposal written and waiting for a person. Nothing
	// leaves it but continue or stop: a ruler nobody read is not run against.
	LoopAwaitingReview = "awaiting_review"
	// LoopWorking runs a work session against a folder's criteria.
	LoopWorking = "working"
	// LoopFinished, LoopStopped and LoopFailed are the ends. Finished ran out
	// of work, stopped was asked to, and failed could not go on and says why.
	LoopFinished = "finished"
	LoopStopped  = "stopped"
	LoopFailed   = "failed"
)

// The phases of one spec folder inside a run.
const (
	SpecPending    = "pending"
	SpecQualifying = "qualifying"
	SpecProposed   = "proposed"
	SpecWorking    = "working"
	SpecDone       = "done"
	SpecUnfinished = "unfinished"
	SpecSkipped    = "skipped"
)

// LoopRun is a `/loop` the daemon is running, or ran.
//
// The daemon conducts it, not the client that asked: closing that client does
// not stop it, and every client sees the same run. It does not outlive the
// daemon.
type LoopRun struct {
	ID        string `json:"id"`
	Workspace string `json:"workspace"`
	// Args is what was typed after `/loop`, as it was typed: how a client tells
	// "the loop I asked for" from another one waiting in the same workspace.
	Args string `json:"args,omitempty"`
	// Goal is the sentence of a `/loop <goal>`; empty for a folder.
	Goal    string        `json:"goal,omitempty"`
	Protect []string      `json:"protect,omitempty"`
	State   string        `json:"state"`
	Specs   []LoopSpecRun `json:"specs"`
	// Current indexes Specs; -1 outside any folder.
	Current int `json:"current"`
	// SessionID is the session the run is in now, or was in last.
	SessionID string       `json:"session_id,omitempty"`
	Verdict   *LoopVerdict `json:"verdict,omitempty"`
	// Warnings say what went wrong without stopping the run — a survey that
	// failed before the session opened anyway. Going on is not staying quiet.
	Warnings []string `json:"warnings,omitempty"`
	// Error says why a failed run could not go on.
	Error    string    `json:"error,omitempty"`
	Started  time.Time `json:"started"`
	Finished time.Time `json:"finished,omitempty"`
}

// Live reports whether the run has not reached an end.
func (r LoopRun) Live() bool {
	return r.State != LoopFinished && r.State != LoopStopped && r.State != LoopFailed
}

// LoopSpecRun is one spec folder of a run and where it stands.
type LoopSpecRun struct {
	Path      string `json:"path"`
	Phase     string `json:"phase"`
	SessionID string `json:"session_id,omitempty"`
	// Criteria, Unmet, Unavailable and Error are what the survey said of the
	// folder before anything opened: the plan a client draws, from the one
	// measurement the run made rather than a second one of its own. Unmet and
	// Unavailable are counted only when a goal measured the folders; absent is
	// zero. Criteria 0 with an Error means unknown — the folder could not be
	// read — never "declares nothing".
	Criteria    int    `json:"criteria,omitempty"`
	Unmet       int    `json:"unmet,omitempty"`
	Unavailable int    `json:"unavailable,omitempty"`
	Error       string `json:"error,omitempty"`
	// Proposal is the done.toml written for review, when one was, with how
	// many criteria went into it and the summary a person reads.
	Proposal         string `json:"proposal,omitempty"`
	ProposalCriteria int    `json:"proposal_criteria,omitempty"`
	ProposalSummary  string `json:"proposal_summary,omitempty"`
	// Reason is how the folder's last turn ended (a TurnCompleted reason). A
	// turn that did not end normally leaves the folder unfinished and fails
	// the run, whatever its criteria say.
	Reason string `json:"reason,omitempty"`
	// Standing is the last completion of this folder's work: the last word for
	// a folder wins, because a folder can complete several turns.
	Standing *Completion `json:"standing,omitempty"`
}

// LoopVerdict is where done stands for the whole run, not its last folder.
type LoopVerdict struct {
	// Worked is how many folders were worked.
	Worked int `json:"worked"`
	// Met and Total sum the criteria over every folder worked.
	Met   int `json:"met"`
	Total int `json:"total"`
	// Unmet and Unchecked name criteria, each once. Unchecked counts as left:
	// a criterion nobody could check is not one that passed.
	Unmet     []string `json:"unmet,omitempty"`
	Unchecked []string `json:"unchecked,omitempty"`
	// Unfinished names the folders that ended with anything outstanding.
	Unfinished []string `json:"unfinished,omitempty"`
}

// StopLoopRequest asks a run to stop now, or after the folder it is in.
type StopLoopRequest struct {
	When string `json:"when"`
}

// When a run stops.
const (
	// StopNow interrupts the turn under way and ends the run.
	StopNow = "now"
	// StopAfterSpec lets the current folder finish and opens no other.
	StopAfterSpec = "after_spec"
)

// ListLoopsResponse answers GET /loops: the runs the daemon holds, oldest
// first.
type ListLoopsResponse struct {
	Loops []LoopRun `json:"loops"`
}

// The kinds of frame on GET /loops/events. A run is never removed while the
// daemon lives — its verdict is what a client shows after the end — so the
// stream has no removed frame.
const (
	// LoopSnapshot opens the stream with every run.
	LoopSnapshot = "snapshot"
	// LoopChanged carries one run that moved.
	LoopChanged = "changed"
)

// LoopChange is one frame of the stream of runs.
type LoopChange struct {
	Kind  string    `json:"kind"`
	Loops []LoopRun `json:"loops,omitempty"`
	Loop  *LoopRun  `json:"loop,omitempty"`
}
