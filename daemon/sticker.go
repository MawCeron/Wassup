package main

import (
	"context"
	"database/sql"
	"encoding/hex"
	"errors"

	"github.com/MawCeron/Wassup/daemon/ipc"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"google.golang.org/protobuf/proto"
)

// recentStickers is how many stickers the picker offers.
const recentStickers = 48

// listStickers returns the stickers most recently sent or received,
// one per file.
func (a *account) listStickers(ctx context.Context) ([]*ipc.Sticker, error) {
	rows, err := a.db.QueryContext(ctx, `SELECT chat_jid, id, media FROM wassup_messages
		WHERE media IS NOT NULL ORDER BY timestamp_ms DESC`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	seen := map[string]bool{}
	var stickers []*ipc.Sticker
	for rows.Next() && len(stickers) < recentStickers {
		var chat, id string
		var blob []byte
		if err := rows.Scan(&chat, &id, &blob); err != nil {
			return nil, err
		}
		var m waE2E.Message
		if proto.Unmarshal(blob, &m) != nil || m.GetStickerMessage() == nil {
			continue
		}
		key := hex.EncodeToString(m.GetStickerMessage().GetFileSHA256())
		if seen[key] {
			continue
		}
		seen[key] = true
		media := a.ipcMedia(blob)
		stickers = append(stickers, &ipc.Sticker{ChatJid: chat, MessageId: id, Path: media.GetPath()})
	}
	return stickers, rows.Err()
}

// sendSticker sends a sticker from an earlier message again. The file is
// already on WhatsApp's servers, so nothing is uploaded.
func (a *account) sendSticker(ctx context.Context, chat, sourceChat, sourceID string) (*ipc.Message, *ipc.Chat, error) {
	var blob []byte
	err := a.db.QueryRowContext(ctx, `SELECT media FROM wassup_messages WHERE chat_jid = ? AND id = ?`,
		sourceChat, sourceID).Scan(&blob)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil, errors.New("unknown sticker")
	} else if err != nil {
		return nil, nil, err
	}
	var m waE2E.Message
	if err := proto.Unmarshal(blob, &m); err != nil || m.GetStickerMessage() == nil {
		return nil, nil, errors.New("not a sticker")
	}
	sticker := proto.Clone(m.GetStickerMessage()).(*waE2E.StickerMessage)
	sticker.ContextInfo = nil // Not a reply or forward of the original.
	return a.send(ctx, chat, &waE2E.Message{StickerMessage: sticker})
}
