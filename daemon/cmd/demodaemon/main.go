// Command demodaemon answers the UI with an invented account, for screenshots:
// nothing here is real, and nothing reaches WhatsApp.
//
//	XDG_RUNTIME_DIR=... go run ./cmd/demodaemon -media <dir with summit.jpg and link.jpg>
package main

import (
	"bufio"
	"flag"
	"log"
	"net"
	"os"
	"path/filepath"
	"time"

	"google.golang.org/protobuf/encoding/protodelim"

	"github.com/MawCeron/Wassup/daemon/ipc"
)

const (
	account = "demo"
	me      = "15550100000@s.whatsapp.net"
	ana     = "15550100101@s.whatsapp.net"
	luis    = "15550100102@s.whatsapp.net"
	marta   = "15550100103@s.whatsapp.net"
	hikers  = "120363000000000010@g.us"
)

var media string

func main() {
	flag.StringVar(&media, "media", ".", "directory with the pictures")
	flag.Parse()
	path := filepath.Join(os.Getenv("XDG_RUNTIME_DIR"), "wassup.sock")
	os.Remove(path)
	l, err := net.Listen("unix", path)
	if err != nil {
		log.Fatal(err)
	}
	for {
		conn, err := l.Accept()
		if err != nil {
			log.Fatal(err)
		}
		go serve(conn)
	}
}

func serve(conn net.Conn) {
	defer conn.Close()
	r := bufio.NewReader(conn)
	for {
		var req ipc.Request
		if err := protodelim.UnmarshalFrom(r, &req); err != nil {
			return
		}
		ev := &ipc.Event{RequestId: req.GetId(), Kind: &ipc.Event_Ack{Ack: &ipc.Ack{}}}
		switch req.Kind.(type) {
		case *ipc.Request_ListAccounts:
			ev.Kind = &ipc.Event_Accounts{Accounts: &ipc.AccountList{Accounts: []*ipc.Account{{
				Id: account, Jid: me, PushName: "Sam", State: ipc.ConnectionState_CONNECTION_STATE_CONNECTED,
			}}}}
		case *ipc.Request_GetSettings:
			ev.Kind = &ipc.Event_Settings{Settings: &ipc.Settings{MediaCacheMb: 500, LinkPreviews: true}}
		case *ipc.Request_ListChats:
			ev.Kind = &ipc.Event_Chats{Chats: &ipc.ChatList{Chats: chats()}}
		case *ipc.Request_ListMessages:
			var list []*ipc.Message
			if req.GetListMessages().GetChatJid() == hikers && req.GetListMessages().GetBeforeId() == "" {
				list = hikerMessages()
			}
			ev.Kind = &ipc.Event_Messages{Messages: &ipc.MessageList{Messages: list}}
		case *ipc.Request_ListStickers:
			ev.Kind = &ipc.Event_Stickers{Stickers: &ipc.StickerList{}}
		case *ipc.Request_SearchMessages:
			ev.Kind = &ipc.Event_Messages{Messages: &ipc.MessageList{}}
		}
		if _, err := protodelim.MarshalTo(conn, ev); err != nil {
			return
		}
	}
}

// at is a time of the day today, "15:04", so the scene looks the same whenever it is run.
func at(clock string) int64 {
	t, err := time.ParseInLocation("15:04", clock, time.Local)
	if err != nil {
		log.Fatal(err)
	}
	now := time.Now()
	return time.Date(now.Year(), now.Month(), now.Day(), t.Hour(), t.Minute(), 0, 0, time.Local).UnixMilli()
}

func chats() []*ipc.Chat {
	read := ipc.MessageStatus_MESSAGE_STATUS_READ
	return []*ipc.Chat{
		{AccountId: account, Jid: hikers, Name: "Weekend hikers", IsGroup: true, LastMessageAtMs: at("10:20"),
			LastMessage: "Perfect, I'll bring the snacks 🥪", LastSenderName: "Luis Ortega"},
		{AccountId: account, Jid: ana, Name: "Ana Torres", UnreadCount: 2, LastMessageAtMs: at("10:24"),
			LastMessage: "Did you see the photos from last month?"},
		{AccountId: account, Jid: "120363000000000011@g.us", Name: "Family", IsGroup: true, UnreadCount: 5, LastMessageAtMs: at("10:02"),
			LastMessage: "Dinner at my place on Sunday?", LastSenderName: "Marta Ruiz"},
		{AccountId: account, Jid: luis, Name: "Luis Ortega", LastMessageAtMs: at("09:30"),
			LastMessage: "Sounds good, see you at 8", LastFromMe: true, LastStatus: read},
		{AccountId: account, Jid: "120363000000000012@g.us", Name: "Book club", IsGroup: true, LastMessageAtMs: at("08:47"),
			LastMessage: "Next one is The Overstory 📚", LastSenderName: "Camila Reyes"},
		{AccountId: account, Jid: "15550100104@s.whatsapp.net", Name: "Camila Reyes", LastMessageAtMs: at("08:15"),
			LastMessage: "👍"},
		{AccountId: account, Jid: "15550100105@s.whatsapp.net", Name: "Dr. Silva's office", LastMessageAtMs: at("08:02"),
			LastMessage: "Your appointment is confirmed for Thursday at 10:30"},
	}
}

// hikerMessages are the open chat's, newest first.
func hikerMessages() []*ipc.Message {
	read := ipc.MessageStatus_MESSAGE_STATUS_READ
	m := func(id, sender, clock, text string) *ipc.Message {
		msg := &ipc.Message{AccountId: account, ChatJid: hikers, Id: id, SenderJid: sender, FromMe: sender == me,
			TimestampMs: at(clock), Text: text}
		switch sender {
		case ana:
			msg.SenderName = "Ana Torres"
		case luis:
			msg.SenderName = "Luis Ortega"
		case marta:
			msg.SenderName = "Marta Ruiz"
		default:
			msg.Status = read
		}
		return msg
	}
	driving := m("m3", me, "09:18", "I can take four people. Leaving at 6:30 from the plaza.")
	driving.Reactions = []*ipc.Reaction{{Emoji: "👍", Count: 3}}
	reply := m("m4", ana, "09:25", "Count me in! Should I bring coffee?")
	reply.Quote = &ipc.Quote{MessageId: "m3", SenderJid: me, FromMe: true, Text: driving.Text}
	photo := m("m5", marta, "09:40", "Last month from the summit 😍")
	photo.Media = &ipc.Media{Kind: ipc.MediaKind_MEDIA_KIND_IMAGE, Path: filepath.Join(media, "summit.jpg"), Width: 1200, Height: 800}
	photo.Reactions = []*ipc.Reaction{{Emoji: "❤️", Count: 4}}
	mention := m("m6", me, "09:52", "Nice! @15550100101 could you bring the trail map?")
	mention.Mentions = []*ipc.Mention{{Token: "15550100101", Name: "Ana Torres"}}
	forecast := m("m7", ana, "09:55", "On it. Here's the forecast for the weekend: https://example.org/forecast")
	forecast.Link = &ipc.LinkPreview{Url: "https://example.org/forecast", Title: "Weekend forecast: clear skies",
		Description: "Sunny with a light wind, 18°C at the summit.", ThumbnailPath: filepath.Join(media, "link.jpg")}
	snacks := m("m8", luis, "10:20", "Perfect, I'll bring the snacks 🥪")
	snacks.Quote = &ipc.Quote{MessageId: "m6", SenderJid: me, FromMe: true, Text: "Nice! @Ana Torres could you bring the trail map?"}
	return []*ipc.Message{
		snacks,
		forecast,
		mention,
		photo,
		reply,
		driving,
		m("m2", marta, "09:14", "Great news. Who's driving on Saturday?"),
		m("m1", luis, "09:12", "The trail is dry after yesterday's rain, and the creek crossing is easy again."),
	}
}
