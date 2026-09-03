package cratebase

import "fmt"

// FieldError is a single field's validation failure: a stable
// machine-readable Code (e.g. "value_too_short") alongside a
// human-readable Message.
type FieldError struct {
	Code    string `json:"code"`
	Message string `json:"message"`
}

// ApiErrorBody is the shape of every non-2xx JSON response from the
// Cratebase API.
type ApiErrorBody struct {
	Code    int                   `json:"code"`
	Message string                `json:"message"`
	Data    map[string]FieldError `json:"data"`
}

// ClientResponseError is returned for any non-2xx response. Data carries
// per-field validation errors when Status == 400.
type ClientResponseError struct {
	URL     string
	Status  int
	Message string
	Data    map[string]FieldError
}

// Error implements the error interface.
func (e *ClientResponseError) Error() string {
	if e.Message != "" {
		return e.Message
	}
	return fmt.Sprintf("request to %s failed with status %d", e.URL, e.Status)
}
