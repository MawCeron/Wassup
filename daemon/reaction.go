package main

import (
	"context"
	"errors"
	"sort"
	"strings"

	"github.com/MawCeron/Wassup/daemon/ipc"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
)

// onReaction stores a reaction, or its removal, and returns the message's
// reactions now; nil if the event is not a reaction.
func (a *account) onReaction(ctx context.Context, e *events.Message) *ipc.ReactionsChanged {
	r := e.Message.GetReactionMessage()
	if r == nil || r.GetKey().GetID() == "" {
		return nil
	}
	chat, msgID := e.Info.Chat.String(), r.GetKey().GetID()
	if err := a.setReaction(ctx, chat, msgID, e.Info.Sender.ToNonAD().String(), r.GetText()); err != nil {
		a.log.Warn("save reaction", "err", err)
		return nil
	}
	return a.reactionsChanged(ctx, chat, msgID)
}

// setReaction records a sender's reaction to a message; an empty emoji
// takes it back. Each sender has at most one reaction per message.
func (a *account) setReaction(ctx context.Context, chat, msgID, sender, emoji string) error {
	if emoji == "" {
		_, err := a.db.ExecContext(ctx, `DELETE FROM wassup_reactions
			WHERE chat_jid = ? AND message_id = ? AND sender_jid = ?`, chat, msgID, sender)
		return err
	}
	_, err := a.db.ExecContext(ctx, `INSERT OR REPLACE INTO wassup_reactions
		(chat_jid, message_id, sender_jid, emoji) VALUES (?, ?, ?, ?)`, chat, msgID, sender, emoji)
	return err
}

func (a *account) reactionsChanged(ctx context.Context, chat, msgID string) *ipc.ReactionsChanged {
	all, err := a.reactions(ctx, chat, []string{msgID})
	if err != nil {
		a.log.Warn("read reactions", "err", err)
	}
	return &ipc.ReactionsChanged{AccountId: a.id, ChatJid: chat, MessageId: msgID, Reactions: all[msgID]}
}

// reactions groups the reactions to some messages of a chat by emoji, most
// used first.
func (a *account) reactions(ctx context.Context, chat string, ids []string) (map[string][]*ipc.Reaction, error) {
	if len(ids) == 0 {
		return nil, nil
	}
	args := []any{chat}
	for _, id := range ids {
		args = append(args, id)
	}
	rows, err := a.db.QueryContext(ctx, `SELECT message_id, emoji, sender_jid FROM wassup_reactions
		WHERE chat_jid = ? AND message_id IN (?`+strings.Repeat(", ?", len(ids)-1)+`)`, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := map[string][]*ipc.Reaction{}
	for rows.Next() {
		var id, emoji, sender string
		if err := rows.Scan(&id, &emoji, &sender); err != nil {
			return nil, err
		}
		var r *ipc.Reaction
		for _, existing := range out[id] {
			if existing.Emoji == emoji {
				r = existing
			}
		}
		if r == nil {
			r = &ipc.Reaction{Emoji: emoji}
			out[id] = append(out[id], r)
		}
		r.Count++
		r.Mine = r.Mine || a.isOwn(sender)
	}
	for _, list := range out {
		sort.SliceStable(list, func(i, j int) bool { return list[i].Count > list[j].Count })
	}
	return out, rows.Err()
}

// isOwn reports whether a user id is this account, by phone number or LID.
func (a *account) isOwn(jid string) bool {
	user, _, _ := strings.Cut(jid, "@")
	user, _, _ = strings.Cut(user, ":")
	store := a.client.Store
	return user != "" && ((store.ID != nil && store.ID.User == user) || store.LID.User == user)
}

// react sends the user's reaction to a message, or takes it back with an
// empty emoji, and returns the message's reactions now.
func (a *account) react(ctx context.Context, chat, msgID, emoji string) (*ipc.ReactionsChanged, error) {
	if !a.client.IsLoggedIn() {
		return nil, errors.New("account is not connected")
	}
	var sender string
	var fromMe bool
	err := a.db.QueryRowContext(ctx, `SELECT sender_jid, from_me FROM wassup_messages
		WHERE chat_jid = ? AND id = ?`, chat, msgID).Scan(&sender, &fromMe)
	if err != nil {
		return nil, err
	}
	chatJID, err := types.ParseJID(chat)
	if err != nil {
		return nil, err
	}
	senderJID := *a.client.Store.ID
	if !fromMe {
		if senderJID, err = types.ParseJID(sender); err != nil {
			return nil, err
		}
	}
	if _, err := a.client.SendMessage(ctx, chatJID, a.client.BuildReaction(chatJID, senderJID, msgID, emoji)); err != nil {
		return nil, err
	}
	if err := a.setReaction(ctx, chat, msgID, a.client.Store.ID.ToNonAD().String(), emoji); err != nil {
		return nil, err
	}
	return a.reactionsChanged(ctx, chat, msgID), nil
}
