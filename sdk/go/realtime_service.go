package cratebase

import (
	"bufio"
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
	"sync"
)

// Event is a realtime record change notification.
type Event struct {
	Action string      `json:"action"` // "create" | "update" | "delete"
	Record RecordModel `json:"record"`
}

// Callback receives realtime events for a subscribed topic.
type Callback func(Event)

// RealtimeService provides live record change subscriptions over
// Server-Sent Events. Subscribe to a bare collection name for every
// change, or "collection/recordId" for just one record.
//
//	unsubscribe, err := client.Realtime.Subscribe("posts", func(e cratebase.Event) {
//		fmt.Println(e.Action, e.Record.ID())
//	})
//	// later
//	unsubscribe()
type RealtimeService struct {
	client *Client

	mu            sync.Mutex
	clientID      string
	connected     bool
	connCh        chan error // set while a connection attempt is in flight
	stop          chan struct{}
	subscriptions map[string][]subscriberEntry
	nextID        uint64
}

type subscriberEntry struct {
	id       uint64
	callback Callback
}

func newRealtimeService(client *Client) *RealtimeService {
	return &RealtimeService{
		client:        client,
		subscriptions: make(map[string][]subscriberEntry),
	}
}

// Subscribe registers callback for topic, opening the SSE connection if
// it isn't already open, and returns an unsubscribe func.
func (r *RealtimeService) Subscribe(topic string, callback Callback) (func(), error) {
	r.mu.Lock()
	r.nextID++
	id := r.nextID
	r.subscriptions[topic] = append(r.subscriptions[topic], subscriberEntry{id: id, callback: callback})
	r.mu.Unlock()

	if err := r.ensureConnected(); err != nil {
		return nil, err
	}
	if err := r.syncSubscriptions(); err != nil {
		return nil, err
	}

	return func() {
		r.removeSubscriber(topic, id)
	}, nil
}

func (r *RealtimeService) removeSubscriber(topic string, id uint64) {
	r.mu.Lock()
	entries := r.subscriptions[topic]
	for i, e := range entries {
		if e.id == id {
			entries = append(entries[:i], entries[i+1:]...)
			break
		}
	}
	if len(entries) == 0 {
		delete(r.subscriptions, topic)
	} else {
		r.subscriptions[topic] = entries
	}
	empty := len(r.subscriptions) == 0
	r.mu.Unlock()

	if empty {
		r.Disconnect()
	} else {
		_ = r.syncSubscriptions()
	}
}

// Unsubscribe removes every callback registered for topic and stops the
// stream if nothing remains subscribed.
func (r *RealtimeService) Unsubscribe(topic string) error {
	r.mu.Lock()
	delete(r.subscriptions, topic)
	empty := len(r.subscriptions) == 0
	r.mu.Unlock()

	if empty {
		r.Disconnect()
		return nil
	}
	return r.syncSubscriptions()
}

// Disconnect closes the SSE stream, if open.
func (r *RealtimeService) Disconnect() {
	r.mu.Lock()
	if r.stop != nil {
		close(r.stop)
		r.stop = nil
	}
	r.connected = false
	r.clientID = ""
	r.mu.Unlock()
}

func (r *RealtimeService) ensureConnected() error {
	r.mu.Lock()
	if r.connected {
		r.mu.Unlock()
		return nil
	}
	if r.connCh != nil {
		ch := r.connCh
		r.mu.Unlock()
		return <-ch
	}
	connCh := make(chan error, 1)
	stop := make(chan struct{})
	r.connCh = connCh
	r.stop = stop
	r.mu.Unlock()

	go r.readLoop(connCh, stop)

	return <-connCh
}

func (r *RealtimeService) readLoop(connCh chan error, stop chan struct{}) {
	req, err := http.NewRequest(http.MethodGet, r.client.BaseURL+"/api/realtime", nil)
	if err != nil {
		connCh <- err
		return
	}
	req.Header.Set("Accept", "text/event-stream")

	httpClient := r.client.HTTPClient
	if httpClient == nil {
		httpClient = http.DefaultClient
	}
	resp, err := httpClient.Do(req)
	if err != nil {
		connCh <- fmt.Errorf("cratebase: connecting to /api/realtime: %w", err)
		return
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		connCh <- fmt.Errorf("cratebase: /api/realtime returned status %d", resp.StatusCode)
		return
	}

	go func() {
		<-stop
		resp.Body.Close()
	}()

	scanner := bufio.NewScanner(resp.Body)
	scanner.Buffer(make([]byte, 0, 64*1024), 1024*1024)

	var eventName string
	var dataLines []string
	connected := false

	flush := func() {
		if len(dataLines) == 0 {
			eventName = ""
			return
		}
		data := strings.Join(dataLines, "\n")
		dataLines = nil
		name := eventName
		eventName = ""

		switch name {
		case "PB_CONNECT":
			var payload struct {
				ClientID string `json:"clientId"`
			}
			if err := json.Unmarshal([]byte(data), &payload); err == nil && payload.ClientID != "" {
				r.mu.Lock()
				r.clientID = payload.ClientID
				r.connected = true
				r.connCh = nil
				r.mu.Unlock()
				if !connected {
					connected = true
					connCh <- nil
				}
			}
		default: // "message" or unnamed
			var event Event
			if err := json.Unmarshal([]byte(data), &event); err != nil || event.Record == nil {
				return
			}
			r.dispatch(event)
		}
	}

	for scanner.Scan() {
		select {
		case <-stop:
			return
		default:
		}

		line := scanner.Text()
		switch {
		case line == "":
			flush()
		case strings.HasPrefix(line, "event:"):
			eventName = strings.TrimSpace(strings.TrimPrefix(line, "event:"))
		case strings.HasPrefix(line, "data:"):
			dataLines = append(dataLines, strings.TrimSpace(strings.TrimPrefix(line, "data:")))
		case strings.HasPrefix(line, ":"):
			// comment/keepalive, ignore
		}
	}

	if !connected {
		connCh <- fmt.Errorf("cratebase: /api/realtime stream closed before connecting")
	}

	r.mu.Lock()
	r.connected = false
	r.clientID = ""
	r.mu.Unlock()
}

func (r *RealtimeService) dispatch(event Event) {
	collectionName := event.Record.CollectionName()
	recordID := event.Record.ID()
	topics := []string{collectionName, collectionName + "/" + recordID}

	r.mu.Lock()
	var callbacks []Callback
	for _, topic := range topics {
		for _, e := range r.subscriptions[topic] {
			callbacks = append(callbacks, e.callback)
		}
	}
	r.mu.Unlock()

	for _, cb := range callbacks {
		cb(event)
	}
}

func (r *RealtimeService) syncSubscriptions() error {
	r.mu.Lock()
	clientID := r.clientID
	topics := make([]string, 0, len(r.subscriptions))
	for topic := range r.subscriptions {
		topics = append(topics, topic)
	}
	r.mu.Unlock()

	if clientID == "" {
		return nil
	}
	return r.client.Send("/api/realtime", SendOptions{
		Method: "POST",
		Body:   map[string]any{"clientId": clientID, "subscriptions": topics},
	}, nil)
}
