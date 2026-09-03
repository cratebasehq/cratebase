package cratebase

// QueueJob is a `_queue_jobs` record as returned by
// POST /api/plugins/queue/enqueue.
type QueueJob = RecordModel

// QueueService is a client for the `queue` plugin
// (crates/server/src/plugins/queue.rs): durable background job
// processing backed by a collection, not a separate broker. Requires an
// authenticated caller (superuser or an auth-collection record) —
// anonymous enqueueing is rejected server-side.
type QueueService struct {
	client *Client
}

// Enqueue submits a job to queue with the given payload and optional
// maxAttempts (0 uses the server default). The returned QueueJob exposes
// job-specific fields via Get/GetString/GetFloat64 (e.g.
// job.GetString("status"), job.GetFloat64("attempts")).
func (s *QueueService) Enqueue(queue string, payload any, maxAttempts int) (QueueJob, error) {
	body := map[string]any{"queue": queue, "payload": payload}
	if maxAttempts > 0 {
		body["maxAttempts"] = maxAttempts
	}
	var job QueueJob
	err := s.client.Send("/api/plugins/queue/enqueue", SendOptions{Method: "POST", Body: body}, &job)
	return job, err
}
