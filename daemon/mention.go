package main

import (
	"context"
	"errors"
	"os"
	"regexp"
	"sort"
	"strings"
	"sync"
	"time"

	"go.mau.fi/whatsmeow/proto/waE2E"
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

// mentionsUser reports whether a message mentions any of these users, given
// as the user part of an id: the ids a message lists as mentioned may be
// phone numbers or LIDs.
func mentionsUser(m *waE2E.Message, users ...string) bool {
	for _, c := range contextsOf(m) {
		for _, id := range c.GetMentionedJID() {
			jid, err := types.ParseJID(id)
			if err != nil || jid.User == "" {
				continue
			}
			for _, user := range users {
				if user != "" && jid.User == user {
					return true
				}
			}
		}
	}
	return false
}

// mentionsMe reports whether a message mentions the account's user.
func (a *account) mentionsMe(m *waE2E.Message) bool {
	store := a.client.Store
	var phone string
	if store.ID != nil {
		phone = store.ID.User
	}
	return mentionsUser(m, phone, store.LID.User)
}

// mentionedIn keeps the ids from a request that the text really mentions: an
// id whose user part is not written as "@user" in the text would tag a
// person the message does not show.
func mentionedIn(text string, ids []string) []string {
	var kept []string
	for _, id := range ids {
		jid, err := types.ParseJID(id)
		if err != nil || jid.User == "" {
			continue
		}
		for _, m := range mentionToken.FindAllStringSubmatch(text, -1) {
			if m[2] == jid.User {
				kept = append(kept, jid.ToNonAD().String())
				break
			}
		}
	}
	return kept
}

// participantList names the members of a group, without the user, in
// alphabetical order. name gives the best known name of a member, or "", and
// phone the phone number of an id that is not one, or "", and
// avatar the cached picture of the first of some ids that has one, or "".
func participantList(members []types.GroupParticipant, self []string, name func(types.JID) string, phone func(types.JID) string, avatar func(...types.JID) string) []*ipc.Participant {
	var people []*ipc.Participant
	for _, m := range members {
		jid := m.JID.ToNonAD()
		if jid.User == "" || slicesContain(self, jid.User) {
			continue
		}
		shown := name(jid)
		for _, other := range []types.JID{m.PhoneNumber, m.LID} {
			if shown == "" && !other.IsEmpty() {
				shown = name(other.ToNonAD())
			}
		}
		if shown == "" {
			shown = m.DisplayName
		}
		if shown == "" {
			// No name at all: the phone number, as WhatsApp shows it.
			if shown = m.PhoneNumber.User; shown == "" {
				shown = phone(jid)
			}
			if shown != "" {
				shown = "+" + shown
			}
		}
		if shown == "" {
			shown = jid.User
		}
		people = append(people, &ipc.Participant{Jid: jid.String(), Name: shown, AvatarPath: avatar(jid, m.PhoneNumber, m.LID)})
	}
	sort.Slice(people, func(i, j int) bool {
		return strings.ToLower(people[i].Name) < strings.ToLower(people[j].Name)
	})
	return people
}

func slicesContain(list []string, s string) bool {
	for _, x := range list {
		if x != "" && x == s {
			return true
		}
	}
	return false
}

// participantsTTL is how long the members of a group are remembered, so that
// typing "@" does not ask the server every time.
const participantsTTL = 10 * time.Minute

type cachedParticipants struct {
	at     time.Time
	people []*ipc.Participant
}

var participantsCache sync.Map // "account/chat" -> cachedParticipants

// listParticipants lists the members of a group to mention.
func (a *account) listParticipants(ctx context.Context, chat string) ([]*ipc.Participant, error) {
	key := chatKey(a.id, chat)
	if c, ok := participantsCache.Load(key); ok && time.Since(c.(cachedParticipants).at) < participantsTTL {
		return c.(cachedParticipants).people, nil
	}
	jid, err := types.ParseJID(chat)
	if err != nil || jid.Server != types.GroupServer {
		return nil, errors.New("not a group")
	}
	info, err := a.client.GetGroupInfo(ctx, jid)
	if err != nil {
		return nil, err
	}
	store := a.client.Store
	var self []string
	if store.ID != nil {
		self = append(self, store.ID.User)
	}
	self = append(self, store.LID.User)
	people := participantList(info.Participants, self, func(j types.JID) string { return a.contactName(ctx, j) },
		func(j types.JID) string { return a.phoneOf(ctx, j) },
		func(ids ...types.JID) string { return a.cachedAvatar(ctx, ids...) })
	participantsCache.Store(key, cachedParticipants{at: time.Now(), people: people})
	return people, nil
}

// cachedAvatar is the cached picture of the chat of the first of these ids
// that has one: a member's picture is only kept when they have a chat here.
func (a *account) cachedAvatar(ctx context.Context, ids ...types.JID) string {
	for _, id := range ids {
		if id.IsEmpty() {
			continue
		}
		var avatarID string
		if a.db.QueryRowContext(ctx, `SELECT avatar_id FROM wassup_chats WHERE jid = ?`, id.ToNonAD().String()).Scan(&avatarID) != nil {
			continue
		}
		if path := a.avatarPath(id.ToNonAD().String(), avatarID); path != "" {
			if _, err := os.Stat(path); err == nil {
				return path
			}
		}
	}
	return ""
}

// phoneOf is the phone number behind a LID, or "" when it is not known.
func (a *account) phoneOf(ctx context.Context, id types.JID) string {
	if id.Server == types.DefaultUserServer {
		return id.User
	}
	if lids := a.client.Store.LIDs; lids != nil {
		if pn, err := lids.GetPNForLID(ctx, id); err == nil {
			return pn.User
		}
	}
	return ""
}
