package main

import (
	"context"
	"crypto/pbkdf2"
	"crypto/sha512"
	"crypto/subtle"
	"encoding/base64"
	"errors"
	"hash"

	"github.com/MawCeron/Wassup/daemon/ipc"
	"go.mau.fi/whatsmeow/proto/waChatLockSettings"
	"go.mau.fi/whatsmeow/proto/waUserPassword"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
)

var (
	errNoSecretCode    = errors.New("set a secret code for locked chats on the phone first")
	errWrongSecretCode = errors.New("wrong secret code")
)

// setLocked records whether the phone locked a chat. The lock may name the
// chat by phone number or by hidden id (LID) while messages use the other,
// so both are recorded.
func (a *account) setLocked(ctx context.Context, chatJID string, locked bool) error {
	ids := []string{chatJID}
	if jid, err := types.ParseJID(chatJID); err == nil && a.client.Store.LIDs != nil {
		var other types.JID
		if jid.Server == types.HiddenUserServer {
			other, _ = a.client.Store.LIDs.GetPNForLID(ctx, jid)
		} else if jid.Server == types.DefaultUserServer {
			other, _ = a.client.Store.LIDs.GetLIDForPN(ctx, jid)
		}
		if !other.IsEmpty() {
			ids = append(ids, other.String())
		}
	}
	for _, id := range ids {
		query := `DELETE FROM wassup_locked_chats WHERE jid = ?`
		if locked {
			query = `INSERT OR IGNORE INTO wassup_locked_chats (jid) VALUES (?)`
		}
		if _, err := a.db.ExecContext(ctx, query, id); err != nil {
			return err
		}
	}
	return nil
}

// saveLockSettings keeps the phone's secret code, as the salted hash the
// phone sends; the code itself is never known.
func (a *account) saveLockSettings(ctx context.Context, s *waChatLockSettings.ChatLockSettings) error {
	blob, err := proto.Marshal(s.GetSecretCode())
	if err != nil {
		return err
	}
	_, err = a.db.ExecContext(ctx, `INSERT OR REPLACE INTO wassup_meta (key, value) VALUES ('chat_lock_secret', ?)`,
		base64.StdEncoding.EncodeToString(blob))
	return err
}

// unlockChats checks a secret code against the phone's.
func (a *account) unlockChats(ctx context.Context, code string) error {
	var stored string
	a.db.QueryRowContext(ctx, `SELECT value FROM wassup_meta WHERE key = 'chat_lock_secret'`).Scan(&stored)
	blob, err := base64.StdEncoding.DecodeString(stored)
	var secret waUserPassword.UserPassword
	if err != nil || proto.Unmarshal(blob, &secret) != nil || len(secret.GetTransformedData()) == 0 {
		return errNoSecretCode
	}
	ok, err := secretMatches(&secret, code)
	if err != nil {
		var names []string
		for _, arg := range secret.GetTransformerArg() {
			names = append(names, arg.GetKey())
		}
		a.log.Warn("secret code format", "transformer", secret.GetTransformer(), "args", names, "err", err)
		return err
	}
	if !ok {
		return errWrongSecretCode
	}
	return nil
}

// secretMatches derives the hash of code the way the phone did and
// compares it in constant time.
func secretMatches(secret *waUserPassword.UserPassword, code string) (bool, error) {
	want := secret.GetTransformedData()
	var newHash func() hash.Hash
	switch secret.GetTransformer() {
	case waUserPassword.UserPassword_NONE:
		return subtle.ConstantTimeCompare([]byte(code), want) == 1, nil
	case waUserPassword.UserPassword_PBKDF2_HMAC_SHA512:
		newHash = sha512.New
	case waUserPassword.UserPassword_PBKDF2_HMAC_SHA384:
		newHash = sha512.New384
	default:
		return false, errors.New("unknown secret code format")
	}
	// The argument names are not documented; the salt is the one blob and
	// the iteration count the one number.
	var salt []byte
	var iterations int
	for _, arg := range secret.GetTransformerArg() {
		switch v := arg.GetValue(); {
		case v.GetAsBlob() != nil:
			salt = v.GetAsBlob()
		case v.GetAsUnsignedInteger() > 0:
			iterations = int(v.GetAsUnsignedInteger())
		}
	}
	if salt == nil || iterations == 0 {
		return false, errors.New("secret code without salt or iteration count")
	}
	got, err := pbkdf2.Key(newHash, code, salt, iterations, len(want))
	if err != nil {
		return false, err
	}
	return subtle.ConstantTimeCompare(got, want) == 1, nil
}

// lockedChat returns the chat to show for a lock change, if it is known.
func (a *account) lockedChat(ctx context.Context, chatJID string) *ipc.Chat {
	chat, err := a.scanChat(a.db.QueryRowContext(ctx, chatQuery+` WHERE jid = ?`, chatJID))
	if err != nil {
		return nil
	}
	return a.named(ctx, chat)
}
