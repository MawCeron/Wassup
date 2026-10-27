package main

import (
	"context"
	"strings"
	"testing"
	"time"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
)

func TestQuotes(t *testing.T) {
	ctx := context.Background()
	accounts, err := openAccounts(ctx, t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer closeAccounts(accounts)
	a := accounts[0]
	chat := types.NewJID("120363000000000001", types.GroupServer)
	reply := func(id string, quoted *waE2E.Message) {
		t.Helper()
		_, _, err := a.saveMessage(ctx, &events.Message{
			Info: types.MessageInfo{
				MessageSource: types.MessageSource{Chat: chat, Sender: chat, IsGroup: true},
				ID:            id,
				Timestamp:     time.Unix(1_700_000_000, 0),
			},
			Message: &waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{
				Text: proto.String("answer " + id),
				ContextInfo: &waE2E.ContextInfo{
					StanzaID:      proto.String("Q" + id),
					Participant:   proto.String("5215550000000@s.whatsapp.net"),
					QuotedMessage: quoted,
				},
			}},
		})
		if err != nil {
			t.Fatal(err)
		}
	}
	reply("1", &waE2E.Message{Conversation: proto.String("original")})
	reply("2", &waE2E.Message{ImageMessage: &waE2E.ImageMessage{}})
	reply("3", &waE2E.Message{Conversation: proto.String(strings.Repeat("x", 500))})

	msgs, err := a.listMessages(ctx, chat.String(), "", "", 0)
	if err != nil {
		t.Fatal(err)
	}
	quotes := map[string]string{}
	for _, m := range msgs {
		if m.Quote == nil || m.Quote.MessageId != "Q"+m.Id || m.Quote.SenderJid != "5215550000000@s.whatsapp.net" {
			t.Fatalf("message %s quote %v", m.Id, m.Quote)
		}
		quotes[m.Id] = m.Quote.Text
	}
	if quotes["1"] != "original" || quotes["2"] != "Photo" || len([]rune(quotes["3"])) != maxQuoteRunes {
		t.Fatalf("quote texts %q", quotes)
	}
	if quoteOf(&waE2E.Message{Conversation: proto.String("not a reply")}) != nil {
		t.Fatal("a plain message got a quote")
	}
}
