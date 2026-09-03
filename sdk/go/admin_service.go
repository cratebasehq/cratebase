package cratebase

// AdminService manages superuser (admin panel) sessions.
type AdminService struct {
	client *Client
}

// AuthWithPassword logs a superuser in with email/password.
func (s *AdminService) AuthWithPassword(email, password string) (*AdminAuthResponse, error) {
	var result AdminAuthResponse
	err := s.client.Send("/api/admins/auth-with-password", SendOptions{
		Method: "POST",
		Body:   map[string]any{"email": email, "password": password},
	}, &result)
	if err != nil {
		return nil, err
	}
	s.client.AuthStore.Save(result.Token, result.Admin)
	return &result, nil
}

// AuthRefresh renews the current superuser session.
func (s *AdminService) AuthRefresh() (*AdminAuthResponse, error) {
	var result AdminAuthResponse
	err := s.client.Send("/api/admins/auth-refresh", SendOptions{Method: "POST"}, &result)
	if err != nil {
		return nil, err
	}
	s.client.AuthStore.Save(result.Token, result.Admin)
	return &result, nil
}
