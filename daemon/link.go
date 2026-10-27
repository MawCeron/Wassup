package main

import (
	"crypto/sha256"
	"encoding/hex"
	"os"
	"path/filepath"

	"github.com/MawCeron/Wassup/daemon/ipc"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"google.golang.org/protobuf/proto"
)

// linkOf returns the link preview a message carries, as the sender's phone
// made it, or nil. Only the preview's fields are kept, not the text.
func linkOf(m *waE2E.Message) *waE2E.ExtendedTextMessage {
	t := m.GetExtendedTextMessage()
	if t.GetTitle() == "" && len(t.GetJPEGThumbnail()) == 0 {
		return nil
	}
	return &waE2E.ExtendedTextMessage{
		MatchedText: proto.String(t.GetMatchedText()),
		Title:       proto.String(t.GetTitle()), Description: proto.String(t.GetDescription()),
		JPEGThumbnail: t.GetJPEGThumbnail(),
	}
}

// ipcLink describes a stored link preview for the UI; blob is the link column.
func (a *account) ipcLink(blob []byte) *ipc.LinkPreview {
	var t waE2E.ExtendedTextMessage
	if blob == nil || proto.Unmarshal(blob, &t) != nil {
		return nil
	}
	return &ipc.LinkPreview{
		Url: t.GetMatchedText(), Title: t.GetTitle(), Description: t.GetDescription(),
		ThumbnailPath: a.linkThumbnail(t.GetJPEGThumbnail()),
	}
}

// linkThumbPath is where a link preview's image is cached, by its hash.
func (a *account) linkThumbPath(thumb []byte) string {
	if len(thumb) == 0 || a.mediaDir == "" {
		return ""
	}
	sum := sha256.Sum256(thumb)
	return filepath.Join(a.mediaDir, "link-"+hex.EncodeToString(sum[:])+".jpg")
}

// linkThumbnail writes a preview's image to the cache once and returns its path.
func (a *account) linkThumbnail(thumb []byte) string {
	path := a.linkThumbPath(thumb)
	if path == "" {
		return ""
	}
	if _, err := os.Stat(path); err == nil {
		return path
	}
	tmp, err := os.CreateTemp(a.mediaDir, ".thumb-*")
	if err != nil {
		return ""
	}
	defer os.Remove(tmp.Name())
	_, err = tmp.Write(thumb)
	if closeErr := tmp.Close(); err != nil || closeErr != nil || os.Rename(tmp.Name(), path) != nil {
		return ""
	}
	return path
}
