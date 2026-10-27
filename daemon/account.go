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
	"sync/atomic"
	"time"

	"github.com/MawCeron/Wassup/daemon/ipc"
	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waCompanionReg"
	"go.mau.fi/whatsmeow/store"
	"go.mau.fi/whatsmeow/store/sqlstore"
	"go.mau.fi/whatsmeow/types"
	waLog "go.mau.fi/whatsmeow/util/log"
	"google.golang.org/protobuf/proto"
	_ "modernc.org/sqlite"
)

// account owns one WhatsApp session and its database file.
type account struct {
	id     string
	db     *sql.DB
	client *whatsmeow.Client
	log    *slog.Logger
	// When the current connection started, in Unix ms; 0 while disconnected.
	// Messages sent before it are backlog delivered on connect, not news.
	connectedAt atomic.Int64
	// Where this account's media is cached; empty disables downloads.
	mediaDir     string
	avatarDir    string
	mediaCacheMB atomic.Int32
	// Whether sent links get a preview of the page.
	linkPreviews atomic.Bool
	// Texts to send, one after another, so a slow link preview does not
	// let a later message overtake it.
	sends     chan func()
	downloads *downloads
}

// version is the release, matching ui/Cargo.toml; the phone shows it for
// devices linked from now on.
var version = [3]uint32{0, 1, 0}

func init() {
	// Shown under Linked devices on the phone. WhatsApp offers no custom
	// icon, so the desktop one it is.
	store.SetOSInfo("Wassup", version)
	store.DeviceProps.PlatformType = waCompanionReg.DeviceProps_DESKTOP.Enum()
	// The phone sends this much history when linking; older messages come
	// on demand. See history.go.
	store.DeviceProps.HistorySyncConfig.FullSyncDaysLimit = proto.Uint32(historyDays)
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
	// TODO: create the only account here until accounts can be added over IPC.
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
	l := slog.With("account", id)
	log := waLogger{l}
	container := sqlstore.NewWithDB(db, "sqlite3", log.Sub("store"))
	if err := container.Upgrade(ctx); err != nil {
		return nil, errors.Join(err, db.Close())
	}
	if err := migrate(ctx, db); err != nil {
		return nil, errors.Join(err, db.Close())
	}
	// Returns a fresh, unlinked device when the store is empty.
	device, err := container.GetFirstDevice(ctx)
	if err != nil {
		return nil, errors.Join(err, db.Close())
	}
	client := whatsmeow.NewClient(device, log.Sub("client"))
	// Favorite stickers only arrive as app state events, also on full syncs.
	client.EmitAppStateEventsOnFullSync = true
	a := &account{id: id, db: db, client: client, log: l, downloads: newDownloads(), sends: make(chan func(), 64)}
	go func() {
		for send := range a.sends {
			send()
		}
	}()
	return a, nil
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
	info := &ipc.Account{Id: a.id, PushName: a.client.Store.PushName}
	if jid := a.client.Store.ID; jid != nil {
		info.Jid = jid.String()
	}
	switch {
	case a.client.IsLoggedIn():
		info.State = ipc.ConnectionState_CONNECTION_STATE_CONNECTED
	case a.client.IsConnected():
		info.State = ipc.ConnectionState_CONNECTION_STATE_CONNECTING
	case a.client.Store.ID == nil:
		info.State = ipc.ConnectionState_CONNECTION_STATE_LOGGED_OUT
	default:
		info.State = ipc.ConnectionState_CONNECTION_STATE_DISCONNECTED
	}
	return info
}

// link connects an unlinked account and pushes QR codes until it is
// scanned or linking fails. Success is reported by the Connected event.
func (d *daemon) link(a *account) error {
	if a.client.Store.ID != nil {
		return errors.New("account is already linked")
	}
	if a.client.IsConnected() {
		return errors.New("account is already linking")
	}
	ctx, cancel := context.WithCancel(context.Background())
	qr, err := a.client.GetQRChannel(ctx)
	if err != nil {
		cancel()
		return err
	}
	if err := a.client.Connect(); err != nil {
		cancel()
		return err
	}
	go func() {
		defer cancel()
		for item := range qr {
			reason := item.Event
			switch item.Event {
			case whatsmeow.QRChannelEventCode:
				d.broadcast(&ipc.Event{Kind: &ipc.Event_QrCode{QrCode: &ipc.QrCode{
					AccountId: a.id, Code: item.Code, TimeoutMs: uint32(item.Timeout.Milliseconds()),
				}}})
				continue
			case whatsmeow.QRChannelSuccess.Event, whatsmeow.QRChannelEventPasskeyResponse,
				whatsmeow.QRChannelScannedWithoutMultidevice.Event:
				continue
			case whatsmeow.QRChannelEventPasskeyRequest:
				reason = "passkey pairing is not supported"
				cancel()
			case whatsmeow.QRChannelEventError:
				reason = item.Error.Error()
			}
			a.client.Disconnect()
			d.broadcast(&ipc.Event{Kind: &ipc.Event_LinkFailed{LinkFailed: &ipc.LinkFailed{AccountId: a.id, Reason: reason}}})
		}
	}()
	return nil
}

// waLogger routes whatsmeow's logs into slog.
type waLogger struct{ l *slog.Logger }

func (w waLogger) Errorf(msg string, args ...any) { w.l.Error(fmt.Sprintf(msg, args...)) }
func (w waLogger) Warnf(msg string, args ...any)  { w.l.Warn(fmt.Sprintf(msg, args...)) }
func (w waLogger) Infof(msg string, args ...any)  { w.l.Info(fmt.Sprintf(msg, args...)) }
func (w waLogger) Debugf(msg string, args ...any) { w.l.Debug(fmt.Sprintf(msg, args...)) }
func (w waLogger) Sub(module string) waLog.Logger { return waLogger{w.l.With("module", module)} }

// muted reports whether the user silenced the chat in WhatsApp.
func (a *account) muted(ctx context.Context, chat types.JID) bool {
	settings := a.client.Store.ChatSettings
	if settings == nil { // Nil until the device is linked.
		return false
	}
	s, err := settings.GetChatSettings(ctx, chat)
	return err == nil && s.MutedUntil.After(time.Now())
}

// isNews reports whether a received message arrived while connected, as
// opposed to the backlog WhatsApp delivers on connect, which is not notified.
func (a *account) isNews(msg *ipc.Message) bool {
	since := a.connectedAt.Load()
	return !msg.FromMe && since != 0 && msg.TimestampMs >= since
}
