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

	"github.com/MawCeron/Wassup/daemon/ipc"
	"google.golang.org/protobuf/encoding/protodelim"
)

func main() {
	debug := flag.Bool("debug", false, "log every request")
	flag.Parse()
	level := slog.LevelInfo
	if *debug {
		level = slog.LevelDebug
	}
	// stderr is collected by journald when run as a systemd unit.
	slog.SetDefault(slog.New(slog.NewTextHandler(os.Stderr, &slog.HandlerOptions{Level: level})))

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
	slog.Info("started", "socket", ln.Addr(), "data", data, "cache", cache, "accounts", len(accounts))
	serve(ln, accounts)
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

func serve(ln net.Listener, accounts []*account) {
	for {
		conn, err := ln.Accept()
		if err != nil {
			if !errors.Is(err, net.ErrClosed) {
				slog.Error("accept", "err", err)
			}
			return
		}
		go handle(conn, accounts)
	}
}

func handle(conn net.Conn, accounts []*account) {
	defer conn.Close()
	r := bufio.NewReader(conn)
	for {
		var req ipc.Request
		if err := protodelim.UnmarshalFrom(r, &req); err != nil {
			if !errors.Is(err, io.EOF) {
				slog.Warn("read request", "err", err)
			}
			return
		}
		slog.Debug("request", "id", req.GetId(), "kind", fmt.Sprintf("%T", req.Kind))
		if _, err := protodelim.MarshalTo(conn, reply(&req, accounts)); err != nil {
			slog.Warn("write event", "err", err)
			return
		}
	}
}

func reply(req *ipc.Request, accounts []*account) *ipc.Event {
	ev := &ipc.Event{RequestId: req.GetId()}
	switch req.Kind.(type) {
	case *ipc.Request_ListAccounts:
		list := &ipc.AccountList{}
		for _, a := range accounts {
			list.Accounts = append(list.Accounts, a.info())
		}
		ev.Kind = &ipc.Event_Accounts{Accounts: list}
	default:
		ev.Kind = &ipc.Event_Error{Error: &ipc.Error{Message: "not implemented"}}
	}
	return ev
}
