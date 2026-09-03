package cratebase

// SchemaService manages collection schemas. Requires a superuser token in
// Client.AuthStore.
type SchemaService struct {
	client *Client
}

// GetList lists every collection.
func (s *SchemaService) GetList() ([]CollectionModel, error) {
	var result []CollectionModel
	err := s.client.Send("/api/collections", SendOptions{Method: "GET"}, &result)
	return result, err
}

// GetOne fetches a single collection by id or name.
func (s *SchemaService) GetOne(idOrName string) (*CollectionModel, error) {
	var result CollectionModel
	err := s.client.Send("/api/collections/"+idOrName, SendOptions{Method: "GET"}, &result)
	if err != nil {
		return nil, err
	}
	return &result, nil
}

// Create creates a new collection.
func (s *SchemaService) Create(data CollectionModel) (*CollectionModel, error) {
	var result CollectionModel
	err := s.client.Send("/api/collections", SendOptions{Method: "POST", Body: data}, &result)
	if err != nil {
		return nil, err
	}
	return &result, nil
}

// Update updates a collection's schema/rules by id or name.
func (s *SchemaService) Update(idOrName string, data CollectionModel) (*CollectionModel, error) {
	var result CollectionModel
	err := s.client.Send("/api/collections/"+idOrName, SendOptions{Method: "PATCH", Body: data}, &result)
	if err != nil {
		return nil, err
	}
	return &result, nil
}

// Delete deletes a collection by id or name.
func (s *SchemaService) Delete(idOrName string) error {
	return s.client.Send("/api/collections/"+idOrName, SendOptions{Method: "DELETE"}, nil)
}
