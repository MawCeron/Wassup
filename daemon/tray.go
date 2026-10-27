package main

import (
	"context"
	"log/slog"
	"os"
	"strconv"
	"strings"
	"sync"

	"github.com/godbus/dbus/v5"
	"github.com/godbus/dbus/v5/prop"
	"go.mau.fi/whatsmeow/types"
)

// The StatusNotifierItem protocol, which GNOME shows with the AppIndicator
// extension and other desktops natively.
const (
	sniInterface = "org.kde.StatusNotifierItem"
	sniPath      = dbus.ObjectPath("/StatusNotifierItem")
	sniWatcher   = "org.kde.StatusNotifierWatcher"
)

// tray is the status icon with the unread count. It shows only while a
// tray host is running; without one it waits for it to appear.
type tray struct {
	mu     sync.Mutex
	conn   *dbus.Conn // Nil while the icon is off.
	props  *prop.Properties
	unread int
}

// sniToolTip is the (sa(iiay)ss) tooltip structure.
type sniToolTip struct {
	IconName string
	Pixmaps  []struct {
		W, H int32
		Data []byte
	}
	Title, Text string
}

// setEnabled shows or removes the icon.
func (t *tray) setEnabled(on bool) {
	t.mu.Lock()
	defer t.mu.Unlock()
	if !on {
		if t.conn != nil {
			t.conn.Close() // Hosts drop the item when its name goes away.
			t.conn, t.props = nil, nil
		}
		return
	}
	if t.conn != nil {
		return
	}
	conn, err := dbus.ConnectSessionBus()
	if err != nil {
		slog.Warn("tray icon disabled: no session bus", "err", err)
		return
	}
	if err := conn.Export(sniMethods{conn}, sniPath, sniInterface); err != nil {
		conn.Close()
		slog.Warn("tray icon", "err", err)
		return
	}
	label, tip := t.labels()
	props, err := prop.Export(conn, sniPath, prop.Map{sniInterface: {
		"Category":           {Value: "Communications"},
		"Id":                 {Value: appID},
		"Title":              {Value: "Wassup"},
		"Status":             {Value: "Active"},
		"IconName":           {Value: appID + "-symbolic"},
		"IconThemePath":      {Value: ""},
		"ItemIsMenu":         {Value: false},
		"ToolTip":            {Value: tip},
		"XAyatanaLabel":      {Value: label},
		"XAyatanaLabelGuide": {Value: "99"},
	}})
	if err != nil {
		conn.Close()
		slog.Warn("tray icon", "err", err)
		return
	}
	name := "org.kde.StatusNotifierItem-" + strconv.Itoa(os.Getpid()) + "-1"
	if _, err := conn.RequestName(name, dbus.NameFlagDoNotQueue); err != nil {
		conn.Close()
		slog.Warn("tray icon", "err", err)
		return
	}
	t.conn, t.props = conn, props
	// The host may start later, or restart when the extension is toggled.
	conn.AddMatchSignal(
		dbus.WithMatchInterface("org.freedesktop.DBus"),
		dbus.WithMatchMember("NameOwnerChanged"),
		dbus.WithMatchArg(0, sniWatcher),
	)
	signals := make(chan *dbus.Signal, 4)
	conn.Signal(signals)
	go func() {
		for sig := range signals {
			if len(sig.Body) == 3 && sig.Body[2] != "" {
				register(conn, name)
			}
		}
	}()
	register(conn, name)
}

func register(conn *dbus.Conn, name string) {
	err := conn.Object(sniWatcher, "/StatusNotifierWatcher").
		Call(sniWatcher+".RegisterStatusNotifierItem", 0, name).Err
	if err != nil {
		slog.Debug("no tray host yet", "err", err)
	}
}

// setUnread updates the count shown beside the icon.
func (t *tray) setUnread(n int) {
	t.mu.Lock()
	defer t.mu.Unlock()
	if n == t.unread {
		return
	}
	t.unread = n
	if t.props == nil {
		return
	}
	label, tip := t.labels()
	t.props.SetMust(sniInterface, "XAyatanaLabel", label)
	t.props.SetMust(sniInterface, "ToolTip", tip)
	// Hosts re-read properties on these signals, not on PropertiesChanged.
	t.conn.Emit(sniPath, sniInterface+".XAyatanaNewLabel", label, "99")
	t.conn.Emit(sniPath, sniInterface+".NewToolTip")
}

func (t *tray) labels() (string, sniToolTip) {
	tip := sniToolTip{Title: "Wassup"}
	if t.unread == 0 {
		return "", tip
	}
	// TRANSLATORS: {} is how many.
	tip.Text = strings.ReplaceAll(trn("{} unread message", "{} unread messages", t.unread), "{}", strconv.Itoa(t.unread))
	return strconv.Itoa(t.unread), tip
}

// sniMethods answers the host's calls on the icon.
type sniMethods struct{ conn *dbus.Conn }

// Activate is a click: it shows the UI, starting it if needed.
func (m sniMethods) Activate(x, y int32) *dbus.Error {
	ui := m.conn.Object(appID, dbus.ObjectPath("/"+strings.ReplaceAll(appID, ".", "/")))
	if err := ui.Call("org.freedesktop.Application.Activate", 0, map[string]dbus.Variant{}).Err; err != nil {
		slog.Warn("open UI from tray", "err", err)
	}
	return nil
}

func (m sniMethods) SecondaryActivate(x, y int32) *dbus.Error           { return m.Activate(x, y) }
func (m sniMethods) ContextMenu(x, y int32) *dbus.Error                 { return nil }
func (m sniMethods) Scroll(delta int32, orientation string) *dbus.Error { return nil }

// unreadCount is what the tray shows: unread messages in chats that are
// neither muted nor locked, since locked chats must not give themselves away.
func (a *account) unreadCount(ctx context.Context) int {
	rows, err := a.db.QueryContext(ctx, `SELECT jid, unread_count FROM wassup_chats
		WHERE unread_count > 0 AND jid NOT IN (SELECT jid FROM wassup_locked_chats)`)
	if err != nil {
		return 0
	}
	defer rows.Close()
	total := 0
	for rows.Next() {
		var jid string
		var n int
		if rows.Scan(&jid, &n) != nil {
			continue
		}
		if chat, err := types.ParseJID(jid); err == nil && a.muted(ctx, chat) {
			continue
		}
		total += n
	}
	return total
}
