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

func TestReceivedLinkPreview(t *testing.T) {
	ctx := context.Background()
	accounts, err := openAccounts(ctx, t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer closeAccounts(accounts)
	a := accounts[0]
	a.mediaDir = t.TempDir()
	chat := types.NewJID("5215550000000", types.DefaultUserServer)
	save := func(id string, m *waE2E.Message) {
		t.Helper()
		if _, _, err := a.saveMessage(ctx, &events.Message{
			Info:    types.MessageInfo{MessageSource: types.MessageSource{Chat: chat, Sender: chat}, ID: id, Timestamp: time.Unix(1_700_000_000, 0)},
			Message: m,
		}); err != nil {
			t.Fatal(err)
		}
	}
	save("L", &waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{
		Text: proto.String("look https://gnome.org"), MatchedText: proto.String("https://gnome.org"),
		Title: proto.String("GNOME"), Description: proto.String("The desktop"), JPEGThumbnail: []byte("\xff\xd8\xffjpeg"),
	}})
	save("P", &waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{Text: proto.String("plain https://x.org")}})

	msgs, err := a.listMessages(ctx, chat.String(), "", "", 0)
	if err != nil || len(msgs) != 2 {
		t.Fatalf("listed %v, %v", msgs, err)
	}
	byID := map[string]int{msgs[0].Id: 0, msgs[1].Id: 1}
	link := msgs[byID["L"]].GetLink()
	if link.GetUrl() != "https://gnome.org" || link.GetTitle() != "GNOME" || link.GetDescription() != "The desktop" {
		t.Fatalf("preview: %v", link)
	}
	if data, err := os.ReadFile(link.GetThumbnailPath()); err != nil || string(data) != "\xff\xd8\xffjpeg" {
		t.Fatalf("thumbnail %q: %q, %v", link.GetThumbnailPath(), data, err)
	}
	if msgs[byID["P"]].GetLink() != nil {
		t.Fatalf("a link without a preview got one: %v", msgs[byID["P"]].GetLink())
	}
}
