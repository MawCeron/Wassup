// Command wassupctl is a test client that talks to the daemon over IPC.
package main

import (
	"bufio"
	"errors"
	"fmt"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"github.com/MawCeron/Wassup/daemon/ipc"
	"google.golang.org/protobuf/encoding/protodelim"
)

const usage = `usage:
  wassupctl accounts
  wassupctl link <account>
  wassupctl chats <account>
  wassupctl messages <account> <chat> [limit]
  wassupctl send <account> <chat> <text...>
  wassupctl watch

<chat> is a JID, or a phone number with country code for a contact.`

func main() {
	if err := run(os.Args[1:]); err != nil {
		fmt.Fprintln(os.Stderr, "wassupctl:", err)
		os.Exit(1)
	}
}

type client struct {
	conn net.Conn
	r    *bufio.Reader
	id   uint64
}

func (c *client) next() (*ipc.Event, error) {
	var ev ipc.Event
	return &ev, protodelim.UnmarshalFrom(c.r, &ev)
}

// call sends a request and returns its reply, skipping unrelated events.
func (c *client) call(req *ipc.Request) (*ipc.Event, error) {
	c.id++
	req.Id = c.id
	if _, err := protodelim.MarshalTo(c.conn, req); err != nil {
		return nil, err
	}
	for {
		ev, err := c.next()
		if err != nil {
			return nil, err
		}
		if ev.GetRequestId() == req.Id {
			if e := ev.GetError(); e != nil {
				return nil, errors.New(e.GetMessage())
			}
			return ev, nil
		}
	}
}

func run(args []string) error {
	if len(args) == 0 {
		return errors.New(usage)
	}
	dir := os.Getenv("XDG_RUNTIME_DIR")
	if dir == "" {
		return errors.New("XDG_RUNTIME_DIR is not set")
	}
	conn, err := net.Dial("unix", filepath.Join(dir, "wassup.sock"))
	if err != nil {
		return err
	}
	defer conn.Close()
	c := &client{conn: conn, r: bufio.NewReader(conn)}

	cmd, args := args[0], args[1:]
	need := map[string]int{"accounts": 0, "link": 1, "chats": 1, "messages": 2, "send": 3, "watch": 0}
	if n, ok := need[cmd]; !ok || len(args) < n {
		return errors.New(usage)
	}
	switch cmd {
	case "accounts":
		ev, err := c.call(&ipc.Request{Kind: &ipc.Request_ListAccounts{ListAccounts: &ipc.ListAccounts{}}})
		if err != nil {
			return err
		}
		for _, a := range ev.GetAccounts().GetAccounts() {
			fmt.Printf("%s  %s  %s  %s\n", a.GetId(), a.GetState(), a.GetJid(), a.GetPushName())
		}
	case "link":
		return link(c, args[0])
	case "chats":
		ev, err := c.call(&ipc.Request{Kind: &ipc.Request_ListChats{ListChats: &ipc.ListChats{AccountId: args[0]}}})
		if err != nil {
			return err
		}
		for _, ch := range ev.GetChats().GetChats() {
			fmt.Printf("%s  %-30s  unread=%d  %s\n", stamp(ch.GetLastMessageAtMs()), ch.GetJid(), ch.GetUnreadCount(), ch.GetName())
		}
	case "messages":
		var limit uint64
		if len(args) > 2 {
			if limit, err = strconv.ParseUint(args[2], 10, 32); err != nil {
				return err
			}
		}
		ev, err := c.call(&ipc.Request{Kind: &ipc.Request_ListMessages{ListMessages: &ipc.ListMessages{
			AccountId: args[0], ChatJid: chatJID(args[1]), Limit: uint32(limit),
		}}})
		if err != nil {
			return err
		}
		msgs := ev.GetMessages().GetMessages()
		for i := len(msgs) - 1; i >= 0; i-- { // Oldest first, like a chat.
			printMessage(msgs[i])
		}
	case "send":
		ev, err := c.call(&ipc.Request{Kind: &ipc.Request_SendText{SendText: &ipc.SendText{
			AccountId: args[0], ChatJid: chatJID(args[1]), Text: strings.Join(args[2:], " "),
		}}})
		if err != nil {
			return err
		}
		printMessage(ev.GetMessageReceived())
	case "watch":
		for {
			ev, err := c.next()
			if err != nil {
				return err
			}
			switch {
			case ev.GetMessageReceived() != nil:
				printMessage(ev.GetMessageReceived())
			case ev.GetAccountChanged() != nil:
				a := ev.GetAccountChanged()
				fmt.Printf("account %s is %s\n", a.GetId(), a.GetState())
			}
		}
	}
	return nil
}

func link(c *client, account string) error {
	if _, err := c.call(&ipc.Request{Kind: &ipc.Request_LinkAccount{LinkAccount: &ipc.LinkAccount{AccountId: account}}}); err != nil {
		return err
	}
	fmt.Println("Scan with WhatsApp > Linked devices > Link a device.")
	for {
		ev, err := c.next()
		if err != nil {
			return err
		}
		switch {
		case ev.GetQrCode().GetAccountId() == account:
			showQR(ev.GetQrCode().GetCode())
		case ev.GetLinkFailed().GetAccountId() == account:
			return errors.New("linking failed: " + ev.GetLinkFailed().GetReason())
		case ev.GetAccountChanged().GetId() == account &&
			ev.GetAccountChanged().GetState() == ipc.ConnectionState_CONNECTION_STATE_CONNECTED:
			fmt.Println("linked as", ev.GetAccountChanged().GetJid())
			return nil
		}
	}
}

// showQR draws the code with qrencode when it is installed.
func showQR(code string) {
	cmd := exec.Command("qrencode", "-t", "ansiutf8", code)
	cmd.Stdout = os.Stdout
	if err := cmd.Run(); err != nil {
		fmt.Println("Install qrencode to see the QR here, or render this text as one:")
		fmt.Println(code)
	}
}

// chatJID accepts a bare phone number for one-to-one chats.
func chatJID(s string) string {
	if strings.Contains(s, "@") {
		return s
	}
	return strings.TrimPrefix(s, "+") + "@s.whatsapp.net"
}

func stamp(ms int64) string {
	return time.UnixMilli(ms).Format("2006-01-02 15:04")
}

func printMessage(m *ipc.Message) {
	who := m.GetSenderJid()
	if m.GetFromMe() {
		who = "me"
	}
	fmt.Printf("%s  [%s] %s: %s\n", stamp(m.GetTimestampMs()), m.GetChatJid(), who, m.GetText())
}
