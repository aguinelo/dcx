package protocol

import (
	"encoding/json"
	"errors"
	"net/http"
	"testing"
	"time"
)

// roundTrip marshals v, unmarshals into a fresh value of the same type, and
// marshals again. Both encodings must be byte-identical: the wire contract has
// to survive a hop through a client that only knows these types.
func roundTrip[T any](t *testing.T, v T) {
	t.Helper()
	first, err := json.Marshal(v)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	var back T
	if err := json.Unmarshal(first, &back); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	second, err := json.Marshal(back)
	if err != nil {
		t.Fatalf("re-marshal: %v", err)
	}
	if string(first) != string(second) {
		t.Errorf("round trip changed the encoding:\n first=%s\nsecond=%s", first, second)
	}
}

func TestRoundTrip(t *testing.T) {
	at := time.Date(2026, 8, 8, 12, 0, 0, 0, time.UTC)

	t.Run("event", func(t *testing.T) {
		roundTrip(t, Event{
			Seq: 7, SessionID: "01J", Type: EventMessageDelta, At: at,
			Payload: json.RawMessage(`{"turn_id":"t1","text":"oi"}`),
		})
	})
	t.Run("session", func(t *testing.T) {
		roundTrip(t, Session{
			ID: "01J", State: SessionStateRunning, Workspace: "/w",
			Model: "MiniMax-M3", SandboxMode: "workspace-write", CreatedAt: at, LastSeq: 3,
		})
	})
	t.Run("approval request", func(t *testing.T) {
		roundTrip(t, ApprovalRequest{
			ApprovalID: "a1", TurnID: "t1", ToolCallID: "c1", Tool: "bash",
			Command: "curl https://x", BoundaryCrossed: "network", ExpiresAt: at,
		})
	})
	t.Run("plan", func(t *testing.T) {
		roundTrip(t, PlanUpdated{Items: []PlanItem{
			{ID: 1, Text: "map", Status: PlanDone},
			{ID: 2, Text: "run", Status: PlanBlocked, Blocked: "missing dep"},
		}})
	})
	t.Run("error", func(t *testing.T) {
		roundTrip(t, Error{Code: CodeTurnAlreadyActive, Message: "busy"})
	})
}

// Optional fields must vanish when empty. A payload that always carries empty
// keys wastes bytes on every event, and events are the highest-volume message.
func TestOptionalFieldsOmitted(t *testing.T) {
	for _, tc := range []struct {
		name   string
		v      any
		absent string
	}{
		{"approval command", ApprovalRequest{ApprovalID: "a"}, "command"},
		{"plan blocked", PlanItem{ID: 1, Status: PlanPending}, "blocked"},
		{"error details", Error{Code: CodeInternal}, "details"},
		{"create session model", CreateSessionRequest{Workspace: "/w"}, "model"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			b, err := json.Marshal(tc.v)
			if err != nil {
				t.Fatal(err)
			}
			var m map[string]json.RawMessage
			if err := json.Unmarshal(b, &m); err != nil {
				t.Fatal(err)
			}
			if _, present := m[tc.absent]; present {
				t.Errorf("field %q should be omitted when empty: %s", tc.absent, b)
			}
		})
	}
}

func TestSessionStateValid(t *testing.T) {
	for _, s := range []SessionState{SessionStateIdle, SessionStateRunning, SessionStateBlocked, SessionStateClosed} {
		if !s.Valid() {
			t.Errorf("%q should be valid", s)
		}
	}
	for _, s := range []SessionState{"", "paused", "IDLE"} {
		if SessionState(s).Valid() {
			t.Errorf("%q should not be valid", s)
		}
	}
}

func TestApprovalDecisionValid(t *testing.T) {
	for _, d := range []ApprovalDecision{ApprovalAllow, ApprovalAllowSession, ApprovalDeny} {
		if !d.Valid() {
			t.Errorf("%q should be valid", d)
		}
	}
	// An unknown decision must not be accepted: the safe reading of a typo is
	// never "allow".
	for _, d := range []string{"", "yes", "ALLOW", "allow-session"} {
		if ApprovalDecision(d).Valid() {
			t.Errorf("%q should not be valid", d)
		}
	}
}

func TestHTTPStatus(t *testing.T) {
	for code, want := range map[string]int{
		CodeSessionNotFound:    http.StatusNotFound,
		CodeTurnAlreadyActive:  http.StatusConflict,
		CodeApprovalResolved:   http.StatusConflict,
		CodeApprovalExpired:    http.StatusGone,
		CodeEventsExpired:      http.StatusGone,
		CodeWorkspaceInvalid:   http.StatusBadRequest,
		CodeMaxSessionsReached: http.StatusServiceUnavailable,
		CodeGhUnavailable:      http.StatusServiceUnavailable,
		CodePRRefused:          http.StatusConflict,
		CodeInternal:           http.StatusInternalServerError,
	} {
		if got := HTTPStatus(code); got != want {
			t.Errorf("%s: got %d want %d", code, got, want)
		}
	}
	// An unmapped code must fail loudly, not pass for success.
	if got := HTTPStatus("not_a_real_code"); got != http.StatusInternalServerError {
		t.Errorf("unmapped code: got %d want 500", got)
	}
}

func TestErrorFormatting(t *testing.T) {
	e := Errorf(CodeWorkspaceInvalid, "workspace %q is not absolute", "rel/path")
	if want := `workspace_invalid: workspace "rel/path" is not absolute`; e.Error() != want {
		t.Errorf("got %q want %q", e.Error(), want)
	}
	if e.Status() != http.StatusBadRequest {
		t.Errorf("status: got %d", e.Status())
	}
	// Code alone is a legitimate error, and must not render a stray separator.
	if bare := (&Error{Code: CodeInternal}); bare.Error() != CodeInternal {
		t.Errorf("bare error: got %q", bare.Error())
	}
}

func TestAsError(t *testing.T) {
	e := Errorf(CodeSessionNotFound, "gone")
	// Wrapped errors must still be recoverable: callers wrap with context and
	// the server still needs the code to pick a status.
	if got, ok := AsError(wrap(e)); !ok || got.Code != CodeSessionNotFound {
		t.Errorf("wrapped: got %v ok=%v", got, ok)
	}
	if _, ok := AsError(errors.New("plain")); ok {
		t.Error("a plain error must not be reported as a protocol error")
	}
	if _, ok := AsError(nil); ok {
		t.Error("nil must not be reported as a protocol error")
	}
}

func wrap(err error) error { return errWrap{err} }

type errWrap struct{ error }

func (e errWrap) Unwrap() error { return e.error }

// The two standing answers, and the reason they are separate from the two that
// are not: a decision the user makes about a project should not be asked again
// next week, and a decision they make about this moment should.
func TestTheStandingAnswersAreDistinguishableFromTheMomentaryOnes(t *testing.T) {
	for _, c := range []struct {
		d          ApprovalDecision
		valid      bool
		remembered bool
		grants     bool
	}{
		{ApprovalAllow, true, false, true},
		{ApprovalAllowSession, true, false, true},
		{ApprovalAllowProject, true, true, true},
		{ApprovalAllowAlways, true, true, true},
		{ApprovalDeny, true, false, false},
		{ApprovalDecision("allow_everything"), false, false, false},
		{ApprovalDecision(""), false, false, false},
	} {
		t.Run(string(c.d), func(t *testing.T) {
			if got := c.d.Valid(); got != c.valid {
				t.Errorf("Valid() = %v, want %v", got, c.valid)
			}
			if got := c.d.Remembered(); got != c.remembered {
				t.Errorf("Remembered() = %v, want %v", got, c.remembered)
			}
			if got := c.d.Grants(); got != c.grants {
				t.Errorf("Grants() = %v, want %v", got, c.grants)
			}
		})
	}
}

// An answer nobody defined must never be read as permission. A decision the
// server does not understand is not a decision, and treating it as one would
// let a malformed client grant what the user never did.
func TestAnUnknownAnswerGrantsNothing(t *testing.T) {
	for _, s := range []string{"", "yes", "ALLOW", "allow_forever", "true"} {
		if d := ApprovalDecision(s); d.Grants() {
			t.Errorf("%q was read as permission", s)
		}
	}
}
