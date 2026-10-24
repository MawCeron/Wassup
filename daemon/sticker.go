package main

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"errors"
	"os"
	"path/filepath"
	"time"

	"github.com/MawCeron/Wassup/daemon/ipc"
	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/proto/waSyncAction"
	"google.golang.org/protobuf/proto"
)

const (
	// recentStickers is how many recent stickers the picker offers.
	recentStickers = 48
	// maxFavorites bounds how many favorites are kept and downloaded.
	maxFavorites = 100
)

// listStickers returns the favorite stickers, then the ones most recently
// sent or received, one per file.
func (a *account) listStickers(ctx context.Context) ([]*ipc.Sticker, error) {
	seen := map[string]bool{}
	stickers, err := a.listFavorites(ctx, seen)
	if err != nil {
		return nil, err
	}
	favorites := len(stickers)
	rows, err := a.db.QueryContext(ctx, `SELECT chat_jid, id, media FROM wassup_messages
		WHERE media IS NOT NULL ORDER BY timestamp_ms DESC`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	for rows.Next() && len(stickers)-favorites < recentStickers {
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

// sendSticker sends a sticker from an earlier message, or a favorite,
// again. The file is already on WhatsApp's servers, so nothing is uploaded.
func (a *account) sendSticker(ctx context.Context, chat, sourceChat, sourceID, favoriteID string) (*ipc.Message, *ipc.Chat, error) {
	if favoriteID != "" {
		sticker, err := a.favoriteSticker(ctx, favoriteID)
		if err != nil {
			return nil, nil, err
		}
		return a.send(ctx, chat, &waE2E.Message{StickerMessage: sticker})
	}
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

// onAppState keeps the favorite stickers in step with the phone.
func (a *account) onAppState(ctx context.Context, index []string, value *waSyncAction.SyncActionValue) {
	if len(index) < 2 || index[0] != appstate.IndexFavoriteSticker {
		return
	}
	id, action := index[1], value.GetStickerAction()
	if action == nil || !action.GetIsFavorite() || action.GetIsLottie() {
		if _, err := a.db.ExecContext(ctx, `DELETE FROM wassup_favorite_stickers WHERE id = ?`, id); err != nil {
			a.log.Warn("remove favorite sticker", "err", err)
		}
		return
	}
	blob, err := proto.Marshal(action)
	if err == nil {
		_, err = a.db.ExecContext(ctx, `INSERT INTO wassup_favorite_stickers (id, action, added_ms) VALUES (?, ?, ?)
			ON CONFLICT (id) DO UPDATE SET action = excluded.action`, id, blob, value.GetTimestamp())
	}
	if err != nil {
		a.log.Warn("save favorite sticker", "err", err)
		return
	}
	a.fetchFavorite(id, action)
}

// resyncFavorites replays the whole app state once, to learn the
// favorites the phone already had when this version first connected.
func (a *account) resyncFavorites(ctx context.Context) {
	var done string
	a.db.QueryRowContext(ctx, `SELECT value FROM wassup_meta WHERE key = 'favorites_synced'`).Scan(&done)
	if done != "" {
		return
	}
	for _, name := range appstate.AllPatchNames {
		if err := a.client.FetchAppState(ctx, name, true, false); err != nil {
			a.log.Warn("app state resync", "patch", name, "err", err)
			return // Retried on the next connection.
		}
	}
	var n int
	a.db.QueryRowContext(ctx, `SELECT count(*) FROM wassup_favorite_stickers`).Scan(&n)
	a.log.Info("favorite stickers synced", "count", n)
	a.db.ExecContext(ctx, `INSERT OR REPLACE INTO wassup_meta (key, value) VALUES ('favorites_synced', '1')`)
}

// fetchFavorite downloads a favorite in the background. The sync does not
// carry the decrypted file's hash, so it is computed here: integrity is
// still checked against the encrypted hash and the MAC.
func (a *account) fetchFavorite(id string, action *waSyncAction.StickerAction) {
	a.inBackground("favorite/"+id, func() {
		ctx := context.Background()
		var known []byte
		a.db.QueryRowContext(ctx, `SELECT file_sha256 FROM wassup_favorite_stickers WHERE id = ?`, id).Scan(&known)
		if known != nil {
			if _, err := os.Stat(a.favoritePath(known)); err == nil {
				return
			}
		}
		data, err := a.client.DownloadMediaWithPath(ctx, action.GetDirectPath(), action.GetFileEncSHA256(), nil,
			action.GetMediaKey(), whatsmeow.MediaImage, "", true)
		if err != nil {
			a.log.Warn("download favorite sticker", "id", id, "err", err)
			return
		}
		sum := sha256.Sum256(data)
		if err := writeAtomic(a.mediaDir, a.favoritePath(sum[:]), data); err != nil {
			a.log.Warn("save favorite sticker", "id", id, "err", err)
			return
		}
		a.db.ExecContext(ctx, `UPDATE wassup_favorite_stickers SET file_sha256 = ? WHERE id = ?`, sum[:], id)
	})
}

// favoritePath shares the naming of received stickers, so a favorite that
// was also received is cached once.
func (a *account) favoritePath(fileSHA256 []byte) string {
	if a.mediaDir == "" {
		return ""
	}
	return filepath.Join(a.mediaDir, hex.EncodeToString(fileSHA256)+".webp")
}

func (a *account) listFavorites(ctx context.Context, seen map[string]bool) ([]*ipc.Sticker, error) {
	rows, err := a.db.QueryContext(ctx, `SELECT id, action, file_sha256 FROM wassup_favorite_stickers
		ORDER BY added_ms DESC LIMIT ?`, maxFavorites)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var stickers []*ipc.Sticker
	for rows.Next() {
		var id string
		var blob, hash []byte
		if err := rows.Scan(&id, &blob, &hash); err != nil {
			return nil, err
		}
		s := &ipc.Sticker{FavoriteId: id}
		if hash != nil {
			seen[hex.EncodeToString(hash)] = true
			if path := a.favoritePath(hash); path != "" {
				if _, err := os.Stat(path); err == nil {
					s.Path = path
				}
			}
		}
		if s.Path == "" {
			// Not downloaded yet, or the download failed: try again now
			// that the user wants it.
			var action waSyncAction.StickerAction
			if proto.Unmarshal(blob, &action) == nil {
				a.fetchFavorite(id, &action)
			}
		}
		stickers = append(stickers, s)
	}
	return stickers, rows.Err()
}

func (a *account) favoriteSticker(ctx context.Context, id string) (*waE2E.StickerMessage, error) {
	var blob, hash []byte
	err := a.db.QueryRowContext(ctx, `SELECT action, file_sha256 FROM wassup_favorite_stickers WHERE id = ?`,
		id).Scan(&blob, &hash)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, errors.New("unknown favorite sticker")
	} else if err != nil {
		return nil, err
	}
	if hash == nil {
		return nil, errors.New("favorite sticker is still downloading")
	}
	var action waSyncAction.StickerAction
	if err := proto.Unmarshal(blob, &action); err != nil {
		return nil, err
	}
	return &waE2E.StickerMessage{
		URL: proto.String(action.GetURL()), DirectPath: proto.String(action.GetDirectPath()),
		MediaKey: action.GetMediaKey(), Mimetype: proto.String(action.GetMimetype()),
		FileEncSHA256: action.GetFileEncSHA256(), FileSHA256: hash, FileLength: proto.Uint64(action.GetFileLength()),
		Width: proto.Uint32(action.GetWidth()), Height: proto.Uint32(action.GetHeight()),
		StickerSentTS: proto.Int64(time.Now().UnixMilli()),
	}, nil
}

// writeAtomic writes data next to dest and renames it into place.
func writeAtomic(dir, dest string, data []byte) error {
	tmp, err := os.CreateTemp(dir, ".write-*")
	if err != nil {
		return err
	}
	defer os.Remove(tmp.Name())
	_, err = tmp.Write(data)
	if closeErr := tmp.Close(); err == nil {
		err = closeErr
	}
	if err != nil {
		return err
	}
	return os.Rename(tmp.Name(), dest)
}
