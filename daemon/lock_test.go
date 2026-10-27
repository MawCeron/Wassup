package main

import (
	"context"
	"crypto/pbkdf2"
	"crypto/sha512"
	"testing"
	"time"

	"github.com/MawCeron/Wassup/daemon/ipc"
	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/proto/waChatLockSettings"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/proto/waSyncAction"
	"go.mau.fi/whatsmeow/proto/waUserPassword"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
)

// phoneSecret hashes a code the way the phone sends it.
func phoneSecret(t *testing.T, code string) *waUserPassword.UserPassword {
	t.Helper()
	salt := []byte("0123456789abcdef")
	hash, err := pbkdf2.Key(sha512.New, code, salt, 1000, 64)
	if err != nil {
		t.Fatal(err)
	}
	return &waUserPassword.UserPassword{
		Transformer: waUserPassword.UserPassword_PBKDF2_HMAC_SHA512.Enum(),
		TransformerArg: []*waUserPassword.UserPassword_TransformerArg{
			{Key: proto.String("salt"), Value: &waUserPassword.UserPassword_TransformerArg_Value{
				Value: &waUserPassword.UserPassword_TransformerArg_Value_AsBlob{AsBlob: salt}}},
			{Key: proto.String("iterations"), Value: &waUserPassword.UserPassword_TransformerArg_Value{
				Value: &waUserPassword.UserPassword_TransformerArg_Value_AsUnsignedInteger{AsUnsignedInteger: 1000}}},
		},
		TransformedData: hash,
	}
}

func TestLockedChats(t *testing.T) {
	ctx := context.Background()
	accounts, err := openAccounts(ctx, t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer closeAccounts(accounts)
	a := accounts[0]
	chat := types.NewJID("5215550000000", types.DefaultUserServer)
	if _, _, err := a.saveMessage(ctx, &events.Message{
		Info:    types.MessageInfo{MessageSource: types.MessageSource{Chat: chat, Sender: chat}, ID: "1", Timestamp: time.Unix(1_700_000_000, 0)},
		Message: &waE2E.Message{Conversation: proto.String("secret stuff")},
	}); err != nil {
		t.Fatal(err)
	}

	if err := a.unlockChats(ctx, "🙈42"); err != errNoSecretCode {
		t.Fatalf("unlock without a code set: %v", err)
	}
	a.onAppState(ctx, []string{appstate.IndexSettingChatLock}, &waSyncAction.SyncActionValue{
		ChatLockSettings: &waChatLockSettings.ChatLockSettings{SecretCode: phoneSecret(t, "🙈42")},
	})
	if err := a.unlockChats(ctx, "🙈42"); err != nil {
		t.Fatalf("right code rejected: %v", err)
	}
	if err := a.unlockChats(ctx, "🙈43"); err != errWrongSecretCode {
		t.Fatalf("wrong code: %v", err)
	}

	lock := func(on bool) *ipc.Chat {
		return a.onAppState(ctx, []string{appstate.IndexLock, chat.String()}, &waSyncAction.SyncActionValue{
			LockChatAction: &waSyncAction.LockChatAction{Locked: proto.Bool(on)},
		})
	}
	if c := lock(true); c == nil || !c.Locked {
		t.Fatalf("locking returned %v", c)
	}
	chats, _ := a.listChats(ctx)
	if len(chats) != 1 || !chats[0].Locked {
		t.Fatalf("listed %v; want the chat marked locked", chats)
	}
	title, body := notificationText(chats[0], &ipc.Message{Text: "secret stuff", SenderName: "Ana"}, false)
	if title != "Wassup" || body != "New message" {
		t.Fatalf("locked chat notification %q / %q leaks content", title, body)
	}
	if c := lock(false); c == nil || c.Locked {
		t.Fatalf("unlocking returned %v", c)
	}
}
