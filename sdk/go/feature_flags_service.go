package cratebase

// FeatureFlagsService is a client for the `feature-flags` plugin
// (crates/server/src/plugins/feature_flags.rs). An unknown key resolves
// to false rather than erroring.
type FeatureFlagsService struct {
	client *Client
}

type checkFlagResponse struct {
	Key     string `json:"key"`
	Enabled bool   `json:"enabled"`
}

// IsEnabled reports whether the flag named key is enabled.
func (s *FeatureFlagsService) IsEnabled(key string) (bool, error) {
	var result checkFlagResponse
	err := s.client.Send("/api/plugins/feature-flags/"+key, SendOptions{Method: "GET"}, &result)
	if err != nil {
		return false, err
	}
	return result.Enabled, nil
}
