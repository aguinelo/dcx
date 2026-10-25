package protocol

// How a file differs from the last commit, as GitFile.Status says it.
const (
	GitModified   = "modified"
	GitAdded      = "added"
	GitDeleted    = "deleted"
	GitUntracked  = "untracked"
	GitConflicted = "conflicted"
)

// GitDiffLimit is how many bytes of one file's diff GET /git/diff hands over.
// Past it the diff is cut at a line and says so: a generated file rewritten
// whole would otherwise travel as megabytes nobody reads in a side panel.
const GitDiffLimit = 256 << 10

// GitFile is one file git lists as changed, against the last commit.
type GitFile struct {
	// Path is relative to the top of the work tree, with forward slashes.
	Path string `json:"path"`
	// Status is one of the Git* constants.
	Status  string `json:"status"`
	Added   int    `json:"added"`
	Removed int    `json:"removed"`
	// Binary says git counts no lines for it; Added and Removed are then zero
	// because there are no lines, not because nothing changed.
	Binary bool `json:"binary"`
}

// GitResponse answers GET /git: where a workspace is in git now, and what
// changed in it.
//
// Read when asked, never kept: the branch is the one checked out now, which
// may not be the one a session started on (Session.Branch is that). The files
// are the folder's, every change since the last commit, staged or not — not
// only what one conversation wrote.
type GitResponse struct {
	// Repository is false for a folder outside any repository: the ordinary
	// case for a scratch folder, said rather than answered as a clean tree.
	Repository bool `json:"repository"`
	// Root is the top of the work tree the workspace is in.
	Root string `json:"root,omitempty"`
	// Branch is empty when Detached.
	Branch   string `json:"branch,omitempty"`
	Detached bool   `json:"detached"`
	// Head is the abbreviated commit checked out; empty before the first one.
	Head string `json:"head,omitempty"`
	// Worktree says Root is a linked worktree (`git worktree add`), and
	// MainCheckout is then the repository's own working tree — or its git
	// directory, for a bare repository.
	Worktree     bool   `json:"worktree"`
	MainCheckout string `json:"main_checkout,omitempty"`
	// Files is in path order; none is an empty list, never null.
	Files []GitFile `json:"files"`
	// Truncated says there were more changed files than the list carries.
	Truncated bool `json:"truncated"`
}

// GitDiffResponse answers GET /git/diff: one changed file's unified diff
// against the last commit, or against nothing for a file git does not track.
type GitDiffResponse struct {
	Path   string `json:"path"`
	Status string `json:"status"`
	// Diff is as git writes it, without colour, cut at a line past Limit.
	Diff string `json:"diff"`
	// Binary says git has no lines to show for it, and Diff only says so.
	Binary    bool `json:"binary"`
	Truncated bool `json:"truncated"`
	// Limit is the cap the diff was cut at, in bytes (GitDiffLimit).
	Limit int `json:"limit"`
}

// How a PR's checks add up, as GitChecks.State says it. A failure beats a
// check still running, which beats a pass: never pass over a red check.
const (
	GitChecksPass    = "pass"
	GitChecksFail    = "fail"
	GitChecksPending = "pending"
	// GitChecksNone is a PR with no check that passed, failed or waits —
	// none at all, or only skipped ones.
	GitChecksNone = "none"
)

// How a PR stands, as GitPR.State says it.
const (
	GitPROpen   = "open"
	GitPRClosed = "closed"
	GitPRMerged = "merged"
)

// GitChecks is a PR's checks, counted and summed up in one word.
type GitChecks struct {
	// State is one of the GitChecks* constants.
	State   string `json:"state"`
	Passed  int    `json:"passed"`
	Failed  int    `json:"failed"`
	Pending int    `json:"pending"`
	// Skipped counts checks that ended skipped or neutral: neither green nor red.
	Skipped int `json:"skipped"`
}

// GitPR is one pull request as GitHub has it, read through gh.
type GitPR struct {
	Number int    `json:"number"`
	URL    string `json:"url"`
	Title  string `json:"title"`
	// State is one of the GitPR* constants.
	State  string    `json:"state"`
	Draft  bool      `json:"draft"`
	Base   string    `json:"base"`
	Head   string    `json:"head"`
	Checks GitChecks `json:"checks"`
}

// GitPRResponse answers GET /git/pr, and POST /git/pr once the PR is open:
// the branch checked out now, the base a new PR targets unless asked
// otherwise, and the branch's PR.
//
// Read when asked, never kept, like GitResponse: CI moves on GitHub, and the
// client asks again when it wants to know.
type GitPRResponse struct {
	Branch string `json:"branch"`
	// Base is the repository's default branch, as the clone knows it or else
	// as GitHub says.
	Base string `json:"base"`
	// PR is absent when the branch has none — open, merged or closed.
	PR *GitPR `json:"pr,omitempty"`
}

// CreateGitPRRequest is POST /git/pr: what a person confirmed, every field as
// they saw it. The daemon pushes Workspace's branch to origin, never by
// force, and opens the PR; it never merges and never turns auto-merge on.
type CreateGitPRRequest struct {
	// Workspace is absolute; the branch is the one checked out there now.
	Workspace string `json:"workspace"`
	// Title is one line, at most 256 characters.
	Title string `json:"title"`
	// Body is at most 65536 bytes.
	Body string `json:"body"`
	// Base empty is GitPRResponse.Base.
	Base  string `json:"base"`
	Draft bool   `json:"draft"`
}
