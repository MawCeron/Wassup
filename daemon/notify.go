package main

import (
	"encoding/json"
	"errors"
	"log/slog"
	"os"
	"path/filepath"
	"strings"
	"sync"

	"github.com/MawCeron/Wassup/daemon/ipc"
	"github.com/godbus/dbus/v5"
)

// appID matches the UI's application id, so the desktop groups both.
const appID = "io.github.MawCeron.Wassup"

type config struct {
	// Notifications say who wrote but not what, for screens others can see.
	HideNotificationContent bool `json:"hide_notification_content"`
}

// loadConfig reads configDir/config.json; a missing file means defaults.
// Anything it cannot read exactly is an error rather than a silent default,
// since a misspelled privacy option must not quietly turn itself off.
func loadConfig(configDir string) (config, error) {
	var c config
	f, err := os.Open(filepath.Join(configDir, "config.json"))
	if errors.Is(err, os.ErrNotExist) {
		return c, nil
	} else if err != nil {
		return c, err
	}
	defer f.Close()
	dec := json.NewDecoder(f)
	dec.DisallowUnknownFields()
	if err := dec.Decode(&c); err != nil {
		return c, errors.New(f.Name() + ": " + err.Error())
	}
	return c, nil
}

// notifier shows desktop notifications through org.freedesktop.Notifications.
// A nil notifier does nothing, so the daemon still runs without a session bus.
type notifier struct {
	bus         dbus.BusObject
	hideContent bool
	mu          sync.Mutex
	ids         map[string]uint32 // Shown notification per chat, replaced by the next one.
}

func newNotifier(hideContent bool) *notifier {
	conn, err := dbus.ConnectSessionBus()
	if err != nil {
		slog.Warn("notifications disabled: no session bus", "err", err)
		return nil
	}
	return &notifier{
		bus:         conn.Object("org.freedesktop.Notifications", "/org/freedesktop/Notifications"),
		hideContent: hideContent,
		ids:         map[string]uint32{},
	}
}

// notificationText builds the title and body for a received message.
func notificationText(chat *ipc.Chat, msg *ipc.Message, hideContent bool) (title, body string) {
	title = chat.Name
	if title == "" {
		title = "New message"
	}
	body = msg.Text
	if hideContent {
		body = "New message"
	} else if chat.IsGroup && msg.SenderName != "" {
		body = msg.SenderName + ": " + body
	}
	// The body may be read as markup; message text must stay plain text.
	return title, strings.NewReplacer("&", "&amp;", "<", "&lt;", ">", "&gt;").Replace(body)
}

func (n *notifier) message(chat *ipc.Chat, msg *ipc.Message) {
	if n == nil {
		return
	}
	title, body := notificationText(chat, msg, n.hideContent)
	key := msg.AccountId + "/" + chat.Jid
	n.mu.Lock()
	defer n.mu.Unlock()
	hints := map[string]dbus.Variant{
		"desktop-entry": dbus.MakeVariant(appID),
		"category":      dbus.MakeVariant("im.received"),
	}
	var id uint32
	err := n.bus.Call("org.freedesktop.Notifications.Notify", 0,
		"Wassup", n.ids[key], "", title, body, []string{}, hints, int32(-1)).Store(&id)
	if err != nil {
		slog.Warn("notify", "err", err)
		return
	}
	n.ids[key] = id
}

// dismiss withdraws a chat's notification once the chat has been read.
func (n *notifier) dismiss(accountID, chatJID string) {
	if n == nil {
		return
	}
	key := accountID + "/" + chatJID
	n.mu.Lock()
	id, ok := n.ids[key]
	delete(n.ids, key)
	n.mu.Unlock()
	if ok {
		n.bus.Call("org.freedesktop.Notifications.CloseNotification", 0, id)
	}
}
