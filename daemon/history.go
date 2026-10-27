package main

import (
	"context"
	"errors"
	"time"

	"github.com/MawCeron/Wassup/daemon/ipc"
	"go.mau.fi/whatsmeow/proto/waHistorySync"
	"go.mau.fi/whatsmeow/types"
)

// historyDays is how much history the phone sends when linking; older
// messages come on demand, when the user scrolls to the top of a chat.
const historyDays = 90

// onDemandCount is how many older messages each on-demand request asks for.
const onDemandCount = 50

// onHistory stores the messages the phone sends: the recent history after
// linking, and older messages asked for on demand. It returns the chats
// that got messages.
func (a *account) onHistory(ctx context.Context, data *waHistorySync.HistorySync) []string {
	initial := data.GetSyncType() == waHistorySync.HistorySync_INITIAL_BOOTSTRAP ||
		data.GetSyncType() == waHistorySync.HistorySync_RECENT
	var changed []string
	for _, conv := range data.GetConversations() {
		chat, err := types.ParseJID(conv.GetID())
		if err != nil {
			continue
		}
		stored := 0
		for _, hm := range conv.GetMessages() {
			evt, err := a.client.ParseWebMessage(chat, hm.GetMessage())
			if err != nil || evt.Message == nil {
				continue
			}
			if isEphemeralSetting(evt.Message) {
				continue
			}
			if a.onReaction(ctx, evt) != nil {
				continue
			}
			if msg, _, err := a.storeMessage(ctx, evt, false); err != nil {
				a.log.Warn("store history message", "chat", conv.GetID(), "err", err)
			} else if msg != nil {
				stored++
			}
		}
		if initial {
			// The phone knows what the user has not read yet.
			if _, err := a.db.ExecContext(ctx, `UPDATE wassup_chats SET unread_count = ? WHERE jid = ?`,
				conv.GetUnreadCount(), chat.String()); err != nil {
				a.log.Warn("history unread count", "err", err)
			}
			if timer := conv.GetEphemeralExpiration(); timer > 0 {
				a.setDisappearing(ctx, chat.String(), timer)
			}
		}
		if stored > 0 {
			changed = append(changed, chat.String())
		}
	}
	a.log.Info("history synced", "type", data.GetSyncType().String(), "chats", len(changed))
	return changed
}

// loadOlder asks the phone for the messages before the oldest one stored
// for a chat. They arrive later, as an on-demand history sync.
func (a *account) loadOlder(ctx context.Context, chatJID string) error {
	if !a.client.IsLoggedIn() {
		return errors.New("account is not connected")
	}
	var id string
	var fromMe bool
	var ms int64
	err := a.db.QueryRowContext(ctx, `SELECT id, from_me, timestamp_ms FROM wassup_messages
		WHERE chat_jid = ? ORDER BY timestamp_ms, id LIMIT 1`, chatJID).Scan(&id, &fromMe, &ms)
	if err != nil {
		return errors.New("no message to load older ones from")
	}
	chat, err := types.ParseJID(chatJID)
	if err != nil {
		return err
	}
	info := &types.MessageInfo{
		MessageSource: types.MessageSource{Chat: chat, IsFromMe: fromMe},
		ID:            id,
		Timestamp:     time.UnixMilli(ms),
	}
	_, err = a.client.SendPeerMessage(ctx, a.client.BuildHistorySyncRequest(info, onDemandCount))
	return err
}

// historyAdded tells the UIs which chats got messages from the history.
func historyAdded(accountID string, chats []string) *ipc.Event {
	return &ipc.Event{Kind: &ipc.Event_HistoryAdded{HistoryAdded: &ipc.HistoryAdded{
		AccountId: accountID, ChatJids: chats,
	}}}
}
