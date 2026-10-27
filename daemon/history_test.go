package main

import (
	"context"
	"testing"

	"go.mau.fi/whatsmeow/proto/waCommon"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/proto/waHistorySync"
	"go.mau.fi/whatsmeow/proto/waWeb"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
)

func webMessage(chat, id string, sec uint64, fromMe bool, m *waE2E.Message) *waHistorySync.HistorySyncMsg {
	return &waHistorySync.HistorySyncMsg{Message: &waWeb.WebMessageInfo{
		Key:              &waCommon.MessageKey{RemoteJID: proto.String(chat), FromMe: proto.Bool(fromMe), ID: proto.String(id)},
		MessageTimestamp: proto.Uint64(sec),
		Message:          m,
	}}
}

func TestHistory(t *testing.T) {
	ctx := context.Background()
	accounts, err := openAccounts(ctx, t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer closeAccounts(accounts)
	a := accounts[0]
	a.client.Store.ID = &types.JID{User: "5215511111111", Device: 3, Server: types.DefaultUserServer}
	chat := "5215522222222@s.whatsapp.net"
	text := func(s string) *waE2E.Message { return &waE2E.Message{Conversation: proto.String(s)} }

	changed := a.onHistory(ctx, &waHistorySync.HistorySync{
		SyncType: waHistorySync.HistorySync_INITIAL_BOOTSTRAP.Enum(),
		Conversations: []*waHistorySync.Conversation{{
			ID:          proto.String(chat),
			UnreadCount: proto.Uint32(1),
			Messages: []*waHistorySync.HistorySyncMsg{
				webMessage(chat, "B", 1_700_000_200, false, text("segundo")),
				webMessage(chat, "A", 1_700_000_100, true, text("primero")),
				webMessage(chat, "R", 1_700_000_300, true, &waE2E.Message{ReactionMessage: &waE2E.ReactionMessage{
					Key: &waCommon.MessageKey{ID: proto.String("B")}, Text: proto.String("👍"),
				}}),
			},
		}},
	})
	if len(changed) != 1 || changed[0] != chat {
		t.Fatalf("changed chats %v", changed)
	}
	c := a.chatInfo(ctx, chat)
	if c.UnreadCount != 1 || c.LastMessageAtMs != 1_700_000_200_000 || c.LastMessage != "segundo" {
		t.Fatalf("chat after the initial history: %v", c)
	}
	msgs, _ := a.listMessages(ctx, chat, "", "", 0)
	if len(msgs) != 2 || msgs[0].Id != "B" || len(msgs[0].Reactions) != 1 || !msgs[1].FromMe {
		t.Fatalf("messages after the initial history: %v", msgs)
	}

	// Older messages on demand neither count as unread nor move the chat back.
	a.onHistory(ctx, &waHistorySync.HistorySync{
		SyncType: waHistorySync.HistorySync_ON_DEMAND.Enum(),
		Conversations: []*waHistorySync.Conversation{{
			ID:          proto.String(chat),
			UnreadCount: proto.Uint32(9),
			Messages:    []*waHistorySync.HistorySyncMsg{webMessage(chat, "OLD", 1_600_000_000, false, text("viejo"))},
		}},
	})
	c = a.chatInfo(ctx, chat)
	if c.UnreadCount != 1 || c.LastMessageAtMs != 1_700_000_200_000 {
		t.Fatalf("chat after loading older messages: %v", c)
	}
	if msgs, _ := a.listMessages(ctx, chat, "A", "", 0); len(msgs) != 1 || msgs[0].Id != "OLD" {
		t.Fatalf("page before A: %v", msgs)
	}
}
