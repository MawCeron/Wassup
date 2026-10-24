package main

import (
	"context"
	"database/sql"
	"encoding/hex"
	"errors"
	"io/fs"
	"log/slog"
	"os"
	"path/filepath"
	"slices"
	"sync"
	"time"

	"github.com/MawCeron/Wassup/daemon/ipc"
	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"google.golang.org/protobuf/proto"
)

const (
	// mediaCacheLimit bounds all cached media; the least recently used
	// files go first. A removed file is downloaded again when shown.
	mediaCacheLimit = 500 << 20
	// Downloads running at once per account.
	parallelDownloads = 3
)

// downloads tracks an account's running downloads.
type downloads struct {
	mu       sync.Mutex
	inFlight map[string]bool
	slots    chan struct{}
}

func newDownloads() *downloads {
	return &downloads{inFlight: map[string]bool{}, slots: make(chan struct{}, parallelDownloads)}
}

// mediaOf returns the part of a message needed to download its media, as a
// message holding only that part, or nil when there is nothing to download.
func mediaOf(m *waE2E.Message) *waE2E.Message {
	switch {
	case m.GetImageMessage() != nil:
		return &waE2E.Message{ImageMessage: m.GetImageMessage()}
	case m.GetStickerMessage() != nil:
		return &waE2E.Message{StickerMessage: m.GetStickerMessage()}
	}
	return nil
}

// downloadable is the image or sticker inside a message from mediaOf.
func downloadable(m *waE2E.Message) (whatsmeow.DownloadableMessage, string) {
	if img := m.GetImageMessage(); img != nil {
		return img, img.GetMimetype()
	}
	return m.GetStickerMessage(), m.GetStickerMessage().GetMimetype()
}

// mediaPath names cached files after their content hash, so the same file
// forwarded to several chats is stored once.
func (a *account) mediaPath(m *waE2E.Message) string {
	if a.mediaDir == "" {
		return ""
	}
	file, mime := downloadable(m)
	ext := map[string]string{"image/jpeg": ".jpg", "image/png": ".png", "image/webp": ".webp"}[mime]
	return filepath.Join(a.mediaDir, hex.EncodeToString(file.GetFileSHA256())+ext)
}

// ipcMedia describes stored media for the UI; blob is the media column.
func (a *account) ipcMedia(blob []byte) *ipc.Media {
	if blob == nil {
		return nil
	}
	var m waE2E.Message
	if err := proto.Unmarshal(blob, &m); err != nil {
		return nil
	}
	media := &ipc.Media{Kind: ipc.MediaKind_MEDIA_KIND_IMAGE}
	if s := m.GetStickerMessage(); s != nil {
		media.Kind = ipc.MediaKind_MEDIA_KIND_STICKER
		media.Width, media.Height = s.GetWidth(), s.GetHeight()
	} else {
		media.Width, media.Height = m.GetImageMessage().GetWidth(), m.GetImageMessage().GetHeight()
	}
	if path := a.mediaPath(&m); path != "" {
		if _, err := os.Stat(path); err == nil {
			media.Path = path
		}
	}
	return media
}

// download fetches a message's media in the background and tells every UI
// when it is done. Repeated requests for a running download are ignored.
func (d *daemon) download(a *account, chatJID, msgID string) {
	key := chatKey(chatJID, msgID)
	a.downloads.mu.Lock()
	if a.downloads.inFlight[key] {
		a.downloads.mu.Unlock()
		return
	}
	a.downloads.inFlight[key] = true
	a.downloads.mu.Unlock()

	go func() {
		a.downloads.slots <- struct{}{}
		path, err := a.fetchMedia(context.Background(), chatJID, msgID)
		<-a.downloads.slots
		a.downloads.mu.Lock()
		delete(a.downloads.inFlight, key)
		a.downloads.mu.Unlock()

		ready := &ipc.MediaReady{AccountId: a.id, ChatJid: chatJID, MessageId: msgID, Path: path}
		if err != nil {
			a.log.Warn("download media", "id", msgID, "err", err)
			ready.Error = err.Error()
		}
		d.broadcast(&ipc.Event{Kind: &ipc.Event_MediaReady{MediaReady: ready}})
	}()
}

func (a *account) fetchMedia(ctx context.Context, chatJID, msgID string) (string, error) {
	var blob []byte
	err := a.db.QueryRowContext(ctx, `SELECT media FROM wassup_messages WHERE chat_jid = ? AND id = ?`,
		chatJID, msgID).Scan(&blob)
	if errors.Is(err, sql.ErrNoRows) || (err == nil && blob == nil) {
		return "", errors.New("message has no media")
	} else if err != nil {
		return "", err
	}
	var m waE2E.Message
	if err := proto.Unmarshal(blob, &m); err != nil {
		return "", err
	}
	path := a.mediaPath(&m)
	if path == "" {
		return "", errors.New("no media cache directory")
	}
	now := time.Now()
	if err := os.Chtimes(path, now, now); err == nil {
		return path, nil // Cached; marked as recently used.
	}

	// Written aside and renamed, so a cancelled download never looks complete.
	tmp, err := os.CreateTemp(a.mediaDir, ".download-*")
	if err != nil {
		return "", err
	}
	defer os.Remove(tmp.Name())
	file, _ := downloadable(&m)
	err = a.client.DownloadToFile(ctx, file, tmp)
	if closeErr := tmp.Close(); err == nil {
		err = closeErr
	}
	if err != nil {
		return "", err
	}
	if err := os.Rename(tmp.Name(), path); err != nil {
		return "", err
	}
	if err := evictMedia(filepath.Dir(a.mediaDir), mediaCacheLimit); err != nil {
		slog.Warn("media cache cleanup", "err", err)
	}
	return path, nil
}

// evictMedia deletes the least recently used files under root until they
// fit in limit bytes.
func evictMedia(root string, limit int64) error {
	type entry struct {
		path string
		size int64
		used time.Time
	}
	var files []entry
	var total int64
	err := filepath.WalkDir(root, func(path string, d fs.DirEntry, err error) error {
		if err != nil || d.IsDir() {
			return err
		}
		info, err := d.Info()
		if err != nil {
			return err
		}
		files = append(files, entry{path, info.Size(), info.ModTime()})
		total += info.Size()
		return nil
	})
	if err != nil || total <= limit {
		return err
	}
	slices.SortFunc(files, func(x, y entry) int { return x.used.Compare(y.used) })
	for _, f := range files {
		if total <= limit {
			break
		}
		if err := os.Remove(f.path); err != nil {
			return err
		}
		total -= f.size
	}
	return nil
}
