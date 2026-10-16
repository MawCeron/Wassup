package main

import (
	"context"
	"testing"
	"time"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
)

func TestSaveAndListMessages(t *testing.T) {
	ctx := context.Background()
	accounts, err := openAccounts(ctx, t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer closeAccounts(accounts)
	a := accounts[0]

	chat := types.NewJID("5215550000000", types.DefaultUserServer)
	base := time.Unix(1_700_000_000, 0)
	text := func(id string, sec int, fromMe bool) *events.Message {
		return &events.Message{
			Info: types.MessageInfo{
				MessageSource: types.MessageSource{Chat: chat, Sender: chat, IsFromMe: fromMe},
				ID:            id,
				PushName:      "Ana",
				Timestamp:     base.Add(time.Duration(sec) * time.Second),
			},
			Message: &waE2E.Message{Conversation: proto.String("msg " + id)},
		}
	}
	save := func(evt *events.Message) bool {
		t.Helper()
		msg, _, err := a.saveMessage(ctx, evt)
		if err != nil {
			t.Fatal(err)
		}
		return msg != nil
	}

	if !save(text("A", 1, false)) || !save(text("B", 2, false)) {
		t.Fatal("new messages not saved")
	}
	if save(text("A", 1, false)) {
		t.Fatal("redelivered message saved twice")
	}
	if save(&events.Message{Info: text("C", 3, false).Info, Message: &waE2E.Message{}}) {
		t.Fatal("non-text message saved")
	}

	chats, err := a.listChats(ctx)
	if err != nil || len(chats) != 1 {
		t.Fatalf("chats: %v, %v", chats, err)
	}
	if c := chats[0]; c.Name != "Ana" || c.UnreadCount != 2 || c.LastMessageAtMs != base.Add(2*time.Second).UnixMilli() {
		t.Fatalf("chat after two incoming: %v", c)
	}

	save(text("D", 4, true))
	if chats, _ = a.listChats(ctx); chats[0].UnreadCount != 0 {
		t.Fatalf("reply did not clear unread: %v", chats[0])
	}

	page, err := a.listMessages(ctx, chat.String(), "", 2)
	if err != nil || len(page) != 2 || page[0].Id != "D" || page[1].Id != "B" {
		t.Fatalf("first page: %v, %v", page, err)
	}
	page, err = a.listMessages(ctx, chat.String(), "B", 0)
	if err != nil || len(page) != 1 || page[0].Id != "A" {
		t.Fatalf("page before B: %v, %v", page, err)
	}

	if _, _, err := a.sendText(ctx, chat.String(), ""); err == nil {
		t.Fatal("sent an empty message")
	}
	if _, _, err := a.sendText(ctx, chat.String(), "hi"); err == nil {
		t.Fatal("sent from an unlinked account")
	}
}
