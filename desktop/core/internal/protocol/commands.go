package protocol

// Where a user command comes from: the user's config root, or the project's
// .dcode. A project's command wins over the user's of the same name, as the
// TUI resolves them (config.DiscoverCommands).
const (
	CommandSourceUser    = "user"
	CommandSourceProject = "project"
)

// CommandInfo is one user-defined slash command available in the workspace.
//
// The body is carried, unlike a skill's: a command is a prompt the person
// invokes, and invoking it is sending that text. A client expands it the way
// config.Expand does — $ARGUMENTS replaced by what followed the name, or what
// followed appended after a blank line — and sends the result as a message.
// A command never runs anything, so carrying its text gives a client nothing a
// person could not type.
type CommandInfo struct {
	Name        string `json:"name"`
	Description string `json:"description"`
	// Source is CommandSourceUser or CommandSourceProject.
	Source string `json:"source"`
	// Path is the command's file, relative to its source's commands directory.
	Path string `json:"path"`
	Body string `json:"body"`
}

// CommandsResponse answers GET /commands: the user-defined commands a client
// in the workspace offers, sorted by name, and every project command that
// overrode a user one of the same name. No command is an empty list, never
// null, and so is no override.
//
// Which names the client itself answers (its built-ins) is the client's
// decision, not the daemon's: a built-in of the same name wins there, and the
// list does not guess which those are.
type CommandsResponse struct {
	Commands []CommandInfo `json:"commands"`
	// Overrides names each command the project's file took from the user's,
	// by name, so the override is visible and not debugged as the wrong file.
	Overrides []string `json:"overrides"`
}
