package main

import (
	"encoding/json"
	"errors"
	"log/slog"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"sync"
	"sync/atomic"

	"github.com/MawCeron/Wassup/daemon/ipc"
	"github.com/godbus/dbus/v5"
)

// appID matches the UI's application id, so the desktop groups both.
const appID = "io.github.MawCeron.Wassup"

type config struct {
	// Notifications say who wrote but not what, for screens others can see.
	HideNotificationContent bool `json:"hide_notification_content"`
	// Size of the media cache in MB; 0 means the default.
	MediaCacheMB int `json:"media_cache_mb"`
	// Read by the UI: draw with the GPU instead of in software. Declared
	// here so the daemon's strict parsing accepts it.
	GPURendering bool `json:"gpu_rendering"`
	// Language code such as "es"; empty follows the system.
	Language string `json:"language"`
	// The tray icon is on unless turned off, as it only shows where the
	// desktop has a tray.
	HideTrayIcon bool `json:"hide_tray_icon"`
}

// languageCode is what a language setting may look like. It becomes part
// of a file path, so nothing else is accepted.
var languageCode = regexp.MustCompile(`^([a-z]{2,3}(_[A-Z]{2})?)?$`)

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
	if err := c.validate(); err != nil {
		return c, errors.New(f.Name() + ": " + err.Error())
	}
	return c, nil
}

func (c config) validate() error {
	if c.MediaCacheMB < 0 {
		return errors.New("media_cache_mb cannot be negative")
	}
	if !languageCode.MatchString(c.Language) {
		return errors.New("language must be a code such as es or pt_BR")
	}
	return nil
}

// saveConfig replaces configDir/config.json in one step, so the daemon
// never reads half a file.
func saveConfig(configDir string, c config) error {
	data, err := json.MarshalIndent(c, "", "  ")
	if err != nil {
		return err
	}
	tmp, err := os.CreateTemp(configDir, ".config-*.json")
	if err != nil {
		return err
	}
	defer os.Remove(tmp.Name())
	if _, err := tmp.Write(append(data, '\n')); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	return os.Rename(tmp.Name(), filepath.Join(configDir, "config.json"))
}

// notifier shows desktop notifications through org.freedesktop.Notifications.
// A nil notifier does nothing, so the daemon still runs without a session bus.
type notifier struct {
	conn        *dbus.Conn
	bus         dbus.BusObject
	hideContent atomic.Bool
	mu          sync.Mutex
	ids         map[string]uint32 // Shown notification per chat, replaced by the next one.
	tokens      map[uint32]string // Activation token sent just before a click.
}

func newNotifier(hideContent bool) *notifier {
	conn, err := dbus.ConnectSessionBus()
	if err != nil {
		slog.Warn("notifications disabled: no session bus", "err", err)
		return nil
	}
	n := &notifier{
		conn:   conn,
		bus:    conn.Object("org.freedesktop.Notifications", "/org/freedesktop/Notifications"),
		ids:    map[string]uint32{},
		tokens: map[uint32]string{},
	}
	n.hideContent.Store(hideContent)
	err = conn.AddMatchSignal(
		dbus.WithMatchObjectPath("/org/freedesktop/Notifications"),
		dbus.WithMatchInterface("org.freedesktop.Notifications"),
	)
	if err != nil {
		slog.Warn("notification clicks disabled", "err", err)
		return n
	}
	signals := make(chan *dbus.Signal, 16)
	conn.Signal(signals)
	go n.listen(signals)
	return n
}

// listen opens the chat of a clicked notification in the UI.
func (n *notifier) listen(signals <-chan *dbus.Signal) {
	for sig := range signals {
		if len(sig.Body) < 2 {
			continue
		}
		id, _ := sig.Body[0].(uint32)
		text, _ := sig.Body[1].(string)
		switch sig.Name {
		case "org.freedesktop.Notifications.ActivationToken":
			n.mu.Lock()
			n.tokens[id] = text
			n.mu.Unlock()
		case "org.freedesktop.Notifications.ActionInvoked":
			n.mu.Lock()
			token := n.tokens[id]
			delete(n.tokens, id)
			key := ""
			for k, shown := range n.ids {
				if shown == id {
					key = k
				}
			}
			n.mu.Unlock()
			if account, chat, ok := strings.Cut(key, "/"); ok && text == "default" {
				n.openChat(account, chat, token)
			}
		}
	}
}

// openChat asks the UI to show a chat, starting it through D-Bus
// activation when it is not running.
func (n *notifier) openChat(account, chat, token string) {
	platform := map[string]dbus.Variant{}
	if token != "" {
		// Lets the compositor raise the window on Wayland.
		platform["activation-token"] = dbus.MakeVariant(token)
	}
	param := dbus.MakeVariant(struct{ Account, Chat string }{account, chat})
	ui := n.conn.Object(appID, dbus.ObjectPath("/"+strings.ReplaceAll(appID, ".", "/")))
	err := ui.Call("org.freedesktop.Application.ActivateAction", 0,
		"open-chat", []dbus.Variant{param}, platform).Err
	if err != nil {
		slog.Warn("open chat in UI", "err", err)
	}
}

// notificationText builds the title and body for a received message.
func notificationText(chat *ipc.Chat, msg *ipc.Message, hideContent bool) (title, body string) {
	if chat.Locked {
		// Locking a chat on the phone hides who wrote and what.
		return "Wassup", tr("New message")
	}
	title = chat.Name
	if title == "" {
		title = tr("New message")
	}
	body = msg.Text
	if kind := msg.GetMedia().GetKind(); kind != ipc.MediaKind_MEDIA_KIND_UNSPECIFIED {
		label := map[ipc.MediaKind]string{
			ipc.MediaKind_MEDIA_KIND_IMAGE:    tr("Photo"),
			ipc.MediaKind_MEDIA_KIND_STICKER:  tr("Sticker"),
			ipc.MediaKind_MEDIA_KIND_DOCUMENT: tr("Document"),
			ipc.MediaKind_MEDIA_KIND_AUDIO:    tr("Audio"),
			ipc.MediaKind_MEDIA_KIND_VIDEO:    tr("Video"),
		}[kind]
		if msg.GetMedia().GetVoiceNote() {
			label = tr("Voice message")
		}
		if body == "" {
			body = msg.GetMedia().GetFileName()
		}
		if body == "" {
			body = label
		} else {
			body = label + ": " + body
		}
	}
	if hideContent {
		body = tr("New message")
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
	title, body := notificationText(chat, msg, n.hideContent.Load())
	key := chatKey(msg.AccountId, chat.Jid)
	n.mu.Lock()
	defer n.mu.Unlock()
	hints := map[string]dbus.Variant{
		"desktop-entry": dbus.MakeVariant(appID),
		"category":      dbus.MakeVariant("im.received"),
	}
	var id uint32
	err := n.bus.Call("org.freedesktop.Notifications.Notify", 0,
		"Wassup", n.ids[key], "", title, body, []string{"default", "Open"}, hints, int32(-1)).Store(&id)
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
	key := chatKey(accountID, chatJID)
	n.mu.Lock()
	id, ok := n.ids[key]
	delete(n.ids, key)
	n.mu.Unlock()
	if ok {
		n.bus.Call("org.freedesktop.Notifications.CloseNotification", 0, id)
	}
}
