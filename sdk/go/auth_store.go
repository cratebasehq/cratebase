package cratebase

import (
	"encoding/base64"
	"encoding/json"
	"strings"
	"sync"
	"time"
)

// AuthChangeListener is invoked whenever an AuthStore's token/model change.
type AuthChangeListener func(token string, model map[string]any)

type authListenerEntry struct {
	id       uint64
	listener AuthChangeListener
}

// AuthStore holds the current auth token and record/admin in memory for
// the process lifetime, and is safe for concurrent use. Share one
// instance across goroutines via a Client; logging in through one caller
// updates every other caller reading the same AuthStore. Bring your own
// persistence (e.g. write Token()/Model() to a file or secret store) by
// registering an OnChange listener.
type AuthStore struct {
	mu        sync.RWMutex
	token     string
	model     map[string]any
	listeners []authListenerEntry
	nextID    uint64
}

// NewAuthStore returns an empty, logged-out AuthStore.
func NewAuthStore() *AuthStore {
	return &AuthStore{}
}

// Token returns the current auth token, or "" if logged out.
func (s *AuthStore) Token() string {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return s.token
}

// Model returns the current auth record/admin, or nil if logged out.
func (s *AuthStore) Model() map[string]any {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return s.model
}

// IsValid reports whether there's a token and, if it's a JWT with an exp
// claim, that it hasn't expired yet.
func (s *AuthStore) IsValid() bool {
	s.mu.RLock()
	token := s.token
	s.mu.RUnlock()
	if token == "" {
		return false
	}
	exp, ok := decodeJWTExp(token)
	if !ok {
		return true
	}
	return exp.After(time.Now())
}

// Save stores a new token/model and notifies every OnChange listener.
func (s *AuthStore) Save(token string, model map[string]any) {
	s.mu.Lock()
	s.token = token
	s.model = model
	listeners := make([]authListenerEntry, len(s.listeners))
	copy(listeners, s.listeners)
	s.mu.Unlock()

	for _, entry := range listeners {
		entry.listener(token, model)
	}
}

// Clear logs out, equivalent to Save("", nil).
func (s *AuthStore) Clear() {
	s.Save("", nil)
}

// OnChange registers a listener called after every Save/Clear. It returns
// an unsubscribe func.
func (s *AuthStore) OnChange(listener AuthChangeListener) func() {
	s.mu.Lock()
	s.nextID++
	id := s.nextID
	s.listeners = append(s.listeners, authListenerEntry{id: id, listener: listener})
	s.mu.Unlock()

	return func() {
		s.mu.Lock()
		defer s.mu.Unlock()
		for i, entry := range s.listeners {
			if entry.id == id {
				s.listeners = append(s.listeners[:i], s.listeners[i+1:]...)
				break
			}
		}
	}
}

func decodeJWTExp(token string) (time.Time, bool) {
	parts := strings.Split(token, ".")
	if len(parts) != 3 {
		return time.Time{}, false
	}
	payload, err := base64.RawURLEncoding.DecodeString(parts[1])
	if err != nil {
		return time.Time{}, false
	}
	var claims struct {
		Exp int64 `json:"exp"`
	}
	if err := json.Unmarshal(payload, &claims); err != nil || claims.Exp == 0 {
		return time.Time{}, false
	}
	return time.Unix(claims.Exp, 0), true
}
