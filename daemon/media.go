package main

import (
	"context"
	"database/sql"
	"encoding/hex"
	"errors"
	"image"
	_ "image/jpeg" // For image.DecodeConfig.
	_ "image/png"
	"io"
	"io/fs"
	"log/slog"
	"mime"
	"net/http"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"time"

	"github.com/MawCeron/Wassup/daemon/ipc"
	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"google.golang.org/protobuf/proto"
)

const (
	// defaultMediaCacheMB bounds all cached media unless the config says
	// otherwise; the least recently used files go first. A removed file is
	// downloaded again when shown.
	defaultMediaCacheMB = 500
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
	case m.GetDocumentMessage() != nil:
		return &waE2E.Message{DocumentMessage: m.GetDocumentMessage()}
	case m.GetAudioMessage() != nil:
		return &waE2E.Message{AudioMessage: m.GetAudioMessage()}
	case m.GetVideoMessage() != nil:
		return &waE2E.Message{VideoMessage: m.GetVideoMessage()}
	}
	return nil
}

// downloadable is the file inside a message from mediaOf, with the file
// extension to cache it under.
func downloadable(m *waE2E.Message) (whatsmeow.DownloadableMessage, string) {
	switch {
	case m.GetImageMessage() != nil:
		return m.GetImageMessage(), extension("", m.GetImageMessage().GetMimetype())
	case m.GetDocumentMessage() != nil:
		d := m.GetDocumentMessage()
		return d, extension(d.GetFileName(), d.GetMimetype())
	case m.GetAudioMessage() != nil:
		return m.GetAudioMessage(), extension("", m.GetAudioMessage().GetMimetype())
	case m.GetVideoMessage() != nil:
		return m.GetVideoMessage(), extension("", m.GetVideoMessage().GetMimetype())
	}
	return m.GetStickerMessage(), ".webp"
}

// extension picks a file extension so the desktop opens cached files with
// the right application. The sender controls the name, so only short
// alphanumeric extensions are kept.
func extension(fileName, mimeType string) string {
	ext := filepath.Ext(fileName)
	if len(ext) > 1 && len(ext) <= 10 && strings.TrimLeft(strings.ToLower(ext[1:]), "abcdefghijklmnopqrstuvwxyz0123456789") == "" {
		return strings.ToLower(ext)
	}
	if exts, _ := mime.ExtensionsByType(mimeType); len(exts) > 0 {
		return exts[0]
	}
	return ""
}

// mediaPath names cached files after their content hash, so the same file
// forwarded to several chats is stored once.
func (a *account) mediaPath(m *waE2E.Message) string {
	if a.mediaDir == "" {
		return ""
	}
	file, ext := downloadable(m)
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
	media := &ipc.Media{}
	switch {
	case m.GetStickerMessage() != nil:
		s := m.GetStickerMessage()
		media.Kind, media.Width, media.Height, media.Size = ipc.MediaKind_MEDIA_KIND_STICKER, s.GetWidth(), s.GetHeight(), s.GetFileLength()
	case m.GetDocumentMessage() != nil:
		d := m.GetDocumentMessage()
		media.Kind, media.FileName, media.Size = ipc.MediaKind_MEDIA_KIND_DOCUMENT, d.GetFileName(), d.GetFileLength()
	case m.GetAudioMessage() != nil:
		au := m.GetAudioMessage()
		media.Kind, media.Size = ipc.MediaKind_MEDIA_KIND_AUDIO, au.GetFileLength()
		media.DurationSeconds, media.VoiceNote = au.GetSeconds(), au.GetPTT()
	case m.GetVideoMessage() != nil:
		v := m.GetVideoMessage()
		media.Kind, media.Width, media.Height, media.Size = ipc.MediaKind_MEDIA_KIND_VIDEO, v.GetWidth(), v.GetHeight(), v.GetFileLength()
		media.DurationSeconds = v.GetSeconds()
		media.ThumbnailPath = a.videoThumbnail(&m)
	default:
		i := m.GetImageMessage()
		media.Kind, media.Width, media.Height, media.Size = ipc.MediaKind_MEDIA_KIND_IMAGE, i.GetWidth(), i.GetHeight(), i.GetFileLength()
	}
	if path := a.mediaPath(&m); path != "" {
		if _, err := os.Stat(path); err == nil {
			media.Path = path
		}
	}
	return media
}

// thumbPath is where a video's preview image is cached, next to the video.
func (a *account) thumbPath(m *waE2E.Message) string {
	path := a.mediaPath(m)
	if path == "" {
		return ""
	}
	return strings.TrimSuffix(path, filepath.Ext(path)) + ".thumb.jpg"
}

// videoThumbnail writes the small JPEG that comes inside a video message to
// the cache, once, and returns its path; "" if the message has none.
func (a *account) videoThumbnail(m *waE2E.Message) string {
	thumb := m.GetVideoMessage().GetJPEGThumbnail()
	path := a.thumbPath(m)
	if len(thumb) == 0 || path == "" {
		return ""
	}
	if _, err := os.Stat(path); err == nil {
		return path
	}
	tmp, err := os.CreateTemp(a.mediaDir, ".thumb-*")
	if err != nil {
		return ""
	}
	defer os.Remove(tmp.Name())
	_, err = tmp.Write(thumb)
	if closeErr := tmp.Close(); err != nil || closeErr != nil || os.Rename(tmp.Name(), path) != nil {
		return ""
	}
	return path
}

// inBackground runs a download, at most parallelDownloads at a time, and
// ignores repeated requests for one that is still running.
func (a *account) inBackground(key string, work func()) {
	a.downloads.mu.Lock()
	if a.downloads.inFlight[key] {
		a.downloads.mu.Unlock()
		return
	}
	a.downloads.inFlight[key] = true
	a.downloads.mu.Unlock()

	go func() {
		a.downloads.slots <- struct{}{}
		work()
		<-a.downloads.slots
		a.downloads.mu.Lock()
		delete(a.downloads.inFlight, key)
		a.downloads.mu.Unlock()
	}()
}

// download fetches a message's media in the background and tells every UI
// when it is done.
func (d *daemon) download(a *account, chatJID, msgID string) {
	a.inBackground("media/"+chatKey(chatJID, msgID), func() {
		path, err := a.fetchMedia(context.Background(), chatJID, msgID)
		ready := &ipc.MediaReady{AccountId: a.id, ChatJid: chatJID, MessageId: msgID, Path: path}
		if err != nil {
			a.log.Warn("download media", "id", msgID, "err", err)
			ready.Error = err.Error()
		}
		d.broadcast(&ipc.Event{Kind: &ipc.Event_MediaReady{MediaReady: ready}})
	})
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
	if err := evictMedia(filepath.Dir(a.mediaDir), a.cacheLimit()); err != nil {
		slog.Warn("media cache cleanup", "err", err)
	}
	return path, nil
}

// cacheLimit is the media cache size in bytes.
func (a *account) cacheLimit() int64 {
	mb := a.mediaCacheMB.Load()
	if mb <= 0 {
		mb = defaultMediaCacheMB
	}
	return int64(mb) << 20
}

// removeLeftovers deletes temporary files that an interrupted download,
// upload or write left behind. They all start with a dot.
func removeLeftovers(dirs ...string) {
	for _, dir := range dirs {
		entries, _ := os.ReadDir(dir)
		for _, e := range entries {
			if strings.HasPrefix(e.Name(), ".") && !e.IsDir() {
				os.Remove(filepath.Join(dir, e.Name()))
			}
		}
	}
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

// sendMedia uploads a local file and sends it: JPEG and PNG as a photo,
// anything else as a document, or a WebP file as a sticker if asked.
func (a *account) sendMedia(ctx context.Context, chat, path, caption string, asSticker bool) (*ipc.Message, *ipc.Chat, error) {
	if !a.client.IsLoggedIn() {
		return nil, nil, errors.New("account is not connected")
	}
	f, err := os.Open(path)
	if err != nil {
		return nil, nil, err
	}
	defer f.Close()
	if info, err := f.Stat(); err != nil {
		return nil, nil, err
	} else if !info.Mode().IsRegular() || info.Size() == 0 {
		return nil, nil, errors.New("not a file that can be sent")
	}
	head := make([]byte, 512)
	n, _ := io.ReadFull(f, head)
	sniffed := http.DetectContentType(head[:n])
	photo := !asSticker && (sniffed == "image/jpeg" || sniffed == "image/png")
	video := !asSticker && sniffed == "video/mp4"
	if asSticker && sniffed != "image/webp" {
		return nil, nil, errors.New("stickers must be WebP images")
	}
	mimeType := sniffed
	if byName := mime.TypeByExtension(filepath.Ext(path)); !photo && !video && byName != "" {
		mimeType = byName
	}

	if _, err := f.Seek(0, io.SeekStart); err != nil {
		return nil, nil, err
	}
	// UploadReader encrypts through this file instead of holding it in memory.
	tmp, err := os.CreateTemp(a.mediaDir, ".upload-*")
	if err != nil {
		return nil, nil, err
	}
	defer os.Remove(tmp.Name())
	defer tmp.Close()
	kind := whatsmeow.MediaDocument
	if photo || asSticker {
		kind = whatsmeow.MediaImage // Stickers use the image keys too.
	} else if video {
		kind = whatsmeow.MediaVideo
	}
	up, err := a.client.UploadReader(ctx, f, tmp, kind)
	if err != nil {
		return nil, nil, err
	}

	var text *string
	if caption != "" {
		text = proto.String(caption)
	}
	var content *waE2E.Message
	if asSticker {
		// The UI makes stickers at WhatsApp's fixed size.
		content = &waE2E.Message{StickerMessage: &waE2E.StickerMessage{
			URL: &up.URL, DirectPath: &up.DirectPath, MediaKey: up.MediaKey, Mimetype: proto.String("image/webp"),
			FileEncSHA256: up.FileEncSHA256, FileSHA256: up.FileSHA256, FileLength: &up.FileLength,
			Width: proto.Uint32(512), Height: proto.Uint32(512),
		}}
	} else if photo {
		if _, err := f.Seek(0, io.SeekStart); err != nil {
			return nil, nil, err
		}
		// Only the header is read; decoding the whole photo for a preview
		// would briefly take tens of MB.
		size, _, _ := image.DecodeConfig(f)
		content = &waE2E.Message{ImageMessage: &waE2E.ImageMessage{
			URL: &up.URL, DirectPath: &up.DirectPath, MediaKey: up.MediaKey, Mimetype: &mimeType,
			FileEncSHA256: up.FileEncSHA256, FileSHA256: up.FileSHA256, FileLength: &up.FileLength,
			Width: proto.Uint32(uint32(size.Width)), Height: proto.Uint32(uint32(size.Height)), Caption: text,
		}}
	} else if video {
		info, err := f.Stat()
		if err != nil {
			return nil, nil, err
		}
		seconds, width, height := mp4Info(f, info.Size())
		// TODO: send a thumbnail; it needs a decoded frame, which the daemon cannot make.
		content = &waE2E.Message{VideoMessage: &waE2E.VideoMessage{
			URL: &up.URL, DirectPath: &up.DirectPath, MediaKey: up.MediaKey, Mimetype: &mimeType,
			FileEncSHA256: up.FileEncSHA256, FileSHA256: up.FileSHA256, FileLength: &up.FileLength,
			Seconds: &seconds, Width: &width, Height: &height, Caption: text,
		}}
	} else {
		name := filepath.Base(path)
		content = &waE2E.Message{DocumentMessage: &waE2E.DocumentMessage{
			URL: &up.URL, DirectPath: &up.DirectPath, MediaKey: up.MediaKey, Mimetype: &mimeType,
			FileEncSHA256: up.FileEncSHA256, FileSHA256: up.FileSHA256, FileLength: &up.FileLength,
			FileName: &name, Title: &name, Caption: text,
		}}
	}
	if err := a.cacheCopy(f, content); err != nil {
		a.log.Warn("cache sent media", "err", err)
	}
	return a.send(ctx, chat, content)
}

// cacheCopy keeps a sent file in the cache, so it shows without being
// downloaded back.
func (a *account) cacheCopy(f *os.File, content *waE2E.Message) error {
	dest := a.mediaPath(content)
	if dest == "" {
		return nil
	}
	if _, err := f.Seek(0, io.SeekStart); err != nil {
		return err
	}
	tmp, err := os.CreateTemp(a.mediaDir, ".copy-*")
	if err != nil {
		return err
	}
	defer os.Remove(tmp.Name())
	_, err = io.Copy(tmp, f)
	if closeErr := tmp.Close(); err == nil {
		err = closeErr
	}
	if err == nil {
		err = os.Rename(tmp.Name(), dest)
	}
	if err != nil {
		return err
	}
	return evictMedia(filepath.Dir(a.mediaDir), a.cacheLimit())
}
