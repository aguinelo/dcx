package protocol

// MemoryEntry is one thing an earlier session in the workspace learned, as the
// memory file holds it.
//
// The identity is the one the memory package has: a kind from a closed list
// and a subject. There is no id, because the file is edited by hand and an id
// nobody writes is an id nobody keeps.
type MemoryEntry struct {
	// Kind is gotcha, decision or convention.
	Kind    string `json:"kind"`
	Subject string `json:"subject"`
	Body    string `json:"body,omitempty"`
	// Learned and Commit are the provenance, either of them empty in a memory
	// written by hand.
	Learned string `json:"learned,omitempty"`
	Commit  string `json:"commit,omitempty"`
	// Stale says the commit it was true at is no longer in the repository.
	// Marked, never dropped: a session reads it with the same mark.
	Stale bool `json:"stale"`
	// Shown says a session reads it: memory is on, the file could be read, and
	// the entry is within the cap, which keeps the most recent.
	Shown bool `json:"shown"`
}

// MemoryMalformed is a block that looked like a memory and was not, with why.
type MemoryMalformed struct {
	Line   string `json:"line"`
	Reason string `json:"reason"`
}

// MemoryResponse answers GET /memory: what a session in the workspace reads as
// memory, and what in the file it cannot read.
//
// Never anything but the workspace's memory file. Entries are in file order,
// the oldest first, as the file keeps them. No entries is an empty list, never
// null, and so is no malformed block.
type MemoryResponse struct {
	// Path is the file, relative to the workspace.
	Path string `json:"path"`
	// Exists says the file is there. Absent is the ordinary case, and answers
	// no entries rather than an error.
	Exists bool `json:"exists"`
	// Enabled says sessions in the workspace read memory at all
	// (`memory.enabled`). Off, the file is still listed, and nothing in it is
	// shown.
	Enabled bool `json:"enabled"`
	// MaxEntries is how many memories reach a session (`memory.max_entries`).
	MaxEntries int               `json:"max_entries"`
	Entries    []MemoryEntry     `json:"entries"`
	Malformed  []MemoryMalformed `json:"malformed"`
	// Unreadable is why the file could not be read, when it could not. A
	// session there opens without its memory and says the same.
	Unreadable string `json:"unreadable,omitempty"`
}
