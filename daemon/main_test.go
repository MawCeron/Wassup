package main

import (
	"bufio"
	"net"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/MawCeron/Wassup/daemon/ipc"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/encoding/protodelim"
	"google.golang.org/protobuf/proto"
)

func TestRoundTrip(t *testing.T) {
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	ln, err := listen()
	if err != nil {
		t.Fatal(err)
	}
	defer ln.Close()
	go newDaemon(nil, nil).serve(ln)

	path, _ := socketPath()
	if fi, err := os.Stat(path); err != nil || fi.Mode().Perm() != 0o600 {
		t.Fatalf("socket mode: %v, %v", fi.Mode(), err)
	}
	if _, err := listen(); err == nil {
		t.Fatal("second listen succeeded while the first daemon is running")
	}

	conn, err := net.Dial("unix", path)
	if err != nil {
		t.Fatal(err)
	}
	defer conn.Close()

	req := &ipc.Request{Id: 7, Kind: &ipc.Request_ListAccounts{ListAccounts: &ipc.ListAccounts{}}}
	if _, err := protodelim.MarshalTo(conn, req); err != nil {
		t.Fatal(err)
	}
	var ev ipc.Event
	if err := protodelim.UnmarshalFrom(bufio.NewReader(conn), &ev); err != nil {
		t.Fatal(err)
	}
	if ev.GetRequestId() != 7 || ev.GetAccounts() == nil {
		t.Fatalf("unexpected event: %v", &ev)
	}
}

func TestQuit(t *testing.T) {
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	ln, err := listen()
	if err != nil {
		t.Fatal(err)
	}
	d := newDaemon(nil, nil)
	d.stop = func() { ln.Close() }
	done := make(chan struct{})
	go func() { d.serve(ln); close(done) }()

	path, _ := socketPath()
	conn, err := net.Dial("unix", path)
	if err != nil {
		t.Fatal(err)
	}
	defer conn.Close()
	req := &ipc.Request{Id: 3, Kind: &ipc.Request_Quit{Quit: &ipc.Quit{}}}
	if _, err := protodelim.MarshalTo(conn, req); err != nil {
		t.Fatal(err)
	}
	var ev ipc.Event
	if err := protodelim.UnmarshalFrom(bufio.NewReader(conn), &ev); err != nil || ev.GetAck() == nil {
		t.Fatalf("quit answered %v, %v", &ev, err)
	}
	select {
	case <-done:
	case <-time.After(5 * time.Second):
		t.Fatal("serve still running after quit")
	}
}

func TestXDGDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	t.Setenv("XDG_DATA_HOME", "relative/path")
	got, err := xdgDir("XDG_DATA_HOME", ".local/share")
	if want := filepath.Join(home, ".local/share/wassup"); err != nil || got != want {
		t.Fatalf("relative env: got %q, %v; want %q", got, err, want)
	}

	base := t.TempDir()
	if err := os.Mkdir(filepath.Join(base, "wassup"), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("XDG_DATA_HOME", base)
	got, err = xdgDir("XDG_DATA_HOME", ".local/share")
	if want := filepath.Join(base, "wassup"); err != nil || got != want {
		t.Fatalf("absolute env: got %q, %v; want %q", got, err, want)
	}
	if fi, _ := os.Stat(got); fi.Mode().Perm() != 0o700 {
		t.Fatalf("existing dir kept mode %v", fi.Mode().Perm())
	}
}

func TestBroadcastAndLinkErrors(t *testing.T) {
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	ln, err := listen()
	if err != nil {
		t.Fatal(err)
	}
	defer ln.Close()
	d := newDaemon(nil, nil)
	go d.serve(ln)
	path, _ := socketPath()

	call := func(conn net.Conn, r *bufio.Reader, req *ipc.Request) *ipc.Event {
		t.Helper()
		if _, err := protodelim.MarshalTo(conn, req); err != nil {
			t.Fatal(err)
		}
		var ev ipc.Event
		if err := protodelim.UnmarshalFrom(r, &ev); err != nil {
			t.Fatal(err)
		}
		return &ev
	}
	var readers []*bufio.Reader
	for range 2 {
		conn, err := net.Dial("unix", path)
		if err != nil {
			t.Fatal(err)
		}
		defer conn.Close()
		r := bufio.NewReader(conn)
		// A reply proves the daemon has registered the connection.
		ev := call(conn, r, &ipc.Request{Id: 1, Kind: &ipc.Request_LinkAccount{LinkAccount: &ipc.LinkAccount{AccountId: "nope"}}})
		if ev.GetError() == nil {
			t.Fatalf("linking an unknown account: %v", ev)
		}
		readers = append(readers, r)
	}

	d.broadcast(&ipc.Event{Kind: &ipc.Event_LinkFailed{LinkFailed: &ipc.LinkFailed{AccountId: "a"}}})
	for i, r := range readers {
		var ev ipc.Event
		if err := protodelim.UnmarshalFrom(r, &ev); err != nil || ev.GetLinkFailed() == nil || ev.GetRequestId() != 0 {
			t.Fatalf("client %d got %v, %v", i, &ev, err)
		}
	}
}

func TestActiveChatPerConnection(t *testing.T) {
	d := newDaemon(nil, nil)
	a, b := net.Pipe()
	defer a.Close()
	defer b.Close()
	set := func(conn net.Conn, chat string) {
		ev := d.reply(conn, &ipc.Request{Id: 1, Kind: &ipc.Request_SetActiveChat{SetActiveChat: &ipc.SetActiveChat{AccountId: "acc", ChatJid: chat}}})
		if ev.GetAck() == nil {
			t.Fatalf("set active chat: %v", ev)
		}
	}
	set(a, "x@g.us")
	set(b, "y@g.us")
	if !d.isActive("acc/x@g.us") || !d.isActive("acc/y@g.us") || d.isActive("acc/z@g.us") {
		t.Fatal("active chats not tracked per connection")
	}
	set(a, "")
	if d.isActive("acc/x@g.us") {
		t.Fatal("clearing did not stop suppressing the chat")
	}
}

func TestViewOnceNoticeBecomesMessage(t *testing.T) {
	ctx := t.Context()
	accounts, err := openAccounts(ctx, t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer closeAccounts(accounts)
	a := accounts[0]
	d := newDaemon(accounts, nil)
	defer d.wait() // Before the accounts close.
	chat := types.NewJID("120363000000000001", types.GroupServer)
	d.onMessage(a, &events.Message{
		Info: types.MessageInfo{
			MessageSource: types.MessageSource{Chat: chat, Sender: types.NewJID("1", types.HiddenUserServer), IsGroup: true},
			ID:            "V1",
			Timestamp:     time.Unix(1_700_000_000, 0),
		},
		Message: &waE2E.Message{Conversation: proto.String(viewOncePlaceholder())},
	})
	msgs, err := a.listMessages(ctx, chat.String(), "", "", 0)
	if err != nil || len(msgs) != 1 || msgs[0].Text != viewOncePlaceholder() {
		t.Fatalf("messages %v, %v", msgs, err)
	}
}

func TestCheckPeer(t *testing.T) {
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	ln, err := listen()
	if err != nil {
		t.Fatal(err)
	}
	defer ln.Close()
	path, _ := socketPath()
	go func() {
		if c, err := net.Dial("unix", path); err == nil {
			defer c.Close()
			time.Sleep(100 * time.Millisecond)
		}
	}()
	conn, err := ln.Accept()
	if err != nil {
		t.Fatal(err)
	}
	defer conn.Close()
	if err := checkPeer(conn); err != nil {
		t.Fatalf("own process refused: %v", err)
	}
	a, b := net.Pipe()
	defer a.Close()
	defer b.Close()
	if checkPeer(a) == nil {
		t.Fatal("accepted a connection that is not a Unix socket")
	}
}
