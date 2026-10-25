package protocol

import (
	"encoding/json"
	"flag"
	"os"
	"path/filepath"
	"testing"
	"time"
)

var update = flag.Bool("update", false, "rewrite the golden files")

// golden compares against a recorded shape, or rewrites it under -update.
//
// The point is not that the bytes are pretty. It is that the wire format of a
// contract marked `stable` cannot drift by accident: a renamed field, a tag
// that stops being omitempty, a type that becomes a pointer — each is a silent
// break for every client, and each is invisible to a test that only round-trips
// a value through the same code that produced it.
func golden(t *testing.T, name string, v any) {
	t.Helper()
	got, err := json.MarshalIndent(v, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	got = append(got, '\n')

	path := filepath.Join("testdata", name+".json")
	if *update {
		if err := os.MkdirAll("testdata", 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, got, 0o644); err != nil {
			t.Fatal(err)
		}
		return
	}
	want, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("%s: %v\nRun `go test ./internal/protocol -update` to record it.", path, err)
	}
	if string(got) != string(want) {
		t.Errorf("%s no longer matches its recorded shape.\n\nwant:\n%s\ngot:\n%s\n\n"+
			"This type is part of a contract with third parties. If the change is intended, "+
			"re-record with -update and say so in a changelog entry.", path, want, got)
	}
}

// A fixed instant, because a clock in a golden file breaks the test on every
// run — the trap the .i spec names by hand.
var at = time.Date(2026, 8, 11, 12, 0, 0, 0, time.UTC)

func TestGoldenEvent(t *testing.T) {
	golden(t, "event", Event{
		Seq: 42, Type: EventToolCompleted, At: at,
		Payload: json.RawMessage(`{"tool":"read"}`),
	})
}

func TestGoldenSession(t *testing.T) {
	golden(t, "session", Session{
		ID: "s-1", Workspace: "/w", Model: "MiniMax-M3",
		State: SessionStateIdle, SandboxMode: "workspace-write",
		ContextWindow: 1000000, CreatedAt: at,
	})
}

func TestGoldenApprovalRequest(t *testing.T) {
	golden(t, "approval_request", ApprovalRequest{
		ApprovalID: "a-1", TurnID: "t-1", ToolCallID: "c-1",
		Tool: "bash", Command: "rm -rf build",
		BoundaryCrossed: "workspace-write", Reason: "matches a confirm rule",
		Rule: "rm -rf*", ExpiresAt: at,
	})
}

func TestGoldenTurnCompleted(t *testing.T) {
	golden(t, "turn_completed", TurnCompleted{
		TurnID: "t-1", Reason: StopDone,
		Usage: &Usage{InputTokens: 100, OutputTokens: 20, CacheReadTokens: 80},
		Completion: &Completion{
			Verification: "passed", Met: []string{"tests"},
			TouchedProtected: []string{"a_test.go"},
		},
	})
}

// The absent halves matter as much as the present ones: `omitempty` on Usage
// and Completion is what lets a client tell "no tokens" from "unknown", and
// dropping it would be a silent break.
func TestGoldenTurnCompletedMinimal(t *testing.T) {
	golden(t, "turn_completed_minimal", TurnCompleted{TurnID: "t-1", Reason: StopInterrupted})
}

// moved, stall and stall_limit are never omitted: zero stalls is an answer, and
// a client that read a missing count as zero would be guessing.
func TestGoldenDoneChecked(t *testing.T) {
	golden(t, "done_checked", DoneChecked{
		TurnID: "t1", Cycle: 3,
		Met: []string{"lint", "unit"}, Unmet: []string{"e2e"}, Unavailable: []string{"a11y"},
		Moved: DoneMovedBackward, Stall: 1, StallLimit: 3,
		Undone: []string{"src/cart.ts"}, TouchedProtected: []string{"cart_test.go"},
	})
	golden(t, "done_checked_minimal", DoneChecked{
		TurnID: "t1", Cycle: 1, Met: []string{"unit"}, Moved: DoneMovedFirst, StallLimit: 3,
	})
}

func TestGoldenError(t *testing.T) {
	golden(t, "error", Error{Code: CodeWorkspaceInvalid, Message: "not a directory"})
}

func TestGoldenCreateSessionRequest(t *testing.T) {
	golden(t, "create_session_request", CreateSessionRequest{
		Workspace: "/w", Model: "MiniMax-M3", SandboxMode: "read-only",
	})
}

// A measured choice and one that is not, side by side. `measured` is never
// omitted — false is the answer a menu exists to show — while an endpoint, a
// window and a notice are, when there is none.
func TestGoldenModelsResponse(t *testing.T) {
	golden(t, "models_response", ModelsResponse{
		Default: ModelChoice{
			Name: "MiniMax-M3", Model: "MiniMax-M3", Family: "minimax-m3",
			Transport: "openai", Window: 1000000, Measured: true,
		},
		Profiles: []ModelChoice{{
			Name: "local", Model: "qwen3.5-9b", Family: "generic", Transport: "openai",
			BaseURL: "http://127.0.0.1:1234/v1", Window: 32000,
			Notice: "using --family generic: nobody measured this endpoint",
		}},
	})
}

// A memory with its provenance and one written by hand, a stale one, and a
// block that is not a memory. `stale` and `shown` are never omitted — false is
// what a list of memories exists to show — while a body and a provenance are,
// when there is none.
func TestGoldenMemoryResponse(t *testing.T) {
	golden(t, "memory_response", MemoryResponse{
		Path: ".dcode/memory.md", Exists: true, Enabled: true, MaxEntries: 40,
		Entries: []MemoryEntry{
			{Kind: "gotcha", Subject: "make test needs go generate first",
				Body: "the generated files go stale.", Learned: "2026-08-18", Commit: "abc1234", Shown: true},
			{Kind: "convention", Subject: "errors carry the path", Commit: "0ddba11", Stale: true, Shown: true},
		},
		Malformed: []MemoryMalformed{{Line: "## diary: what I did today",
			Reason: `"diary" is not a kind of memory; the kinds are gotcha, decision and convention`}},
	})
}

// A loaded skill and a held one, and a file that did not load. `held` is never
// omitted; triggers and claims are, when there are none.
func TestGoldenSkillsResponse(t *testing.T) {
	golden(t, "skills_response", SkillsResponse{
		Enabled: true,
		Skills: []SkillInfo{
			{Name: "release", WhenToUse: "cutting a release: version, changelog, tag",
				Triggers: []string{"release"}, Source: SkillSourceProject, Path: "release/SKILL.md"},
			{Name: "yolo", WhenToUse: "moving fast", Source: SkillSourceUser, Path: "yolo.md",
				Held: true, Claims: []string{"claims the sandbox is off (sandbox is disabled)"}},
		},
		Notices: []SkillNotice{{Source: SkillSourceProject, Path: "draft.md",
			Reason: "has no `when_to_use` line"}},
	})
}

// A user command and a project one that took the name from the user's. The
// body travels, and `description` is never omitted: an empty one is what a
// client shows as a command with nothing said about it.
func TestGoldenCommandsResponse(t *testing.T) {
	golden(t, "commands_response", CommandsResponse{
		Commands: []CommandInfo{
			{Name: "fix-issue", Description: "", Source: CommandSourceUser, Path: "fix-issue.md",
				Body: "Fix the issue described here: $ARGUMENTS"},
			{Name: "review", Description: "review the staged change", Source: CommandSourceProject,
				Path: "review.md", Body: "Review the staged change against AGENTS.md."},
		},
		Overrides: []string{"review"},
	})
}

// A repository in a linked worktree with two changed files, one of them
// binary. `repository`, `detached`, `worktree`, `binary` and `truncated` are
// never omitted — false is what a client reads them for.
func TestGoldenGitResponse(t *testing.T) {
	golden(t, "git_response", GitResponse{
		Repository: true, Root: "/src/dcode-side", Branch: "side", Head: "6ec45f1",
		Worktree: true, MainCheckout: "/src/dcode",
		Files: []GitFile{
			{Path: "internal/vcs/tree.go", Status: GitModified, Added: 12, Removed: 3},
			{Path: "logo.png", Status: GitUntracked, Binary: true},
		},
	})
}

// One file's diff, cut at the cap.
func TestGoldenGitDiffResponse(t *testing.T) {
	golden(t, "git_diff_response", GitDiffResponse{
		Path: "a.txt", Status: GitModified, Diff: "--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-two\n+TWO\n",
		Truncated: true, Limit: GitDiffLimit,
	})
}

// A branch's PR, its checks summed up. `draft` and every count are never
// omitted — zero is an answer.
func TestGoldenGitPRResponse(t *testing.T) {
	golden(t, "git_pr_response", GitPRResponse{
		Branch: "feat/protocol-git-pr", Base: "main",
		PR: &GitPR{
			Number: 481, URL: "https://github.com/aguinelo/claudecodeclone/pull/481", Title: "feat(protocol): the daemon opens a branch's PR",
			State: GitPROpen, Base: "main", Head: "feat/protocol-git-pr",
			Checks: GitChecks{State: GitChecksPending, Passed: 4, Pending: 1, Skipped: 1},
		},
	})
}

// A branch with no PR: no `pr` at all, rather than one full of zeros.
func TestGoldenGitPRResponseWithoutAPR(t *testing.T) {
	golden(t, "git_pr_response_none", GitPRResponse{Branch: "side", Base: "main"})
}

func TestGoldenCreateGitPRRequest(t *testing.T) {
	golden(t, "create_git_pr_request", CreateGitPRRequest{
		Workspace: "/src/dcode", Title: "Add b", Body: "It adds b.\n", Base: "main", Draft: true,
	})
}
