package main

import (
	"context"
	"testing"
	"time"

	"go.mau.fi/whatsmeow/proto/waCommon"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
)

func TestReactions(t *testing.T) {
	ctx := context.Background()
	accounts, err := openAccounts(ctx, t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer closeAccounts(accounts)
	a := accounts[0]
	me := types.NewJID("5215511111111", types.DefaultUserServer)
	a.client.Store.ID = &types.JID{User: me.User, Device: 3, Server: types.DefaultUserServer}
	chat := types.NewJID("120363000000000001", types.GroupServer)
	ana := types.NewJID("5215522222222", types.DefaultUserServer)
	bob := types.NewJID("5215533333333", types.DefaultUserServer)
	at := time.Unix(1_700_000_000, 0)
	if _, _, err := a.saveMessage(ctx, &events.Message{
		Info:    types.MessageInfo{MessageSource: types.MessageSource{Chat: chat, Sender: ana, IsGroup: true}, ID: "M", Timestamp: at},
		Message: &waE2E.Message{Conversation: proto.String("hola")},
	}); err != nil {
		t.Fatal(err)
	}
	react := func(from types.JID, emoji string) {
		t.Helper()
		changed := a.onReaction(ctx, &events.Message{
			Info: types.MessageInfo{MessageSource: types.MessageSource{Chat: chat, Sender: from, IsGroup: true}, ID: "R" + from.User + emoji, Timestamp: at},
			Message: &waE2E.Message{ReactionMessage: &waE2E.ReactionMessage{
				Key: &waCommon.MessageKey{ID: proto.String("M")}, Text: proto.String(emoji),
			}},
		})
		if changed == nil || changed.MessageId != "M" {
			t.Fatalf("reaction gave %v", changed)
		}
	}
	react(ana, "❤️")
	react(bob, "👍")
	react(bob, "❤️") // Replaces bob's 👍.
	react(types.JID{User: me.User, Device: 7, Server: types.DefaultUserServer}, "😂")

	msgs, _ := a.listMessages(ctx, chat.String(), "", "", 0)
	got := msgs[0].Reactions
	if len(got) != 2 || got[0].Emoji != "❤️" || got[0].Count != 2 || got[0].Mine || got[1].Emoji != "😂" || !got[1].Mine {
		t.Fatalf("reactions %v; want ❤️×2 then 😂 mine", got)
	}
	react(ana, "") // Ana takes hers back.
	if r := a.reactionsChanged(ctx, chat.String(), "M").Reactions; len(r) != 2 || r[0].Count != 1 {
		t.Fatalf("after taking one back: %v", r)
	}
	if a.onReaction(ctx, &events.Message{Message: &waE2E.Message{Conversation: proto.String("x")}}) != nil {
		t.Fatal("a plain message was taken for a reaction")
	}
}
