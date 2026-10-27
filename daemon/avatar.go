package main

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"time"

	"github.com/MawCeron/Wassup/daemon/ipc"
	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/types"
)

const (
	// avatarRecheck is how long a picture, or its absence, is trusted before
	// asking the server again. Picture change events refresh it sooner.
	avatarRecheck = 24 * time.Hour
	// Pictures are small previews; anything bigger is not one.
	avatarMaxBytes = 1 << 20
)

var avatarClient = &http.Client{Timeout: 30 * time.Second}

// avatarPath names a chat's cached picture after the chat and the picture
// id, so a new picture never shows an old file.
func (a *account) avatarPath(chatJID, id string) string {
	if a.avatarDir == "" || id == "" {
		return ""
	}
	sum := sha256.Sum256([]byte(chatJID))
	return filepath.Join(a.avatarDir, hex.EncodeToString(sum[:8])+"-"+hex.EncodeToString([]byte(id))+".jpg")
}

// fetchAvatar checks a chat's picture in the background and tells every UI
// when a new one is cached.
func (d *daemon) fetchAvatar(a *account, chatJID string) {
	a.inBackground("avatar/"+chatJID, func() {
		ctx := context.Background()
		changed, err := a.refreshAvatar(ctx, chatJID)
		if err != nil {
			a.log.Warn("fetch avatar", "chat", chatJID, "err", err)
			return
		}
		if !changed {
			return
		}
		chat, err := a.scanChat(ctx, a.db.QueryRowContext(ctx, chatQuery+` WHERE jid = ?`, chatJID))
		if err != nil {
			a.log.Warn("fetch avatar", "chat", chatJID, "err", err)
			return
		}
		d.broadcast(&ipc.Event{Kind: &ipc.Event_ChatChanged{ChatChanged: a.named(ctx, chat)}})
	})
}

// refreshAvatar asks the server for a chat's picture unless it was checked
// recently, and reports whether the cached picture changed.
func (a *account) refreshAvatar(ctx context.Context, chatJID string) (bool, error) {
	if !a.client.IsLoggedIn() {
		return false, nil
	}
	var known string
	var checked int64
	err := a.db.QueryRowContext(ctx, `SELECT avatar_id, avatar_checked_ms FROM wassup_chats WHERE jid = ?`,
		chatJID).Scan(&known, &checked)
	if errors.Is(err, sql.ErrNoRows) {
		return false, nil // A contact without a chat here.
	} else if err != nil {
		return false, err
	}
	cached := a.avatarPath(chatJID, known)
	if _, err := os.Stat(cached); err != nil {
		known, cached = "", "" // The file is gone; fetch it again.
	}
	if time.Since(time.UnixMilli(checked)) < avatarRecheck && (known == "" || cached != "") {
		return false, nil
	}

	jid, err := types.ParseJID(chatJID)
	if err != nil {
		return false, err
	}
	// With the known id the server only answers if the picture changed.
	info, err := a.client.GetProfilePictureInfo(ctx, jid, &whatsmeow.GetProfilePictureParams{Preview: true, ExistingID: known})
	id := known
	switch {
	case errors.Is(err, whatsmeow.ErrProfilePictureNotSet), errors.Is(err, whatsmeow.ErrProfilePictureUnauthorized):
		id = ""
	case err != nil:
		return false, err
	case info != nil:
		if err := a.saveAvatar(info.URL, a.avatarPath(chatJID, info.ID)); err != nil {
			return false, err
		}
		id = info.ID
	}
	if id != known && cached != "" {
		os.Remove(cached)
	}
	_, err = a.db.ExecContext(ctx, `UPDATE wassup_chats SET avatar_id = ?, avatar_checked_ms = ? WHERE jid = ?`,
		id, time.Now().UnixMilli(), chatJID)
	return id != known, err
}

// forgetAvatar makes the next request ask the server again; called when a
// chat's picture changes.
func (a *account) forgetAvatar(ctx context.Context, chatJID string) error {
	_, err := a.db.ExecContext(ctx, `UPDATE wassup_chats SET avatar_checked_ms = 0 WHERE jid = ?`, chatJID)
	return err
}

func (a *account) saveAvatar(url, dest string) error {
	resp, err := avatarClient.Get(url)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("picture download: %s", resp.Status)
	}
	tmp, err := os.CreateTemp(a.avatarDir, ".avatar-*")
	if err != nil {
		return err
	}
	defer os.Remove(tmp.Name())
	n, err := io.Copy(tmp, io.LimitReader(resp.Body, avatarMaxBytes+1))
	if closeErr := tmp.Close(); err == nil {
		err = closeErr
	}
	if err == nil && n > avatarMaxBytes {
		err = errors.New("picture is too big")
	}
	if err != nil {
		return err
	}
	return os.Rename(tmp.Name(), dest)
}
