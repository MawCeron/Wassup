package main

import (
	"context"
	"crypto/rand"
	"database/sql"
	"errors"
	"fmt"
	"log/slog"
	"os"
	"path/filepath"

	"github.com/MawCeron/Wassup/daemon/ipc"
	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/store/sqlstore"
	waLog "go.mau.fi/whatsmeow/util/log"
	_ "modernc.org/sqlite"
)

// account owns one WhatsApp session and its database file.
type account struct {
	id     string
	db     *sql.DB
	client *whatsmeow.Client
}

// openAccounts opens every account under dataDir/accounts.
func openAccounts(ctx context.Context, dataDir string) ([]*account, error) {
	root := filepath.Join(dataDir, "accounts")
	if err := os.MkdirAll(root, 0o700); err != nil {
		return nil, err
	}
	entries, err := os.ReadDir(root)
	if err != nil {
		return nil, err
	}
	var ids []string
	for _, e := range entries {
		if e.IsDir() {
			ids = append(ids, e.Name())
		}
	}
	// ponytail: single-account phase creates its one account here; account management replaces this.
	if len(ids) == 0 {
		ids = append(ids, rand.Text())
	}
	var accounts []*account
	for _, id := range ids {
		a, err := openAccount(ctx, filepath.Join(root, id), id)
		if err != nil {
			closeAccounts(accounts)
			return nil, fmt.Errorf("account %s: %w", id, err)
		}
		accounts = append(accounts, a)
	}
	return accounts, nil
}

func openAccount(ctx context.Context, dir, id string) (*account, error) {
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return nil, err
	}
	dsn := "file:" + filepath.Join(dir, "account.db") +
		"?_pragma=foreign_keys(1)&_pragma=journal_mode(WAL)&_pragma=busy_timeout(5000)"
	db, err := sql.Open("sqlite", dsn)
	if err != nil {
		return nil, err
	}
	log := waLogger{slog.With("account", id)}
	container := sqlstore.NewWithDB(db, "sqlite3", log.Sub("store"))
	if err := container.Upgrade(ctx); err != nil {
		return nil, errors.Join(err, db.Close())
	}
	// Returns a fresh, unlinked device when the store is empty.
	device, err := container.GetFirstDevice(ctx)
	if err != nil {
		return nil, errors.Join(err, db.Close())
	}
	return &account{id: id, db: db, client: whatsmeow.NewClient(device, log.Sub("client"))}, nil
}

func closeAccounts(accounts []*account) {
	for _, a := range accounts {
		a.client.Disconnect()
		if err := a.db.Close(); err != nil {
			slog.Warn("close account", "account", a.id, "err", err)
		}
	}
}

func (a *account) info() *ipc.Account {
	info := &ipc.Account{Id: a.id, State: ipc.ConnectionState_CONNECTION_STATE_DISCONNECTED}
	if jid := a.client.Store.ID; jid != nil {
		info.Jid = jid.String()
	} else {
		info.State = ipc.ConnectionState_CONNECTION_STATE_LOGGED_OUT
	}
	info.PushName = a.client.Store.PushName
	return info
}

// waLogger routes whatsmeow's logs into slog.
type waLogger struct{ l *slog.Logger }

func (w waLogger) Errorf(msg string, args ...any) { w.l.Error(fmt.Sprintf(msg, args...)) }
func (w waLogger) Warnf(msg string, args ...any)  { w.l.Warn(fmt.Sprintf(msg, args...)) }
func (w waLogger) Infof(msg string, args ...any)  { w.l.Info(fmt.Sprintf(msg, args...)) }
func (w waLogger) Debugf(msg string, args ...any) { w.l.Debug(fmt.Sprintf(msg, args...)) }
func (w waLogger) Sub(module string) waLog.Logger { return waLogger{w.l.With("module", module)} }
