package main

import (
	"context"
	"testing"

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
