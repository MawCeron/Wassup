package main

import (
	"crypto/sha256"
	"encoding/base64"
	"testing"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"google.golang.org/protobuf/proto"
)

func TestFavoriteAction(t *testing.T) {
	hash := sha256.Sum256([]byte("a sticker"))
	sticker := &waE2E.StickerMessage{
		URL: proto.String("https://example.invalid/sticker"), DirectPath: proto.String("/v/sticker"),
		FileSHA256: hash[:], FileEncSHA256: []byte{1}, MediaKey: []byte{2}, Mimetype: proto.String("image/webp"),
		Width: proto.Uint32(512), Height: proto.Uint32(512), FileLength: proto.Uint64(1234),
	}
	id, action, err := favoriteAction(sticker, true)
	if err != nil {
		t.Fatal(err)
	}
	if got, err := favoriteHash(id); err != nil || string(got) != string(hash[:]) {
		t.Errorf("the id %q does not lead back to the file hash: %v", id, err)
	}
	if id != base64.StdEncoding.EncodeToString(hash[:]) {
		t.Errorf("id = %q", id)
	}
	if !action.GetIsFavorite() || action.GetDirectPath() != "/v/sticker" || action.GetWidth() != 512 || action.GetFileLength() != 1234 {
		t.Errorf("action = %v", action)
	}
	if _, removed, _ := favoriteAction(sticker, false); removed.GetIsFavorite() {
		t.Error("removing keeps isFavorite set")
	}
	if _, _, err := favoriteAction(&waE2E.StickerMessage{FileSHA256: []byte{1, 2}}, true); err == nil {
		t.Error("a short file hash was accepted")
	}
}
