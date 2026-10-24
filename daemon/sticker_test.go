package main

import (
	"context"
	"testing"
	"time"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
)

func TestListStickers(t *testing.T) {
	ctx := context.Background()
	accounts, err := openAccounts(ctx, t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer closeAccounts(accounts)
	a := accounts[0]
	chat := types.NewJID("5215550000000", types.DefaultUserServer)
	save := func(id string, sec int, m *waE2E.Message) {
		t.Helper()
		_, _, err := a.saveMessage(ctx, &events.Message{
			Info: types.MessageInfo{
				MessageSource: types.MessageSource{Chat: chat, Sender: chat},
				ID:            id,
				Timestamp:     time.Unix(int64(1_700_000_000+sec), 0),
			},
			Message: m,
		})
		if err != nil {
			t.Fatal(err)
		}
	}
	sticker := func(hash byte) *waE2E.Message {
		return &waE2E.Message{StickerMessage: &waE2E.StickerMessage{FileSHA256: []byte{hash}}}
	}
	save("old", 1, sticker(1))
	save("photo", 2, &waE2E.Message{ImageMessage: &waE2E.ImageMessage{FileSHA256: []byte{9}}})
	save("other", 3, sticker(2))
	save("again", 4, sticker(1)) // Same file as "old", sent again later.

	got, err := a.listStickers(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 2 || got[0].MessageId != "again" || got[1].MessageId != "other" {
		t.Fatalf("stickers %v; want again, other", got)
	}

	if _, _, err := a.sendSticker(ctx, chat.String(), chat.String(), "photo"); err == nil {
		t.Fatal("sent a photo as a sticker")
	}
}
