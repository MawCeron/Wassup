package main

import (
	"context"
	"os"
	"testing"
	"time"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
)

func TestWithExpiration(t *testing.T) {
	text := withExpiration(&waE2E.Message{Conversation: proto.String("hi")}, 86400)
	if text.GetConversation() != "" || text.GetExtendedTextMessage().GetText() != "hi" || expirationOf(text) != 86400 {
		t.Fatalf("text: %v", text)
	}
	photo := withExpiration(&waE2E.Message{ImageMessage: &waE2E.ImageMessage{Caption: proto.String("c")}}, 604800)
	if expirationOf(photo) != 604800 || photo.GetImageMessage().GetCaption() != "c" {
		t.Fatalf("photo: %v", photo)
	}
}

func TestDisappearingMessages(t *testing.T) {
	ctx := context.Background()
	accounts, err := openAccounts(ctx, t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer closeAccounts(accounts)
	a := accounts[0]
	a.mediaDir = t.TempDir()
	d := newDaemon(accounts, nil)
	chat := types.NewJID("5215550000000", types.DefaultUserServer)
	sent := time.Unix(1_700_000_000, 0)
	save := func(id string, m *waE2E.Message) {
		t.Helper()
		d.onMessage(a, &events.Message{
			Info:    types.MessageInfo{MessageSource: types.MessageSource{Chat: chat, Sender: chat}, ID: id, Timestamp: sent},
			Message: m,
		})
	}
	// The chat turns on a 24 h timer, then two messages arrive with it.
	save("setting", &waE2E.Message{ProtocolMessage: &waE2E.ProtocolMessage{
		Type: waE2E.ProtocolMessage_EPHEMERAL_SETTING.Enum(), EphemeralExpiration: proto.Uint32(86400),
	}})
	day := &waE2E.ContextInfo{Expiration: proto.Uint32(86400)}
	save("text", &waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{Text: proto.String("bye"), ContextInfo: day}})
	photo := &waE2E.Message{ImageMessage: &waE2E.ImageMessage{FileSHA256: []byte{1}, Mimetype: proto.String("image/jpeg"), ContextInfo: day}}
	save("photo", photo)
	sticker := &waE2E.Message{StickerMessage: &waE2E.StickerMessage{FileSHA256: []byte{2}, ContextInfo: day}}
	save("sticker", sticker)
	save("plain", &waE2E.Message{Conversation: proto.String("stays")})

	if got := a.disappearing(ctx, chat.String()); got != 86400 {
		t.Fatalf("chat timer %d, want 86400", got)
	}
	photoFile, stickerFile := a.mediaPath(mediaOf(photo)), a.mediaPath(mediaOf(sticker))
	os.WriteFile(photoFile, []byte("x"), 0o600)
	os.WriteFile(stickerFile, []byte("x"), 0o600)

	if got, _ := a.expire(ctx, sent.Add(23*time.Hour)); len(got) != 0 {
		t.Fatalf("expired early: %v", got)
	}
	got, err := a.expire(ctx, sent.Add(25*time.Hour))
	if err != nil || len(got[chat.String()]) != 3 {
		t.Fatalf("expired %v, %v; want text, photo and sticker", got, err)
	}
	msgs, _ := a.listMessages(ctx, chat.String(), "", "", 0)
	if len(msgs) != 1 || msgs[0].Id != "plain" {
		t.Fatalf("left %v; want only the message without a timer", msgs)
	}
	if _, err := os.Stat(photoFile); !os.IsNotExist(err) {
		t.Fatal("the expired photo stayed in the cache")
	}
	if _, err := os.Stat(stickerFile); err != nil {
		t.Fatal("a shared sticker file was deleted")
	}
}
