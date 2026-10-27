package main

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"os"
	"strings"
	"time"

	"github.com/MawCeron/Wassup/daemon/ipc"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
)

// Our tables live next to whatsmeow's in the account database. Each
// migration runs once, in order; PRAGMA user_version counts those applied.
var migrations = []string{`
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
`, `
ALTER TABLE wassup_messages ADD COLUMN status INTEGER NOT NULL DEFAULT 0;
UPDATE wassup_messages SET status = 1 WHERE from_me = 1;
`, `
-- Serialized waE2E.Message holding just what is needed to download it.
ALTER TABLE wassup_messages ADD COLUMN media BLOB;
`, `
-- Profile or group picture: the server's id for it, and when it was last
-- checked, so it is not asked for again on every start.
ALTER TABLE wassup_chats ADD COLUMN avatar_id TEXT NOT NULL DEFAULT '';
ALTER TABLE wassup_chats ADD COLUMN avatar_checked_ms INTEGER NOT NULL DEFAULT 0;
`, `
-- Favorite stickers synced from the phone. action is the serialized
-- StickerAction; file_sha256 is filled once the sticker is downloaded,
-- since the sync does not carry it and sending needs it.
CREATE TABLE wassup_favorite_stickers (
	id          TEXT PRIMARY KEY,
	action      BLOB NOT NULL,
	file_sha256 BLOB,
	added_ms    INTEGER NOT NULL
);
CREATE TABLE wassup_meta (
	key   TEXT PRIMARY KEY,
	value TEXT NOT NULL
);
`, `
-- Favorites whose file is no longer on WhatsApp's servers; not retried.
ALTER TABLE wassup_favorite_stickers ADD COLUMN gone INTEGER NOT NULL DEFAULT 0;
`, `
-- Serialized ipc.Quote of a reply, without the quoted sender's name.
ALTER TABLE wassup_messages ADD COLUMN quote BLOB;
`, `
-- Disappearing messages: each chat's timer, and when each message goes.
ALTER TABLE wassup_chats ADD COLUMN disappearing_seconds INTEGER NOT NULL DEFAULT 0;
ALTER TABLE wassup_messages ADD COLUMN expires_at_ms INTEGER NOT NULL DEFAULT 0;
CREATE INDEX wassup_messages_by_expiry ON wassup_messages (expires_at_ms) WHERE expires_at_ms > 0;
`, `
-- Chats locked on the phone, by any of their ids.
CREATE TABLE wassup_locked_chats (jid TEXT PRIMARY KEY);
`}

func migrate(ctx context.Context, db *sql.DB) error {
	var version int
	if err := db.QueryRowContext(ctx, `PRAGMA user_version`).Scan(&version); err != nil {
		return err
	}
	for ; version < len(migrations); version++ {
		tx, err := db.BeginTx(ctx, nil)
		if err != nil {
			return err
		}
		_, err = tx.ExecContext(ctx, migrations[version])
		if err == nil {
			_, err = tx.ExecContext(ctx, fmt.Sprintf(`PRAGMA user_version = %d`, version+1))
		}
		if err == nil {
			err = tx.Commit()
		}
		if err != nil {
			tx.Rollback()
			return fmt.Errorf("migration %d: %w", version+1, err)
		}
	}
	return nil
}

const (
	defaultPageSize = 50
	maxPageSize     = 500
)

// messageText returns a message's text, a bracketed placeholder for content
// that is not shown yet, or "" for messages that are not shown at all.
func messageText(evt *events.Message) string {
	if evt.IsEdit {
		return "" // TODO: apply edits to the stored message instead of skipping them.
	}
	m := evt.Message
	label := func(kind string, details ...string) string {
		for _, d := range details {
			if d != "" {
				return kind + " " + d
			}
		}
		return kind
	}
	switch {
	case m.GetConversation() != "":
		return m.GetConversation()
	case m.GetExtendedTextMessage() != nil:
		return m.GetExtendedTextMessage().GetText()
	case m.GetStickerMessage() != nil:
		return "" // Carried as media.
	case m.GetImageMessage() != nil:
		return m.GetImageMessage().GetCaption() // The image is carried as media.
	case m.GetVideoMessage() != nil:
		return label(tr("[Video]"), m.GetVideoMessage().GetCaption())
	case m.GetAudioMessage() != nil:
		return "" // Carried as media.
	case m.GetDocumentMessage() != nil:
		return m.GetDocumentMessage().GetCaption() // The file is carried as media.
	case m.GetContactMessage() != nil:
		return label(tr("[Contact]"), m.GetContactMessage().GetDisplayName())
	case m.GetLocationMessage() != nil, m.GetLiveLocationMessage() != nil:
		return tr("[Location]")
	case m.GetPollCreationMessage() != nil:
		return label(tr("[Poll]"), m.GetPollCreationMessage().GetName())
	case m.GetPollCreationMessageV3() != nil:
		return label(tr("[Poll]"), m.GetPollCreationMessageV3().GetName())
	}
	// Reactions, receipts and protocol messages are not chat messages.
	return ""
}

// saveMessage stores a text message and updates its chat. It returns nil
// for messages that are not text or were already stored.
func (a *account) saveMessage(ctx context.Context, evt *events.Message) (*ipc.Message, *ipc.Chat, error) {
	text := messageText(evt)
	media := mediaOf(evt.Message)
	info := evt.Info
	if evt.IsEdit || (text == "" && media == nil) || info.Chat == types.StatusBroadcastJID {
		return nil, nil, nil
	}
	quote := quoteOf(evt.Message)
	var quoteBlob []byte
	if quote != nil {
		var err error
		if quoteBlob, err = proto.Marshal(quote); err != nil {
			return nil, nil, err
		}
		a.nameQuote(ctx, quote)
	}
	var mediaBlob []byte
	if media != nil {
		var err error
		if mediaBlob, err = proto.Marshal(media); err != nil {
			return nil, nil, err
		}
	}
	msg := &ipc.Message{
		AccountId:   a.id,
		ChatJid:     info.Chat.String(),
		Id:          info.ID,
		SenderJid:   info.Sender.ToNonAD().String(),
		FromMe:      info.IsFromMe,
		TimestampMs: info.Timestamp.UnixMilli(),
		Text:        text,
		Media:       a.ipcMedia(mediaBlob),
		Quote:       quote,
	}
	if msg.FromMe {
		msg.Status = ipc.MessageStatus_MESSAGE_STATUS_SENT
	} else {
		msg.SenderName = a.contactName(ctx, info.Sender.ToNonAD())
		if msg.SenderName == "" {
			msg.SenderName = info.PushName
		}
	}

	// Resolved before the transaction: it may ask the server for a group name.
	var known bool
	err := a.db.QueryRowContext(ctx, `SELECT 1 FROM wassup_chats WHERE jid = ?`, msg.ChatJid).Scan(&known)
	if err != nil && !errors.Is(err, sql.ErrNoRows) {
		return nil, nil, err
	}
	name, timer := "", uint32(0)
	if !known {
		name, timer = a.chatName(ctx, evt)
	}
	expiration := expirationOf(evt.Message)
	var expiresAt int64
	if expiration > 0 {
		expiresAt = msg.TimestampMs + int64(expiration)*1000
		timer = expiration // The latest message tells the chat's timer best.
	}

	tx, err := a.db.BeginTx(ctx, nil)
	if err != nil {
		return nil, nil, err
	}
	defer tx.Rollback()
	_, err = tx.ExecContext(ctx, `INSERT OR IGNORE INTO wassup_chats (jid, name, is_group, disappearing_seconds)
		VALUES (?, ?, ?, ?)`, msg.ChatJid, name, info.IsGroup, timer)
	if err != nil {
		return nil, nil, err
	}
	if expiration > 0 {
		if _, err := tx.ExecContext(ctx, `UPDATE wassup_chats SET disappearing_seconds = ? WHERE jid = ?`,
			expiration, msg.ChatJid); err != nil {
			return nil, nil, err
		}
	}

	res, err := tx.ExecContext(ctx, `INSERT OR IGNORE INTO wassup_messages
		(chat_jid, id, sender_jid, from_me, timestamp_ms, text, status, media, quote, expires_at_ms)
		VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
		msg.ChatJid, msg.Id, msg.SenderJid, msg.FromMe, msg.TimestampMs, msg.Text, msg.Status, mediaBlob, quoteBlob, expiresAt)
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
	chat, err := a.scanChat(tx.QueryRowContext(ctx, chatQuery+` WHERE jid = ?`, msg.ChatJid))
	if err != nil {
		return nil, nil, err
	}
	if err := tx.Commit(); err != nil {
		return nil, nil, err
	}
	return msg, a.named(ctx, chat), nil
}

// chatName looks up a display name for a chat seen for the first time.
// For groups it also returns their disappearing timer.
func (a *account) chatName(ctx context.Context, evt *events.Message) (string, uint32) {
	if evt.Info.IsGroup {
		group, err := a.client.GetGroupInfo(ctx, evt.Info.Chat)
		if err != nil {
			a.log.Warn("group info", "chat", evt.Info.Chat, "err", err)
			return "", 0
		}
		var timer uint32
		if group.IsEphemeral {
			timer = group.DisappearingTimer
		}
		return group.Name, timer
	}
	if name := a.contactName(ctx, evt.Info.Chat); name != "" {
		return name, 0
	}
	if !evt.Info.IsFromMe {
		return evt.Info.PushName, 0
	}
	return "", 0
}

// contactName returns the best known name for a user: the address book
// name first, then the name they chose. Hidden (LID) users are looked up
// by their phone number too, since address book names are stored there.
func (a *account) contactName(ctx context.Context, user types.JID) string {
	store := a.client.Store
	if store.Contacts == nil { // Nil until the device is linked.
		return ""
	}
	candidates := []types.JID{user}
	if user.Server == types.HiddenUserServer && store.LIDs != nil {
		if pn, err := store.LIDs.GetPNForLID(ctx, user); err == nil && !pn.IsEmpty() {
			candidates = []types.JID{pn, user}
		}
	}
	var contacts []types.ContactInfo
	for _, jid := range candidates {
		if c, err := store.Contacts.GetContact(ctx, jid); err == nil && c.Found {
			contacts = append(contacts, c)
		}
	}
	for _, c := range contacts {
		for _, name := range []string{c.FullName, c.FirstName, c.BusinessName} {
			if name != "" {
				return name
			}
		}
	}
	for _, c := range contacts {
		if c.PushName != "" {
			return c.PushName
		}
	}
	return ""
}

// named refreshes a one-to-one chat's name from the contacts, which may
// have learned it after the chat was stored.
func (a *account) named(ctx context.Context, chat *ipc.Chat) *ipc.Chat {
	if chat.IsGroup {
		return chat
	}
	if jid, err := types.ParseJID(chat.Jid); err == nil {
		if name := a.contactName(ctx, jid); name != "" {
			chat.Name = name
		}
	}
	return chat
}

const chatQuery = `SELECT jid, name, is_group, unread_count, last_message_at_ms, avatar_id,
	EXISTS (SELECT 1 FROM wassup_locked_chats l WHERE l.jid = wassup_chats.jid) FROM wassup_chats`

func (a *account) scanChat(row interface{ Scan(...any) error }) (*ipc.Chat, error) {
	c := &ipc.Chat{AccountId: a.id}
	var avatarID string
	if err := row.Scan(&c.Jid, &c.Name, &c.IsGroup, &c.UnreadCount, &c.LastMessageAtMs, &avatarID, &c.Locked); err != nil {
		return nil, err
	}
	if path := a.avatarPath(c.Jid, avatarID); path != "" {
		if _, err := os.Stat(path); err == nil {
			c.AvatarPath = path
		}
	}
	return c, nil
}

func (a *account) listChats(ctx context.Context) ([]*ipc.Chat, error) {
	rows, err := a.db.QueryContext(ctx, chatQuery+` ORDER BY last_message_at_ms DESC`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var chats []*ipc.Chat
	for rows.Next() {
		c, err := a.scanChat(rows)
		if err != nil {
			return nil, err
		}
		chats = append(chats, a.named(ctx, c))
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
		SELECT id, sender_jid, from_me, timestamp_ms, text, status, media, quote FROM wassup_messages
		WHERE chat_jid = ?1 AND (?2 = '' OR (timestamp_ms, id) <
			(SELECT timestamp_ms, id FROM wassup_messages WHERE chat_jid = ?1 AND id = ?2))
		ORDER BY timestamp_ms DESC, id DESC LIMIT ?3`, chatJID, beforeID, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var msgs []*ipc.Message
	names := map[string]string{}
	for rows.Next() {
		m := &ipc.Message{AccountId: a.id, ChatJid: chatJID}
		var mediaBlob, quoteBlob []byte
		if err := rows.Scan(&m.Id, &m.SenderJid, &m.FromMe, &m.TimestampMs, &m.Text, &m.Status, &mediaBlob, &quoteBlob); err != nil {
			return nil, err
		}
		m.Media = a.ipcMedia(mediaBlob)
		m.Quote = a.quoteFromBlob(ctx, quoteBlob)
		if !m.FromMe {
			name, ok := names[m.SenderJid]
			if !ok {
				if jid, err := types.ParseJID(m.SenderJid); err == nil {
					name = a.contactName(ctx, jid)
				}
				names[m.SenderJid] = name
			}
			m.SenderName = name
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
	return a.send(ctx, chat, &waE2E.Message{Conversation: proto.String(text)})
}

// send delivers content to a chat and stores it like a received message.
func (a *account) send(ctx context.Context, chat string, content *waE2E.Message) (*ipc.Message, *ipc.Chat, error) {
	if !a.client.IsLoggedIn() {
		return nil, nil, errors.New("account is not connected")
	}
	to, err := types.ParseJID(chat)
	if err != nil {
		return nil, nil, err
	}
	// Own messages follow the chat's timer, as the other members expect.
	if timer := a.disappearing(ctx, chat); timer > 0 {
		content = withExpiration(content, timer)
	}
	resp, err := a.client.SendMessage(ctx, to, content)
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
		Message: content,
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
	c, err := a.scanChat(a.db.QueryRowContext(ctx, chatQuery+` WHERE jid = ?`, chatJID))
	if err != nil {
		return nil, err
	}
	return a.named(ctx, c), nil
}

// receiptStatus maps a receipt from a contact to the status it proves, or
// unspecified for receipts that say nothing about own messages.
func receiptStatus(t types.ReceiptType) ipc.MessageStatus {
	switch t {
	case types.ReceiptTypeDelivered:
		return ipc.MessageStatus_MESSAGE_STATUS_DELIVERED
	case types.ReceiptTypeRead, types.ReceiptTypePlayed:
		return ipc.MessageStatus_MESSAGE_STATUS_READ
	}
	return ipc.MessageStatus_MESSAGE_STATUS_UNSPECIFIED
}

// advanceStatus moves own messages forward to status and returns the ids
// that changed, by chat. Statuses never go back.
func (a *account) advanceStatus(ctx context.Context, ids []types.MessageID, status ipc.MessageStatus) (map[string][]string, error) {
	if len(ids) == 0 {
		return nil, nil
	}
	args := []any{status, status}
	for _, id := range ids {
		args = append(args, id)
	}
	// Message ids are random and unique enough to match without the chat,
	// whose id may differ between phone-number and hidden (LID) forms.
	rows, err := a.db.QueryContext(ctx, `UPDATE wassup_messages SET status = ?
		WHERE from_me = 1 AND status < ? AND id IN (?`+strings.Repeat(", ?", len(ids)-1)+`)
		RETURNING chat_jid, id`, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	changed := map[string][]string{}
	for rows.Next() {
		var chat, id string
		if err := rows.Scan(&chat, &id); err != nil {
			return nil, err
		}
		changed[chat] = append(changed[chat], id)
	}
	return changed, rows.Err()
}
