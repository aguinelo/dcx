// Package protocol is the shared vocabulary between the dcode daemon and its
// clients. It holds no logic and performs no I/O: both sides import it so the
// wire contract has exactly one definition.
//
// Spec: docs/specs/architecture/client-server-protocol/202608072240-*.
package protocol

import (
	"encoding/json"
	"time"
)

// Version is the protocol version carried in the URL prefix.
const Version = "v1"

// EventType identifies the payload shape of an Event.
type EventType string

// Event types. Payload shapes are defined in section 5.1 of the planning spec.
const (
	EventSessionCreated EventType = "session.created"
	// EventSessionResumed opens the conversation this session continues.
	//
	// The events after it happened somewhere else, and a marker is what keeps
	// the transcript honest about that: without one, a replayed conversation
	// reads as work this session did. It also carries the only thing the reader
	// cannot reconstruct from the events themselves — which session they came
	// from.
	EventSessionResumed EventType = "session.resumed"
	EventTurnStarted    EventType = "turn.started"
	EventMessageDelta   EventType = "message.delta"
	// EventMessageReasoning carries the model's thinking, which is not its
	// answer. It is an event so a client can show it and so replaying a session
	// looks like having watched it — but it never enters the history the model
	// is sent, because a model that reads its own reasoning back as something
	// it said out loud starts defending it.
	EventMessageReasoning EventType = "message.reasoning"
	EventToolRequested    EventType = "tool.requested"
	EventApprovalRequired EventType = "tool.approval_required"
	EventApprovalResolved EventType = "tool.approval_resolved"

	// EventDoneProposed carries a measured definition of done to the operator.
	//
	// Measured, not merely proposed: every criterion in it has already been run
	// against the repository as it stands, and each one says what that run did.
	// A proposal without those results is a list of good intentions.
	EventDoneProposed  EventType = "done.proposed"
	EventToolCompleted EventType = "tool.completed"
	// EventTurnSteered is the person correcting a turn already under way.
	//
	// Its own event rather than a message delta: a correction the transcript
	// cannot tell apart from the model's own words is a turn nobody can audit
	// afterwards, and "why did it change direction" is the first question asked
	// of any run that went sideways.
	EventTurnSteered   EventType = "turn.steered"
	EventTurnCompleted EventType = "turn.completed"
	EventPlanUpdated   EventType = "plan.updated"
	// EventProgress answers "how far along", and it is the only event here
	// that is not a fact worth replaying on its own.
	//
	// One event rather than one per question. A tool counting files and a turn
	// counting rounds are the same question asked of different subjects, and
	// adding a versioned surface twice for one kind of question is how it comes
	// out crooked — the second one always answers slightly differently.
	//
	// It joins the log and the record like message.delta does: chatty, batched
	// rather than flushed, and carrying a Seq. Giving it no Seq would have been
	// the alternative, and it would have put a gap in the one property the
	// record is built on.
	EventProgress EventType = "progress"
	// EventSessionRenamed is a name a person gave this conversation.
	//
	// An event rather than a file beside the record, and the record is where it
	// belongs for one reason above the others: a name for a conversation that
	// no longer exists is nothing. Pruning removes the transcript, and a name
	// stored anywhere else would outlive what it named — a listing full of
	// titles for sessions nobody can open.
	//
	// It also keeps the count at one. A second store beside the log is a second
	// thing that can disagree with the first, and this protocol's whole shape
	// is that every observable fact travels the same way.
	EventSessionRenamed   EventType = "session.renamed"
	EventSessionCompacted EventType = "session.compacted"
	// EventSkillLoaded announces a skill body entering the turn.
	//
	// The index is auditable — it is in the prefix, and `--dump-prompt` prints
	// it. The body was not: it was appended to the history as a reminder with
	// nothing emitted, so a block of text joined the turn, cost tokens and
	// changed how the model behaved, with no way for the person to know it had
	// happened or which skill it was.
	//
	// Thirty lines above the injection, the same package refuses to drop a
	// skill from the index in silence. This is the sentence at the top of this
	// block applied to the one observable fact that was not travelling.
	EventSkillLoaded  EventType = "skill.loaded"
	EventContextBand  EventType = "context.band"
	EventSessionError EventType = "session.error"
	// EventSessionModeChanged announces a switch between plan, assist and auto.
	//
	// Carried over the event log rather than read from a side channel so a
	// client that attaches after the change still sees what the session is
	// running under, the same way SessionCreated already does for the original.
	EventSessionModeChanged EventType = "session.mode_changed"
	// EventDoneChecked is one check of the definition of done inside a turn.
	//
	// The loop re-enters a turn while the criteria shrink, and every one of
	// those checks used to happen in silence: a client learned where the
	// criteria stood only at turn.completed, so "cycle 3, two of five" did not
	// exist on the wire. A turn that wrote nothing runs no check and emits
	// nothing, as before.
	EventDoneChecked EventType = "done.checked"
)

// WorkspaceDoneDir is the workspace's own definition-of-done directory.
//
// It is where a proposal lands when no spec folder claims it — a goal typed as
// a sentence — and it lives here because both halves need it: the client
// decides to anchor there, and the daemon writes there. It was a literal in
// two places before that was true of only one of them.
const WorkspaceDoneDir = ".dcode"

// Mode names. The behavioural mode is what the user picks; the wire carries the
// name, and Session.SetMode maps it to the pair of (SandboxMode, Policy) the
// engine actually runs under.
const (
	ModePlan   = "plan"
	ModeAssist = "assist"
	ModeAuto   = "auto"
)

// ValidMode reports whether name is one of ModePlan, ModeAssist, ModeAuto.
func ValidMode(name string) bool {
	switch name {
	case ModePlan, ModeAssist, ModeAuto:
		return true
	}
	return false
}

// Event is the envelope every observable fact travels in. Seq is per session,
// monotonic from 1, never reused and never gapped.
//
// At is the only non-deterministic field; golden comparisons zero it.
type Event struct {
	Seq       uint64          `json:"seq"`
	SessionID string          `json:"session_id"`
	Type      EventType       `json:"type"`
	At        time.Time       `json:"at"`
	Payload   json.RawMessage `json:"payload"`
}

// SessionState is the lifecycle state of a session.
type SessionState string

const (
	SessionStateIdle    SessionState = "idle"
	SessionStateRunning SessionState = "running"
	SessionStateBlocked SessionState = "blocked"
	SessionStateClosed  SessionState = "closed"
)

// Valid reports whether s is a state the server can be in.
func (s SessionState) Valid() bool {
	switch s {
	case SessionStateIdle, SessionStateRunning, SessionStateBlocked, SessionStateClosed:
		return true
	}
	return false
}

// Session is the server-owned session record. Clients hold no session state of
// their own beyond scroll position, panel visibility and their input queue.
type Session struct {
	ID        string       `json:"id"`
	State     SessionState `json:"state"`
	Workspace string       `json:"workspace"`
	Model     string       `json:"model"`
	// Family, Transport and BaseURL are the rest of the bundle a name like
	// `qwen-local` resolves to (202608072334-provider-adapter.p.spec.md §2.2).
	// Carried so a resumed session can reconnect to the same endpoint rather
	// than falling back to whatever `model.name` resolves to on the machine
	// that types `dcode -c` — empty means the family's own default, the same
	// reading `model.family`/`model.transport`/`model.base_url` already give
	// an unset override.
	Family    string `json:"family,omitempty"`
	Transport string `json:"transport,omitempty"`
	BaseURL   string `json:"base_url,omitempty"`
	// Branch is the git branch the workspace was on when this session was
	// built, frozen at that moment like the rest of the session's facts.
	// Empty when the workspace has no repository, git is not installed, or
	// the head is detached — the status bar draws nothing for it then, the
	// same reading "no data" already gets everywhere else on that bar.
	Branch      string `json:"branch,omitempty"`
	SandboxMode string `json:"sandbox_mode"`
	// Mode is the behavioural mode the session runs under: plan, assist or
	// auto. Defaults to assist when the request does not set it. SandboxMode is
	// the technical consequence; Mode is the name the user picks.
	Mode      string    `json:"mode"`
	CreatedAt time.Time `json:"created_at"`
	LastSeq   uint64    `json:"last_seq"`
	// FirstSeq is the earliest event the session still holds. It is not always
	// 1: continuing a long conversation puts every carried event in the log,
	// and retention drops the oldest. A client that assumes 1 asks for events
	// that are gone and is refused, so the session it asked for never opens.
	FirstSeq uint64 `json:"first_seq,omitempty"`
	// ContextWindow is the model's window in tokens. The client needs it to
	// turn a token count into the percentage a person can act on; without it,
	// "12400 tokens" answers nothing.
	ContextWindow int `json:"context_window,omitempty"`
	// DoneCriteria is how many criteria this session is measured against.
	//
	// Zero is a real answer and the one worth carrying: a session with no
	// definition of done reports done at the end of the first turn, and asking
	// for a spec and getting zero is exactly the moment someone needs to be
	// told. Without this the client would have to say "loop opened" and mean
	// nothing by it.
	DoneCriteria int `json:"done_criteria"`
	// LoopID is the `/loop` run that opened this session, when one did.
	LoopID string `json:"loop_id,omitempty"`
}

// CreateSessionRequest opens a session. Workspace must be absolute.
type CreateSessionRequest struct {
	Workspace   string `json:"workspace"`
	Model       string `json:"model,omitempty"`
	SandboxMode string `json:"sandbox_mode,omitempty"`
	// Resume names a recorded session whose conversation this one continues.
	//
	// A new session either way: the old one ended with the client that ran it,
	// and nothing survives what created it. What carries over is the history,
	// rebuilt from the record — not the approvals, which were consent given in
	// a moment that has passed, and not the background processes, which died
	// with the session that started them.
	Resume string `json:"resume,omitempty"`
	// LoopSpec is a directory holding a tasks.md, read as this session's
	// definition of done.
	//
	// A path rather than the parsed criteria: the client may be nowhere near
	// the daemon's filesystem, and a client that read the spec itself would be
	// asserting what a file it cannot see contains. Relative to the workspace,
	// and refused if it climbs out of it.
	LoopSpec string `json:"loop_spec,omitempty"`
	// Protect are globs added to whatever the spec declares as protected.
	//
	// Union, never precedence: Protected surfaces a change rather than
	// forbidding it, so an argument must not be able to REMOVE a protection
	// the file asked for.
	Protect []string `json:"protect,omitempty"`
	// Qualify opens the session that works out what "done" means for LoopSpec,
	// rather than the one that does the work.
	//
	// The LOOP decides there is a qualifying session, not the model: reading,
	// projecting and qualifying are what the loop does before it executes, and
	// a model that chose when to qualify would be choosing when to be measured.
	//
	// It runs in plan mode, always — deciding what you will be measured by is
	// reading, and an agent that could write while deciding can move the thing
	// it is about to be measured against.
	Qualify bool `json:"qualify,omitempty"`
}

// ExecRequest runs a command the person typed, outside a turn.
//
// Its own request rather than a flag on SubmitTurnRequest: submitting starts a
// turn and this one does not. The command runs through the same tool, under the
// same sandbox and the same approvals, and its output is put in the history as
// something the user did — because they did.
type ExecRequest struct {
	Command string `json:"command"`
}

// SubmitTurnRequest submits user input. Rejected with turn_already_active if a
// turn is already running: one turn per session.
type SubmitTurnRequest struct {
	Text string `json:"text"`
	// Images are pictures shown with this turn, base64 encoded with their
	// media type. Sent by value rather than as paths: the daemon may be on
	// another machine, and a path only means something where it was typed.
	Images []TurnImage `json:"images,omitempty"`
	// Documents are files read with this turn: a PDF, or plain text. By
	// value, for the same reason as Images. Checked where they arrive —
	// type, name, size and count — and a PDF for a model whose family does
	// not read one refuses the turn rather than reaching it without the file.
	Documents []TurnDocument `json:"documents,omitempty"`
}

// TurnImage is one picture on the wire.
type TurnImage struct {
	MediaType string `json:"media_type"`
	Data      string `json:"data"`
}

// TurnDocument is one file on the wire.
//
// Name is what the person sees on the message, not a path: the daemon may be
// on another machine, and a path tells it nothing it can use and the log
// something it should not keep. A name with a separator in it is refused.
type TurnDocument struct {
	Name      string `json:"name"`
	MediaType string `json:"media_type"`
	Data      string `json:"data"`
}

// The kinds of document a turn carries, and nothing else.
//
// A PDF goes to the model as a document, and only to a family that says it
// reads one. Text goes to every family, written into the message between
// markers that name the file: every model reads text, so there is nothing for
// a family to declare.
const (
	DocumentPDF      = "application/pdf"
	DocumentText     = "text/plain"
	DocumentMarkdown = "text/markdown"
	DocumentCSV      = "text/csv"
)

// What one turn may carry, declared here so a client can refuse before it
// sends and the daemon refuses the same numbers when it receives.
//
// A PDF is bounded like a picture is. Text is bounded far lower because all of
// it is read: 256 KiB is some sixty thousand tokens, a third of the smallest
// window a family declares, and a file larger than that is one to point the
// agent at rather than to paste.
const (
	MaxDocumentsPerTurn  = 5
	MaxPDFBytes          = 10 << 20
	MaxTextDocumentBytes = 256 << 10
	MaxDocumentNameBytes = 255
)

// AttachedDocument is what the log says about a document: which file, of what
// kind, how large. Never its bytes — the log is replayed to every client that
// attaches and kept on disk, and the conversation already has the content.
type AttachedDocument struct {
	Name      string `json:"name"`
	MediaType string `json:"media_type"`
	Size      int    `json:"size"`
}

// UndoResult is what putting the last turn back changed, and what it would not
// touch.
//
// Refused is per file rather than all-or-nothing: seven files changed and one
// edited by hand should still give six back, and naming the one that stayed is
// more useful than refusing everything on account of it.
type UndoResult struct {
	Restored []string `json:"restored"`
	Refused  []string `json:"refused"`
}

// ApprovalDecision is the user's answer to a boundary crossing.
type ApprovalDecision string

const (
	ApprovalAllow        ApprovalDecision = "allow"
	ApprovalAllowSession ApprovalDecision = "allow_session"
	ApprovalDeny         ApprovalDecision = "deny"

	// The two answers that outlive the session, because some questions are
	// about the project rather than about the moment. Asking again next week
	// about a decision already made is how a prompt becomes something people
	// dismiss without reading — and a prompt nobody reads protects nobody.
	//
	// They are recorded in the USER's config root, never in the workspace: a
	// grant living inside a project would let a repository arrive
	// pre-approved, so cloning something would permit it before anyone read a
	// line of it.
	ApprovalAllowProject ApprovalDecision = "allow_project"
	ApprovalAllowAlways  ApprovalDecision = "allow_always"
)

// Valid reports whether d is a decision the server accepts.
func (d ApprovalDecision) Valid() bool {
	switch d {
	case ApprovalAllow, ApprovalAllowSession, ApprovalDeny,
		ApprovalAllowProject, ApprovalAllowAlways:
		return true
	}
	return false
}

// Remembered reports whether a decision outlives the session, and how widely.
//
// Named rather than compared inline, so the two places that persist a grant and
// the one that renders it cannot drift into disagreeing about which answers are
// standing ones.
func (d ApprovalDecision) Remembered() bool {
	return d == ApprovalAllowProject || d == ApprovalAllowAlways
}

// Grants reports whether a decision permits the crossing at all.
func (d ApprovalDecision) Grants() bool {
	return d != ApprovalDeny && d.Valid()
}

// RenameSessionRequest names a conversation. An empty name restores the title
// derived from the first question, which is the way back rather than a second
// command for undoing.
type RenameSessionRequest struct {
	Name string `json:"name"`
}

// ResolveApprovalRequest answers a pending approval. First writer wins.
type ResolveApprovalRequest struct {
	Decision ApprovalDecision `json:"decision"`
}

// CompactResult answers a forced compaction directly, alongside whatever
// EventSessionCompacted already carried to an attached client. The event
// says nothing when there was nothing worth compacting; this is the one
// place that says so.
type CompactResult struct {
	Compacted bool `json:"compacted"`
}

// ApprovalRequest is emitted when execution crosses the sandbox boundary. The
// turn blocks until it is resolved or ExpiresAt passes, which denies.
//
// ExpiresAt is set before the question is announced, and it is the instant the
// approver denies at, so a client can count down to it. The zero time means
// whoever answers has no deadline — a terminal waits as long as the person
// takes.
type ApprovalRequest struct {
	ApprovalID      string    `json:"approval_id"`
	TurnID          string    `json:"turn_id"`
	ToolCallID      string    `json:"tool_call_id"`
	Tool            string    `json:"tool"`
	Command         string    `json:"command,omitempty"`
	BoundaryCrossed string    `json:"boundary_crossed"`
	ExpiresAt       time.Time `json:"expires_at"`
	// Reason says why in a sentence, and Rule names the pattern that raised the
	// question when one did. Consent to a rule nobody can see is consent to
	// nothing, and Rule is also what an "allow for the session" answer is
	// remembered against.
	Reason string `json:"reason,omitempty"`
	Rule   string `json:"rule,omitempty"`
}

// DoneProposal is a measured definition of done, written for review.
//
// It is the payload of done.proposed, announced in the qualifying session's
// log once the file at Path is written, and never before. It is a notice, not
// a question: the qualifying turn has already ended, the signature is the
// person reviewing the file, and nothing here waits for an answer or expires.
// It carries what the file carries, so a client can draw the review without
// parsing the summary's prose.
type DoneProposal struct {
	ProposalID string `json:"proposal_id"`
	// Path is the done.toml the proposal was written to.
	Path      string          `json:"path"`
	Criteria  []DoneCriterion `json:"criteria"`
	Protected []string        `json:"protected,omitempty"`
	// NoAcceptance says nothing in the set is red. Such a set reports done
	// without anything having to change — usually a defect, and legitimately a
	// refactor. Named, never refused.
	NoAcceptance bool `json:"no_acceptance,omitempty"`
}

// DoneCriterion is one criterion as the operator sees it: what was proposed,
// what was claimed about it, and what running it actually did.
type DoneCriterion struct {
	Name     string `json:"name"`
	Command  string `json:"command"`
	ExitCode int    `json:"exit_code"`
	Expects  string `json:"expects,omitempty"`
	Why      string `json:"why,omitempty"`
	Class    string `json:"class"`
	Exit     int    `json:"exit"`
	Output   string `json:"output,omitempty"`
	// Mismatch is the proposer's claim disagreeing with the measurement. Not an
	// error and not a rejection: it is where the operator's eye should land.
	Mismatch bool `json:"mismatch,omitempty"`
}

// CommitDoneResponse is what came of writing a proposed definition of done.
type CommitDoneResponse struct {
	// Path is the file the proposal was written to.
	Path string `json:"path"`
	// Summary is what a person reads: every criterion, and what running it did
	// before any work happened.
	Summary string `json:"summary"`
	// Criteria is how many were written.
	Criteria int `json:"criteria"`
}

// SpecFolder is one spec folder and where it stands.
//
// Where it stands, not what it declares: "pending" is answered by running the
// folder's own criteria, because a checkbox in a tasks.md is marked by whoever
// felt like marking it.
type SpecFolder struct {
	Path        string `json:"path"`
	Criteria    int    `json:"criteria"`
	Unmet       int    `json:"unmet"`
	Unavailable int    `json:"unavailable,omitempty"`
	// Pending is the answer the client acts on, decided by the daemon so two
	// clients cannot disagree about what counts as work left.
	Pending bool `json:"pending"`
	// Measured says the criteria were actually run to decide Pending.
	//
	// False when the caller asked only what each folder DECLARES, which is a
	// read and costs nothing. Pending is not an answer then, and the field
	// exists so a client cannot mistake "not measured" for "not pending".
	Measured bool   `json:"measured"`
	Error    string `json:"error,omitempty"`
}

// ListSpecsResponse answers "what is there and what is left".
type ListSpecsResponse struct {
	Specs []SpecFolder `json:"specs"`
}

// PlanItem is one entry of the session plan, maintained by the plan tool.
type PlanItem struct {
	ID      int    `json:"id"`
	Text    string `json:"text"`
	Status  string `json:"status"`
	Blocked string `json:"blocked,omitempty"`
}

// Plan item statuses.
const (
	PlanPending = "pending"
	PlanActive  = "active"
	PlanDone    = "done"
	PlanBlocked = "blocked"
)

// PlanUpdated is the payload of EventPlanUpdated.
type PlanUpdated struct {
	Items []PlanItem `json:"items"`
}

// Turn stop reasons, carried in TurnCompleted.Reason.
const (
	StopDone          = "done"
	StopInterrupted   = "interrupted"
	StopMaxIterations = "max_iterations"
	StopRepeatLoop    = "repeat_loop"
	StopMaxTokens     = "max_tokens"
	StopError         = "error"
	// StopUnverified and StopIncomplete are RESULTS, not errors. They are the
	// honest state of work delivered without a check having passed.
	//
	// Treating them as failure would create exactly the wrong incentive: the
	// easy way out becomes switching the checking off.
	StopUnverified = "unverified"
	StopIncomplete = "incomplete"
)

// What one check did to the unmet criteria, carried in DoneChecked.Moved. A
// closed set, so a client says it in the reader's language.
const (
	// DoneMovedFirst is the first check of a turn: there was nothing before it
	// to compare against. Calling it forward would claim progress nobody made.
	DoneMovedFirst = "first"
	// DoneMovedForward: fewer criteria unmet, and none newly broken.
	DoneMovedForward = "forward"
	// DoneMovedStalled: nothing got better and nothing got worse.
	DoneMovedStalled = "stalled"
	// DoneMovedBackward: a criterion that passed does not any more.
	DoneMovedBackward = "backward"
)

// What a Progress counts. A closed set, so a client can say it in the reader's
// language instead of printing the daemon's.
//
// Only what something actually emits is declared. A kind nobody writes is a
// promise on a versioned surface that no code keeps.
const (
	// ProgressRounds is the turn against its iteration ceiling.
	ProgressRounds = "rounds"
	// ProgressInFlight is how many tool calls are running together, against
	// the concurrency ceiling this session allows.
	ProgressInFlight = "in_flight"
	// ProgressArguments is a tool call still arriving from the model, counted
	// in bytes of its arguments.
	//
	// Bytes rather than lines: what has landed is a fragment of JSON, and
	// counting lines inside half an escaped string counts something that is
	// not there yet. It has no total — the model does not say how long the
	// call will be, and a denominator nobody sent is one somebody would trust.
	ProgressArguments = "arguments"
	// ProgressFiles is a scan working through files. Total is known when the
	// tool had the list before it started and absent when it is still walking.
	ProgressFiles = "files"
)

// There is no kind for lines, and that is a finding rather than an omission.
//
// `read` takes the whole file and splits it, so it learns the total at the same
// moment it learns the content: there is no point at which "n of 240" is true.
// Counting passing tests is worse — it would mean parsing `bash` output, which
// ToolCompleted's own comment forbids in as many words. A kind that could only
// be filled dishonestly is a kind that does not get declared.

// Payloads for the remaining event types.
type (
	// TurnStarted announces an accepted input.
	// SteerRequest is what the person says to a turn already running.
	SteerRequest struct {
		Text string `json:"text"`
	}

	// TurnSteered is what the person said while the turn was running, and the
	// round it landed at.
	TurnSteered struct {
		TurnID string `json:"turn_id"`
		Text   string `json:"text"`
	}

	TurnStarted struct {
		TurnID string `json:"turn_id"`
		// Text is what the user asked for.
		//
		// It rides here because a turn starts BECAUSE of it, and because
		// nothing else carried it: the log held the model's side of a
		// conversation and none of the questions. A transcript could not be
		// read, a session could not be titled, and a client attaching to a
		// session already under way saw answers to questions it never saw.
		Text string `json:"text,omitempty"`
		// Documents names the files read with the question, so history shows
		// what was attached to it and not only what was typed.
		Documents []AttachedDocument `json:"documents,omitempty"`
	}
	// MessageReasoning is a fragment of the model's thinking.
	MessageReasoning struct {
		TurnID string `json:"turn_id"`
		Text   string `json:"text"`
	}
	// MessageDelta is a fragment of model text.
	MessageDelta struct {
		TurnID string `json:"turn_id"`
		Text   string `json:"text"`
	}
	// ToolRequested announces a tool call before policy evaluation.
	ToolRequested struct {
		TurnID     string          `json:"turn_id"`
		ToolCallID string          `json:"tool_call_id"`
		Name       string          `json:"name"`
		Input      json.RawMessage `json:"input"`
		// Typed marks a call the PERSON asked for, through `!`, rather than one
		// the model asked for.
		//
		// The two want opposite things on screen. A model's call is a means: it
		// runs `ls` to orient itself and then says what mattered, so the output
		// collapses and the prose carries the point. A typed command IS the
		// point — someone wrote `ls -la` because they want to see `ls -la`, and
		// collapsing it hides the only thing they asked for.
		//
		// Carried on the event rather than inferred from the call id, because
		// an id's shape is not a fact about who wanted the call.
		Typed bool `json:"typed,omitempty"`
	}
	// ApprovalResolved records the decision that unblocked a turn.
	ApprovalResolved struct {
		ApprovalID string           `json:"approval_id"`
		Decision   ApprovalDecision `json:"decision"`
	}
	// ToolCompleted carries the result of an execution.
	ToolCompleted struct {
		ToolCallID string `json:"tool_call_id"`
		OK         bool   `json:"ok"`
		Output     string `json:"output"`
		Truncated  bool   `json:"truncated"`

		// What the call did, stated once by the tool that knows.
		//
		// A client that parses Output to rebuild these numbers breaks silently
		// the day the wording changes, and every client has to reimplement the
		// same parsing. All optional: a tool reports what applies to it.
		Lines      int  `json:"lines,omitempty"`
		Files      int  `json:"files,omitempty"`
		Added      int  `json:"added,omitempty"`
		Removed    int  `json:"removed,omitempty"`
		ExitCode   int  `json:"exit_code,omitempty"`
		HasExit    bool `json:"has_exit,omitempty"`
		DurationMS int  `json:"duration_ms,omitempty"`

		// StartedAt and FinishedAt are the REAL execution order (RN-3.3).
		//
		// A duration cannot answer which of two concurrent calls went first,
		// and results arrive in emission order rather than the order they ran.
		// A client showing a batch has a person looking at it, and "which of
		// these happened first" is a reasonable thing for them to ask.
		//
		// They live HERE and never in the context sent to the model: a
		// timestamp in the prefix would make two runs of the same session
		// differ, which ADR-03 forbids. The event is a person's window; the
		// context is not.
		StartedAt  time.Time `json:"started_at,omitempty"`
		FinishedAt time.Time `json:"finished_at,omitempty"`

		// Diff is the unified diff of a change. Present only for tools that
		// modify a file, and never part of what the model was sent.
		Diff string `json:"diff,omitempty"`
	}
	// Progress is how far along something running has got.
	//
	// Done without Total is honest: a scan that has not finished enumerating
	// knows how many it has seen and not how many there are. A client shows the
	// count alone rather than inventing a denominator.
	//
	// Kind comes from the closed set below rather than being a word to print.
	// The daemon's language is not the reader's, and a client that renders the
	// payload's text shows the wrong one to half its users.
	//
	// Never part of the context the model is sent: it is a person's window, the
	// same rule StartedAt already carries, and a count that changes between two
	// runs of one session is exactly what ADR-03 forbids in a prefix.
	Progress struct {
		TurnID string `json:"turn_id"`
		// ToolCallID names the call this is about; empty means the turn itself.
		ToolCallID string `json:"tool_call_id,omitempty"`
		// Name is the tool, sent only while a call is still ARRIVING — before
		// tool.requested exists to carry it. A subject that does not exist yet
		// has to name itself, or the report has nowhere to land.
		Name  string `json:"name,omitempty"`
		Kind  string `json:"kind"`
		Done  int    `json:"done"`
		Total int    `json:"total,omitempty"`
	}
	// SessionRenamed is the name a person gave, which beats the one derived
	// from the first question. Empty gives the derived title back.
	SessionRenamed struct {
		Name string `json:"name"`
	}
	// TurnCompleted ends a turn with one of the Stop* reasons.
	TurnCompleted struct {
		TurnID string `json:"turn_id"`
		Reason string `json:"reason"`
		// Usage is what the turn cost. Absent when the provider did not report
		// it, which is why it is a pointer: zero tokens and unknown tokens are
		// different facts and a client shows them differently.
		Usage *Usage `json:"usage,omitempty"`
		// Completion is what was and was not checked. Absent when the turn had
		// no definition of done, which is why it is a pointer: "nothing to
		// check" and "checked, all met" are different facts.
		//
		// It travels on the wire because it is the guarantee that survives a
		// model claiming success in prose. The text can lie; this cannot.
		Completion *Completion `json:"completion,omitempty"`
	}

	// Completion is the state of the done criteria when a turn ended.
	Completion struct {
		// Verification is the single-criterion seal: clean, passed, failed,
		// stale or unavailable.
		Verification string `json:"verification"`
		// Met and Unmet name the criteria, so a client can show which.
		Met   []string `json:"met,omitempty"`
		Unmet []string `json:"unmet,omitempty"`
		// Unavailable are criteria that could not be run at all. Different from
		// unmet, and shown differently: nothing was learned about them.
		Unavailable []string `json:"unavailable,omitempty"`
		// TouchedProtected are paths that are part of how the work is measured
		// and were written this turn. Never omitted when present.
		TouchedProtected []string `json:"touched_protected,omitempty"`
	}

	// Usage is the token accounting for a turn.
	//
	// InputTokens is CUMULATIVE across the turn's rounds: every round re-sends
	// the context, so a forty-round turn sums forty readings of it. That is the
	// right number for what a turn COST and the wrong one for how full the
	// context is — a client that divided it by the window showed `ctx 175%`.
	Usage struct {
		InputTokens      int `json:"input_tokens"`
		OutputTokens     int `json:"output_tokens"`
		CacheReadTokens  int `json:"cache_read_tokens,omitempty"`
		CacheWriteTokens int `json:"cache_write_tokens,omitempty"`
		// ContextTokens is what the assembled context costs NOW — not summed,
		// and stated by the daemon rather than derived by a client.
		//
		// It is the same estimate the compaction trigger reads, deliberately:
		// a meter and a threshold that disagree are worse than either alone,
		// because the person sees the summary happen at a number the screen
		// never showed. Provider counts could not give this — the two families
		// disagree about whether input_tokens already includes the cached
		// prefix, so the same context reads differently depending on who
		// answered.
		ContextTokens int `json:"context_tokens,omitempty"`
	}
	// SessionCompacted records a context compaction.
	SessionCompacted struct {
		FromSeq uint64 `json:"from_seq"`
		ToSeq   uint64 `json:"to_seq"`
		// Messages is how many were replaced by the summary, and Kept how many
		// survived it. "Earlier history was summarised" says that something
		// happened; these say how much, which is the difference between a
		// notice and an answer.
		Messages int `json:"messages,omitempty"`
		Kept     int `json:"kept,omitempty"`
	}
	// SkillLoaded is one skill body appended to the turn.
	//
	// Name and WhenToUse, and not the path: the event log is read by another
	// client, possibly on another machine, and an absolute path from the
	// machine that wrote it is not a fact there. Which root a skill came from
	// is a question `--dump-prompt` and the filesystem answer; which skill
	// fired is the one nothing answered.
	SkillLoaded struct {
		Name      string `json:"name"`
		WhenToUse string `json:"when_to_use,omitempty"`
	}
	// ContextBand is the context crossing a threshold on the way to being
	// summarised.
	//
	// The model is told this already, and has been for a while. The PERSON was
	// not, so the summary arrived as a line saying it had happened — after the
	// fact, with no warning that it was coming and no way to finish a thought
	// first.
	//
	// Fraction is of the BUDGET, not of the window: the budget is the space
	// before compaction, so 0.80 means eighty per cent of the way to a summary.
	// Against the window it would be a number about a limit that never arrives.
	ContextBand struct {
		Band     int     `json:"band"`
		Fraction float64 `json:"fraction"`
	}

	// SessionResumed names the conversation the events after it came from.
	//
	// Turns is what happened there, counted before the replay rather than by
	// the reader, so a client that joins late reads the same number as one that
	// was there.
	SessionResumed struct {
		SourceID  string    `json:"source_id"`
		Turns     int       `json:"turns"`
		StartedAt time.Time `json:"started_at,omitempty"`
	}
	// SessionModeChanged announces a behavioural mode switch.
	//
	// Previous is empty on the very first announce, where there was nothing to
	// come from. Carried alongside Mode so the transcript shows the transition
	// rather than only the destination.
	SessionModeChanged struct {
		Previous string `json:"previous,omitempty"`
		Mode     string `json:"mode"`
		// SandboxMode is the technical half the switch just installed.
		//
		// Carried rather than derived by the client, for the same reason the
		// mode name is derived from the pair on the daemon: the mapping has one
		// home. A client computing it from the name would be a second copy of
		// the table, and the copy is what drifts.
		//
		// Without it the top bar went on announcing the boundary the session
		// was created with — and §2.1 of client-tui calls the sandbox field the
		// one place where being wrong is dangerous.
		SandboxMode string `json:"sandbox_mode"`
	}
	// DoneChecked is one check of the definition of done inside a turn.
	//
	// Names, never counts of tests: what a criterion printed is the model's to
	// read, and counting passes would mean parsing a command's output, which
	// ToolCompleted's own comment forbids.
	//
	// Never part of the context the model is sent. It is a person's window, as
	// Progress is: the model is told the same facts as a reminder, in its own
	// words.
	DoneChecked struct {
		TurnID string `json:"turn_id"`
		// Cycle counts the checks of this turn, from 1.
		Cycle       int      `json:"cycle"`
		Met         []string `json:"met,omitempty"`
		Unmet       []string `json:"unmet,omitempty"`
		Unavailable []string `json:"unavailable,omitempty"`
		// Moved is one of the DoneMoved* values.
		Moved string `json:"moved"`
		// Stall is how many checks in a row moved nothing forward, against
		// StallLimit, the count at which the turn ends incomplete. Carried
		// together so a client never holds a second copy of the ceiling.
		Stall      int `json:"stall"`
		StallLimit int `json:"stall_limit"`
		// Undone are the files the loop put back because this check found a
		// criterion broken that had passed.
		Undone []string `json:"undone,omitempty"`
		// TouchedProtected are paths that measure the work and were written.
		TouchedProtected []string `json:"touched_protected,omitempty"`
	}
)
