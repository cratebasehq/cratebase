package cratebase

import (
	"io"
	"mime/multipart"
)

// MultipartForm builds a multipart/form-data request body for
// RecordService.Create/Update calls against collections with file
// fields, mirroring passing a FormData to the JS SDK.
type MultipartForm struct {
	fields []multipartField
}

type multipartField struct {
	name     string
	value    string
	isFile   bool
	filename string
	reader   io.Reader
}

// NewMultipartForm returns an empty form.
func NewMultipartForm() *MultipartForm {
	return &MultipartForm{}
}

// Set adds a plain text field.
func (f *MultipartForm) Set(name, value string) *MultipartForm {
	f.fields = append(f.fields, multipartField{name: name, value: value})
	return f
}

// SetFile attaches a file field, reading its content from reader. If
// reader also implements io.Closer, it's closed once written.
func (f *MultipartForm) SetFile(name, filename string, reader io.Reader) *MultipartForm {
	f.fields = append(f.fields, multipartField{name: name, isFile: true, filename: filename, reader: reader})
	return f
}

func (f *MultipartForm) write(w *multipart.Writer) error {
	for _, field := range f.fields {
		if field.isFile {
			part, err := w.CreateFormFile(field.name, field.filename)
			if err != nil {
				return err
			}
			if _, err := io.Copy(part, field.reader); err != nil {
				return err
			}
			if closer, ok := field.reader.(io.Closer); ok {
				closer.Close()
			}
			continue
		}
		if err := w.WriteField(field.name, field.value); err != nil {
			return err
		}
	}
	return w.Close()
}
