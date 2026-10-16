package main

import (
	"bufio"
	"errors"
	"fmt"
	"io"
	"log"
	"net"
	"os"
	"path/filepath"

	"github.com/MawCeron/Wassup/daemon/ipc"
	"google.golang.org/protobuf/encoding/protodelim"
)

func main() {
	ln, err := listen()
	if err != nil {
		log.Fatal(err)
	}
	log.Printf("listening on %s", ln.Addr())
	serve(ln)
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

func serve(ln net.Listener) {
	for {
		conn, err := ln.Accept()
		if err != nil {
			if !errors.Is(err, net.ErrClosed) {
				log.Print(err)
			}
			return
		}
		go handle(conn)
	}
}

func handle(conn net.Conn) {
	defer conn.Close()
	r := bufio.NewReader(conn)
	for {
		var req ipc.Request
		if err := protodelim.UnmarshalFrom(r, &req); err != nil {
			if !errors.Is(err, io.EOF) {
				log.Printf("read request: %v", err)
			}
			return
		}
		if _, err := protodelim.MarshalTo(conn, reply(&req)); err != nil {
			log.Printf("write event: %v", err)
			return
		}
	}
}

func reply(req *ipc.Request) *ipc.Event {
	ev := &ipc.Event{RequestId: req.GetId()}
	switch req.Kind.(type) {
	case *ipc.Request_ListAccounts:
		ev.Kind = &ipc.Event_Accounts{Accounts: &ipc.AccountList{}}
	default:
		ev.Kind = &ipc.Event_Error{Error: &ipc.Error{Message: "not implemented"}}
	}
	return ev
}
