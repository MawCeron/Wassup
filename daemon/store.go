package main

import (
	"context"
	"database/sql"
	"errors"
	"time"

	"github.com/MawCeron/Wassup/daemon/ipc"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
)

// Our tables live next to whatsmeow's in the account database.
const schema = `
CREATE TABLE IF NOT EXISTS wassup_chats (
	jid                TEXT PRIMARY KEY,
	name               TEXT NOT NULL DEFAULT '',
	is_group           INTEGER NOT NULL,
	unread_count       INTEGER NOT NULL DEFAULT 0,
	last_message_at_ms INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS wassup_messages (
	chat_jid     TEXT NOT NULL REFERENCES wassup_chats(jid),
	id           TEXT NOT NULL,
	sender_jid   TEXT NOT NULL,
	from_me      INTEGER NOT NULL,
	timestamp_ms INTEGER NOT NULL,
	text         TEXT NOT NULL,
	PRIMARY KEY (chat_jid, id)
);
CREATE INDEX IF NOT EXISTS wassup_messages_by_time ON wassup_messages (chat_jid, timestamp_ms, id);
`

const (
	defaultPageSize = 50
	maxPageSize     = 500
)

// messageText returns the text of a plain text message, or "" for anything else.
func messageText(evt *events.Message) string {
	if evt.IsEdit {
		return "" // TODO: apply edits to the stored message instead of skipping them.
	}
	if t := evt.Message.GetConversation(); t != "" {
		return t
	}
	return evt.Message.GetExtendedTextMessage().GetText()
}

// saveMessage stores a text message and updates its chat. It returns nil
// for messages that are not text or were already stored.
func (a *account) saveMessage(ctx context.Context, evt *events.Message) (*ipc.Message, *ipc.Chat, error) {
	text := messageText(evt)
	info := evt.Info
	if text == "" || info.Chat == types.StatusBroadcastJID {
		return nil, nil, nil
	}
	msg := &ipc.Message{
		AccountId:   a.id,
		ChatJid:     info.Chat.String(),
		Id:          info.ID,
		SenderJid:   info.Sender.ToNonAD().String(),
		FromMe:      info.IsFromMe,
		TimestampMs: info.Timestamp.UnixMilli(),
		Text:        text,
	}

	// Resolved before the transaction: it may ask the server for a group name.
	var known bool
	err := a.db.QueryRowContext(ctx, `SELECT 1 FROM wassup_chats WHERE jid = ?`, msg.ChatJid).Scan(&known)
	if err != nil && !errors.Is(err, sql.ErrNoRows) {
		return nil, nil, err
	}
	name := ""
	if !known {
		name = a.chatName(ctx, evt)
	}

	tx, err := a.db.BeginTx(ctx, nil)
	if err != nil {
		return nil, nil, err
	}
	defer tx.Rollback()
	_, err = tx.ExecContext(ctx, `INSERT OR IGNORE INTO wassup_chats (jid, name, is_group) VALUES (?, ?, ?)`,
		msg.ChatJid, name, info.IsGroup)
	if err != nil {
		return nil, nil, err
	}

	res, err := tx.ExecContext(ctx, `INSERT OR IGNORE INTO wassup_messages
		(chat_jid, id, sender_jid, from_me, timestamp_ms, text) VALUES (?, ?, ?, ?, ?, ?)`,
		msg.ChatJid, msg.Id, msg.SenderJid, msg.FromMe, msg.TimestampMs, msg.Text)
	if err != nil {
		return nil, nil, err
	}
	if n, _ := res.RowsAffected(); n == 0 {
		return nil, nil, nil // Redelivered.
	}

	// Replying from any device means the chat was read.
	_, err = tx.ExecContext(ctx, `UPDATE wassup_chats SET
		last_message_at_ms = max(last_message_at_ms, ?),
		unread_count = CASE WHEN ? THEN 0 ELSE unread_count + 1 END
		WHERE jid = ?`, msg.TimestampMs, msg.FromMe, msg.ChatJid)
	if err != nil {
		return nil, nil, err
	}
	chat, err := scanChat(a.id, tx.QueryRowContext(ctx, chatQuery+` WHERE jid = ?`, msg.ChatJid))
	if err != nil {
		return nil, nil, err
	}
	return msg, chat, tx.Commit()
}

// chatName looks up a display name for a chat seen for the first time.
func (a *account) chatName(ctx context.Context, evt *events.Message) string {
	if evt.Info.IsGroup {
		group, err := a.client.GetGroupInfo(ctx, evt.Info.Chat)
		if err != nil {
			a.log.Warn("group info", "chat", evt.Info.Chat, "err", err)
			return ""
		}
		return group.Name
	}
	// Contacts is nil until the device is linked.
	if contacts := a.client.Store.Contacts; contacts != nil {
		if c, err := contacts.GetContact(ctx, evt.Info.Chat); err == nil && c.Found {
			for _, name := range []string{c.FullName, c.FirstName, c.BusinessName, c.PushName} {
				if name != "" {
					return name
				}
			}
		}
	}
	if !evt.Info.IsFromMe {
		return evt.Info.PushName
	}
	return ""
}

const chatQuery = `SELECT jid, name, is_group, unread_count, last_message_at_ms FROM wassup_chats`

func scanChat(accountID string, row interface{ Scan(...any) error }) (*ipc.Chat, error) {
	c := &ipc.Chat{AccountId: accountID}
	return c, row.Scan(&c.Jid, &c.Name, &c.IsGroup, &c.UnreadCount, &c.LastMessageAtMs)
}

func (a *account) listChats(ctx context.Context) ([]*ipc.Chat, error) {
	rows, err := a.db.QueryContext(ctx, chatQuery+` ORDER BY last_message_at_ms DESC`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var chats []*ipc.Chat
	for rows.Next() {
		c, err := scanChat(a.id, rows)
		if err != nil {
			return nil, err
		}
		chats = append(chats, c)
	}
	return chats, rows.Err()
}

// listMessages returns up to limit messages, newest first, older than
// beforeID when it is set.
func (a *account) listMessages(ctx context.Context, chatJID, beforeID string, limit uint32) ([]*ipc.Message, error) {
	if limit == 0 {
		limit = defaultPageSize
	}
	limit = min(limit, maxPageSize)
	rows, err := a.db.QueryContext(ctx, `
		SELECT id, sender_jid, from_me, timestamp_ms, text FROM wassup_messages
		WHERE chat_jid = ?1 AND (?2 = '' OR (timestamp_ms, id) <
			(SELECT timestamp_ms, id FROM wassup_messages WHERE chat_jid = ?1 AND id = ?2))
		ORDER BY timestamp_ms DESC, id DESC LIMIT ?3`, chatJID, beforeID, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var msgs []*ipc.Message
	for rows.Next() {
		m := &ipc.Message{AccountId: a.id, ChatJid: chatJID}
		if err := rows.Scan(&m.Id, &m.SenderJid, &m.FromMe, &m.TimestampMs, &m.Text); err != nil {
			return nil, err
		}
		msgs = append(msgs, m)
	}
	return msgs, rows.Err()
}

// sendText sends a text message and stores it like a received one.
func (a *account) sendText(ctx context.Context, chat, text string) (*ipc.Message, *ipc.Chat, error) {
	if text == "" {
		return nil, nil, errors.New("empty message")
	}
	if !a.client.IsLoggedIn() {
		return nil, nil, errors.New("account is not connected")
	}
	to, err := types.ParseJID(chat)
	if err != nil {
		return nil, nil, err
	}
	resp, err := a.client.SendMessage(ctx, to, &waE2E.Message{Conversation: proto.String(text)})
	if err != nil {
		return nil, nil, err
	}
	// The server may route the message to the contact's other identity (LID or phone number).
	if !resp.Chat.IsEmpty() {
		to = resp.Chat
	}
	sender := resp.Sender
	if sender.IsEmpty() {
		sender = *a.client.Store.ID
	}
	return a.saveMessage(ctx, &events.Message{
		Info: types.MessageInfo{
			MessageSource: types.MessageSource{Chat: to, Sender: sender, IsFromMe: true, IsGroup: to.Server == types.GroupServer},
			ID:            resp.ID,
			Timestamp:     resp.Timestamp,
		},
		Message: &waE2E.Message{Conversation: proto.String(text)},
	})
}

// markRead clears a chat's unread count and, when connected, tells the
// senders their messages were read.
func (a *account) markRead(ctx context.Context, chatJID string) (*ipc.Chat, error) {
	chat, err := types.ParseJID(chatJID)
	if err != nil {
		return nil, err
	}
	rows, err := a.db.QueryContext(ctx, `
		SELECT id, sender_jid FROM wassup_messages WHERE chat_jid = ? AND from_me = 0
		ORDER BY timestamp_ms DESC LIMIT (SELECT unread_count FROM wassup_chats WHERE jid = ?)`,
		chatJID, chatJID)
	if err != nil {
		return nil, err
	}
	bySender := map[string][]types.MessageID{}
	for rows.Next() {
		var id, sender string
		if err := rows.Scan(&id, &sender); err != nil {
			rows.Close()
			return nil, err
		}
		bySender[sender] = append(bySender[sender], id)
	}
	rows.Close()
	if err := rows.Err(); err != nil {
		return nil, err
	}

	if a.client.IsLoggedIn() {
		// Receipts are best effort: the chat is read here even if they fail.
		for sender, ids := range bySender {
			from, err := types.ParseJID(sender)
			if err == nil {
				err = a.client.MarkRead(ctx, ids, time.Now(), chat, from)
			}
			if err != nil {
				a.log.Warn("read receipt", "chat", chatJID, "err", err)
			}
		}
	}

	if _, err := a.db.ExecContext(ctx, `UPDATE wassup_chats SET unread_count = 0 WHERE jid = ?`, chatJID); err != nil {
		return nil, err
	}
	return scanChat(a.id, a.db.QueryRowContext(ctx, chatQuery+` WHERE jid = ?`, chatJID))
}
