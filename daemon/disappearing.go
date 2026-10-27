package main

import (
	"context"
	"log/slog"
	"os"
	"time"

	"github.com/MawCeron/Wassup/daemon/ipc"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"google.golang.org/protobuf/proto"
)

// sweepInterval is how often expired disappearing messages are deleted.
const sweepInterval = time.Minute

// expirationOf is the disappearing timer a message was sent with, in
// seconds, or 0.
func expirationOf(m *waE2E.Message) uint32 {
	for _, c := range contextsOf(m) {
		if e := c.GetExpiration(); e > 0 {
			return e
		}
	}
	return 0
}

// withExpiration returns a copy of content that disappears after secs, as
// the chat's other members expect. Plain text has no context to carry the
// timer, so it becomes extended text.
func withExpiration(content *waE2E.Message, secs uint32) *waE2E.Message {
	m := proto.Clone(content).(*waE2E.Message)
	if text := m.GetConversation(); text != "" {
		m.Conversation = nil
		m.ExtendedTextMessage = &waE2E.ExtendedTextMessage{Text: proto.String(text)}
	}
	set := func(c **waE2E.ContextInfo) {
		if *c == nil {
			*c = &waE2E.ContextInfo{}
		}
		(*c).Expiration = proto.Uint32(secs)
	}
	switch {
	case m.ExtendedTextMessage != nil:
		set(&m.ExtendedTextMessage.ContextInfo)
	case m.ImageMessage != nil:
		set(&m.ImageMessage.ContextInfo)
	case m.DocumentMessage != nil:
		set(&m.DocumentMessage.ContextInfo)
	case m.AudioMessage != nil:
		set(&m.AudioMessage.ContextInfo)
	case m.StickerMessage != nil:
		set(&m.StickerMessage.ContextInfo)
	}
	return m
}

// disappearing is a chat's current timer in seconds; 0 when off or unknown.
func (a *account) disappearing(ctx context.Context, chatJID string) uint32 {
	var secs uint32
	a.db.QueryRowContext(ctx, `SELECT disappearing_seconds FROM wassup_chats WHERE jid = ?`, chatJID).Scan(&secs)
	return secs
}

func (a *account) setDisappearing(ctx context.Context, chatJID string, secs uint32) {
	_, err := a.db.ExecContext(ctx, `UPDATE wassup_chats SET disappearing_seconds = ? WHERE jid = ?`, secs, chatJID)
	if err != nil {
		a.log.Warn("disappearing timer", "chat", chatJID, "err", err)
	}
}

// expire deletes messages whose time ran out, with their cached media, and
// returns their ids by chat. Stickers stay cached: they are shared with
// other chats and favorites and say nothing about the conversation.
func (a *account) expire(ctx context.Context, now time.Time) (map[string][]string, error) {
	rows, err := a.db.QueryContext(ctx, `SELECT chat_jid, id, media, link FROM wassup_messages
		WHERE expires_at_ms > 0 AND expires_at_ms <= ?`, now.UnixMilli())
	if err != nil {
		return nil, err
	}
	expired := map[string][]string{}
	var files []string
	for rows.Next() {
		var chat, id string
		var blob, link []byte
		if err := rows.Scan(&chat, &id, &blob, &link); err != nil {
			rows.Close()
			return nil, err
		}
		expired[chat] = append(expired[chat], id)
		var preview waE2E.ExtendedTextMessage
		if link != nil && proto.Unmarshal(link, &preview) == nil {
			if path := a.linkThumbPath(preview.GetJPEGThumbnail()); path != "" {
				files = append(files, path)
			}
		}
		var m waE2E.Message
		if blob != nil && proto.Unmarshal(blob, &m) == nil && m.GetStickerMessage() == nil {
			if path := a.mediaPath(&m); path != "" {
				files = append(files, path, a.thumbPath(&m))
			}
		}
	}
	rows.Close()
	if err := rows.Err(); err != nil {
		return nil, err
	}
	if len(expired) == 0 {
		return nil, nil
	}
	_, err = a.db.ExecContext(ctx, `DELETE FROM wassup_messages WHERE expires_at_ms > 0 AND expires_at_ms <= ?`, now.UnixMilli())
	if err != nil {
		return nil, err
	}
	for _, f := range files {
		if err := os.Remove(f); err != nil && !os.IsNotExist(err) {
			a.log.Warn("delete expired media", "err", err)
		}
	}
	return expired, nil
}

// sweepExpired deletes expired messages now and then every sweepInterval,
// and tells the UIs which ones went.
func (d *daemon) sweepExpired(ctx context.Context) {
	for {
		for _, a := range d.accounts {
			expired, err := a.expire(ctx, time.Now())
			if err != nil {
				slog.Warn("expire messages", "account", a.id, "err", err)
			}
			for chat, ids := range expired {
				a.log.Info("disappearing messages expired", "chat", chat, "count", len(ids))
				d.broadcast(&ipc.Event{Kind: &ipc.Event_MessagesExpired{MessagesExpired: &ipc.MessagesExpired{
					AccountId: a.id, ChatJid: chat, MessageIds: ids,
				}}})
			}
		}
		select {
		case <-ctx.Done():
			return
		case <-time.After(sweepInterval):
		}
	}
}

// isEphemeralSetting reports whether a message only announces a chat's new
// disappearing timer, as one-to-one chats do.
func isEphemeralSetting(m *waE2E.Message) bool {
	return m.GetProtocolMessage().GetType() == waE2E.ProtocolMessage_EPHEMERAL_SETTING
}
