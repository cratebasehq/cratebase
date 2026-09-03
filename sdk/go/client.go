// Package cratebase is the official Go client SDK for Cratebase
// (https://github.com/cratebase/cratebase). It uses only the standard
// library — no third-party HTTP client dependency.
package cratebase

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"mime/multipart"
	"net/http"
	"net/url"
	"strings"
)

// SendOptions configures a low-level Client.Send call.
type SendOptions struct {
	// Method defaults to GET when empty.
	Method string
	// Body is either a *MultipartForm (encoded as multipart/form-data) or
	// any other JSON-marshalable value (map[string]any, a struct, ...),
	// which is sent as an application/json body. Leave nil for no body.
	Body any
	// Headers are merged into the request, overriding any default header
	// with the same (case-insensitive) name.
	Headers map[string]string
	// Query values are set on the request URL; empty strings are omitted
	// so callers can pass optional filters directly.
	Query map[string]string
}

// Client is the Cratebase API client. One instance per backend URL; safe
// to share across goroutines. Records/collections/auth all read AuthStore
// lazily on every request, so logging in updates every in-flight caller.
//
//	cb := cratebase.NewClient("https://api.example.com", nil)
//	_, err := cb.Collection("users").AuthWithPassword("a@b.com", "secret")
//	page, err := cb.Collection("posts").GetList(1, 20, cratebase.ListOptions{Filter: "published = true"})
type Client struct {
	// BaseURL is the backend origin, with no trailing slash.
	BaseURL string
	// AuthStore holds the current session token/model.
	AuthStore *AuthStore
	// HTTPClient performs requests; defaults to http.DefaultClient.
	HTTPClient *http.Client

	Admins       *AdminService
	Collections  *SchemaService
	Realtime     *RealtimeService
	FeatureFlags *FeatureFlagsService
	Queue        *QueueService
}

// NewClient creates a Client for baseUrl (e.g. "http://127.0.0.1:8090").
// Pass nil for authStore to get a fresh in-memory one.
func NewClient(baseUrl string, authStore *AuthStore) *Client {
	if authStore == nil {
		authStore = NewAuthStore()
	}
	c := &Client{
		BaseURL:    strings.TrimRight(baseUrl, "/"),
		AuthStore:  authStore,
		HTTPClient: http.DefaultClient,
	}
	c.Admins = &AdminService{client: c}
	c.Collections = &SchemaService{client: c}
	c.Realtime = newRealtimeService(c)
	c.FeatureFlags = &FeatureFlagsService{client: c}
	c.Queue = &QueueService{client: c}
	return c
}

// Collection returns a RecordService bound to one collection (by id or
// name).
func (c *Client) Collection(idOrName string) *RecordService {
	return &RecordService{client: c, collectionIdOrName: idOrName}
}

// FileURL returns the download URL for a file field's stored filename.
func (c *Client) FileURL(record RecordModel, filename string) string {
	collection := record.CollectionName()
	if collection == "" {
		collection = record.CollectionID()
	}
	return fmt.Sprintf("%s/api/files/%s/%s/%s", c.BaseURL, collection, record.ID(), filename)
}

// Send is the low-level request helper every service is built on. It
// returns a *ClientResponseError for non-2xx responses. Pass a non-nil
// pointer in out to decode the JSON response body into it; nil skips
// decoding (used for endpoints that return 204 No Content).
func (c *Client) Send(path string, opts SendOptions, out any) error {
	method := opts.Method
	if method == "" {
		method = http.MethodGet
	}

	u, err := url.Parse(c.BaseURL + path)
	if err != nil {
		return fmt.Errorf("cratebase: invalid path %q: %w", path, err)
	}
	q := u.Query()
	for k, v := range opts.Query {
		if v != "" {
			q.Set(k, v)
		}
	}
	u.RawQuery = q.Encode()

	headers := make(map[string]string, len(opts.Headers)+1)
	for k, v := range opts.Headers {
		headers[k] = v
	}

	var body io.Reader
	switch b := opts.Body.(type) {
	case nil:
		// no body
	case *MultipartForm:
		buf := &bytes.Buffer{}
		w := multipart.NewWriter(buf)
		if err := b.write(w); err != nil {
			return fmt.Errorf("cratebase: encoding multipart body: %w", err)
		}
		headers["Content-Type"] = w.FormDataContentType()
		body = buf
	default:
		encoded, err := json.Marshal(opts.Body)
		if err != nil {
			return fmt.Errorf("cratebase: encoding request body: %w", err)
		}
		headers["Content-Type"] = "application/json"
		body = bytes.NewReader(encoded)
	}

	req, err := http.NewRequest(method, u.String(), body)
	if err != nil {
		return fmt.Errorf("cratebase: building request: %w", err)
	}
	for k, v := range headers {
		req.Header.Set(k, v)
	}
	if token := c.AuthStore.Token(); token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}

	httpClient := c.HTTPClient
	if httpClient == nil {
		httpClient = http.DefaultClient
	}
	resp, err := httpClient.Do(req)
	if err != nil {
		return fmt.Errorf("cratebase: request to %s failed: %w", u.String(), err)
	}
	defer resp.Body.Close()

	if resp.StatusCode == http.StatusNoContent {
		return nil
	}

	data, err := io.ReadAll(resp.Body)
	if err != nil {
		return fmt.Errorf("cratebase: reading response from %s: %w", u.String(), err)
	}

	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		var errBody ApiErrorBody
		_ = json.Unmarshal(data, &errBody)
		return &ClientResponseError{
			URL:     u.String(),
			Status:  resp.StatusCode,
			Message: errBody.Message,
			Data:    errBody.Data,
		}
	}

	if out == nil || len(data) == 0 {
		return nil
	}
	if err := json.Unmarshal(data, out); err != nil {
		return fmt.Errorf("cratebase: decoding response from %s: %w", u.String(), err)
	}
	return nil
}
