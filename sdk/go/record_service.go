package cratebase

import "strconv"

// RecordService provides CRUD + auth operations scoped to one collection.
// Auth-only methods (AuthWithPassword, AuthRefresh, ...) only make sense
// for auth-typed collections but are exposed unconditionally since the
// client has no way to know a collection's type ahead of a request.
type RecordService struct {
	client             *Client
	collectionIdOrName string
}

func (s *RecordService) basePath() string {
	return "/api/collections/" + s.collectionIdOrName + "/records"
}

func (s *RecordService) authPath(action string) string {
	return "/api/collections/" + s.collectionIdOrName + "/" + action
}

// GetList fetches one page of records. page defaults to 1, perPage to 30
// when 0.
func (s *RecordService) GetList(page, perPage int, options ListOptions) (*ListResult[RecordModel], error) {
	if page == 0 {
		page = 1
	}
	if perPage == 0 {
		perPage = 30
	}
	var result ListResult[RecordModel]
	err := s.client.Send(s.basePath(), SendOptions{
		Method: "GET",
		Query: map[string]string{
			"page":    strconv.Itoa(page),
			"perPage": strconv.Itoa(perPage),
			"filter":  options.Filter,
			"sort":    options.Sort,
		},
	}, &result)
	if err != nil {
		return nil, err
	}
	return &result, nil
}

// GetFullList fetches every page and concatenates the results. Convenient
// for small collections; prefer GetList with pagination for large ones.
// batchSize defaults to 200 when 0.
func (s *RecordService) GetFullList(options ListOptions, batchSize int) ([]RecordModel, error) {
	if batchSize == 0 {
		batchSize = 200
	}
	var items []RecordModel
	page := 1
	for {
		result, err := s.GetList(page, batchSize, options)
		if err != nil {
			return nil, err
		}
		items = append(items, result.Items...)
		if page >= result.TotalPages {
			break
		}
		page++
	}
	return items, nil
}

// GetOne fetches a single record by id.
func (s *RecordService) GetOne(id string) (RecordModel, error) {
	var record RecordModel
	err := s.client.Send(s.basePath()+"/"+id, SendOptions{Method: "GET"}, &record)
	return record, err
}

// Create creates a record. Pass a map[string]any/struct for a JSON body,
// or a *MultipartForm for collections with file fields.
func (s *RecordService) Create(data any) (RecordModel, error) {
	var record RecordModel
	err := s.client.Send(s.basePath(), SendOptions{Method: "POST", Body: data}, &record)
	return record, err
}

// Update partially updates a record. Pass a map[string]any/struct for a
// JSON body, or a *MultipartForm for collections with file fields.
func (s *RecordService) Update(id string, data any) (RecordModel, error) {
	var record RecordModel
	err := s.client.Send(s.basePath()+"/"+id, SendOptions{Method: "PATCH", Body: data}, &record)
	return record, err
}

// Delete deletes a record by id.
func (s *RecordService) Delete(id string) error {
	return s.client.Send(s.basePath()+"/"+id, SendOptions{Method: "DELETE"}, nil)
}

// AuthWithPassword logs in with identity (whatever the collection's
// configured identity field is — email by default) and password.
func (s *RecordService) AuthWithPassword(identity, password string) (*AuthResponse[RecordModel], error) {
	var result AuthResponse[RecordModel]
	err := s.client.Send(s.authPath("auth-with-password"), SendOptions{
		Method: "POST",
		Body:   map[string]any{"identity": identity, "password": password},
	}, &result)
	if err != nil {
		return nil, err
	}
	s.client.AuthStore.Save(result.Token, result.Record)
	return &result, nil
}

// AuthRefresh renews the current session, returning a fresh token/record.
func (s *RecordService) AuthRefresh() (*AuthResponse[RecordModel], error) {
	var result AuthResponse[RecordModel]
	err := s.client.Send(s.authPath("auth-refresh"), SendOptions{Method: "POST"}, &result)
	if err != nil {
		return nil, err
	}
	s.client.AuthStore.Save(result.Token, result.Record)
	return &result, nil
}

// ListAuthMethods lists available auth methods. Pass your app's OAuth2
// redirect URI/deep link and each provider's AuthURL comes back ready to
// open directly.
func (s *RecordService) ListAuthMethods(redirectUri string) (*AuthMethodsResponse, error) {
	var result AuthMethodsResponse
	err := s.client.Send(s.authPath("auth-methods"), SendOptions{
		Method: "GET",
		Query:  map[string]string{"redirectUri": redirectUri},
	}, &result)
	if err != nil {
		return nil, err
	}
	return &result, nil
}

// AuthWithOAuth2 completes an OAuth2 login: exchange the code your app
// received at redirectUri (the same one used to build the authUrl from
// ListAuthMethods) for a session.
func (s *RecordService) AuthWithOAuth2(provider, code, redirectUri string) (*AuthResponse[RecordModel], error) {
	var result AuthResponse[RecordModel]
	err := s.client.Send(s.authPath("auth-with-oauth2"), SendOptions{
		Method: "POST",
		Body:   map[string]any{"provider": provider, "code": code, "redirectUri": redirectUri},
	}, &result)
	if err != nil {
		return nil, err
	}
	s.client.AuthStore.Save(result.Token, result.Record)
	return &result, nil
}

// RequestVerification sends a verification email if email matches an
// account — always resolves regardless, so it can't be used to enumerate
// accounts.
func (s *RecordService) RequestVerification(email string) error {
	return s.client.Send(s.authPath("request-verification"), SendOptions{
		Method: "POST",
		Body:   map[string]any{"email": email},
	}, nil)
}

// ConfirmVerification confirms a verification token from the emailed link.
func (s *RecordService) ConfirmVerification(token string) error {
	return s.client.Send(s.authPath("confirm-verification"), SendOptions{
		Method: "POST",
		Body:   map[string]any{"token": token},
	}, nil)
}

// RequestPasswordReset sends a password reset email if email matches an
// account — always resolves regardless, so it can't be used to enumerate
// accounts.
func (s *RecordService) RequestPasswordReset(email string) error {
	return s.client.Send(s.authPath("request-password-reset"), SendOptions{
		Method: "POST",
		Body:   map[string]any{"email": email},
	}, nil)
}

// ConfirmPasswordReset confirms a password reset token and sets a new
// password.
func (s *RecordService) ConfirmPasswordReset(token, password, passwordConfirm string) error {
	return s.client.Send(s.authPath("confirm-password-reset"), SendOptions{
		Method: "POST",
		Body:   map[string]any{"token": token, "password": password, "passwordConfirm": passwordConfirm},
	}, nil)
}

// RequestEmailChange requires the current session's record to be
// authenticated. Sends a confirmation link to newEmail — the identity
// only changes once that link is confirmed, proving ownership of the new
// address.
func (s *RecordService) RequestEmailChange(newEmail string) error {
	return s.client.Send(s.authPath("request-email-change"), SendOptions{
		Method: "POST",
		Body:   map[string]any{"newEmail": newEmail},
	}, nil)
}

// ConfirmEmailChange confirms an email-change token from the emailed link.
func (s *RecordService) ConfirmEmailChange(token string) error {
	return s.client.Send(s.authPath("confirm-email-change"), SendOptions{
		Method: "POST",
		Body:   map[string]any{"token": token},
	}, nil)
}
