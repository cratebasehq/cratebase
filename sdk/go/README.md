# cratebase-go

Official Go client SDK for [Cratebase](../../README.md). Standard library
only — no third-party HTTP client dependency.

```bash
go get github.com/cratebase/cratebase-go
```

```go
package main

import (
	"fmt"
	"log"

	"github.com/cratebase/cratebase-go"
)

func main() {
	cb := cratebase.NewClient("http://127.0.0.1:8090", nil)

	// auth
	if _, err := cb.Collection("users").AuthWithPassword("alice@example.com", "secret123"); err != nil {
		log.Fatal(err)
	}

	// records
	page, err := cb.Collection("posts").GetList(1, 20, cratebase.ListOptions{
		Filter: "published = true",
		Sort:   "-created",
	})
	if err != nil {
		log.Fatal(err)
	}
	for _, post := range page.Items {
		fmt.Println(post.ID(), post.GetString("title"))
	}

	post, err := cb.Collection("posts").Create(map[string]any{"title": "Hello", "published": true})
	if err != nil {
		log.Fatal(err)
	}

	// files: pass a *cratebase.MultipartForm for collections with file fields
	form := cratebase.NewMultipartForm().
		Set("title", "Report").
		SetFile("attachment", "report.pdf", reportFile)
	doc, err := cb.Collection("docs").Create(form)
	if err != nil {
		log.Fatal(err)
	}
	url := cb.FileURL(doc, doc.GetString("attachment"))

	// realtime
	unsubscribe, err := cb.Realtime.Subscribe("posts", func(e cratebase.Event) {
		fmt.Println(e.Action, e.Record.ID())
	})
	if err != nil {
		log.Fatal(err)
	}
	defer unsubscribe()

	_ = post
	_ = url
}
```

`Client` is safe to share across goroutines: `AuthStore` is read on every
request, so logging in through one goroutine updates every other caller
sharing the same client.

`RecordModel` is a `map[string]any` with typed accessors (`ID()`,
`Created()`, `Updated()`, `CollectionID()`, `CollectionName()`, and for
auth records `Email()`/`Verified()`) plus `Get`/`GetString`/`GetBool`/
`GetFloat64` for your collection's custom schema fields.

See the [API reference](../../openapi.yaml) for the full request/response
shapes this client wraps.
