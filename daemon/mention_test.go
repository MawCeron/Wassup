package main

import (
	"context"
	"testing"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"google.golang.org/protobuf/proto"

	"github.com/MawCeron/Wassup/daemon/ipc"
)

func TestMentionNamesReplaceOnlyKnownMentions(t *testing.T) {
	mentions := []*ipc.Mention{{Token: "5215551111111", Name: "Ana"}, {Token: "98765432109876", Name: "Luis"}}
	for text, want := range map[string]string{
		"hola @5215551111111":                      "hola @Ana",
		"@5215551111111, y @98765432109876!":       "@Ana, y @Luis!",
		"@5215559999999 no se conoce":              "@5215559999999 no se conoce",
		"correo a@5215551111111.com no es mención": "correo a@5215551111111.com no es mención",
		"@123 es corto":                            "@123 es corto",
		"sin menciones":                            "sin menciones",
	} {
		if got := withMentionNames(text, mentions); got != want {
			t.Errorf("withMentionNames(%q) = %q, want %q", text, got, want)
		}
	}
	if got := withMentionNames("@5215551111111", nil); got != "@5215551111111" {
		t.Errorf("without mentions the text changed: %q", got)
	}
}

func TestMentionsOfUnknownPeopleAreLeftOut(t *testing.T) {
	ctx := context.Background()
	accounts, err := openAccounts(ctx, t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer closeAccounts(accounts)
	a := accounts[0]
	// An account that is not linked knows nobody.
	if got := a.mentionsIn(ctx, "hola @5215551111111 @5215551111111", map[string]string{}); len(got) != 0 {
		t.Errorf("mentions of unknown numbers: %v", got)
	}
	if got := a.namedMentions(ctx, "hola @5215551111111"); got != "hola @5215551111111" {
		t.Errorf("an unknown number changed: %q", got)
	}
}

func TestMentionsUser(t *testing.T) {
	mentioning := func(ids ...string) *waE2E.Message {
		return &waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{
			Text: proto.String("hola"), ContextInfo: &waE2E.ContextInfo{MentionedJID: ids},
		}}
	}
	phone, lid := "5215551111111", "98765432109876"
	for name, tc := range map[string]struct {
		message *waE2E.Message
		want    bool
	}{
		"by phone number": {mentioning("5215551111111@s.whatsapp.net"), true},
		"by LID":          {mentioning("5215550000000@s.whatsapp.net", "98765432109876@lid"), true},
		"somebody else":   {mentioning("5215550000000@s.whatsapp.net"), false},
		"nobody":          {mentioning(), false},
		"no context":      {&waE2E.Message{Conversation: proto.String("hola")}, false},
		"a bad id":        {mentioning("not an id"), false},
	} {
		if got := mentionsUser(tc.message, phone, lid); got != tc.want {
			t.Errorf("%s: mentionsUser = %v, want %v", name, got, tc.want)
		}
	}
	if mentionsUser(mentioning("@s.whatsapp.net"), "") {
		t.Error("an empty user matched")
	}
}
