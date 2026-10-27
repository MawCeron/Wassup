package main

import (
	"context"
	"regexp"

	"go.mau.fi/whatsmeow/types"

	"github.com/MawCeron/Wassup/daemon/ipc"
)

// mentionToken matches how WhatsApp writes a mention in a text: "@" and the
// user part of the person's id, a phone number or a LID. The "@" starts a
// word, so the "@" of an address is not one.
var mentionToken = regexp.MustCompile(`(^|[^\p{L}\p{N}_])@(\d{5,25})\b`)

// mentionName is the name to show for the number after an "@": the account's
// own name for a mention of the user, otherwise the best known name of that
// phone number or LID. It is "" when nobody of that number is known, which
// leaves the text alone: a number is not taken for a mention without a name.
func (a *account) mentionName(ctx context.Context, token string, cache map[string]string) string {
	if name, ok := cache[token]; ok {
		return name
	}
	store := a.client.Store
	var name string
	if (store.ID != nil && store.ID.User == token) || (store.LID.User != "" && store.LID.User == token) {
		name = store.PushName
	} else {
		for _, server := range []string{types.DefaultUserServer, types.HiddenUserServer} {
			if name = a.contactName(ctx, types.NewJID(token, server)); name != "" {
				break
			}
		}
	}
	cache[token] = name
	return name
}

// mentionsIn lists the mentions in a text whose person is known. cache keeps
// the names already looked up, for a caller that does many texts.
func (a *account) mentionsIn(ctx context.Context, text string, cache map[string]string) []*ipc.Mention {
	var mentions []*ipc.Mention
	seen := map[string]bool{}
	for _, m := range mentionToken.FindAllStringSubmatch(text, -1) {
		token := m[2]
		if seen[token] {
			continue
		}
		seen[token] = true
		if name := a.mentionName(ctx, token, cache); name != "" {
			mentions = append(mentions, &ipc.Mention{Token: token, Name: name})
		}
	}
	return mentions
}

// withMentionNames puts "@name" where a text has the "@number" of a known
// mention, for the places that show plain text.
func withMentionNames(text string, mentions []*ipc.Mention) string {
	if len(mentions) == 0 {
		return text
	}
	names := make(map[string]string, len(mentions))
	for _, m := range mentions {
		names[m.GetToken()] = m.GetName()
	}
	return mentionToken.ReplaceAllStringFunc(text, func(match string) string {
		m := mentionToken.FindStringSubmatch(match)
		if name, ok := names[m[2]]; ok {
			return m[1] + "@" + name
		}
		return match
	})
}

// namedMentions is withMentionNames for a text whose mentions are not known yet.
func (a *account) namedMentions(ctx context.Context, text string) string {
	if !mentionToken.MatchString(text) {
		return text
	}
	return withMentionNames(text, a.mentionsIn(ctx, text, map[string]string{}))
}
