package protocol

// Where a skill comes from: the user's config root, or the project's .dcode.
// A project's skill wins over the user's of the same name, as a session loads
// them.
const (
	SkillSourceUser    = "user"
	SkillSourceProject = "project"
)

// SkillInfo is one skill a session in the workspace has available, as its
// file declares it.
//
// Only what the skill declares about itself: never its body, which is loaded
// into a turn when a trigger fires and is no business of a list.
type SkillInfo struct {
	Name      string   `json:"name"`
	WhenToUse string   `json:"when_to_use"`
	Triggers  []string `json:"triggers,omitempty"`
	// Source is SkillSourceUser or SkillSourceProject.
	Source string `json:"source"`
	// Path is the skill's file, relative to its source's skills directory.
	Path string `json:"path"`
	// Held says the skill reaches for the boundary, and a session asks a
	// person before loading it. Claims is what it reaches for, in the words
	// the question would use.
	Held   bool     `json:"held"`
	Claims []string `json:"claims,omitempty"`
}

// SkillNotice is a skill file that was trimmed or not loaded, with why.
type SkillNotice struct {
	Source string `json:"source"`
	Path   string `json:"path"`
	Reason string `json:"reason"`
}

// SkillsResponse answers GET /skills: the skills a session in the workspace
// has available, sorted by name, and what was said while loading them. No
// skills is an empty list, never null, and so is no notice.
type SkillsResponse struct {
	// Enabled says sessions in the workspace index skills at all
	// (`behavior.skills_enabled`). Off, the skills are still listed.
	Enabled bool          `json:"enabled"`
	Skills  []SkillInfo   `json:"skills"`
	Notices []SkillNotice `json:"notices"`
}
