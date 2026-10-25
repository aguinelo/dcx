package protocol

import "time"

// ConversationRecorded is the state of a conversation that is not live in this
// daemon: it ended, and its record is what remains. Continuing one opens a new
// session (CreateSessionRequest.Resume).
const ConversationRecorded SessionState = "recorded"

// Conversation is one session as a list of them shows it: live in this daemon,
// or recorded on disk.
//
// A summary, folded from the session's events by the same code whether they
// arrive live or are read from its record — so a conversation that ends
// changes in the list only by having ended.
type Conversation struct {
	ID string `json:"id"`
	// Title is the name a person gave the conversation, else the first thing
	// asked in it, else — for one that continues another and has not asked
	// anything yet — the title of the one it continues. Empty when there is
	// none of those; Named says when it is a chosen name.
	Title string `json:"title"`
	Named bool   `json:"named,omitempty"`

	Workspace string `json:"workspace"`
	Branch    string `json:"branch,omitempty"`
	Model     string `json:"model,omitempty"`

	// State is a live session's state now, or ConversationRecorded.
	State SessionState `json:"state"`
	Live  bool         `json:"live"`

	// Turns is how many completed.
	Turns int `json:"turns"`
	// Verification is the seal of the last completed turn: clean, passed,
	// failed, stale or unavailable. Empty when no turn has completed.
	Verification string `json:"verification,omitempty"`
	// Added, Removed and Files sum what the tools reported changing, the way
	// the TUI's bar sums them: a call that changed nothing is not a file.
	Added   int `json:"added,omitempty"`
	Removed int `json:"removed,omitempty"`
	Files   int `json:"files,omitempty"`

	Started      time.Time `json:"started"`
	LastActivity time.Time `json:"last_activity"`
	// LastEvent is the last event that moved the conversation along — a turn
	// started or completed, a tool completed, an approval asked or answered, a
	// rename — never a fragment of text arriving.
	LastEvent EventType `json:"last_event,omitempty"`

	// ContinuedFrom is the conversation this one continues, when it does.
	ContinuedFrom string `json:"continued_from,omitempty"`
	// LoopID is the `/loop` run this conversation belongs to, when it does, so
	// a list can group a loop's sessions without reading another stream.
	LoopID string `json:"loop_id,omitempty"`
}

// ListConversationsResponse answers GET /conversations: newest activity first.
type ListConversationsResponse struct {
	Conversations []Conversation `json:"conversations"`
}

// The kinds of frame on GET /conversations/events.
const (
	// ConversationSnapshot opens the stream with the whole list, so a client
	// never has to stitch a listing to a stream it opened after it.
	ConversationSnapshot = "snapshot"
	// ConversationChanged carries one conversation that changed in something
	// a list shows.
	ConversationChanged = "changed"
	// ConversationRemoved names a conversation that left the list: one that
	// ended with nothing recording it.
	ConversationRemoved = "removed"
)

// ConversationChange is one frame of the list's stream.
type ConversationChange struct {
	Kind          string         `json:"kind"`
	Conversations []Conversation `json:"conversations,omitempty"`
	Conversation  *Conversation  `json:"conversation,omitempty"`
	ID            string         `json:"id,omitempty"`
}

// ConversationEventsLimit is the most events one page of a recorded
// conversation carries, and what a page carries when the request does not say.
//
// A record holds every fragment of every answer, so a long conversation is
// tens of thousands of events; one response with all of them would be however
// many megabytes the conversation happened to reach. A limit above this is
// refused rather than lowered, so a client is never handed less than it asked
// for without being told.
const ConversationEventsLimit = 5000

// ConversationEventsResponse answers GET /conversations/{id}/events: one page
// of a recorded conversation, read from its record without loading it.
//
// The events are the conversation the way a session continuing it would
// replay it — the chain it continues first, oldest first, a question already
// answered left out — numbered as one sequence from 1 under the conversation's
// own id. Seq is the position in that reading, which is what from and Next
// count.
type ConversationEventsResponse struct {
	Events []Event `json:"events"`
	// Total is how many events the whole reading has.
	Total int `json:"total"`
	// Next is the position the next page starts at, or zero when this page
	// ends the conversation. Non-zero is the response saying it was cut.
	Next uint64 `json:"next,omitempty"`
}
