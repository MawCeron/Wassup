package main

import (
	"errors"
	"fmt"
	"net"
	"os"
	"syscall"
)

// checkPeer accepts a connection only from a process of the same user, as
// the kernel reports it (SO_PEERCRED). The socket is already private to
// the user; this also covers a socket handed over by systemd, whose
// permissions the daemon does not set. A process of the same user is
// trusted: it could read the account's files anyway.
func checkPeer(conn net.Conn) error {
	unix, ok := conn.(*net.UnixConn)
	if !ok {
		return errors.New("not a Unix socket")
	}
	raw, err := unix.SyscallConn()
	if err != nil {
		return err
	}
	var cred *syscall.Ucred
	var credErr error
	if err := raw.Control(func(fd uintptr) {
		cred, credErr = syscall.GetsockoptUcred(int(fd), syscall.SOL_SOCKET, syscall.SO_PEERCRED)
	}); err != nil {
		return err
	}
	if credErr != nil {
		return credErr
	}
	if int(cred.Uid) != os.Getuid() {
		return fmt.Errorf("connection from user %d, pid %d", cred.Uid, cred.Pid)
	}
	return nil
}
