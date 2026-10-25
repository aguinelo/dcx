package protocol

// ModelChoice is one model a session can ask for, as a menu of them shows it:
// what a session asking for it would run, and whether this product's
// behavioural contracts were ever measured against its family.
//
// Never a credential. Every choice has a key behind it, and neither the key,
// nor its mask, nor its fingerprint, nor where it is kept is anything a list of
// choices has a reason to carry.
type ModelChoice struct {
	// Name is what to ask for: a profile's name, or, for the default, its
	// model.
	Name  string `json:"name"`
	Model string `json:"model"`
	// Family and Transport are the ones a session asking for Name would run
	// on: resolved, so an unset family reads as the one the model's prefix
	// picks, and an unset transport as the one that family prefers. Family is
	// empty only when no family claims the model.
	Family    string `json:"family"`
	Transport string `json:"transport"`
	// BaseURL is the endpoint, when one is configured. Empty is the
	// transport's own.
	BaseURL string `json:"base_url,omitempty"`
	// Window is the context window, in tokens, of a session asking for Name:
	// the configured one, else what the family reports.
	Window int `json:"window,omitempty"`
	// Measured says the family has measurements behind it.
	Measured bool `json:"measured"`
	// Notice is what to say when it has none: the family's own admission, the
	// text a session builds when it opens (provider.Unmeasured), or why no
	// session can be built with this choice at all. Empty when Measured.
	Notice string `json:"notice,omitempty"`
}

// ModelsResponse answers GET /models: what a session in the workspace gets
// when it asks for no model, and every profile it can ask for by name, sorted
// by name. No profiles is an empty list, never null.
type ModelsResponse struct {
	Default  ModelChoice   `json:"default"`
	Profiles []ModelChoice `json:"profiles"`
}
