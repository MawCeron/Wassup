package main

import (
	"context"
	"database/sql"
	"path/filepath"
	"slices"

	"github.com/MawCeron/Wassup/daemon/ipc"
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
		t.Fatal("empty message saved")
	}
	voice := &events.Message{Message: &waE2E.Message{AudioMessage: &waE2E.AudioMessage{PTT: proto.Bool(true), Seconds: proto.Uint32(7)}}}
	if got := messageText(voice); got != "" || mediaOf(voice.Message) == nil {
		t.Fatalf("voice note text %q, media %v; want no text and media", got, mediaOf(voice.Message))
	}
	if m := a.ipcMedia(mustMarshal(t, mediaOf(voice.Message))); m.GetKind() != ipc.MediaKind_MEDIA_KIND_AUDIO || !m.GetVoiceNote() || m.GetDurationSeconds() != 7 {
		t.Fatalf("voice note media %v", m)
	}
	other := types.NewJID("5215551111111", types.DefaultUserServer)
	stickerInfo := text("S", 9, false).Info
	stickerInfo.Chat, stickerInfo.Sender = other, other
	sticker := &events.Message{Info: stickerInfo, Message: &waE2E.Message{StickerMessage: &waE2E.StickerMessage{Width: proto.Uint32(512), Height: proto.Uint32(512)}}}
	stickerMsg, _, err := a.saveMessage(ctx, sticker)
	if err != nil || stickerMsg == nil || stickerMsg.Text != "" || stickerMsg.GetMedia().GetKind() != ipc.MediaKind_MEDIA_KIND_STICKER || stickerMsg.GetMedia().GetWidth() != 512 {
		t.Fatalf("sticker saved as %v, %v", stickerMsg, err)
	}
	if page, _ := a.listMessages(ctx, other.String(), "", "", 1); page[0].GetMedia().GetKind() != ipc.MediaKind_MEDIA_KIND_STICKER {
		t.Fatalf("sticker media not read back: %v", page[0])
	}
	doc := &events.Message{Message: &waE2E.Message{DocumentMessage: &waE2E.DocumentMessage{FileName: proto.String("a.pdf")}}}
	if got := messageText(doc); got != "" || mediaOf(doc.Message) == nil {
		t.Fatalf("document text %q, media %v; want no text and media", got, mediaOf(doc.Message))
	}

	chats, err := a.listChats(ctx)
	chats = slices.DeleteFunc(chats, func(c *ipc.Chat) bool { return c.Jid == other.String() })
	if err != nil || len(chats) != 1 {
		t.Fatalf("chats: %v, %v", chats, err)
	}
	if c := chats[0]; c.Name != "Ana" || c.UnreadCount != 2 || c.LastMessageAtMs != base.Add(2*time.Second).UnixMilli() {
		t.Fatalf("chat after two incoming: %v", c)
	}

	chat2, err := a.markRead(ctx, chat.String())
	if err != nil || chat2.UnreadCount != 0 {
		t.Fatalf("mark read: %v, %v", chat2, err)
	}
	save(text("E", 5, false))
	save(text("D", 4, true))
	chats, _ = a.listChats(ctx)
	chats = slices.DeleteFunc(chats, func(c *ipc.Chat) bool { return c.Jid == other.String() })
	if chats[0].UnreadCount != 0 {
		t.Fatalf("reply did not clear unread: %v", chats[0])
	}

	page, err := a.listMessages(ctx, chat.String(), "", "", 2)
	if err != nil || len(page) != 2 || page[0].Id != "E" || page[1].Id != "D" {
		t.Fatalf("first page: %v, %v", page, err)
	}
	page, err = a.listMessages(ctx, chat.String(), "D", "", 0)
	if err != nil || len(page) != 2 || page[0].Id != "B" || page[1].Id != "A" {
		t.Fatalf("page before D: %v, %v", page, err)
	}
	page, err = a.listMessages(ctx, chat.String(), "", "B", 0)
	if err != nil || len(page) != 3 || page[0].Id != "E" || page[2].Id != "B" {
		t.Fatalf("from B to the newest: %v, %v", page, err)
	}
	song := text("S", 6, false)
	song.Message = &waE2E.Message{Conversation: proto.String("Esa CANCIÓN me encanta")}
	save(song)
	found, err := a.searchMessages(ctx, chat.String(), "  cancion ")
	if err != nil || len(found) != 1 || found[0].Id != "S" {
		t.Fatalf("search ignoring case and accents: %v, %v", found, err)
	}
	found, _ = a.searchMessages(ctx, chat.String(), "MSG")
	if len(found) != 4 || found[0].Id != "E" || found[3].Id != "A" {
		t.Fatalf("search, newest first: %v", found)
	}
	if found, _ = a.searchMessages(ctx, chat.String(), "nothing like this"); len(found) != 0 {
		t.Fatalf("search without matches: %v", found)
	}

	changed, err := a.advanceStatus(ctx, []types.MessageID{"D", "A"}, ipc.MessageStatus_MESSAGE_STATUS_READ)
	if err != nil || len(changed[chat.String()]) != 1 || changed[chat.String()][0] != "D" {
		t.Fatalf("read receipt changed %v, %v; want only own message D", changed, err)
	}
	changed, _ = a.advanceStatus(ctx, []types.MessageID{"D"}, ipc.MessageStatus_MESSAGE_STATUS_DELIVERED)
	if len(changed) != 0 {
		t.Fatalf("status went back from read to delivered: %v", changed)
	}
	page, _ = a.listMessages(ctx, chat.String(), "", "", 0)
	for _, m := range page {
		if m.Id == "D" && m.Status != ipc.MessageStatus_MESSAGE_STATUS_READ {
			t.Fatalf("stored status of D: %v", m.Status)
		}
	}

	if _, _, err := a.sendText(ctx, chat.String(), ""); err == nil {
		t.Fatal("sent an empty message")
	}
	if _, _, err := a.sendText(ctx, chat.String(), "hi"); err == nil {
		t.Fatal("sent from an unlinked account")
	}
}

// A database created before migrations existed has the first tables but
// user_version 0; migrating must keep its rows and mark own ones as sent.
func TestMigrateExistingDatabase(t *testing.T) {
	ctx := context.Background()
	db, err := sql.Open("sqlite", "file:"+filepath.Join(t.TempDir(), "old.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	for _, q := range []string{
		migrations[0],
		`INSERT INTO wassup_chats (jid, is_group) VALUES ('c', 0)`,
		`INSERT INTO wassup_messages VALUES ('c', 'mine', 'me', 1, 1, 'hi'), ('c', 'theirs', 'x', 0, 2, 'yo')`,
	} {
		if _, err := db.ExecContext(ctx, q); err != nil {
			t.Fatal(err)
		}
	}
	if err := migrate(ctx, db); err != nil {
		t.Fatal(err)
	}
	if err := migrate(ctx, db); err != nil {
		t.Fatalf("second run: %v", err)
	}
	var mine, theirs int
	db.QueryRowContext(ctx, `SELECT status FROM wassup_messages WHERE id = 'mine'`).Scan(&mine)
	db.QueryRowContext(ctx, `SELECT status FROM wassup_messages WHERE id = 'theirs'`).Scan(&theirs)
	if mine != 1 || theirs != 0 {
		t.Fatalf("statuses after migration: mine=%d theirs=%d", mine, theirs)
	}
}

func TestBacklogIsNotNews(t *testing.T) {
	a := &account{}
	msg := &ipc.Message{TimestampMs: 1_000}
	if a.isNews(msg) {
		t.Fatal("notified while not connected")
	}
	a.connectedAt.Store(2_000)
	if a.isNews(msg) {
		t.Fatal("notified a message sent before connecting")
	}
	msg.TimestampMs = 3_000
	if !a.isNews(msg) {
		t.Fatal("did not notify a message sent while connected")
	}
	msg.FromMe = true
	if a.isNews(msg) {
		t.Fatal("notified an own message")
	}
}

func mustMarshal(t *testing.T, m proto.Message) []byte {
	t.Helper()
	b, err := proto.Marshal(m)
	if err != nil {
		t.Fatal(err)
	}
	return b
}
