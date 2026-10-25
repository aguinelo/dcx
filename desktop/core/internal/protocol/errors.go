package protocol

import (
	"errors"
	"fmt"
	"net/http"
)

// Error is the wire error. Code is the stable machine identifier; Message is
// human text in English. Localisation, if it ever exists, hangs off Code and
// never off Message.
type Error struct {
	Code    string `json:"code"`
	Message string `json:"message"`
}

func (e *Error) Error() string {
	if e.Message == "" {
		return e.Code
	}
	return fmt.Sprintf("%s: %s", e.Code, e.Message)
}

// Error codes. Section 8 of the planning spec.
const (
	CodeSessionNotFound   = "session_not_found"
	CodeTurnAlreadyActive = "turn_already_active"
	// CodeNoActiveTurn answers a correction aimed at a turn that is not
	// running. Distinct from turn_already_active and its exact mirror: one says
	// "wait", the other says "there is nothing to correct — send it as a
	// message". Collapsing them would leave the client guessing which.
	CodeNoActiveTurn       = "no_active_turn"
	CodeInvalidInput       = "invalid_input"
	CodeApprovalResolved   = "approval_already_resolved"
	CodeApprovalExpired    = "approval_expired"
	CodeEventsExpired      = "events_expired"
	CodeWorkspaceInvalid   = "workspace_invalid"
	CodeMaxSessionsReached = "max_sessions_reached"
	// CodeLoopAlreadyRunning refuses a second loop in a workspace that has one
	// alive: two loops in one tree measure each other's work.
	CodeLoopAlreadyRunning = "loop_already_running"
	// CodeLoopNotAwaitingReview answers a continue for a run that is not
	// waiting for one — usually because another client continued it first.
	CodeLoopNotAwaitingReview = "loop_not_awaiting_review"
	// CodeLoopNotFound names a run id the daemon does not hold.
	CodeLoopNotFound = "loop_not_found"
	// CodeGhUnavailable is the GitHub CLI missing where the daemon runs, or
	// there without a login: the PR routes have nobody to ask GitHub.
	CodeGhUnavailable = "gh_unavailable"
	// CodePRRefused is a branch that cannot become a PR as things stand — no
	// branch, no origin, the base itself, nothing to propose, a PR already
	// open, a push the remote refused — with the reason.
	CodePRRefused = "pr_refused"
	CodeInternal  = "internal"
)

// httpStatus maps each code to its response status. Codes absent from the map
// are a programming error and surface as 500 rather than silently as 200.
var httpStatus = map[string]int{
	CodeSessionNotFound:       http.StatusNotFound,
	CodeTurnAlreadyActive:     http.StatusConflict,
	CodeNoActiveTurn:          http.StatusConflict,
	CodeInvalidInput:          http.StatusBadRequest,
	CodeApprovalResolved:      http.StatusConflict,
	CodeApprovalExpired:       http.StatusGone,
	CodeEventsExpired:         http.StatusGone,
	CodeWorkspaceInvalid:      http.StatusBadRequest,
	CodeMaxSessionsReached:    http.StatusServiceUnavailable,
	CodeLoopAlreadyRunning:    http.StatusConflict,
	CodeLoopNotAwaitingReview: http.StatusConflict,
	CodeLoopNotFound:          http.StatusNotFound,
	CodeGhUnavailable:         http.StatusServiceUnavailable,
	CodePRRefused:             http.StatusConflict,
	CodeInternal:              http.StatusInternalServerError,
}

// HTTPStatus returns the status for a code, or 500 for an unmapped one.
func HTTPStatus(code string) int {
	if s, ok := httpStatus[code]; ok {
		return s
	}
	return http.StatusInternalServerError
}

// Status returns the HTTP status this error should be sent with.
func (e *Error) Status() int { return HTTPStatus(e.Code) }

// Errorf builds an Error with a formatted message.
func Errorf(code, format string, args ...any) *Error {
	return &Error{Code: code, Message: fmt.Sprintf(format, args...)}
}

// AsError extracts a *Error from err, if there is one.
func AsError(err error) (*Error, bool) {
	var pe *Error
	ok := errors.As(err, &pe)
	return pe, ok
}
