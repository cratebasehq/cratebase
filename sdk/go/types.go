package cratebase

// RecordModel represents a record from any collection. It behaves like a
// dynamic JSON object (map[string]any) plus typed accessors for the
// fields every record carries regardless of the owning collection's
// schema. Custom schema fields are reachable via Get/GetString/GetBool/
// GetFloat64 or by treating the value as a plain map.
type RecordModel map[string]any

func (r RecordModel) str(key string) string {
	if v, ok := r[key].(string); ok {
		return v
	}
	return ""
}

// ID returns the record's id.
func (r RecordModel) ID() string { return r.str("id") }

// Created returns the record's creation timestamp.
func (r RecordModel) Created() string { return r.str("created") }

// Updated returns the record's last-updated timestamp.
func (r RecordModel) Updated() string { return r.str("updated") }

// CollectionID returns the id of the collection this record belongs to.
func (r RecordModel) CollectionID() string { return r.str("collectionId") }

// CollectionName returns the name of the collection this record belongs to.
func (r RecordModel) CollectionName() string { return r.str("collectionName") }

// Email is only populated for records from an auth-typed collection.
func (r RecordModel) Email() string { return r.str("email") }

// Verified is only meaningful for records from an auth-typed collection.
func (r RecordModel) Verified() bool {
	v, _ := r["verified"].(bool)
	return v
}

// Get returns the raw value stored under key, or nil if absent.
func (r RecordModel) Get(key string) any { return r[key] }

// GetString is a typed convenience accessor for a custom schema field. It
// returns "" if key is absent or isn't a string.
func (r RecordModel) GetString(key string) string { return r.str(key) }

// GetBool is a typed convenience accessor for a custom schema field. It
// returns false if key is absent or isn't a bool.
func (r RecordModel) GetBool(key string) bool {
	v, _ := r[key].(bool)
	return v
}

// GetFloat64 is a typed convenience accessor for a custom schema field
// (JSON numbers decode as float64). It returns 0 if key is absent or
// isn't a number.
func (r RecordModel) GetFloat64(key string) float64 {
	v, _ := r[key].(float64)
	return v
}

// AuthRecord is a RecordModel returned from an auth-typed collection.
// It's an alias since the JSON shape is identical; Email()/Verified() are
// available on every RecordModel.
type AuthRecord = RecordModel

// AdminModel represents a superuser account.
type AdminModel map[string]any

func (a AdminModel) str(key string) string {
	if v, ok := a[key].(string); ok {
		return v
	}
	return ""
}

// ID returns the admin's id.
func (a AdminModel) ID() string { return a.str("id") }

// Email returns the admin's email.
func (a AdminModel) Email() string { return a.str("email") }

// Created returns the admin's creation timestamp.
func (a AdminModel) Created() string { return a.str("created") }

// Updated returns the admin's last-updated timestamp.
func (a AdminModel) Updated() string { return a.str("updated") }

// ListResult is a paginated collection listing.
type ListResult[T any] struct {
	Page       int `json:"page"`
	PerPage    int `json:"perPage"`
	TotalItems int `json:"totalItems"`
	TotalPages int `json:"totalPages"`
	Items      []T `json:"items"`
}

// FieldType enumerates the schema field kinds a collection can declare.
type FieldType string

const (
	FieldTypeText     FieldType = "text"
	FieldTypeEditor   FieldType = "editor"
	FieldTypeNumber   FieldType = "number"
	FieldTypeBool     FieldType = "bool"
	FieldTypeEmail    FieldType = "email"
	FieldTypeURL      FieldType = "url"
	FieldTypeDate     FieldType = "date"
	FieldTypeAutodate FieldType = "autodate"
	FieldTypeSelect   FieldType = "select"
	FieldTypeJSON     FieldType = "json"
	FieldTypeRelation FieldType = "relation"
	FieldTypeFile     FieldType = "file"
	FieldTypePassword FieldType = "password"
)

// FieldSchema describes one field of a collection's schema.
type FieldSchema struct {
	ID       string         `json:"id"`
	Name     string         `json:"name"`
	Type     FieldType      `json:"type"`
	Required bool           `json:"required,omitempty"`
	Unique   bool           `json:"unique,omitempty"`
	Options  map[string]any `json:"options,omitempty"`
}

// CollectionType is the kind of a collection.
type CollectionType string

const (
	CollectionTypeBase CollectionType = "base"
	CollectionTypeAuth CollectionType = "auth"
	CollectionTypeView CollectionType = "view"
)

// AuthOptions configures an auth-typed collection.
type AuthOptions struct {
	MinPasswordLength int `json:"minPasswordLength,omitempty"`
	// IdentityField is which schema field identifies an auth record for
	// login. Defaults to "email"; set to "username" (or any other field
	// name) to log in with something other than an email address.
	IdentityField            string `json:"identityField,omitempty"`
	RequireEmailVerification bool   `json:"requireEmailVerification,omitempty"`
	TokenTTLSeconds          int    `json:"tokenTtlSeconds,omitempty"`
}

// CollectionModel describes a collection's schema and access rules.
type CollectionModel struct {
	ID          string         `json:"id"`
	Name        string         `json:"name"`
	Type        CollectionType `json:"type"`
	Schema      []FieldSchema  `json:"schema"`
	ListRule    *string        `json:"listRule"`
	ViewRule    *string        `json:"viewRule"`
	CreateRule  *string        `json:"createRule"`
	UpdateRule  *string        `json:"updateRule"`
	DeleteRule  *string        `json:"deleteRule"`
	AuthOptions *AuthOptions   `json:"authOptions,omitempty"`
	Created     string         `json:"created"`
	Updated     string         `json:"updated"`
}

// ListOptions filters/sorts a GetList or GetFullList call.
type ListOptions struct {
	Filter string
	Sort   string
}

// AuthResponse is returned by every auth-with-* / auth-refresh call.
type AuthResponse[T any] struct {
	Token  string `json:"token"`
	Record T      `json:"record"`
}

// AdminAuthResponse is returned by AdminService auth calls.
type AdminAuthResponse struct {
	Token string     `json:"token"`
	Admin AdminModel `json:"admin"`
}

// OAuth2ProviderInfo is one configured OAuth2 provider, ready to redirect
// a user to AuthURL.
type OAuth2ProviderInfo struct {
	Name    string `json:"name"`
	AuthURL string `json:"authUrl"`
}

// AuthMethodsResponse describes which auth methods a collection supports.
type AuthMethodsResponse struct {
	Password bool `json:"password"`
	OAuth2   struct {
		Enabled   bool                 `json:"enabled"`
		Providers []OAuth2ProviderInfo `json:"providers"`
	} `json:"oauth2"`
}
