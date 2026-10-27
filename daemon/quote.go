package main

import (
	"context"

	"github.com/MawCeron/Wassup/daemon/ipc"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
)

// maxQuoteRunes keeps stored quotes short; the UI shows two lines at most.
const maxQuoteRunes = 200

// contextsOf lists the context of whichever parts a message has; most
// messages have one part, so at most one is non-nil.
func contextsOf(m *waE2E.Message) []*waE2E.ContextInfo {
	return []*waE2E.ContextInfo{
		m.GetExtendedTextMessage().GetContextInfo(),
		m.GetImageMessage().GetContextInfo(),
		m.GetVideoMessage().GetContextInfo(),
		m.GetAudioMessage().GetContextInfo(),
		m.GetDocumentMessage().GetContextInfo(),
		m.GetStickerMessage().GetContextInfo(),
		m.GetContactMessage().GetContextInfo(),
		m.GetLocationMessage().GetContextInfo(),
	}
}

// contextOf is the reply information of whichever part a message has.
func contextOf(m *waE2E.Message) *waE2E.ContextInfo {
	for _, c := range contextsOf(m) {
		if c.GetStanzaID() != "" {
			return c
		}
	}
	return nil
}

// quoteOf returns what a reply quotes, without the sender's name, which is
// looked up when read, or nil for messages that are not replies.
func quoteOf(m *waE2E.Message) *ipc.Quote {
	c := contextOf(m)
	if c == nil {
		return nil
	}
	text := []rune(quotedText(c.GetQuotedMessage()))
	if len(text) > maxQuoteRunes {
		text = append(text[:maxQuoteRunes-1], '…')
	}
	return &ipc.Quote{
		MessageId: c.GetStanzaID(), SenderJid: c.GetParticipant(), Text: string(text),
		Thumbnail: quotedThumbnail(c.GetQuotedMessage()),
	}
}

// quotedThumbnail is the small image a quoted photo, video, sticker or
// document carries, if any.
func quotedThumbnail(m *waE2E.Message) []byte {
	for _, thumb := range [][]byte{
		m.GetImageMessage().GetJPEGThumbnail(),
		m.GetVideoMessage().GetJPEGThumbnail(),
		m.GetStickerMessage().GetPngThumbnail(),
		m.GetDocumentMessage().GetJPEGThumbnail(),
	} {
		if len(thumb) > 0 {
			return thumb
		}
	}
	return nil
}

// showQuoteThumbnail caches a quote's image and swaps its bytes for the
// file's path, which is what the UI gets.
func (a *account) showQuoteThumbnail(q *ipc.Quote) {
	q.ThumbnailPath = a.cacheThumbnail(q.Thumbnail)
	q.Thumbnail = nil
}

// quotedText is a quoted message's text or caption, or what kind of
// message it was.
func quotedText(m *waE2E.Message) string {
	if t := m.GetConversation(); t != "" {
		return t
	}
	if t := m.GetExtendedTextMessage().GetText(); t != "" {
		return t
	}
	switch {
	case m.GetImageMessage() != nil:
		return orLabel(m.GetImageMessage().GetCaption(), tr("Photo"))
	case m.GetVideoMessage() != nil:
		return orLabel(m.GetVideoMessage().GetCaption(), tr("Video"))
	case m.GetStickerMessage() != nil:
		return tr("Sticker")
	case m.GetAudioMessage() != nil:
		if m.GetAudioMessage().GetPTT() {
			return tr("Voice message")
		}
		return tr("Audio")
	case m.GetDocumentMessage() != nil:
		d := m.GetDocumentMessage()
		return orLabel(d.GetCaption(), orLabel(d.GetFileName(), tr("Document")))
	case m.GetContactMessage() != nil:
		return orLabel(m.GetContactMessage().GetDisplayName(), tr("Contact"))
	case m.GetLocationMessage() != nil, m.GetLiveLocationMessage() != nil:
		return tr("Location")
	}
	return tr("Message")
}

func orLabel(text, label string) string {
	if text != "" {
		return text
	}
	return label
}

// quoteFromBlob reads a stored quote and fills in who wrote it.
func (a *account) quoteFromBlob(ctx context.Context, blob []byte) *ipc.Quote {
	if blob == nil {
		return nil
	}
	var q ipc.Quote
	if proto.Unmarshal(blob, &q) != nil {
		return nil
	}
	a.nameQuote(ctx, &q)
	a.showQuoteThumbnail(&q)
	return &q
}

func (a *account) nameQuote(ctx context.Context, q *ipc.Quote) {
	sender, err := types.ParseJID(q.SenderJid)
	if err != nil {
		return
	}
	store := a.client.Store
	own := func(j *types.JID) bool { return j != nil && !j.IsEmpty() && j.User == sender.User }
	if own(store.ID) || own(&store.LID) {
		q.FromMe = true
		return
	}
	q.SenderName = a.contactName(ctx, sender)
}
