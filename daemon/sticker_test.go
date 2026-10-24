package main

import (
	"context"
	"testing"
	"time"

	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/proto/waSyncAction"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
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

	if _, _, err := a.sendSticker(ctx, chat.String(), chat.String(), "photo", ""); err == nil {
		t.Fatal("sent a photo as a sticker")
	}
}

func TestFavoriteStickers(t *testing.T) {
	ctx := context.Background()
	accounts, err := openAccounts(ctx, t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer closeAccounts(accounts)
	a := accounts[0]
	favorite := func(id string, at int64, on bool) {
		a.onAppState(ctx, []string{appstate.IndexFavoriteSticker, id}, &waSyncAction.SyncActionValue{
			Timestamp:     proto.Int64(at),
			StickerAction: &waSyncAction.StickerAction{IsFavorite: proto.Bool(on), DirectPath: proto.String("/x")},
		})
	}
	favorite("old", 1, true)
	favorite("new", 2, true)
	favorite("gone", 3, true)
	favorite("gone", 4, false)
	a.onAppState(ctx, []string{"pin_v1", "x"}, &waSyncAction.SyncActionValue{}) // Unrelated.

	got, err := a.listStickers(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 2 || got[0].FavoriteId != "new" || got[1].FavoriteId != "old" {
		t.Fatalf("favorites %v; want new, old", got)
	}
	// Not downloaded (no connection here), so it cannot be sent yet.
	if _, err := a.favoriteSticker(ctx, "new"); err == nil {
		t.Fatal("built a favorite without its file hash")
	}
}
