package main

import (
	"bufio"
	"context"
	"errors"
	"flag"
	"fmt"
	"io"
	"log/slog"
	"net"
	"os"
	"path/filepath"
	"strconv"
	"sync"
	"time"

	"github.com/MawCeron/Wassup/daemon/ipc"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/encoding/protodelim"
	"google.golang.org/protobuf/proto"
)

func main() {
	debug := flag.Bool("debug", false, "log every request")
	flag.Parse()
	level := slog.LevelInfo
	if *debug {
		level = slog.LevelDebug
	}
	opts := &slog.HandlerOptions{Level: level}
	// systemd sets JOURNAL_STREAM when stderr goes to the journal, which
	// stamps every line already.
	if os.Getenv("JOURNAL_STREAM") != "" {
		opts.ReplaceAttr = func(groups []string, a slog.Attr) slog.Attr {
			if a.Key == slog.TimeKey && len(groups) == 0 {
				return slog.Attr{}
			}
			return a
		}
	}
	slog.SetDefault(slog.New(slog.NewTextHandler(os.Stderr, opts)))

	if err := run(); err != nil {
		slog.Error("startup failed", "err", err)
		os.Exit(1)
	}
}

func run() error {
	data, err := xdgDir("XDG_DATA_HOME", ".local/share")
	if err != nil {
		return err
	}
	cache, err := xdgDir("XDG_CACHE_HOME", ".cache")
	if err != nil {
		return err
	}
	configDir, err := xdgDir("XDG_CONFIG_HOME", ".config")
	if err != nil {
		return err
	}
	cfg, err := loadConfig(configDir)
	if err != nil {
		return err
	}
	initLocale(cfg.Language)
	// Listen first: it refuses to start if another daemon already owns the databases.
	ln, err := listen()
	if err != nil {
		return err
	}
	accounts, err := openAccounts(context.Background(), data)
	if err != nil {
		ln.Close()
		return err
	}
	defer closeAccounts(accounts)
	for _, a := range accounts {
		a.mediaDir = filepath.Join(cache, "media", a.id)
		a.avatarDir = filepath.Join(cache, "avatars", a.id)
		a.mediaCacheMB.Store(int32(cfg.MediaCacheMB))
		for _, dir := range []string{a.mediaDir, a.avatarDir} {
			if err := os.MkdirAll(dir, 0o700); err != nil {
				closeAccounts(accounts)
				return err
			}
		}
	}
	for _, a := range accounts {
		removeLeftovers(a.mediaDir, a.avatarDir)
	}
	d := newDaemon(accounts, newNotifier(cfg.HideNotificationContent))
	d.configDir, d.config = configDir, cfg
	go d.sweepExpired(context.Background())
	for _, a := range accounts {
		if a.client.Store.ID != nil {
			// Without network at login, keep retrying in the background.
			a.client.InitialAutoReconnect = true
			if err := a.client.Connect(); err != nil {
				slog.Warn("connect", "account", a.id, "err", err)
			}
		}
	}
	slog.Info("started", "socket", ln.Addr(), "data", data, "cache", cache, "accounts", len(accounts))
	d.stop = func() { ln.Close() }
	d.serve(ln)
	// A clean exit, so systemd does not restart it; the socket unit
	// starts it again when the UI next connects.
	slog.Info("quit")
	return nil
}

// xdgDir returns and creates the wassup directory under an XDG base
// directory. It is owner-only because it will hold account keys.
func xdgDir(env, fallback string) (string, error) {
	base := os.Getenv(env)
	// The XDG spec says relative paths are invalid and must be ignored.
	if !filepath.IsAbs(base) {
		home, err := os.UserHomeDir()
		if err != nil {
			return "", err
		}
		base = filepath.Join(home, fallback)
	}
	dir := filepath.Join(base, "wassup")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return "", err
	}
	// MkdirAll leaves an existing directory's mode alone.
	return dir, os.Chmod(dir, 0o700)
}

func socketPath() (string, error) {
	dir := os.Getenv("XDG_RUNTIME_DIR")
	if dir == "" {
		// No fallback to /tmp: it is shared with other users.
		return "", errors.New("XDG_RUNTIME_DIR is not set")
	}
	return filepath.Join(dir, "wassup.sock"), nil
}

func listen() (net.Listener, error) {
	// Started by wassupd.socket: systemd already listens and hands the
	// socket over as file descriptor 3.
	if os.Getenv("LISTEN_PID") == strconv.Itoa(os.Getpid()) && os.Getenv("LISTEN_FDS") == "1" {
		f := os.NewFile(3, "wassup.sock")
		defer f.Close()
		return net.FileListener(f)
	}
	path, err := socketPath()
	if err != nil {
		return nil, err
	}
	if c, err := net.Dial("unix", path); err == nil {
		c.Close()
		return nil, fmt.Errorf("another daemon is already listening on %s", path)
	}
	// Nobody answers, so any file left there is stale.
	if err := os.Remove(path); err != nil && !errors.Is(err, os.ErrNotExist) {
		return nil, err
	}
	ln, err := net.Listen("unix", path)
	if err != nil {
		return nil, err
	}
	// XDG_RUNTIME_DIR is already 0700; this keeps the socket private even if it is not.
	if err := os.Chmod(path, 0o600); err != nil {
		ln.Close()
		return nil, err
	}
	return ln, nil
}

// daemon serves the IPC clients and pushes account events to all of them.
type daemon struct {
	accounts []*account
	notify   *notifier
	// TODO: one lock for every socket write; per-connection queues if a slow UI ever stalls the others.
	mu    sync.Mutex
	conns map[net.Conn]struct{}
	// Chat each UI is showing, as "account/chat", so it is not notified.
	active map[net.Conn]string
	// Stops serve; set by run.
	stop func()
	// Preferences, saved to configDir/config.json when changed.
	configMu  sync.Mutex
	configDir string
	config    config
}

func newDaemon(accounts []*account, notify *notifier) *daemon {
	d := &daemon{accounts: accounts, notify: notify, conns: map[net.Conn]struct{}{}, active: map[net.Conn]string{}}
	for _, a := range accounts {
		a.client.AddEventHandler(func(evt any) {
			switch e := evt.(type) {
			case *events.Connected, *events.Disconnected, *events.LoggedOut, *events.StreamReplaced:
				if _, ok := e.(*events.Connected); ok {
					a.connectedAt.Store(time.Now().UnixMilli())
					go a.resyncAppState(context.Background())
				} else {
					a.connectedAt.Store(0)
				}
				d.broadcast(&ipc.Event{Kind: &ipc.Event_AccountChanged{AccountChanged: a.info()}})
			case *events.AppState:
				if chat := a.onAppState(context.Background(), e.Index, e.SyncActionValue); chat != nil {
					d.broadcast(&ipc.Event{Kind: &ipc.Event_ChatChanged{ChatChanged: chat}})
				}
			case *events.GroupInfo:
				if g := e.Ephemeral; g != nil {
					timer := g.DisappearingTimer
					if !g.IsEphemeral {
						timer = 0
					}
					a.setDisappearing(context.Background(), e.JID.String(), timer)
				}
			case *events.Picture:
				// One request for the new picture, so open UIs show it right away.
				if err := a.forgetAvatar(context.Background(), e.JID.String()); err != nil {
					a.log.Warn("picture changed", "chat", e.JID, "err", err)
				}
				d.fetchAvatar(a, e.JID.String())
			case *events.Receipt:
				// TODO: in groups WhatsApp waits for every member; this takes the first receipt.
				status := receiptStatus(e.Type)
				if status == ipc.MessageStatus_MESSAGE_STATUS_UNSPECIFIED {
					return
				}
				changed, err := a.advanceStatus(context.Background(), e.MessageIDs, status)
				if err != nil {
					a.log.Error("message status", "err", err)
				}
				for chat, ids := range changed {
					d.broadcast(&ipc.Event{Kind: &ipc.Event_MessageStatusChanged{MessageStatusChanged: &ipc.MessageStatusChanged{
						AccountId: a.id, ChatJid: chat, MessageIds: ids, Status: status,
					}}})
				}
			case *events.Message:
				d.onMessage(a, e)
			case *events.UndecryptableMessage:
				// WhatsApp does not send view-once media to linked devices,
				// only this notice; show that the message exists.
				if e.IsUnavailable && e.UnavailableType == events.UnavailableTypeViewOnce {
					d.onMessage(a, &events.Message{Info: e.Info, Message: &waE2E.Message{
						Conversation: proto.String(viewOncePlaceholder()),
					}})
				}
			}
		})
	}
	return d
}

func (d *daemon) serve(ln net.Listener) {
	for {
		conn, err := ln.Accept()
		if err != nil {
			if !errors.Is(err, net.ErrClosed) {
				slog.Error("accept", "err", err)
			}
			return
		}
		d.mu.Lock()
		d.conns[conn] = struct{}{}
		d.mu.Unlock()
		go d.handle(conn)
	}
}

func (d *daemon) handle(conn net.Conn) {
	defer func() {
		d.mu.Lock()
		delete(d.conns, conn)
		delete(d.active, conn)
		d.mu.Unlock()
		conn.Close()
	}()
	r := bufio.NewReader(conn)
	for {
		var req ipc.Request
		if err := protodelim.UnmarshalFrom(r, &req); err != nil {
			if !errors.Is(err, io.EOF) && !errors.Is(err, net.ErrClosed) {
				slog.Warn("read request", "err", err)
			}
			return
		}
		slog.Debug("request", "id", req.GetId(), "kind", fmt.Sprintf("%T", req.Kind))
		ev := d.reply(conn, &req) // Outside the lock: it may connect and trigger broadcasts.
		if ev == nil {
			continue // Answered later, from the goroutine doing the work.
		}
		d.mu.Lock()
		err := write(conn, ev)
		d.mu.Unlock()
		if err != nil {
			slog.Warn("write event", "err", err)
			return
		}
		if _, quit := req.Kind.(*ipc.Request_Quit); quit && d.stop != nil {
			d.stop() // After the ack, so the UI knows it was heard.
			return
		}
	}
}

// broadcast sends an unsolicited event to every connected UI.
func (d *daemon) broadcast(ev *ipc.Event) {
	d.mu.Lock()
	defer d.mu.Unlock()
	for conn := range d.conns {
		if err := write(conn, ev); err != nil {
			slog.Warn("broadcast", "err", err)
			conn.Close() // handle() notices and forgets it.
		}
	}
}

// write must be called with d.mu held.
func write(conn net.Conn, ev *ipc.Event) error {
	// A UI that stops reading must not block the daemon forever.
	conn.SetWriteDeadline(time.Now().Add(5 * time.Second))
	_, err := protodelim.MarshalTo(conn, ev)
	return err
}

// viewOncePlaceholder stands for view-once media, which only the phone gets.
func viewOncePlaceholder() string { return tr("[View once] Open it on your phone") }

// onMessage stores a message and tells the UIs and, if it is news, the user.
func (d *daemon) onMessage(a *account, e *events.Message) {
	ctx := context.Background()
	if isEphemeralSetting(e.Message) {
		a.setDisappearing(ctx, e.Info.Chat.String(), e.Message.GetProtocolMessage().GetEphemeralExpiration())
		return
	}
	msg, chat, err := a.saveMessage(ctx, e)
	if err != nil {
		a.log.Error("save message", "id", e.Info.ID, "err", err)
		return
	}
	if msg == nil {
		return
	}
	d.broadcast(&ipc.Event{Kind: &ipc.Event_MessageReceived{MessageReceived: msg}})
	d.broadcast(&ipc.Event{Kind: &ipc.Event_ChatChanged{ChatChanged: chat}})
	if a.isNews(msg) && !d.isActive(chatKey(a.id, chat.Jid)) && !a.muted(ctx, e.Info.Chat) {
		d.notify.message(chat, msg)
	}
}

func chatKey(accountID, chatJID string) string { return accountID + "/" + chatJID }

func (d *daemon) isActive(key string) bool {
	d.mu.Lock()
	defer d.mu.Unlock()
	for _, k := range d.active {
		if k == key {
			return true
		}
	}
	return false
}

func (d *daemon) reply(conn net.Conn, req *ipc.Request) *ipc.Event {
	ev := &ipc.Event{RequestId: req.GetId()}
	fail := func(err error) {
		ev.Kind = &ipc.Event_Error{Error: &ipc.Error{Message: err.Error()}}
	}
	switch k := req.Kind.(type) {
	case *ipc.Request_ListAccounts:
		list := &ipc.AccountList{}
		for _, a := range d.accounts {
			list.Accounts = append(list.Accounts, a.info())
		}
		ev.Kind = &ipc.Event_Accounts{Accounts: list}
	case *ipc.Request_ListChats:
		a := d.find(k.ListChats.GetAccountId())
		if a == nil {
			fail(errors.New("unknown account"))
		} else if chats, err := a.listChats(context.Background()); err != nil {
			fail(err)
		} else {
			ev.Kind = &ipc.Event_Chats{Chats: &ipc.ChatList{Chats: chats}}
		}
	case *ipc.Request_ListMessages:
		r := k.ListMessages
		a := d.find(r.GetAccountId())
		if a == nil {
			fail(errors.New("unknown account"))
		} else if msgs, err := a.listMessages(context.Background(), r.GetChatJid(), r.GetBeforeId(), r.GetLimit()); err != nil {
			fail(err)
		} else {
			ev.Kind = &ipc.Event_Messages{Messages: &ipc.MessageList{Messages: msgs}}
		}
	case *ipc.Request_SendText:
		r := k.SendText
		a := d.find(r.GetAccountId())
		if a == nil {
			fail(errors.New("unknown account"))
		} else if msg, chat, err := a.sendText(context.Background(), r.GetChatJid(), r.GetText()); err != nil {
			fail(err)
		} else {
			ev.Kind = &ipc.Event_MessageReceived{MessageReceived: msg}
			// TODO: other connected UIs only learn about it through the chat update.
			d.broadcast(&ipc.Event{Kind: &ipc.Event_ChatChanged{ChatChanged: chat}})
		}
	case *ipc.Request_MarkRead:
		a := d.find(k.MarkRead.GetAccountId())
		if a == nil {
			fail(errors.New("unknown account"))
		} else if chat, err := a.markRead(context.Background(), k.MarkRead.GetChatJid()); err != nil {
			fail(err)
		} else {
			d.notify.dismiss(a.id, chat.Jid)
			ev.Kind = &ipc.Event_ChatChanged{ChatChanged: chat}
		}
	case *ipc.Request_SetActiveChat:
		r := k.SetActiveChat
		d.mu.Lock()
		if r.GetChatJid() == "" {
			delete(d.active, conn)
		} else {
			d.active[conn] = chatKey(r.GetAccountId(), r.GetChatJid())
		}
		d.mu.Unlock()
		ev.Kind = &ipc.Event_Ack{Ack: &ipc.Ack{}}
	case *ipc.Request_SendMedia:
		r := k.SendMedia
		a := d.find(r.GetAccountId())
		if a == nil {
			fail(errors.New("unknown account"))
			break
		}
		// Uploads can take long: reply when done, without holding up the
		// connection's other requests.
		go func() {
			done := &ipc.Event{RequestId: req.GetId()}
			msg, chat, err := a.sendMedia(context.Background(), r.GetChatJid(), r.GetPath(), r.GetCaption(), r.GetAsSticker())
			if err != nil {
				done.Kind = &ipc.Event_Error{Error: &ipc.Error{Message: err.Error()}}
			} else {
				done.Kind = &ipc.Event_MessageReceived{MessageReceived: msg}
			}
			d.mu.Lock()
			if err := write(conn, done); err != nil {
				slog.Warn("write event", "err", err)
			}
			d.mu.Unlock()
			if chat != nil {
				d.broadcast(&ipc.Event{Kind: &ipc.Event_ChatChanged{ChatChanged: chat}})
			}
		}()
		return nil
	case *ipc.Request_Quit:
		ev.Kind = &ipc.Event_Ack{Ack: &ipc.Ack{}}
	case *ipc.Request_GetSettings:
		ev.Kind = &ipc.Event_Settings{Settings: d.settings()}
	case *ipc.Request_SetSettings:
		if s, err := d.setSettings(k.SetSettings.GetSettings()); err != nil {
			fail(err)
		} else {
			ev.Kind = &ipc.Event_Settings{Settings: s}
		}
	case *ipc.Request_ListStickers:
		if a := d.find(k.ListStickers.GetAccountId()); a == nil {
			fail(errors.New("unknown account"))
		} else if stickers, err := a.listStickers(context.Background()); err != nil {
			fail(err)
		} else {
			ev.Kind = &ipc.Event_Stickers{Stickers: &ipc.StickerList{Stickers: stickers}}
		}
	case *ipc.Request_SendSticker:
		r := k.SendSticker
		if a := d.find(r.GetAccountId()); a == nil {
			fail(errors.New("unknown account"))
		} else if msg, chat, err := a.sendSticker(context.Background(), r.GetChatJid(), r.GetSourceChatJid(), r.GetSourceMessageId(), r.GetFavoriteId()); err != nil {
			fail(err)
		} else {
			ev.Kind = &ipc.Event_MessageReceived{MessageReceived: msg}
			d.broadcast(&ipc.Event{Kind: &ipc.Event_ChatChanged{ChatChanged: chat}})
		}
	case *ipc.Request_UnlockChats:
		if a := d.find(k.UnlockChats.GetAccountId()); a == nil {
			fail(errors.New("unknown account"))
		} else if err := a.unlockChats(context.Background(), k.UnlockChats.GetSecretCode()); err != nil {
			// Slows guessing; the code is short and the hash is local.
			switch {
			case errors.Is(err, errWrongSecretCode):
				time.Sleep(time.Second)
				fail(errors.New(tr("Wrong secret code")))
			case errors.Is(err, errNoSecretCode):
				fail(errors.New(tr("Set a secret code for locked chats on your phone first")))
			default:
				fail(err)
			}
		} else {
			ev.Kind = &ipc.Event_Ack{Ack: &ipc.Ack{}}
		}
	case *ipc.Request_FetchAvatar:
		if a := d.find(k.FetchAvatar.GetAccountId()); a == nil {
			fail(errors.New("unknown account"))
		} else {
			d.fetchAvatar(a, k.FetchAvatar.GetChatJid())
			ev.Kind = &ipc.Event_Ack{Ack: &ipc.Ack{}}
		}
	case *ipc.Request_DownloadMedia:
		r := k.DownloadMedia
		if a := d.find(r.GetAccountId()); a == nil {
			fail(errors.New("unknown account"))
		} else {
			d.download(a, r.GetChatJid(), r.GetMessageId())
			ev.Kind = &ipc.Event_Ack{Ack: &ipc.Ack{}}
		}
	case *ipc.Request_LinkAccount:
		a := d.find(k.LinkAccount.GetAccountId())
		if a == nil {
			fail(errors.New("unknown account"))
		} else if err := d.link(a); err != nil {
			fail(err)
		} else {
			ev.Kind = &ipc.Event_AccountChanged{AccountChanged: a.info()}
		}
	default:
		fail(errors.New("not implemented"))
	}
	return ev
}

func (d *daemon) find(id string) *account {
	for _, a := range d.accounts {
		if a.id == id {
			return a
		}
	}
	return nil
}
