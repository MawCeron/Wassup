package main

import (
	"bufio"
	"net"
	"os"
	"testing"

	"github.com/MawCeron/Wassup/daemon/ipc"
	"google.golang.org/protobuf/encoding/protodelim"
)

func TestRoundTrip(t *testing.T) {
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	ln, err := listen()
	if err != nil {
		t.Fatal(err)
	}
	defer ln.Close()
	go serve(ln)

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
